import type { HeldProfile, LiveDevice, Profile, ServiceModule, ServiceRecordPayload, ServiceState, ServiceStatusPayload, SessionPhase, SessionReason, SessionStatePayload, StartServiceRequest, SwitchedProfile } from './contracts';
import { timerDuration, urlOrigin } from './action-model';
import { stableStringify } from './draft-model';
import type { Listen, Scheduler } from './device-feed';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

export type ActivityKind = 'event' | 'action' | 'action_failed' | 'info' | 'error' | 'raw';
// `seq` is assigned when the runner appends the entry: it increases by one per entry
// within a run and survives trimming, so it is a stable React key for the log.
export interface ActivityEntry { kind: ActivityKind; text: string; at: number; seq?: number }
// failed matches service_end.failed_action_count; failed timer alerts (not tied to a press) are counted on their own.
export interface ServiceCounters { events: number; planned: number; executed: number; failed: number; timerAlertsFailed: number }
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
  // The serialized profile the run uses: '' for a run the frontend never saw start (always reads as stale), or
  // unknownSnapshot once a backend state switched it to a profile whose body the frontend does not hold.
  snapshot: string;
  // The module start() was given, so a session event without a process (a start while asleep) finds this run.
  startedOn?: { module: ServiceModule; serialNumber: string | null };
  draftUnsaved: boolean;
  // The saved profile id the run uses, when known. Null for a run started from an unsaved draft, which differs
  // from its saved profile. Focused-app rules skip runs already on the matched profile.
  profileId?: string | null;
  // The session profile generation that profileName describes (0 = the profile the run started with).
  // resume_session names it, so a confirmation never starts a profile the dialog did not show.
  profileGeneration?: number;
  // A focused-app rule or a profile key switched this run to a saved profile; set by switchProfile and by session
  // events, and kept for later pauses.
  profileSwitched?: boolean;
  // What made the latest switch: a profile key ('key', a session event with reason profile_action) or a focused-app
  // rule ('rule'). Unset when not known.
  profileSwitchedBy?: 'rule' | 'key';
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
export const reasonLabels: Record<SessionReason, string> = { sleep: 'the Mac went to sleep', device_removed: 'the module was unplugged', service_exited: 'the service exited', user: 'stopped by you', profile_switch: 'a focused-app rule switched the profile', profile_action: 'a profile key switched the profile' };
const isProfileReason = (reason: SessionReason | null) => reason === 'profile_switch' || reason === 'profile_action';

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
    case 'reconnecting': return info(isProfileReason(next.reason) ? 'Restarting with the new profile' : `Reconnecting (attempt ${next.attempt}/${maxReconnectAttempts})`);
    case 'needs_confirmation': return info(isProfileReason(next.reason) ? 'Paused for the new profile: confirm to resume actions' : 'Paused: confirm to resume actions');
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
// The event carries no profile body, so a newer switched profile leaves the snapshot unknown (isStale skips the run).
function adoptProfile(run: ServiceRun, payload: SessionStatePayload): Partial<ServiceRun> {
  const generation = payload.profile_generation ?? 0;
  if (typeof payload.profile_name !== 'string' || generation < (run.profileGeneration ?? 0)) return {};
  const switched = generation > 0 && typeof payload.profile_id === 'string';
  const newer = generation > (run.profileGeneration ?? 0);
  return {
    profileName: payload.profile_name, profileGeneration: generation,
    ...(switched ? { profileId: payload.profile_id, profileSwitched: true, draftUnsaved: false } : {}),
    // A newer switch says who made it when the event carries the reason; otherwise who switched is not known.
    ...(switched && newer ? { snapshot: unknownSnapshot, profileSwitchedBy: payload.reason === 'profile_action' ? 'key' as const : payload.reason === 'profile_switch' ? 'rule' as const : undefined } : {}),
  };
}
const normalSerial = (serial: string | null | undefined) => serial?.trim() || null;
// A run start() keyed `pending:` that still waits for its session: none named it yet and its process has not ended. Its
// process can already be 'running': start_service reports the process status before the session's first state event.
const waitsForSession = (key: string, run: ServiceRun | undefined): run is ServiceRun => key.startsWith('pending:') && run !== undefined && !run.session && isActiveState(run.state);
// A waiting run that was started on this module. A run keyed by its own service id is named by its session directly
// (or by the start result), so it is never matched by device and two serial-less runs of one module cannot be swapped.
const startedOnMatches = (key: string, run: ServiceRun, payload: SessionStatePayload) => waitsForSession(key, run) && run.startedOn !== undefined
  && run.startedOn.module === payload.module && normalSerial(run.startedOn.serialNumber) === normalSerial(payload.serial_number);
