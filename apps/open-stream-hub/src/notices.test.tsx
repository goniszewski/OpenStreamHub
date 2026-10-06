import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { FakeStorage } from './fake-storage';
import { idleFeedState } from './device-feed';
import type { FeedStoreState } from './device-feed';
import type { LiveDevice } from './contracts';
import { focusWhenRendered, NoticeList } from './NoticeRegion';
import { NoticeStore, coalesceMs, noticeFocusAfterDismiss, defaultTtl, maxVisible, noticesFromInventory, noticesFromRuns, noticesStorageKey } from './notices';
import type { NoticeEnv } from './notices';
import type { ServiceRun } from './service-runner';

const knob = (serial: string | null, path = `p-${serial}`): LiveDevice => ({ path_hex: path, module: 'm003k', serial_number: serial });
const fader = (serial: string): LiveDevice => ({ path_hex: `f-${serial}`, module: 'm005s', serial_number: serial });

function clock(storage: NoticeEnv['storage'] = new FakeStorage()) {
  let time = 1000;
  const timers = new Map<number, { at: number; callback: () => void }>();
  let id = 0;
  const env: NoticeEnv = {
    now: () => time, storage,
    setTimer: (callback, ms) => { timers.set(++id, { at: time + ms, callback }); return id; },
    clearTimer: handle => { timers.delete(handle as number); },
  };
  const advance = (ms: number) => {
    const target = time + ms;
    for (;;) {
      const due = [...timers.entries()].filter(([, timer]) => timer.at <= target).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      timers.delete(due[0]); time = Math.max(time, due[1].at); due[1].callback();
    }
    time = target;
  };
  return { env, advance, timers };
}

function setup(storage?: NoticeEnv['storage']) {
  const c = clock(storage);
  const store = new NoticeStore(c.env);
  let feed: FeedStoreState = idleFeedState;
  const snapshot = (devices: LiveDevice[]) => { feed = { ...feed, devices, inventoryLoaded: true }; store.onFeed(feed); };
  const texts = () => store.getState().notices.map(notice => notice.text);
  return { ...c, store, snapshot, texts };
}

describe('noticesFromInventory', () => {
  it('reports a connected module with no serial tail when the type is unique', () => {
    expect(noticesFromInventory([], [knob('AAAA1111')], 0).notices.map(n => n.text)).toEqual(['3-knob module connected']);
  });
  it('adds a serial tail only when several modules share a type', () => {
    const result = noticesFromInventory([knob('AAAA1111')], [knob('AAAA1111'), knob('BBBB2222')], 0);
    expect(result.notices.map(n => n.text)).toEqual(['3-knob module (…2222) connected']);
  });
  it('holds a removal, then reports disconnected once the window passes', () => {
    const first = noticesFromInventory([knob('A')], [], 0);
    expect(first.notices).toEqual([]);
    expect(first.state.pending).toHaveLength(1);
    const later = noticesFromInventory([], [fader('F')], coalesceMs + 1, first.state);
    expect(later.notices.map(n => n.text)).toEqual(['3-knob module disconnected', '5-fader precision module connected']);
  });
  it('turns remove plus add of the same module and serial into one reconnected notice', () => {
    const removed = noticesFromInventory([knob('A', 'old')], [], 0);
    const added = noticesFromInventory([], [knob('A', 'new')], 1500, removed.state);
    expect(added.notices.map(n => n.text)).toEqual(['3-knob module reconnected']);
    expect(added.state.pending).toEqual([]);
  });
  it('reports a different serial of the same type as replaced', () => {
    const result = noticesFromInventory([knob('A')], [knob('B')], 0);
    expect(result.notices.map(n => [n.kind, n.text])).toEqual([['replaced', '3-knob module replaced']]);
    expect(result.state.pending).toEqual([]);
  });
  it('does not mistake a different type for a swap', () => {
    const result = noticesFromInventory([knob('A')], [fader('F')], 0);
    expect(result.notices.map(n => n.kind)).toEqual(['connected']);
    expect(result.state.pending).toHaveLength(1);
  });
});

