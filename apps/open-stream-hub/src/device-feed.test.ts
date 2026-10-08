import { describe, expect, it } from 'vitest';
import { DeviceFeed, diffDevices, readAutoDetect, writeAutoDetect } from './device-feed';
import type { Scheduler } from './device-feed';

function setup(desktop = true) {
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const unlistened: string[] = [];
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const queue: (() => void)[] = [];
  const results: Record<string, unknown> = { start_feed: 'demo:knobs', list_live_devices: [{ path_hex: 'aabbccdd', module: 'm003k', serial_number: 'S1' }] };
  const failures: Record<string, string> = {};
  const invoke = (async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (failures[command]) throw new Error(failures[command]);
    return results[command];
  }) as never;
  const listen = async (event: string, handler: (event: { payload: unknown }) => void) => { handlers[event] = handler; return () => { unlistened.push(event); }; };
  const schedule: Scheduler = callback => { queue.push(callback); };
  const feed = new DeviceFeed(invoke, listen, desktop, schedule);
  const flush = () => queue.splice(0).forEach(callback => callback());
  const press = (control: number) => handlers['device://input']({ payload: { feed_id: 'demo:knobs', module: 'm4315', serial_number: null, event: { timestamp_micros: 1, control, gesture: 'pressed' } } });
  const status = (state: string, extra = {}) => handlers['device://status']({ payload: { feed_id: 'demo:knobs', state, source: 'demo', module: null, ...extra } });
  return { feed, handlers, unlistened, calls, queue, flush, press, status, failures };
}
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

describe('DeviceFeed', () => {
  it('starts a demo, tracks status and stops', async () => {
    const t = setup(); t.feed.connect(); await tick();
    await t.feed.startDemo('knobs');
    expect(t.calls.at(-1)).toEqual({ command: 'start_feed', args: { source: { kind: 'demo', demo: 'knobs' } } });
    expect(t.feed.getState().feeds['demo:knobs'].state).toBe('running');
    await t.feed.stop('demo:knobs');
    expect(t.calls.at(-1)).toEqual({ command: 'stop_feed', args: { feedId: 'demo:knobs' } });
    t.status('stopped');
    expect(t.feed.getState().feeds['demo:knobs'].state).toBe('stopped');
  });
  it('starts a live device and lists devices', async () => {
    const t = setup();
    await t.feed.refreshDevices();
    expect(t.feed.getState().devices[0].path_hex).toBe('aabbccdd');
    await t.feed.startLive('aabbccdd');
    expect(t.calls.at(-1)?.args).toEqual({ source: { kind: 'live', path_hex: 'aabbccdd' } });
  });
  it('coalesces input to one notification per frame', async () => {
    const t = setup(); t.feed.connect(); await tick();
    let notified = 0; t.feed.subscribe(() => { notified++; });
    t.press(1); t.press(2); t.press(3);
    expect(notified).toBe(0);
    expect(t.queue.length).toBe(1);
    t.flush();
    expect(notified).toBe(1);
    expect(t.feed.getState().live.m4315?.keysDown).toEqual([1, 2, 3]);
  });
  it('keeps failure text and clears live state when no feed runs', async () => {
    const t = setup(); t.feed.connect(); await tick();
    await t.feed.startDemo('knobs');
    t.press(1); t.flush();
    t.status('failed', { error: 'device unplugged' });
    expect(t.feed.getState().error).toBe('device unplugged');
    expect(t.feed.getState().feeds['demo:knobs'].error).toBe('device unplugged');
    expect(t.feed.getState().live).toEqual({});
  });
  it('reports a start error', async () => {
    const t = setup(); t.failures.start_feed = 'already running';
    await t.feed.startDemo('dial');
    expect(t.feed.getState().error).toBe('already running');
    expect(Object.keys(t.feed.getState().feeds)).toEqual([]);
  });
  it('adopts a live feed that survived a window reload instead of reporting an error', async () => {
    const t = setup(); t.failures.start_feed = 'feed live:aabbccdd is already running';
    t.feed.connect(); await tick(); await tick();
    expect(t.feed.getState().error).toBeNull();
    expect(t.feed.getState().feeds['live:aabbccdd']).toMatchObject({ state: 'running', source: 'live' });
    const other = setup(); other.failures.start_feed = 'feed live:ffff is already running';
    other.feed.connect(); await tick(); await tick();
    expect(other.feed.getState().error).toBe('feed live:ffff is already running');
    expect(other.feed.getState().feeds['live:ffff']).toBeUndefined();
  });
  it('unlistens on dispose, even before listen resolves', async () => {
    const t = setup(); t.feed.connect(); await tick();
    t.feed.dispose();
    expect(t.unlistened.sort()).toEqual(['device://input', 'device://inventory', 'device://status']);
    const early = setup(); early.feed.connect(); early.feed.dispose(); await tick();
    expect(early.unlistened.length).toBe(3);
  });
  it('is a no-op outside the desktop app', async () => {
    const t = setup(false); t.feed.connect();
    await t.feed.startDemo('knobs'); await t.feed.refreshDevices();
    expect(t.calls).toEqual([]);
    expect(Object.keys(t.handlers)).toEqual([]);
  });
});

