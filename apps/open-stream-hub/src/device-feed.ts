import type { DeviceInputPayload, DeviceInventoryPayload, DeviceStatusPayload, FeedSource, LiveDevice, ReplayKind } from './contracts';
import { emptyLive, reduceLive, resetLive } from './live-state';
import type { LiveInputPayload, LiveState } from './live-state';
import { errorMessage } from './ui-model';
import type { ModuleId } from './ui-model';
import type { Invoke } from './profile-session';

export type Listen = (event: string, handler: (event: { payload: unknown }) => void) => Promise<() => void>;
export type Scheduler = (callback: () => void) => void;

export interface FeedStatus { feedId: string; state: DeviceStatusPayload['state']; source: 'demo' | 'live'; module: DeviceStatusPayload['module']; error?: string }
export interface FeedStoreState {
  desktop: boolean;
  devices: LiveDevice[];
  // True once the first inventory snapshot (even an empty one) has been applied.
  inventoryLoaded: boolean;
  autoDetect: boolean;
  feeds: Record<string, FeedStatus>;
  live: LiveState;
  // Modules whose live state comes from a running service session's own input records, not from a feed.
  sessionModules: ModuleId[];
  error: string | null;
}

export const isRunning = (state: FeedStoreState) => Object.values(state.feeds).some(feed => feed.state === 'running');
export const idleFeedState: FeedStoreState = { desktop: false, devices: [], inventoryLoaded: false, autoDetect: true, feeds: {}, live: emptyLive, sessionModules: [], error: null };

export const autoDetectStorageKey = 'open-stream-hub.auto-detect';
export const readAutoDetect = (storage: Pick<Storage, 'getItem'>): boolean => {
  try { return storage.getItem(autoDetectStorageKey) !== 'off'; } catch { return true; }
};
export const writeAutoDetect = (storage: Pick<Storage, 'setItem'>, on: boolean) => {
  try { storage.setItem(autoDetectStorageKey, on ? 'on' : 'off'); } catch { /* the toggle still works for this session */ }
};

// Devices are identified by path. A module swap shows up as one removal plus one addition.
export function diffDevices(previous: LiveDevice[], next: LiveDevice[]): { added: LiveDevice[]; removed: LiveDevice[] } {
  const before = new Set(previous.map(device => device.path_hex));
  const after = new Set(next.map(device => device.path_hex));
  return { added: next.filter(device => !before.has(device.path_hex)), removed: previous.filter(device => !after.has(device.path_hex)) };
}

const frame: Scheduler = callback => {
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => callback()); else setTimeout(callback, 16);
};