describe('NoticeStore inventory', () => {
  it('skips the first snapshot, including an empty one', () => {
    const t = setup();
    t.snapshot([knob('A')]);
    expect(t.texts()).toEqual([]);
    const empty = setup();
    empty.snapshot([]);
    empty.snapshot([knob('A')]);
    expect(empty.texts()).toEqual(['3-knob module connected']);
  });
  it('ignores feed states before the inventory is loaded', () => {
    const t = setup();
    t.store.onFeed({ ...idleFeedState, devices: [knob('A')] });
    t.snapshot([knob('A')]);
    expect(t.texts()).toEqual([]);
  });
  it('reports a disconnect after the coalescing window and coalesces a quick replug', () => {
    const t = setup();
    t.snapshot([knob('A')]);
    t.snapshot([]);
    expect(t.texts()).toEqual([]);
    t.advance(coalesceMs);
    expect(t.texts()).toEqual(['3-knob module disconnected']);
    t.snapshot([knob('A')]);
    t.snapshot([]);
    t.advance(1000);
    t.snapshot([knob('A', 'again')]);
    expect(t.texts()).toEqual(['3-knob module disconnected', '3-knob module connected', '3-knob module reconnected']);
    const later: string[] = [];
    t.store.subscribe(() => later.push(...t.texts()));
    t.advance(coalesceMs - 1);
    expect(later.filter(text => text.includes('disconnected'))).toHaveLength(0);
  });
  it('limits the visible notices to three, dropping the oldest', () => {
    const t = setup();
    t.snapshot([]);
    const devices: LiveDevice[] = [];
    for (let i = 0; i < 5; i++) { devices.push(fader(`S${i}000`)); t.snapshot([...devices]); }
    expect(t.store.getState().notices).toHaveLength(maxVisible);
    expect(t.texts()[0]).toContain('…2000');
    expect(t.texts().at(-1)).toContain('…4000');
  });
  it('auto-dismisses after the timeout, pauses while held and can be dismissed by hand', () => {
    const t = setup();
    t.snapshot([]); t.snapshot([knob('A')]);
    const [first] = t.store.getState().notices;
    t.advance(defaultTtl - 1);
    expect(t.texts()).toHaveLength(1);
    t.advance(1);
    expect(t.texts()).toEqual([]);
    t.snapshot([knob('A'), fader('F')]);
    const held = t.store.getState().notices[0];
    t.store.pause(held.id);
    t.advance(defaultTtl * 3);
    expect(t.texts()).toHaveLength(1);
    t.store.resume(held.id);
    t.advance(defaultTtl);
    expect(t.texts()).toEqual([]);
    t.snapshot([knob('A'), fader('F'), fader('G')]);
    t.store.dismiss(t.store.getState().notices[0].id);
    expect(t.texts()).toEqual([]);
    expect(t.timers.size).toBe(0);
    expect(first.id).toBeLessThan(held.id);
  });
});

describe('connection notices preference', () => {
  it('shows nothing while off, clears connection notices and persists the choice', () => {
    const storage = new FakeStorage();
    const t = setup(storage);
    t.snapshot([]); t.snapshot([knob('A')]);
    expect(t.texts()).toHaveLength(1);
    t.store.setEnabled(false);
    expect(t.texts()).toEqual([]);
    expect(storage.getItem(noticesStorageKey)).toBe('off');
    t.snapshot([]);
    t.advance(coalesceMs);
    expect(t.texts()).toEqual([]);
    expect(new NoticeStore({ ...t.env, storage }).getState().enabled).toBe(false);
    t.store.setEnabled(true);
    expect(storage.getItem(noticesStorageKey)).toBe('on');
  });
  it('keeps working when storage throws', () => {
    const broken = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
    const t = setup(broken);
    expect(t.store.getState().enabled).toBe(true);
    t.store.setEnabled(false);
    expect(t.store.getState().enabled).toBe(false);
    expect(setup(null).store.getState().enabled).toBe(true);
  });
  it('still shows session warnings while connection notices are off', () => {
    const t = setup();
    t.store.setEnabled(false);
    t.store.onRuns({ a: run('a', 'needs_confirmation') });
    expect(t.texts()).toEqual(['Actions paused — confirm to resume']);
  });
});

function run(key: string, phase: string, extra: Partial<ServiceRun['session'] & object> = {}, state: ServiceRun['state'] = 'running', error?: string): ServiceRun {
  return {
    serviceId: key, state, execute: true, entries: [], counters: { events: 0, planned: 0, executed: 0, failed: 0 }, startedAt: 0, profileName: 'P', snapshot: '', draftUnsaved: false, ...(error ? { error } : {}),
    session: phase === 'none' ? undefined : { serviceId: key, phase: phase as never, reason: null, attempt: 0, module: 'm003k', serialNumber: null, ...extra },
  };
}