const dev = (path_hex: string, module = 'm003k') => ({ path_hex, module, serial_number: null });

describe('diffDevices', () => {
  it('reports additions and removals by path', () => {
    expect(diffDevices([dev('a'), dev('b')] as never, [dev('b'), dev('c')] as never)).toEqual({ added: [dev('c')], removed: [dev('a')] });
    expect(diffDevices([dev('a')] as never, [dev('a')] as never)).toEqual({ added: [], removed: [] });
  });
});

describe('auto-detect', () => {
  function inventory(initial: unknown[] = []) {
    const results = { list_live_devices: initial } as Record<string, unknown>;
    const calls: { command: string; args?: Record<string, unknown> }[] = [];
    const failures: Record<string, string> = {};
    const handlers: Record<string, (event: { payload: unknown }) => void> = {};
    let counter = 0;
    const invoke = (async (command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args });
      if (failures[command]) throw new Error(failures[command]);
      if (command === 'start_feed') return `live:${(args as { source: { path_hex: string } }).source.path_hex}:${counter++}`;
      return results[command];
    }) as never;
    const feed = new DeviceFeed(invoke, async (event, handler) => { handlers[event] = handler; return () => {}; }, true, callback => callback());
    const starts = () => calls.filter(call => call.command === 'start_feed').map(call => (call.args as { source: { path_hex: string } }).source.path_hex);
    const send = (devices: unknown[]) => handlers['device://inventory']({ payload: { devices } });
    return { feed, calls, failures, handlers, starts, send };
  }

  it('takes the initial snapshot and starts a live feed per device once', async () => {
    const x = inventory([dev('a')]); x.feed.connect(); await tick(); await tick();
    expect(x.feed.getState().devices).toEqual([dev('a')]);
    expect(x.starts()).toEqual(['a']);
    x.send([dev('a')]); await tick();
    expect(x.starts()).toEqual(['a']);
  });
  it('adds, removes and swaps devices from inventory events', async () => {
    const x = inventory(); x.feed.connect(); await tick();
    x.send([dev('a')]); await tick();
    x.send([dev('a'), dev('b', 'm131e')]); await tick();
    expect(x.starts()).toEqual(['a', 'b']);
    expect(Object.keys(x.feed.getState().feeds).length).toBe(2);
    x.send([dev('b', 'm131e'), dev('c')]); await tick();
    expect(x.feed.getState().devices.map(device => device.path_hex)).toEqual(['b', 'c']);
    expect(Object.keys(x.feed.getState().feeds).map(id => id.split(':')[1]).sort()).toEqual(['b', 'c']);
    x.send([dev('b', 'm131e')]); x.send([dev('b', 'm131e'), dev('c')]); await tick();
    expect(x.starts()).toEqual(['a', 'b', 'c', 'c']);
  });
  it('stays quiet when a removed device reports failure, and shows a real failure once', async () => {
    const x = inventory(); x.feed.connect(); await tick();
    x.send([dev('a')]); await tick();
    const feedId = Object.keys(x.feed.getState().feeds)[0];
    x.send([]); await tick();
    x.handlers['device://status']({ payload: { feed_id: feedId, state: 'failed', source: 'live', module: 'm003k', error: 'gone' } });
    expect(x.feed.getState().error).toBeNull();
    expect(x.feed.getState().feeds).toEqual({});
    x.failures.start_feed = 'busy';
    x.send([dev('b')]); await tick(); await tick();
    expect(x.feed.getState().error).toBe('busy');
    x.send([dev('b')]); await tick();
    expect(x.starts()).toEqual(['a', 'b']);
  });
  it('does not start feeds when auto-detect is off, and starts existing devices when turned on', async () => {
    const x = inventory([dev('a')]); x.feed.setAutoDetect(false); x.feed.connect(); await tick(); await tick();
    x.send([dev('a'), dev('b')]); await tick();
    expect(x.starts()).toEqual([]);
    expect(x.feed.getState().devices.length).toBe(2);
    x.feed.setAutoDetect(true); await tick(); await tick();
    expect(x.starts().sort()).toEqual(['a', 'b']);
  });
  it('persists the toggle defensively', () => {
    const data = new Map<string, string>(); const storage = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
    expect(readAutoDetect(storage)).toBe(true);
    writeAutoDetect(storage, false); expect(readAutoDetect(storage)).toBe(false);
    expect(readAutoDetect({ getItem: () => { throw new Error('blocked'); } })).toBe(true);
    expect(() => writeAutoDetect({ setItem: () => { throw new Error('full'); } }, true)).not.toThrow();
  });
});
