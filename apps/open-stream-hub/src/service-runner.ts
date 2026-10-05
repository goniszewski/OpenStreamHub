import type { Profile, ServiceRecordPayload, ServiceState, ServiceStatusPayload, StartServiceRequest } from './contracts';
import { stableStringify } from './draft-model';
import type { Listen, Scheduler } from './device-feed';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

export type ActivityKind = 'event' | 'action' | 'action_failed' | 'info' | 'error' | 'raw';
export interface ActivityEntry { kind: ActivityKind; text: string; at: number }
export interface ServiceCounters { events: number; planned: number; executed: number; failed: number }
export interface ServiceRun {
  serviceId: string;
  state: ServiceState;
  execute: boolean;
  exitCode?: number | null;
  error?: string;
  stderrTail?: string;
  entries: ActivityEntry[];
  counters: ServiceCounters;
  startedAt: number;
  // The profile the service was started with. The service never reloads it.
  profileName: string;
  snapshot: string;
  draftUnsaved: boolean;
}
export interface ServiceStoreState { desktop: boolean; runs: Record<string, ServiceRun>; error: string | null }

export const maxEntries = 200;
export const idleServiceState: ServiceStoreState = { desktop: false, runs: {}, error: null };
export const serviceIdFor = (pathHex: string) => `service:${pathHex.toLowerCase()}`;
export const isActiveState = (state: ServiceState) => state === 'starting' || state === 'running' || state === 'stopping';
export const isActiveRun = (run: ServiceRun | undefined): boolean => run !== undefined && isActiveState(run.state);
export const emptyCounters = (): ServiceCounters => ({ events: 0, planned: 0, executed: 0, failed: 0 });

type Json = Record<string, unknown>;
const isObject = (value: unknown): value is Json => typeof value === 'object' && value !== null && !Array.isArray(value);
const str = (value: unknown) => typeof value === 'string' ? value : undefined;
const clip = (text: string, max = 300) => text.length > max ? `${text.slice(0, max)}…` : text;

function describeAction(action: unknown): string {
  if (!isObject(action)) return 'action';
  switch (action.type) {
    case 'volume': return `volume ${typeof action.delta === 'number' && action.delta > 0 ? '+' : ''}${String(action.delta)}`;
    case 'set_volume': return `set volume to ${String(action.percent)}%`;
    case 'toggle_mute': return 'toggle mute';
    case 'shortcut': {
      const modifiers = Array.isArray(action.modifiers) ? action.modifiers.map(String) : [];
      return `shortcut ${[...modifiers, String(action.key)].join('+')}`;
    }
    case 'launch_app': return `open ${String(action.bundle_id)}`;
    default: return str(action.type) ?? 'action';
  }
}

function describeEvent(event: unknown): string {
  if (!isObject(event)) return 'input';
  const parts = [str(event.module), typeof event.control === 'number' ? `control ${event.control + 1}` : undefined, str(event.gesture)?.replace(/_/g, ' ')];
  const value = event.value;
  if (isObject(value) && typeof value.raw === 'number') parts.push(`${value.raw}/${String(value.maximum)}`);
  return parts.filter(Boolean).join(' ') || 'input';
}