describe('session notices', () => {
  it('keeps the paused notice with a Review action until the phase changes', () => {
    const t = setup();
    t.store.onRuns({ a: run('a', 'needs_confirmation') });
    const [notice] = t.store.getState().notices;
    expect(notice).toMatchObject({ kind: 'paused', text: 'Actions paused — confirm to resume', ttl: null, action: 'review' });
    t.advance(defaultTtl * 10);
    expect(t.texts()).toHaveLength(1);
    t.store.onRuns({ a: run('a', 'needs_confirmation') });
    expect(t.texts()).toHaveLength(1);
    t.store.onRuns({ a: run('a', 'running') });
    expect(t.texts()).toEqual([]);
  });
  it('removes the paused notice when the run disappears and does not duplicate it', () => {
    const t = setup();
    t.store.onRuns({ a: run('a', 'needs_confirmation') });
    t.store.onRuns({});
    expect(t.texts()).toEqual([]);
  });
  it('reports a failed stop but not a user stop or an already stopped run', () => {
    const t = setup();
    t.store.onRuns({ a: run('a', 'running'), b: run('b', 'running') });
    t.store.onRuns({ a: run('a', 'stopped', { reason: 'service_exited', detail: 'exit code 2' }, 'failed'), b: run('b', 'stopped', { reason: 'user' }, 'exited') });
    expect(t.texts()).toEqual(['Run stopped: the service exited (exit code 2)']);
    const quiet = setup();
    quiet.store.onRuns({ c: run('c', 'stopped', { reason: 'service_exited' }, 'failed') });
    expect(quiet.texts()).toEqual([]);
  });
  it('reports a run that failed to start', () => {
    const t = setup();
    t.store.onRuns({ a: run('a', 'none', {}, 'starting') });
    t.store.onRuns({ a: run('a', 'none', {}, 'failed', 'no such device') });
    expect(t.texts()).toEqual(['Run stopped: no such device']);
  });
  it('is a pure function of runs and the last seen phases', () => {
    const first = noticesFromRuns({ a: run('a', 'needs_confirmation') }, {});
    expect(first.add).toHaveLength(1);
    expect(noticesFromRuns({ a: run('a', 'needs_confirmation') }, first.seen).add).toEqual([]);
  });
});

describe('NoticeList', () => {
  const notices = [{ id: 1, kind: 'paused' as const, text: 'Actions paused — confirm to resume', ttl: null, action: 'review' as const }, { id: 2, kind: 'connected' as const, text: '3-knob module connected', ttl: 5000 }];
  const render = (list = notices) => renderToStaticMarkup(<NoticeList notices={list} onDismiss={() => {}} onPause={() => {}} onResume={() => {}} onReview={() => {}} />);
  it('is a polite status region that is present even when empty', () => {
    expect(render([])).toContain('role="status"');
    expect(render([])).toContain('aria-live="polite"');
  });
  it('shows Review only on notices that have the action and never autofocuses', () => {
    const markup = render();
    expect(markup.match(/>Review</g)).toHaveLength(1);
    expect(markup).toContain('Dismiss notice: 3-knob module connected');
    expect(markup).not.toContain('autofocus');
  });
  it('marks each toast with its id so dismiss can find the next one', () => {
    const markup = render();
    expect(markup).toContain('data-notice-id="1"');
    expect(markup).toContain('data-notice-id="2"');
  });
});

describe('notice focus', () => {
  it('moves focus to the next notice, else the previous, else none', () => {
    expect(noticeFocusAfterDismiss([1, 2, 3], 1)).toBe(2);
    expect(noticeFocusAfterDismiss([1, 2, 3], 2)).toBe(3);
    expect(noticeFocusAfterDismiss([1, 2, 3], 3)).toBe(2);
    expect(noticeFocusAfterDismiss([4], 4)).toBeNull();
    expect(noticeFocusAfterDismiss([1, 2], 9)).toBeNull();
  });
  it('focuses a target once it renders and gives up after the frame budget', () => {
    const queue: (() => void)[] = [];
    const schedule = (callback: () => void) => { queue.push(callback); };
    const run = () => { while (queue.length) queue.shift()!(); };
    let focused = 0;
    let frames = 0;
    focusWhenRendered(() => (++frames >= 3 ? { focus: () => { focused += 1; } } : null), schedule, 10);
    run();
    expect(focused).toBe(1);
    expect(frames).toBe(3);
    let lookups = 0;
    focusWhenRendered(() => { lookups += 1; return null; }, schedule, 4);
    run();
    expect(lookups).toBe(5);
  });
});
