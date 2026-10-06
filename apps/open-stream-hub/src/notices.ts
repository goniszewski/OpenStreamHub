import type { LiveDevice } from './contracts';
import { diffDevices } from './device-feed';
import type { FeedStoreState } from './device-feed';
import { reasonLabels } from './service-runner';
import type { ServiceRun } from './service-runner';
import { modules } from './ui-model';

// In-app notices for module connect, disconnect and swap, plus the few session events that need attention.
// The reducer is pure. NoticeStore adds ids, timers and persistence; the clock and timers are injectable.

export type NoticeKind = 'connected' | 'disconnected' | 'reconnected' | 'replaced' | 'paused' | 'stopped';
export interface NoticeDraft { kind: NoticeKind; text: string; ttl: number | null; action?: 'review'; key?: string }
export interface Notice extends NoticeDraft { id: number }
interface PendingRemoval { device: LiveDevice; at: number; crowded: boolean }
export interface InventoryNoticeState { pending: PendingRemoval[] }

export const emptyInventoryState: InventoryNoticeState = { pending: [] };
export const coalesceMs = 3000;
export const defaultTtl = 5000;
export const failureTtl = 10000;
export const maxVisible = 3;
export const noticesStorageKey = 'open-stream-hub.connection-notices';

// Which notice should take focus when `id` is dismissed while focus is inside it:
// the next one, else the previous one, else none (the caller uses a stable target).
export function noticeFocusAfterDismiss(ids: number[], id: number): number | null {
  const index = ids.indexOf(id);
  if (index < 0) return null;
  return ids[index + 1] ?? ids[index - 1] ?? null;
}

const nameOf = (device: LiveDevice) => modules.find(module => module.id === device.module)?.shortName ?? device.module;
const sameModule = (a: LiveDevice, b: LiveDevice) => a.module === b.module && a.serial_number === b.serial_number;
// Serial tails are shown only when several modules share a type, so the notice can tell them apart.
const label = (device: LiveDevice, crowded: boolean) => crowded && device.serial_number ? `${nameOf(device)} (…${device.serial_number.slice(-4)})` : nameOf(device);
const crowdedIn = (list: LiveDevice[], device: LiveDevice) => list.filter(item => item.module === device.module).length > 1;

const disconnected = (removal: PendingRemoval): NoticeDraft => ({ kind: 'disconnected', text: `${label(removal.device, removal.crowded)} disconnected`, ttl: defaultTtl });

// Turns one inventory change into notices. A removal waits coalesceMs for a matching addition
// (same module and serial: reconnected; same module, other serial: replaced) before it is reported.
export function noticesFromInventory(previous: LiveDevice[], next: LiveDevice[], now: number, state: InventoryNoticeState = emptyInventoryState): { state: InventoryNoticeState; notices: NoticeDraft[] } {
  const { added, removed } = diffDevices(previous, next);
  const notices: NoticeDraft[] = [];
  let pending = state.pending.filter(item => now - item.at < coalesceMs);
  notices.push(...state.pending.filter(item => now - item.at >= coalesceMs).map(disconnected));
  pending = [...pending, ...removed.map(device => ({ device, at: now, crowded: crowdedIn(previous, device) }))];
  for (const device of added) {
    const crowded = crowdedIn(next, device);
    let index = pending.findIndex(item => sameModule(item.device, device));
    if (index !== -1) notices.push({ kind: 'reconnected', text: `${label(device, crowded)} reconnected`, ttl: defaultTtl });
    else if ((index = pending.findIndex(item => item.device.module === device.module)) !== -1) notices.push({ kind: 'replaced', text: `${label(device, crowded)} replaced`, ttl: defaultTtl });
    else { notices.push({ kind: 'connected', text: `${label(device, crowded)} connected`, ttl: defaultTtl }); continue; }
    pending.splice(index, 1);
  }
  return { state: { pending }, notices };
}