// Holds live device state fed by Tauri events. Input events are reduced immediately but
// published at most once per scheduled frame. Live input is read-only: nothing here runs actions.
export class DeviceFeed {
  private state: FeedStoreState;
  private pendingLive: LiveState | null = null;
  private frameQueued = false;
  private connected = false;
  private generation = 0;
  private inventorySeen = false;
  private readonly attempted = new Set<string>();
  private readonly feedByPath = new Map<string, string>();
  private readonly droppedFeeds = new Set<string>();
  private unlisteners: (() => void)[] = [];
  private isBlocked: (pathHex: string, device?: LiveDevice) => boolean = () => false;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly invoke: Invoke, private readonly listen: Listen, desktop: boolean, private readonly schedule: Scheduler = frame) {
    this.state = { ...idleFeedState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  // Paths owned by another reader (a running service) never get a live feed from here.
  setBlocker(isBlocked: (pathHex: string, device?: LiveDevice) => boolean) { this.isBlocked = isBlocked; }

  private set(patch: Partial<FeedStoreState>) {
    this.state = { ...this.state, ...(this.pendingLive ? { live: this.pendingLive } : {}), ...patch };
    this.pendingLive = null;
    this.listeners.forEach(listener => listener());
  }

  // Subscribes to backend events. Safe to call again after dispose (React strict mode).
  connect() {
    if (!this.state.desktop || this.connected) return;
    this.connected = true;
    this.inventorySeen = false;
    const generation = ++this.generation;
    const add = (event: string, handler: (payload: unknown) => void) => {
      this.listen(event, e => handler(e.payload)).then(
        unlisten => { if (this.generation === generation && this.connected) this.unlisteners.push(unlisten); else unlisten(); },
        reason => { if (this.generation === generation) this.set({ error: errorMessage(reason) }); });
    };
    add('device://input', payload => this.onInput(payload as DeviceInputPayload));
    add('device://status', payload => this.onStatus(payload as DeviceStatusPayload));
    add('device://inventory', payload => this.onInventory(payload as DeviceInventoryPayload));
    void this.refreshDevices();
  }

  private onInput(payload: DeviceInputPayload) {
    const base = this.pendingLive ?? this.state.live;
    const next = reduceLive(base, payload);
    if (next === base) return;
    this.pendingLive = next;
    if (this.frameQueued) return;
    this.frameQueued = true;
    this.schedule(() => {
      this.frameQueued = false;
      if (this.connected && this.pendingLive) this.set({});
    });
  }

  // A reset of the feeds' live state, keeping what running sessions report for their modules.
  private clearedLive(): LiveState {
    const live = this.pendingLive ?? this.state.live;
    const kept = this.state.sessionModules.filter(module => live[module]);
    return kept.length ? Object.fromEntries(kept.map(module => [module, live[module]])) : resetLive();
  }

  // Input from a running service session's `event` records, reduced exactly like feed input. Read-only like the feed.
  sessionInput(payload: LiveInputPayload) { this.onInput(payload); }

  // The modules running sessions now drive. Every module in `reset` starts over, so a session that starts or ends never
  // leaves a key shown as pressed.
  setSessionModules(modules: ModuleId[], reset: ModuleId[]) {
    const live = { ...(this.pendingLive ?? this.state.live) };
    reset.forEach(module => { delete live[module]; });
    this.pendingLive = live;
    this.set({ sessionModules: modules });
  }

  private onStatus(payload: DeviceStatusPayload) {
    if (!payload || typeof payload.feed_id !== 'string') return;
    if (payload.state === 'failed' && this.droppedFeeds.has(payload.feed_id)) return;
    const status: FeedStatus = { feedId: payload.feed_id, state: payload.state, source: payload.source, module: payload.module ?? null, ...(payload.state === 'failed' ? { error: payload.error ?? 'The feed failed.' } : {}) };
    const feeds = { ...this.state.feeds, [payload.feed_id]: status };
    const anyRunning = Object.values(feeds).some(feed => feed.state === 'running');
    if (!anyRunning) this.pendingLive = this.clearedLive();
    this.set({ feeds, error: payload.state === 'failed' ? status.error! : this.state.error });
  }

  // Initial snapshot (or manual refresh). Once an inventory event has arrived it is newer than any snapshot.
  async refreshDevices() {
    if (!this.state.desktop) return;
    try {
      const devices = await this.invoke<LiveDevice[]>('list_live_devices');
      if (!this.inventorySeen) this.applyInventory(devices);
    } catch (reason) { this.set({ error: errorMessage(reason) }); }
  }

  private onInventory(payload: DeviceInventoryPayload) {
    if (!Array.isArray(payload?.devices)) return;
    this.inventorySeen = true;
    this.applyInventory(payload.devices);
  }

  private applyInventory(devices: LiveDevice[]) {
    const { added, removed } = diffDevices(this.state.devices, devices);
    if (!added.length && !removed.length) { if (!this.state.inventoryLoaded) this.set({ inventoryLoaded: true }); return; }
    const feeds = { ...this.state.feeds };
    let error = this.state.error;
    for (const device of removed) {
      this.attempted.delete(device.path_hex);
      const feedId = this.feedByPath.get(device.path_hex);
      if (feedId === undefined) continue;
      this.feedByPath.delete(device.path_hex);
      this.droppedFeeds.add(feedId);
      if (error !== null && error === feeds[feedId]?.error) error = null;
      delete feeds[feedId];
    }
    if (removed.length && !Object.values(feeds).some(feed => feed.state === 'running')) this.pendingLive = this.clearedLive();
    this.set({ devices, inventoryLoaded: true, feeds, error });
    if (this.state.autoDetect) added.forEach(device => void this.startAuto(device.path_hex));
  }

  // Read-only live feed for a newly appeared device. Tried once per appearance; never retried in a loop.
  private async startAuto(pathHex: string) {
    if (this.attempted.has(pathHex)) return;
    this.attempted.add(pathHex);
    if (this.isBlocked(pathHex, this.state.devices.find(device => device.path_hex === pathHex))) return;
    await this.start({ kind: 'live', path_hex: pathHex }, true);
  }

  setAutoDetect(on: boolean) {
    if (this.state.autoDetect === on) return;
    this.set({ autoDetect: on });
    if (on && this.connected) this.state.devices.forEach(device => void this.startAuto(device.path_hex));
  }

  startDemo(demo: ReplayKind) { return this.start({ kind: 'demo', demo }); }
  startLive(pathHex: string) { return this.start({ kind: 'live', path_hex: pathHex }); }

  private async start(source: FeedSource, auto = false) {
    if (!this.state.desktop) return;
    if (source.kind === 'live' && this.isBlocked(source.path_hex, this.state.devices.find(device => device.path_hex === source.path_hex))) { if (!auto) this.set({ error: 'Stop the running profile on this device before reading live input.' }); return; }
    const present = () => source.kind !== 'live' || this.state.devices.some(device => device.path_hex === source.path_hex);
    if (!auto || !isRunning(this.state)) this.pendingLive = this.clearedLive();
    this.set(auto ? {} : { error: null });
    try {
      const feedId = await this.invoke<string>('start_feed', { source });
      if (auto && !present()) return;
      this.droppedFeeds.delete(feedId);
      if (source.kind === 'live') this.feedByPath.set(source.path_hex, feedId);
      // A status event may already have arrived; never overwrite it.
      if (!this.state.feeds[feedId]) this.set({ feeds: { ...this.state.feeds, [feedId]: { feedId, state: 'running', source: source.kind, module: null } } });
    } catch (reason) { if (!auto || present()) this.set({ error: errorMessage(reason) }); }
  }

  async stop(feedId: string) {
    if (!this.state.desktop) return;
    try { await this.invoke<void>('stop_feed', { feedId }); }
    catch (reason) { this.set({ error: errorMessage(reason) }); }
  }

  dispose() {
    this.connected = false;
    this.pendingLive = null;
    this.unlisteners.forEach(unlisten => unlisten());
    this.unlisteners = [];
  }
}