// Turns one service JSONL record into a log line. Returns null for records that are only noise
// (every raw HID report). Unknown shapes degrade to 'raw' or 'info'.
export function summarizeRecord(record: unknown, at: number): ActivityEntry | null {
  if (!isObject(record)) return { kind: 'raw', text: clip(typeof record === 'string' ? record : JSON.stringify(record) ?? String(record)), at };
  const entry = (kind: ActivityKind, text: string): ActivityEntry => ({ kind, text, at });
  switch (record.type) {
    case 'raw': return entry('raw', clip(str(record.line) ?? ''));
    case 'service_start': {
      const device = isObject(record.device) ? record.device : {};
      return entry('info', `Service started on ${str(device.module) ?? 'device'} (${record.execution === 'macos' ? 'actions execute' : 'dry run'}).`);
    }
    case 'report': {
      const error = str(record.error);
      return error || record.status === 'parse_error' ? entry('error', `Unreadable input report${error ? `: ${clip(error)}` : '.'}`) : null;
    }
    case 'event': {
      const text = describeEvent(record.event);
      switch (record.status) {
        case 'planned': return entry('action', `${text}: planned ${describeAction(record.action)}`);
        case 'navigation': return entry('event', `${text}: folder navigation`);
        case 'unbound': return entry('event', `${text}: no action bound`);
        default: return entry('event', text);
      }
    }
    case 'action_executed': return entry('action', `Executed ${describeAction(record.action)} (${describeEvent(record.event)}).`);
    case 'action_failed': return entry('action_failed', `${describeAction(record.action)} failed: ${clip(str(record.error) ?? 'unknown error')}`);
    case 'service_end': {
      const error = str(record.error);
      if (error) return entry('error', `Service ended with an error: ${clip(error)}`);
      return entry('info', `Service ended (${str(record.reason) ?? 'finished'}).`);
    }
    case 'profile_valid': return entry('info', 'Profile validated.');
    case 'capture_end': return entry(str(record.error) ? 'error' : 'info', `Input ended${str(record.error) ? `: ${clip(str(record.error)!)}` : '.'}`);
    default: return str(record.type) ? entry('info', `${String(record.type).replace(/_/g, ' ')}`) : entry('raw', clip(JSON.stringify(record)));
  }
}

export function countRecord(counters: ServiceCounters, record: unknown): ServiceCounters {
  if (!isObject(record)) return counters;
  switch (record.type) {
    case 'event': return { ...counters, events: counters.events + 1, planned: counters.planned + (record.status === 'planned' ? 1 : 0) };
    case 'action_executed': return { ...counters, executed: counters.executed + 1 };
    case 'action_failed': return { ...counters, failed: counters.failed + 1 };
    default: return counters;
  }
}

// The service uses the draft as it was at start and never hot-reloads.
export const isStale = (run: ServiceRun | undefined, draft: Profile) => run !== undefined && isActiveState(run.state) && stableStringify(draft) !== run.snapshot;

export type StartOptions = { profile: Profile; pathHex: string; execute: boolean; softwareMode: boolean; draftUnsaved: boolean };
export type ConfirmExecute = () => Promise<boolean>;

const frame: Scheduler = callback => {
  if (typeof requestAnimationFrame === 'function') requestAnimationFrame(() => callback()); else setTimeout(callback, 16);
};

// Holds supervised service runs fed by Tauri events. Records are reduced immediately and
// published at most once per scheduled frame.
export class ServiceRunner {
  private state: ServiceStoreState;
  private pending: Record<string, ServiceRun> | null = null;
  private frameQueued = false;
  private connected = false;
  private generation = 0;
  private unlisteners: (() => void)[] = [];
  private readonly listeners = new Set<() => void>();