// Reports removals that nothing replaced within the coalescing window.
export function flushDisconnects(state: InventoryNoticeState, now: number): { state: InventoryNoticeState; notices: NoticeDraft[] } {
  const due = state.pending.filter(item => now - item.at >= coalesceMs);
  return { state: { pending: state.pending.filter(item => now - item.at < coalesceMs) }, notices: due.map(disconnected) };
}

const confirmKey = (run: string) => `confirm:${run}`;

// Only actionable session events become notices: actions paused for confirmation, and runs that stopped on a failure.
export function noticesFromRuns(runs: Record<string, ServiceRun>, seen: Record<string, string>): { seen: Record<string, string>; add: NoticeDraft[]; resolved: string[] } {
  const nextSeen: Record<string, string> = {};
  const add: NoticeDraft[] = [];
  const resolved: string[] = [];
  for (const [key, run] of Object.entries(runs)) {
    const status = run.session?.phase ?? run.state;
    nextSeen[key] = status;
    const before = seen[key];
    if (status === before) continue;
    if (status === 'needs_confirmation') add.push({ kind: 'paused', text: run.session?.reason === 'profile_switch' ? 'Focused-app rule switched the profile — confirm to resume actions' : 'Actions paused — confirm to resume', ttl: null, action: 'review', key: confirmKey(key) });
    else if (before === 'needs_confirmation') resolved.push(confirmKey(key));
    if (before === undefined && status !== 'needs_confirmation') continue;
    const session = run.session;
    if (session?.phase === 'stopped' && session.reason !== null && session.reason !== 'user') {
      add.push({ kind: 'stopped', text: `Run stopped: ${reasonLabels[session.reason]}${session.detail ? ` (${session.detail.slice(0, 120)})` : ''}`, ttl: failureTtl });
    } else if (!session && status === 'failed') {
      add.push({ kind: 'stopped', text: `Run stopped: ${run.error ? run.error.slice(0, 160) : 'the service failed'}`, ttl: failureTtl });
    }
  }
  for (const key of Object.keys(seen)) if (!(key in runs) && seen[key] === 'needs_confirmation') resolved.push(confirmKey(key));
  return { seen: nextSeen, add, resolved };
}

export interface NoticeEnv {
  now: () => number;
  setTimer: (callback: () => void, ms: number) => unknown;
  clearTimer: (handle: unknown) => void;
  storage: Pick<Storage, 'getItem' | 'setItem'> | null;
}
export interface NoticeStoreState { notices: Notice[]; enabled: boolean }
interface Source<T> { subscribe(listener: () => void): () => void; getState(): T }

const defaultEnv = (): NoticeEnv => {
  let storage: Storage | null = null;
  try { storage = localStorage; } catch { /* Preference is kept for this session only. */ }
  return { now: Date.now, setTimer: (callback, ms) => setTimeout(callback, ms), clearTimer: handle => clearTimeout(handle as ReturnType<typeof setTimeout>), storage };
};

export function readNoticesEnabled(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(noticesStorageKey) !== 'off'; } catch { return true; }
}

export class NoticeStore {
  private state: NoticeStoreState;
  private nextId = 1;
  private baseline: LiveDevice[] | null = null;
  private inventory = emptyInventoryState;
  private seen: Record<string, string> = {};
  private flushHandle: unknown = null;
  private readonly timers = new Map<number, unknown>();
  private readonly listeners = new Set<() => void>();
  private readonly env: NoticeEnv;