export const emptyCounters = (): ServiceCounters => ({ events: 0, planned: 0, executed: 0, failed: 0, timerAlertsFailed: 0 });

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
    // Only the origin: paths and queries can carry tokens.
    case 'open_url': return `open URL ${clip(typeof action.origin === 'string' ? action.origin : urlOrigin(String(action.url)), 120)}`;
    case 'media': return `media ${String(action.command)}`;
    case 'system': return `system ${String(action.command)}`;
    // The snippet itself is not written to the activity log.
    case 'text': return `paste text (${typeof action.chars === 'number' ? action.chars : (typeof action.text === 'string' ? action.text.length : 0)} characters)`;
    case 'profile': return `${action.direction === 'previous' ? 'previous' : 'next'} profile`;
    // The service logs only the label's length, never its text.
    case 'timer': return `timer ${typeof action.seconds === 'number' ? timerDuration(action.seconds) : '?'}`;
    // Only the name's length: the service logs `scene_chars`, older records the name itself.
    case 'obs_scene': return `OBS scene (${typeof action.scene_chars === 'number' ? action.scene_chars : (typeof action.scene === 'string' ? [...action.scene].length : 0)} characters)`;
    // The op is a closed word (toggle, start, stop, save), never user text.
    case 'obs_virtual_camera': case 'obs_replay_buffer': return `${action.type === 'obs_virtual_camera' ? 'OBS virtual camera' : 'OBS replay buffer'} ${str(action.op) ?? '?'}`;
    case 'multi_action': return Array.isArray(action.actions) ? `multi-action [${action.actions.slice(0, 8).map(step => isObject(step) && step.type === 'multi_action' ? 'multi-action' : describeAction(step)).join(', ')}]` : 'multi-action';
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
      const function_ = isObject(record.dial_function) ? str(record.dial_function.name) : undefined;
      const text = describeEvent(record.event) + (function_ ? ` [${function_}]` : '');
      switch (record.status) {
        case 'dial_function': {
          const change = isObject(record.navigation) ? record.navigation : {};
          return entry('event', `Dial function: ${str(change.from_function) ?? '?'} → ${str(change.to_function) ?? '?'} (${typeof change.index === 'number' ? change.index + 1 : '?'}/${String(change.count ?? '?')})`);
        }
        case 'planned': return entry('action', `${text}: planned ${describeAction(record.action)}`);
        case 'navigation': return entry('event', `${text}: folder navigation`);
        case 'unbound': return entry('event', `${text}: no action bound`);
        default: return entry('event', text);
      }
    }
    case 'action_executed': return entry('action', `Executed ${describeAction(record.action)} (${describeEvent(record.event)}${isObject(record.dial_function) && str(record.dial_function.name) ? `, ${str(record.dial_function.name)}` : ''}).`);
    case 'action_failed': return entry('action_failed', `${describeAction(record.action)} failed: ${clip(str(record.error) ?? 'unknown error')}`);
    // A profile key: the session manager switches the run when there is another saved profile to switch to.
    case 'profile_switch_requested': return entry('action', `Profile key: ${record.direction === 'previous' ? 'previous' : 'next'} profile requested (${describeEvent(record.event)}).`);
    case 'action_unavailable': return entry('info', `${describeAction(record.action)} not available: ${clip(str(record.reason) ?? 'no action was taken')}`);
    case 'timer_started': return entry('action', `Started ${describeAction(record.action)} (${describeEvent(record.event)}).`);
    case 'timer_cancelled': return entry('action', `Cancelled ${describeAction(record.action)} (${record.reason === 'pressed_again' ? 'pressed again' : 'run ended'}).`);
    case 'timer_expired': return entry('action', `Countdown ended: ${describeAction(record.action)}.`);
    case 'timer_alert_failed': return entry('action_failed', `Countdown alert failed (${describeAction(record.action)}): ${clip(str(record.error) ?? 'unknown error')}`);
    case 'timer_alert_warning': return entry('info', `Countdown alert warning (${describeAction(record.action)}): ${clip(str(record.warning) ?? 'no details')}`);
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

