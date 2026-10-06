import type { LiveDevice, Profile, ServiceModule, ServiceRecordPayload, ServiceState, ServiceStatusPayload, SessionPhase, SessionReason, SessionStatePayload, StartServiceRequest, SwitchedProfile } from './contracts';
import { stableStringify } from './draft-model';
import type { Listen, Scheduler } from './device-feed';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

export type ActivityKind = 'event' | 'action' | 'action_failed' | 'info' | 'error' | 'raw';
// `seq` is assigned when the runner appends the entry: it increases by one per entry
// within a run and survives trimming, so it is a stable React key for the log.
export interface ActivityEntry { kind: ActivityKind; text: string; at: number; seq?: number }
export interface ServiceCounters { events: number; planned: number; executed: number; failed: number }
// Sleep/wake and reconnect state for one run. The run is keyed by the stable session id; serviceId here is the current process.
export interface SessionInfo {
  serviceId: string | null; phase: SessionPhase; reason: SessionReason | null; attempt: number;
  module: ServiceModule; serialNumber: string | null; detail?: string; nextAttemptAt?: number;
  // The backend's latest profile generation for this session (missing = 0).
  profileGeneration?: number;
}
export interface ServiceRun {
  // The run key: the session id once a session names the run, before that the key start() chose.
  serviceId: string;
  // The process id start_service runs, until the run's session reports its current one.
  processId?: string;
  session?: SessionInfo;
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
  // The saved profile id the run uses, when known. Null for a run started from an unsaved draft, which differs
  // from its saved profile. Focused-app rules skip runs already on the matched profile.
  profileId?: string | null;
  // The session profile generation that profileName describes (0 = the profile the run started with).
  // resume_session names it, so a confirmation never starts a profile the dialog did not show.
  profileGeneration?: number;
  // A focused-app rule switched this run to a saved profile; set by switchProfile and kept for later pauses.
  profileSwitched?: boolean;
}
export interface ServiceStoreState { desktop: boolean; runs: Record<string, ServiceRun>; error: string | null }

export const maxEntries = 200;
const appendEntry = (entries: ActivityEntry[], entry: ActivityEntry | null): ActivityEntry[] =>
  entry ? [...entries, { ...entry, seq: (entries.at(-1)?.seq ?? -1) + 1 }].slice(-maxEntries) : entries;
export const idleServiceState: ServiceStoreState = { desktop: false, runs: {}, error: null };
export const serviceIdFor = (pathHex: string) => `service:${pathHex.toLowerCase()}`;
export const isActiveState = (state: ServiceState) => state === 'starting' || state === 'running' || state === 'stopping';
// A session that is suspended, waiting, reconnecting or needs confirmation still owns its device. Only 'stopped' releases it.
export const isActiveRun = (run: ServiceRun | undefined): boolean => run !== undefined && (run.session ? run.session.phase !== 'stopped' : isActiveState(run.state));
export const maxReconnectAttempts = 5;
// resume_session refuses with this (session::PROFILE_CHANGED) when the profile changed after the dialog opened.
export const profileChangedError = "the session's profile changed after the confirmation was shown; review the new profile and confirm again";
export const reasonLabels: Record<SessionReason, string> = { sleep: 'the Mac went to sleep', device_removed: 'the module was unplugged', service_exited: 'the service exited', user: 'stopped by you', profile_switch: 'a focused-app rule switched the profile' };

// The process a run has now. A session owns only its current process's path, never the path its name came from.
const processOf = (key: string, run: ServiceRun) => run.session ? run.session.serviceId : run.processId ?? key;
const ownsProcess = (runs: Record<string, ServiceRun>, id: string) => Object.entries(runs).some(([key, run]) => isActiveRun(run) && processOf(key, run) === id);