  constructor(env: Partial<NoticeEnv> = {}) {
    this.env = { ...defaultEnv(), ...env };
    this.state = { notices: [], enabled: readNoticesEnabled(this.env.storage) };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<NoticeStoreState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Follows the device feed and the service runner. Returns the unsubscribe function.
  connect(feed: Source<FeedStoreState>, runner: Source<{ runs: Record<string, ServiceRun> }>) {
    const onFeed = () => this.onFeed(feed.getState());
    const onRuns = () => this.onRuns(runner.getState().runs);
    const stops = [feed.subscribe(onFeed), runner.subscribe(onRuns)];
    onFeed(); onRuns();
    return () => { stops.forEach(stop => stop()); this.clearAll(); };
  }

  onFeed(feed: FeedStoreState) {
    if (!feed.inventoryLoaded) return;
    // The first snapshot is the starting point, so launching with modules plugged in shows nothing.
    if (this.baseline === null) { this.baseline = feed.devices; return; }
    if (feed.devices === this.baseline) return;
    const result = noticesFromInventory(this.baseline, feed.devices, this.env.now(), this.inventory);
    this.baseline = feed.devices;
    this.inventory = result.state;
    this.push(result.notices);
    this.scheduleFlush();
  }

  onRuns(runs: Record<string, ServiceRun>) {
    const result = noticesFromRuns(runs, this.seen);
    this.seen = result.seen;
    const gone = result.resolved.filter(key => this.state.notices.some(notice => notice.key === key));
    if (gone.length) this.set({ notices: this.state.notices.filter(notice => !gone.includes(notice.key ?? '')) });
    this.push(result.add);
  }

  private scheduleFlush() {
    if (this.flushHandle !== null) this.env.clearTimer(this.flushHandle);
    this.flushHandle = null;
    if (!this.inventory.pending.length) return;
    const oldest = Math.min(...this.inventory.pending.map(item => item.at));
    this.flushHandle = this.env.setTimer(() => {
      this.flushHandle = null;
      const result = flushDisconnects(this.inventory, this.env.now());
      this.inventory = result.state;
      this.push(result.notices);
      this.scheduleFlush();
    }, Math.max(0, oldest + coalesceMs - this.env.now()));
  }

  private push(drafts: NoticeDraft[]) {
    const kept = drafts.filter(draft => this.state.enabled || draft.kind === 'paused' || draft.kind === 'stopped');
    if (!kept.length) return;
    let notices = [...this.state.notices];
    for (const draft of kept) {
      if (draft.key) notices = notices.filter(notice => notice.key !== draft.key);
      const notice: Notice = { ...draft, id: this.nextId++ };
      notices.push(notice);
      // Over the limit, the oldest dismissible notice makes room; paused notices never do.
      while (notices.length > maxVisible) {
        const index = notices.findIndex(item => item.ttl !== null);
        if (index === -1) break;
        this.cancel(notices[index].id);
        notices.splice(index, 1);
      }
    }
    this.set({ notices });
    for (const notice of notices) if (notice.ttl !== null && !this.timers.has(notice.id)) this.arm(notice);
  }

  private arm(notice: Notice) {
    this.timers.set(notice.id, this.env.setTimer(() => { this.timers.delete(notice.id); this.dismiss(notice.id); }, notice.ttl ?? defaultTtl));
  }

  private cancel(id: number) {
    if (this.timers.has(id)) this.env.clearTimer(this.timers.get(id));
    this.timers.delete(id);
  }

  dismiss(id: number) {
    this.cancel(id);
    if (this.state.notices.some(notice => notice.id === id)) this.set({ notices: this.state.notices.filter(notice => notice.id !== id) });
  }

  // Hover or focus holds a notice open; leaving restarts its full time.
  pause(id: number) { this.cancel(id); }
  resume(id: number) {
    const notice = this.state.notices.find(item => item.id === id);
    if (notice && notice.ttl !== null && !this.timers.has(id)) this.arm(notice);
  }

  setEnabled(enabled: boolean) {
    if (enabled === this.state.enabled) return;
    try { this.env.storage?.setItem(noticesStorageKey, enabled ? 'on' : 'off'); } catch { /* The choice still holds for this session. */ }
    const notices = enabled ? this.state.notices : this.state.notices.filter(notice => notice.kind === 'paused' || notice.kind === 'stopped');
    this.state.notices.filter(notice => !notices.includes(notice)).forEach(notice => this.cancel(notice.id));
    this.set({ enabled, notices });
  }

  private clearAll() {
    this.timers.forEach(handle => this.env.clearTimer(handle));
    this.timers.clear();
    if (this.flushHandle !== null) this.env.clearTimer(this.flushHandle);
    this.flushHandle = null;
  }
}