// A timer press is never an action_executed record: in an executing run its timer_started, or its pressed_again
// timer_cancelled, counts as executed instead, so a healthy timer run does not look unfinished. Failed counts only
// action_failed, exactly like service_end.failed_action_count; a failed timer alert (which happens long after the
// press that counted as executed) has its own counter.
export function countRecord(counters: ServiceCounters, record: unknown, execute = true): ServiceCounters {
  if (!isObject(record)) return counters;
  switch (record.type) {
    case 'event': return { ...counters, events: counters.events + 1, planned: counters.planned + (record.status === 'planned' ? 1 : 0) };
    case 'action_executed': return { ...counters, executed: counters.executed + 1 };
    case 'timer_started': return execute ? { ...counters, executed: counters.executed + 1 } : counters;
    case 'timer_cancelled': return execute && record.reason === 'pressed_again' ? { ...counters, executed: counters.executed + 1 } : counters;
    case 'action_failed': return { ...counters, failed: counters.failed + 1 };
    case 'timer_alert_failed': return { ...counters, timerAlertsFailed: counters.timerAlertsFailed + 1 };
    default: return counters;
  }
}

// The service uses the draft as it was at start and never hot-reloads. A run a backend state switched to a profile whose
// body the frontend does not hold is never stale. Distinct from '' (a run the frontend never saw start), which is.
export const unknownSnapshot = '\u0000unknown';
export const isStale = (run: ServiceRun | undefined, draft: Profile) => run !== undefined && isActiveRun(run) && run.snapshot !== unknownSnapshot && stableStringify(draft) !== run.snapshot;