// The run a device belongs to: a live session (matched by current process or module and serial) first, then a run
// started on its path that is not a live session elsewhere.
export function findRun(runs: Record<string, ServiceRun>, device: LiveDevice | undefined): ServiceRun | undefined {
  const open = Object.values(runs).filter(run => run.session && run.session.phase !== 'stopped');
  if (!device) return open[0];
  const id = serviceIdFor(device.path_hex);
  return open.find(run => run.session!.serviceId === id || sessionMatches(run.session!, device))
    ?? Object.entries(runs).find(([key, run]) => !open.includes(run) && (run.processId ?? key) === id)?.[1];
}
const sessionMatches = (session: SessionInfo, device: LiveDevice) => session.module === device.module && (session.serialNumber === null || session.serialNumber === device.serial_number);

function sessionEntry(previous: SessionInfo | undefined, next: SessionInfo, at: number): ActivityEntry | null {
  if (previous && previous.phase === next.phase && previous.attempt === next.attempt) return null;
  const info = (text: string): ActivityEntry => ({ kind: 'info', text, at });
  switch (next.phase) {
    case 'suspended': return info(next.reason === 'sleep' || next.reason === null ? 'Mac went to sleep — actions paused' : `Paused: ${reasonLabels[next.reason]}`);
    case 'waiting_for_device': return info(`Waiting for ${next.module} to reconnect`);
    case 'reconnecting': return info(next.reason === 'profile_switch' ? 'Restarting with the new profile' : `Reconnecting (attempt ${next.attempt}/${maxReconnectAttempts})`);
    case 'needs_confirmation': return info(next.reason === 'profile_switch' ? 'Paused for the new profile: confirm to resume actions' : 'Paused: confirm to resume actions');
    case 'stopped': {
      const text = `Session stopped: ${next.reason ? reasonLabels[next.reason] : 'ended'}${next.detail ? ` (${clip(next.detail)})` : ''}`;
      return { kind: next.reason === 'user' || next.reason === null ? 'info' : 'error', text, at };
    }
    default: return previous && previous.phase !== 'running' ? info('Reconnected') : null;
  }
}
// The backend's profile for the backend's generation, from a session://state payload. A switch reply that timed out or a
// webview reload leaves the run behind; every state event resyncs it, so the Resume dialog always names the profile
// resume_session will start. Never moves back to an older generation (an event that overtook a newer switch reply).
function adoptProfile(run: ServiceRun, payload: SessionStatePayload): Partial<ServiceRun> {
  const generation = payload.profile_generation ?? 0;
  if (typeof payload.profile_name !== 'string' || generation < (run.profileGeneration ?? 0)) return {};
  const switched = generation > 0 && typeof payload.profile_id === 'string';
  return {
    profileName: payload.profile_name, profileGeneration: generation,
    ...(switched ? { profileId: payload.profile_id, profileSwitched: true, draftUnsaved: false } : {}),
  };
}
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
export const isStale = (run: ServiceRun | undefined, draft: Profile) => run !== undefined && isActiveRun(run) && stableStringify(draft) !== run.snapshot;

export type StartOptions = { profile: Profile; pathHex: string; execute: boolean; softwareMode: boolean; draftUnsaved: boolean; profileId?: string | null };
export type ConfirmExecute = () => Promise<boolean>;