  constructor(private readonly invoke: Invoke, private readonly listen: Listen, desktop: boolean,
    private readonly schedule: Scheduler = frame, private readonly now: () => number = Date.now) {
    this.state = { ...idleServiceState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;
  isActivePath = (pathHex: string) => isActiveRun((this.pending ?? this.state.runs)[serviceIdFor(pathHex)]);

  private set(patch: Partial<ServiceStoreState>) {
    this.state = { ...this.state, ...(this.pending ? { runs: this.pending } : {}), ...patch };
    this.pending = null;
    this.listeners.forEach(listener => listener());
  }

  private runs() { return this.pending ?? this.state.runs; }
  private update(id: string, change: (run: ServiceRun) => ServiceRun, immediate: boolean) {
    const run = this.runs()[id];
    if (!run) return;
    this.pending = { ...this.runs(), [id]: change(run) };
    if (immediate) { this.set({}); return; }
    if (this.frameQueued) return;
    this.frameQueued = true;
    this.schedule(() => { this.frameQueued = false; if (this.connected && this.pending) this.set({}); });
  }

  connect() {
    if (!this.state.desktop || this.connected) return;
    this.connected = true;
    const generation = ++this.generation;
    const add = (event: string, handler: (payload: unknown) => void) => {
      this.listen(event, e => handler(e.payload)).then(
        unlisten => { if (this.generation === generation && this.connected) this.unlisteners.push(unlisten); else unlisten(); },
        reason => { if (this.generation === generation) this.set({ error: errorMessage(reason) }); });
    };
    add('service://record', payload => this.onRecord(payload as ServiceRecordPayload));
    add('service://status', payload => this.onStatus(payload as ServiceStatusPayload));
  }

  private onRecord(payload: ServiceRecordPayload) {
    if (!payload || typeof payload.service_id !== 'string' || !this.runs()[payload.service_id]) return;
    const at = this.now();
    this.update(payload.service_id, run => {
      const entry = summarizeRecord(payload.record, at);
      const entries = entry ? [...run.entries, entry].slice(-maxEntries) : run.entries;
      return { ...run, entries, counters: countRecord(run.counters, payload.record) };
    }, false);
  }

  private onStatus(payload: ServiceStatusPayload) {
    if (!payload || typeof payload.service_id !== 'string' || typeof payload.state !== 'string') return;
    const id = payload.service_id;
    if (!this.runs()[id]) {
      this.pending = { ...this.runs(), [id]: this.blankRun(id, payload.execute, 'Unknown profile', '', false) };
    }
    this.update(id, run => ({
      ...run, state: payload.state, execute: payload.execute ?? run.execute,
      ...(payload.exit_code !== undefined ? { exitCode: payload.exit_code } : {}),
      ...(payload.error !== undefined ? { error: payload.error } : {}),
      ...(payload.stderr_tail !== undefined ? { stderrTail: payload.stderr_tail } : {}),
    }), true);
  }

  private blankRun(serviceId: string, execute: boolean, profileName: string, snapshot: string, draftUnsaved: boolean): ServiceRun {
    return { serviceId, state: 'starting', execute, entries: [], counters: emptyCounters(), startedAt: this.now(), profileName, snapshot, draftUnsaved };
  }

  // Execute mode never starts without an explicit confirmation callback that resolves true.
  async start(options: StartOptions, confirm?: ConfirmExecute): Promise<boolean> {
    if (!this.state.desktop) return false;
    const id = serviceIdFor(options.pathHex);
    if (isActiveRun(this.runs()[id])) { this.set({ error: 'A profile is already running on this device.' }); return false; }
    if (options.execute) {
      if (!confirm) { this.set({ error: 'Running actions needs confirmation.' }); return false; }
      if (!(await confirm())) return false;
      if (isActiveRun(this.runs()[id])) return false;
    }
    const run = this.blankRun(id, options.execute, options.profile.name, stableStringify(options.profile), options.draftUnsaved);
    this.set({ runs: { ...this.runs(), [id]: run }, error: null });
    const request: StartServiceRequest = { profile: options.profile, path_hex: options.pathHex, execute: options.execute, software_mode: options.softwareMode };
    try {
      await this.invoke<string>('start_service', { request });
      return true;
    } catch (reason) {
      const message = errorMessage(reason);
      this.update(id, current => current.state === 'starting' ? { ...current, state: 'failed', error: message } : current, true);
      return false;
    }
  }

  async stop(serviceId: string) {
    if (!this.state.desktop) return;
    this.update(serviceId, run => run.state === 'starting' || run.state === 'running' ? { ...run, state: 'stopping' } : run, true);
    try { await this.invoke<void>('stop_service', { serviceId }); }
    catch (reason) { this.set({ error: errorMessage(reason) }); }
  }

  dismissError() { this.set({ error: null }); }

  dispose() {
    this.connected = false;
    this.pending = null;
    this.unlisteners.forEach(unlisten => unlisten());
    this.unlisteners = [];
  }
}