// sourceProfileId: the saved profile the draft was opened from, even when edited or deleted since. The backend refuses Execute
// for a derivative of an imported dry-run-only profile.
export type StartOptions = { profile: Profile; pathHex: string; execute: boolean; softwareMode: boolean; draftUnsaved: boolean; profileId?: string | null; sourceProfileId?: string | null; device?: LiveDevice };
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
  // Session id start_service returned -> the `pending:` run waiting for it, while the old run of that name has not ended
  // yet (the reply overtook the old session's 'stopped' event). Its first non-stopped event adopts the pending run.
  private readonly awaiting = new Map<string, string>();
  // Session ids whose pending run replaced a run that had not reported 'stopped' yet: that late 'stopped' belongs to the
  // replaced session and must not end the adopted one.
  private readonly staleStop = new Set<string>();

  constructor(private readonly invoke: Invoke, private readonly listen: Listen, desktop: boolean,
    private readonly schedule: Scheduler = frame, private readonly now: () => number = Date.now) {
    this.state = { ...idleServiceState, desktop };
  }

  // Longest wait for the newer session state after a PROFILE_CHANGED refusal.
  profileChangeWaitMs = 1000;

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
      return { ...run, entries: appendEntry(run.entries, entry), counters: countRecord(run.counters, payload.record, run.execute) };
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
    if (payload.phase === 'stopped' && this.staleStop.delete(id)) {
      const current = this.runs()[id]?.session;
      if (current && current.phase !== 'stopped' && (payload.module !== current.module || normalSerial(payload.serial_number) !== normalSerial(current.serialNumber)
        || (typeof payload.service_id === 'string' && payload.service_id !== current.serviceId))) return;
    }
    const existing = this.runs()[id];
    // A new session reusing the name of a run that ended (or, defensively, one still on another process): its run is
    // the `pending:` one start() began, never the old run with its log, counters and profile.
    const waiting = existing && payload.phase !== 'stopped' ? this.waitingRun(id, existing, payload) : undefined;
    if (waiting) {
      const runs = { ...this.runs() };
      delete runs[id];
      this.pending = runs;
      this.adopt(waiting, id);
      this.awaiting.delete(id);
      if (isActiveRun(existing)) this.staleStop.add(id);
    }
    if (!this.runs()[id]) {
      // A run started here waits under the key start() chose until its session names it, which may be
      // `session:<uuid>` when its first service id is still the name of a session that moved elsewhere. A session
      // started while asleep has no process yet; its run is the one start() began on the same module.
      const provisional = typeof payload.service_id === 'string' ? this.sessionOf.get(payload.service_id)
        : Object.entries(this.runs()).find(([key, run]) => startedOnMatches(key, run, payload))?.[0];
      if (!this.adopt(provisional, id)) this.pending = { ...this.runs(), [id]: this.blankRun(id, payload.execute, 'Unknown profile', '', false) };
    }
    // The old run's 'stopped' leaves the path's process routed to the pending run that waits for the new session, and so
    // does any other event of the old session: only an event on the path's own process takes it back from a pending run
    // that still waits.
    const owner = this.sessionOf.get(id);
    const ownedElsewhere = owner !== undefined && owner !== id && this.runs()[owner] !== undefined;
    const pendingOwner = ownedElsewhere && waitsForSession(owner, this.runs()[owner]);
    if (!ownedElsewhere || (payload.phase !== 'stopped' && !(pendingOwner && payload.service_id !== id))) this.sessionOf.set(id, id);
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

  // The session-less `pending:` run a new session named `id` belongs to, while a run of that name already exists: the
  // one start_service answered with `id`, else one started on the event's process (or, without one, its module).
  private waitingRun(id: string, existing: ServiceRun, payload: SessionStatePayload): string | undefined {
    const runs = this.runs();
    const isWaiting = (key: string | undefined): key is string => key !== undefined && waitsForSession(key, runs[key]);
    const awaited = this.awaiting.get(id);
    if (isActiveRun(existing)) {
      // Only after the backend confirmed the name, and only for an event on the pending run's own process.
      return existing.session && isWaiting(awaited) && typeof payload.service_id === 'string' && payload.service_id !== existing.session.serviceId
        && runs[awaited].processId === payload.service_id ? awaited : undefined;
    }
    if (isWaiting(awaited)) return awaited;
    return Object.keys(runs).find(key => isWaiting(key)
      && (typeof payload.service_id === 'string' ? runs[key].processId === payload.service_id : startedOnMatches(key, runs[key], payload)));
  }

  private blankRun(serviceId: string, execute: boolean, profileName: string, snapshot: string, draftUnsaved: boolean): ServiceRun {
    return { serviceId, state: 'starting', execute, entries: [], counters: emptyCounters(), startedAt: this.now(), profileName, snapshot, draftUnsaved };
  }

  // Moves the session-less run start() keyed `provisional` to its session id (staged in pending, not published).
  // When session events already created a run under that id, the two are merged: the started run keeps its profile,
  // log and counters, the session run contributes the session state. False when there is nothing to adopt.
  private adopt(provisional: string | undefined, sessionId: string): boolean {
    const started = provisional === undefined ? undefined : this.runs()[provisional];
    if (provisional === undefined || provisional === sessionId || !started || started.session) return false;
    const named = this.runs()[sessionId];
    let run: ServiceRun = { ...started, serviceId: sessionId };
    if (named) {
      const newer = (named.profileGeneration ?? 0) > (started.profileGeneration ?? 0);
      run = {
        ...run, session: named.session, state: named.state, execute: named.execute,
        entries: named.entries.reduce((entries, entry) => appendEntry(entries, entry), started.entries),
        counters: { events: started.counters.events + named.counters.events, planned: started.counters.planned + named.counters.planned,
          executed: started.counters.executed + named.counters.executed, failed: started.counters.failed + named.counters.failed,
          timerAlertsFailed: started.counters.timerAlertsFailed + named.counters.timerAlertsFailed },
        ...(newer ? { profileName: named.profileName, profileGeneration: named.profileGeneration, profileId: named.profileId,
          profileSwitched: named.profileSwitched, profileSwitchedBy: named.profileSwitchedBy, snapshot: named.snapshot, draftUnsaved: named.draftUnsaved } : {}),
      };
    }
    const runs = { ...this.runs(), [sessionId]: run };
    delete runs[provisional];
    this.pending = runs;
    this.sessionOf.forEach((session, key) => { if (session === provisional) this.sessionOf.set(key, sessionId); });
    this.sessionOf.set(sessionId, sessionId);
    this.forgetAwaiting(provisional);
    return true;
  }

  // Drops every `awaiting` entry that targets `key`, a run that was adopted or replaced.
  private forgetAwaiting(key: string) {
    this.awaiting.forEach((target, name) => { if (target === key) this.awaiting.delete(name); });
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
    this.staleStop.delete(key);
    this.forgetAwaiting(key);
    // An unsaved draft is not its saved profile, so a focused-app rule for that profile still switches the run.
    const profileId = options.draftUnsaved ? null : options.profileId ?? null;
    const startedOn = options.device ? { module: options.device.module, serialNumber: options.device.serial_number } : undefined;
    const run = { ...this.blankRun(key, options.execute, options.profile.name, stableStringify(options.profile), options.draftUnsaved), processId: id, profileId, profileGeneration: 0, ...(startedOn ? { startedOn } : {}) };
    this.set({ runs: { ...this.runs(), [key]: run }, error: null });
    const request: StartServiceRequest = { profile: options.profile, path_hex: options.pathHex, execute: options.execute, software_mode: options.softwareMode, ...(profileId ? { profile_id: profileId } : {}), ...(options.sourceProfileId ? { source_profile_id: options.sourceProfileId } : {}) };
    try {
      // The result is the session id (or the service id of an unmanaged run). A run still waiting under its own key
      // moves to it now, or, when the old run of that name has not reported 'stopped' yet, its new session's first
      // event moves it (see `awaiting`), so it never stays 'starting' on a path it does not own.
      const started = await this.invoke<string>('start_service', { request });
      if (typeof started !== 'string' || started === key) return true;
      const named = this.runs()[started];
      // The path's own service id while this run waits under `pending:`. A run still named after a live session whose
      // process is elsewhere keeps the name (this is an unmanaged run, or the backend's events are behind: its 'stopped'
      // is still on its way) and this run keeps its key until its session's first event adopts it. A stopped one is
      // replaced, as on any new start; one its new session already reported is merged.
      if (started === id && named && isActiveRun(named) && named.session?.serviceId !== id) {
        // Not when the new session's first event already adopted this run (its key is gone).
        if (named.session && waitsForSession(key, this.runs()[key])) this.awaiting.set(started, key);
        return true;
      }
      if (started === id && named && !isActiveRun(named)) {
        const runs = { ...this.runs() };
        delete runs[id];
        this.pending = runs;
      }
      this.adopt(key, started);
      if (this.pending) this.set({});
      return true;
    } catch (reason) {
      const message = errorMessage(reason);
      const target = this.sessionOf.get(id) ?? key;
      // A pending run that failed to start gives the path's routing back, so it never receives the old session's events.
      if (key !== id && target === key) this.sessionOf.delete(id);
      this.update(target, current => current.state === 'starting' ? { ...current, state: 'failed', error: message } : current, true);
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
      // A switch without a profile body (a profile key, or a reply that never arrived): fetch the exact profile the
      // backend holds for this generation, so the dialog lists what resuming really runs. Without it no dialog opens:
      // a confirmation must never cover contents the user could not see.
      if (shown.snapshot === unknownSnapshot) {
        try {
          const held = await this.invoke<HeldProfile>('session_profile', { sessionId, profileGeneration: generation });
          this.update(sessionId, run => (run.profileGeneration ?? 0) === held.profile_generation && run.snapshot === unknownSnapshot
            ? { ...run, snapshot: stableStringify(held.profile), profileName: held.profile.name, profileId: held.profile_id ?? run.profileId } : run, true);
        } catch (reason) {
          if (errorMessage(reason) === profileChangedError) { await this.newerProfile(sessionId, generation); continue; }
          this.set({ error: `Could not load the profile this run holds, so it cannot be shown before resuming (${errorMessage(reason)}). Try again.` });
          return false;
        }
        // The generation moved while loading: ask again for the newer one.
        if (this.runs()[sessionId]?.snapshot === unknownSnapshot) continue;
      }
      if (!(await confirm())) return false;
      const current = this.runs()[sessionId];
      if (current?.session?.phase !== 'needs_confirmation') return false;
      if ((current.profileGeneration ?? 0) !== generation || (current.session.profileGeneration ?? 0) > generation) continue;
      try { await this.invoke<void>('resume_session', { sessionId, profileGeneration: generation }); return true; }
      catch (reason) {
        if (errorMessage(reason) === profileChangedError) { await this.newerProfile(sessionId, generation); continue; }
        this.set({ error: errorMessage(reason) });
        return false;
      }
    }
    this.set({ error: 'The profile of this run kept changing. Review it and resume again.' });
    return false;
  }

  // After PROFILE_CHANGED the backend has already re-sent its state, but a Tauri event can arrive after the command's
  // reply. Waits (bounded by profileChangeWaitMs) until the run names a generation past the refused one, so the retry
  // asks about the newer profile instead of retrying the stale one at once.
  private async newerProfile(sessionId: string, stale: number) {
    const moved = () => {
      const run = this.runs()[sessionId];
      return !run || (run.profileGeneration ?? 0) > stale || (run.session?.profileGeneration ?? 0) > stale;
    };
    for (let waited = 0; !moved() && waited < this.profileChangeWaitMs; waited += 10) await new Promise(resolve => setTimeout(resolve, 10));
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
        profileGeneration: switched.profile_generation, profileSwitched: true, profileSwitchedBy: 'rule',
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