interface QueuedSwitch { profileId: string; why: string; settle: (ok: boolean) => void }
interface SwitchQueue { done: Promise<void>; queued: QueuedSwitch | null }

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
  // Every process id seen in a session event -> the stable session id that owns its log.
  private readonly sessionOf = new Map<string, string>();
  // Run key -> the newest profile id a focused-app switch was requested for, so rapid focus changes compare against it.
  private readonly switching = new Map<string, string>();
  // Run key -> its serialized switches: one request in flight, only the newest waiting.
  private readonly switchQueues = new Map<string, SwitchQueue>();

  constructor(private readonly invoke: Invoke, private readonly listen: Listen, desktop: boolean,
    private readonly schedule: Scheduler = frame, private readonly now: () => number = Date.now) {
    this.state = { ...idleServiceState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;
  // A device is owned by the run whose current process is on its path, or by a session that has no
  // process right now but will reclaim the same module.
  isActivePath = (pathHex: string, device?: LiveDevice): boolean => {
    const runs = this.runs();
    if (ownsProcess(runs, serviceIdFor(pathHex))) return true;
    return device !== undefined && Object.values(runs).some(run => run.session && run.session.phase !== 'stopped' && sessionMatches(run.session, device));
  };

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
    add('session://state', payload => this.onSession(payload as SessionStatePayload));
  }

  private onRecord(payload: ServiceRecordPayload) {
    if (!payload || typeof payload.service_id !== 'string') return;
    const id = this.sessionOf.get(payload.service_id) ?? payload.service_id;
    if (!this.runs()[id]) return;
    const at = this.now();
    this.update(id, run => {
      const entry = summarizeRecord(payload.record, at);
      return { ...run, entries: appendEntry(run.entries, entry), counters: countRecord(run.counters, payload.record) };
    }, false);
  }

  private onStatus(payload: ServiceStatusPayload) {
    if (!payload || typeof payload.service_id !== 'string' || typeof payload.state !== 'string') return;
    const id = this.sessionOf.get(payload.service_id) ?? payload.service_id;
    if (!this.runs()[id]) {
      this.pending = { ...this.runs(), [id]: this.blankRun(id, payload.execute, 'Unknown profile', '', false) };
    }
    const terminal = payload.state === 'exited' || payload.state === 'failed';
    const session = this.runs()[id]?.session;
    // A superseded process ending must not end a session that has moved on to a new process.
    if (terminal && session && session.phase !== 'stopped' && payload.service_id !== session.serviceId) return;
    this.update(id, run => ({
      ...run, state: payload.state, execute: payload.execute ?? run.execute,
      ...(payload.exit_code !== undefined ? { exitCode: payload.exit_code } : {}),
      ...(payload.error !== undefined ? { error: payload.error } : {}),
      ...(payload.stderr_tail !== undefined ? { stderrTail: payload.stderr_tail } : {}),
    }), true);
  }

  private onSession(payload: SessionStatePayload) {
    if (!payload || typeof payload.session_id !== 'string' || typeof payload.phase !== 'string') return;
    const id = payload.session_id;
    if (!this.runs()[id]) {
      // A run started here waits under the key start() chose until its session names it, which may be
      // `session:<uuid>` when its first service id is still the name of a session that moved elsewhere.
      const provisional = typeof payload.service_id === 'string' ? this.sessionOf.get(payload.service_id) : undefined;
      const started = provisional === undefined ? undefined : this.runs()[provisional];
      if (provisional !== undefined && started && !started.session) {
        const runs = { ...this.runs(), [id]: { ...started, serviceId: id } };
        delete runs[provisional];
        this.pending = runs;
        this.sessionOf.forEach((session, key) => { if (session === provisional) this.sessionOf.set(key, id); });
      } else this.pending = { ...this.runs(), [id]: this.blankRun(id, payload.execute, 'Unknown profile', '', false) };
    }
    this.sessionOf.set(id, id);
    if (typeof payload.service_id === 'string') this.sessionOf.set(payload.service_id, id);
    const at = this.now();
    this.update(id, run => {
      const session: SessionInfo = {
        serviceId: payload.service_id, phase: payload.phase, reason: payload.reason ?? null, attempt: payload.attempt ?? 0,
        module: payload.module, serialNumber: payload.serial_number ?? null, detail: payload.detail, profileGeneration: payload.profile_generation ?? 0,
        ...(typeof payload.next_attempt_in_ms === 'number' ? { nextAttemptAt: at + payload.next_attempt_in_ms } : {}),
      };
      const entry = sessionEntry(run.session, session, at);
      const state: ServiceState = payload.phase === 'stopped' ? (run.state === 'failed' ? 'failed' : 'exited') : payload.phase === 'running' && run.state !== 'stopping' ? 'running' : run.state;
      return { ...run, ...adoptProfile(run, payload), session, state, execute: payload.execute ?? run.execute, entries: appendEntry(run.entries, entry) };
    }, true);
  }

  private blankRun(serviceId: string, execute: boolean, profileName: string, snapshot: string, draftUnsaved: boolean): ServiceRun {
    return { serviceId, state: 'starting', execute, entries: [], counters: emptyCounters(), startedAt: this.now(), profileName, snapshot, draftUnsaved };
  }

  // Execute mode never starts without an explicit confirmation callback that resolves true.
  async start(options: StartOptions, confirm?: ConfirmExecute): Promise<boolean> {
    if (!this.state.desktop) return false;
    const id = serviceIdFor(options.pathHex);
    if (ownsProcess(this.runs(), id)) { this.set({ error: 'A profile is already running on this device.' }); return false; }
    if (options.execute) {
      if (!confirm) { this.set({ error: 'Running actions needs confirmation.' }); return false; }
      if (!(await confirm())) return false;
      if (ownsProcess(this.runs(), id)) return false;
    }
    // A live session that moved away from this path keeps its name, so this run waits under its own key.
    const key = isActiveRun(this.runs()[id]) ? `pending:${id}` : id;
    this.sessionOf.forEach((session, process) => { if (session === key) this.sessionOf.delete(process); });
    this.sessionOf.set(id, key);
    // An unsaved draft is not its saved profile, so a focused-app rule for that profile still switches the run.
    const profileId = options.draftUnsaved ? null : options.profileId ?? null;
    const run = { ...this.blankRun(key, options.execute, options.profile.name, stableStringify(options.profile), options.draftUnsaved), processId: id, profileId, profileGeneration: 0 };
    this.set({ runs: { ...this.runs(), [key]: run }, error: null });
    const request: StartServiceRequest = { profile: options.profile, path_hex: options.pathHex, execute: options.execute, software_mode: options.softwareMode };
    try {
      await this.invoke<string>('start_service', { request });
      return true;
    } catch (reason) {
      const message = errorMessage(reason);
      this.update(this.sessionOf.get(id) ?? key, current => current.state === 'starting' ? { ...current, state: 'failed', error: message } : current, true);
      return false;
    }
  }

  // serviceId is the run key. A managed run with no live process (suspended, waiting, reconnecting,
  // needs confirmation) is ended through stop_session; otherwise the process is stopped as before.
  async stop(serviceId: string) {
    if (!this.state.desktop) return;
    const run = this.runs()[serviceId];
    const session = run?.session;
    if (session && session.phase !== 'stopped' && (session.phase !== 'running' || session.serviceId === null)) { await this.stopSession(serviceId); return; }
    const target = (run && processOf(serviceId, run)) ?? serviceId;
    this.update(serviceId, run => run.state === 'starting' || run.state === 'running' ? { ...run, state: 'stopping' } : run, true);
    try { await this.invoke<void>('stop_service', { serviceId: target }); }
    catch (reason) { this.set({ error: errorMessage(reason) }); }
  }

  async stopSession(sessionId: string) {
    if (!this.state.desktop) return;
    try { await this.invoke<void>('stop_session', { sessionId }); }
    catch (reason) { this.set({ error: errorMessage(reason) }); }
  }

  // Restarts a paused run with its stored request, including execute. Never without a confirmation that resolves true.
  // The confirmation is tied to the profile generation the dialog showed: when a switch replaces the profile while the
  // dialog is open, the user is asked again for the new profile, and the backend refuses a stale generation too.
  async resumeSession(sessionId: string, confirm?: ConfirmExecute): Promise<boolean> {
    if (!this.state.desktop) return false;
    if (this.runs()[sessionId]?.session?.phase !== 'needs_confirmation') return false;
    if (!confirm) { this.set({ error: 'Resuming actions needs confirmation.' }); return false; }
    for (let attempt = 0; attempt < 3; attempt += 1) {
      // A switch reply still on its way would leave profileName behind the backend's generation.
      await this.switchQueues.get(sessionId)?.done;
      const shown = this.runs()[sessionId];
      if (shown?.session?.phase !== 'needs_confirmation') return false;
      const generation = shown.profileGeneration ?? 0;
      if ((shown.session.profileGeneration ?? 0) > generation) { this.set({ error: 'The profile of this run changed. Review it and resume again.' }); return false; }
      if (!(await confirm())) return false;
      const current = this.runs()[sessionId];
      if (current?.session?.phase !== 'needs_confirmation') return false;
      if ((current.profileGeneration ?? 0) !== generation || (current.session.profileGeneration ?? 0) > generation) continue;
      try { await this.invoke<void>('resume_session', { sessionId, profileGeneration: generation }); return true; }
      catch (reason) {
        if (errorMessage(reason) === profileChangedError) continue;
        this.set({ error: errorMessage(reason) });
        return false;
      }
    }
    this.set({ error: 'The profile of this run kept changing. Review it and resume again.' });
    return false;
  }

  // Switches a managed run to a saved profile because a focused-app rule matched. The session manager restarts a
  // dry run at once; an executing run pauses in needs_confirmation and runs actions again only after resumeSession.
  // Switches of one run are serialized: one request in flight and only the newest waiting (an older waiting request
  // resolves false), so replies arrive in order and the run always names the profile the backend holds.
  async switchProfile(sessionId: string, profileId: string, why: string): Promise<boolean> {
    if (!this.state.desktop) return false;
    const run = this.runs()[sessionId];
    if (!run?.session || run.session.phase === 'stopped') return false;
    if ((this.switching.get(sessionId) ?? run.profileId) === profileId) return false;
    this.switching.set(sessionId, profileId);
    const waiting = this.switchQueues.get(sessionId);
    if (waiting) {
      waiting.queued?.settle(false);
      return new Promise<boolean>(settle => { waiting.queued = { profileId, why, settle }; });
    }
    const queue: SwitchQueue = { done: Promise.resolve(), queued: null };
    this.switchQueues.set(sessionId, queue);
    return new Promise<boolean>(settle => { queue.done = this.drainSwitches(sessionId, queue, { profileId, why, settle }); });
  }

  // Sends one switch at a time; each request settles as soon as its own reply arrived.
  private async drainSwitches(sessionId: string, queue: SwitchQueue, first: QueuedSwitch) {
    try {
      for (let next: QueuedSwitch | null = first; next; next = queue.queued, queue.queued = null) {
        const run = this.runs()[sessionId];
        next.settle(run?.session !== undefined && run.session.phase !== 'stopped' && run.profileId !== next.profileId
          && await this.sendSwitch(sessionId, next.profileId, next.why));
      }
    } finally {
      this.switchQueues.delete(sessionId);
      this.switching.delete(sessionId);
    }
  }

  private async sendSwitch(sessionId: string, profileId: string, why: string): Promise<boolean> {
    try {
      const switched = await this.invoke<SwitchedProfile>('switch_session_profile', { sessionId, profileId });
      const at = this.now();
      this.update(sessionId, current => (current.profileGeneration ?? 0) > switched.profile_generation ? current : ({
        ...current, profileId: switched.id, profileName: switched.profile.name, snapshot: stableStringify(switched.profile), draftUnsaved: false,
        profileGeneration: switched.profile_generation, profileSwitched: true,
        entries: appendEntry(current.entries, { kind: 'info', text: `Switched to profile "${switched.profile.name}" (${why})`, at }),
      }), true);
      return true;
    } catch (reason) {
      this.set({ error: errorMessage(reason) });
      return false;
    }
  }

  dismissError() { this.set({ error: null }); }

  dispose() {
    this.connected = false;
    this.pending = null;
    this.unlisteners.forEach(unlisten => unlisten());
    this.unlisteners = [];
  }
}
