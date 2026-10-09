import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { CircleHelp, LoaderCircle, MoonStar, Play, Square, TriangleAlert } from 'lucide-react';
import type { LiveDevice, ModuleControl, Profile, ServiceModule, ServiceState } from './contracts';
import { SelectField, ModalDialog } from './controls';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { autoStartLabel, idleServiceState, isActiveRun, isStale, maxReconnectAttempts, serviceIdFor } from './service-runner';
import type { ServiceRun, ServiceRunner } from './service-runner';
import { modules } from './ui-model';
import { capabilityLines, profileCapabilities, snapshotProfile, snippetPreview, stepLabel } from './profile-capabilities';
import type { RiskLevel } from './profile-capabilities';
import { appControls, idleDisplaySettingsState } from './display-settings';
import type { DisplaySettingsStore } from './display-settings';

// The Resume actions button, focused after Review on a paused-run notice.
export const serviceResumeId = 'service-resume';

export type PlatformInfo = { platform?: string; userAgent?: string };
export const isMacPlatform = (info: PlatformInfo | undefined) => /mac/i.test(`${info?.platform ?? ''} ${info?.userAgent ?? ''}`);

export const builtInKeypadBlocker = 'This profile has 15-key pages, which run live only when OpenStreamHub drives the module. In Settings, “Let OpenStreamHub drive the display” is set to Built-in animation: set it to App to run this profile.';

// Why Run is unavailable for this selection, or null. Until the Module control settings have loaded nothing runs, and
// settings that cannot be read are named rather than shown as App. The service refuses schema-2 (paged) M4315 profiles
// without display sync, which the app passes only when Module control is App (the default); the session manager refuses
// them with the same reason before spawning anything.
export function runBlocker(input: { desktop: boolean; device: LiveDevice | undefined; profile: Profile; moduleControl?: ModuleControl; settingsLoaded?: boolean; settingsError?: string | null }): string | null {
  if (!input.desktop) return 'Running a profile needs the desktop app. Launch it to run actions.';
  if (!input.device) return 'No module connected. Connect a module to run this profile on it.';
  if (input.settingsLoaded === false) return input.settingsError ? settingsErrorBlocker(input.settingsError) : settingsLoadingBlocker;
  if (input.device.module === 'm4315' && input.profile.schema_version === 2 && input.moduleControl === 'built_in_animation') return builtInKeypadBlocker;
  return null;
}

// The Run panel's software mode box. Software mode exists only on the 15-key module and the dial. Under App module
// control (once loaded) the session manager always enables it there; the box only shows that. `requested` is what goes
// into the start request: what the user ticked, never the displayed value, so the backend decides from the settings it
// read itself (a damaged settings file there never enters software mode, whatever this panel believes).
export function softwareModeBox(input: { module: ServiceModule | undefined; loaded: boolean; moduleControl: ModuleControl; ticked: boolean }) {
  const capable = input.module !== undefined && appControls(input.module);
  const appDriven = capable && input.loaded && input.moduleControl === 'app';
  const requested = capable && input.ticked;
  return { appDriven, requested, checked: appDriven || requested, locked: !input.loaded || !capable || appDriven };
}

export const settingsLoadingBlocker = 'Reading the display settings…';
export const settingsErrorBlocker = (error: string) => `The display settings could not be read, so profiles cannot run yet: ${error} Open Settings for details.`;

// The feed never reads a path a run owns; the live preview shows that module's input from the run's own records instead.
export const runPreviewNote = 'The live preview shows the input this run reads from the module (read-only).';
// The label of a run the session manager started on its own, so it is never taken for a Start by the user. It runs actions
// only while "Run actions automatically" is on; otherwise, running another profile or executing actions needs Stop and then
// the user's own Run. A dry run that should have executed says why.
export function autoStartNote(autoStart: NonNullable<ServiceRun['session']>['autoStart'], execute = false, dryRunReason?: string) {
  if (!autoStart) return null;
  const label = `${autoStartLabel(autoStart, execute)}.`;
  if (execute) return `${label} Actions run without asking. Stop it, or turn off Run actions automatically in Settings, to end them.`;
  const why = dryRunReason ? ` Actions are off for this run: ${dryRunReason}` : '';
  return `${label}${why} Stop it to run another profile on this module or to execute actions.`;
}
// Why the software mode box is ticked and locked for the 15-key module and the dial.
export const appControlNote = '“Let OpenStreamHub drive the display” is set to App in Settings, so OpenStreamHub always runs this module in software mode and keeps its display in sync.';

export const stateLabel = (state: ServiceState | undefined) => state === undefined ? 'Stopped' : state === 'exited' ? 'Stopped' : state[0].toUpperCase() + state.slice(1);

// The visible run state. A live run is named by what it does — Dry run or Executing — never just "Running".
// A session that reported 'stopped' is Stopped even if its last process state has not caught up.
export const phaseLabel = (run: ServiceRun | undefined) => {
  switch (run?.session?.phase) {
    case 'suspended': return 'Paused';
    case 'waiting_for_device': return 'Waiting';
    case 'reconnecting': return 'Reconnecting';
    case 'needs_confirmation': return 'Needs confirmation';
    case 'stopped': return run.state === 'failed' ? 'Failed' : 'Stopped';
    case 'running': return run.state === 'stopping' ? 'Stopping' : run.execute ? 'Executing' : 'Dry run';
    default: return run?.state === 'running' ? (run.execute ? 'Executing' : 'Dry run') : stateLabel(run?.state);
  }
};
const phaseClass = (run: ServiceRun | undefined) => {
  const phase = run?.session?.phase;
  if (phase !== undefined && phase !== 'running') return phase === 'stopped' ? (run!.state === 'failed' ? 'failed' : 'exited') : phase;
  return run?.state ?? 'idle';
};
// The state pill's text and CSS classes. Executing carries its own class so it stands out from a dry run.
export const runStatePill = (run: ServiceRun | undefined) => {
  const label = phaseLabel(run);
  return { label, className: `${phaseClass(run)}${label === 'Executing' ? ' executing' : ''}` };
};
const moduleName = (module: ServiceModule) => modules.find(entry => entry.id === module)?.shortName ?? module;
const sessionDeviceName = (session: { module: ServiceModule; serialNumber: string | null }) => `${moduleName(session.module)}${session.serialNumber ? ` …${session.serialNumber.slice(-6)}` : ''}`;
export const countdownSeconds = (nextAttemptAt: number | undefined, now: number) => nextAttemptAt === undefined ? null : Math.max(0, Math.ceil((nextAttemptAt - now) / 1000));
const stoppedReason = { sleep: 'the Mac went to sleep', device_removed: 'the module was unplugged', service_exited: 'the background helper exited', start_refused: 'the profile could not start', user: 'you stopped it', profile_switch: 'a focused-app rule switched the profile', profile_action: 'a profile key switched the profile', module_control: 'OpenStreamHub no longer drives the display', auto_actions: 'Run actions automatically was turned on or off' } as const;
// The first 8 characters of a saved profile id, enough to tell two profiles of the same name apart.
const shortId = (id: string) => id.length > 8 ? `${id.slice(0, 8)}…` : id;
// Who switched the run, as one phrase for the banner and the Resume dialog.
const switchedTo = (by: ServiceRun['profileSwitchedBy']) => by === 'key' ? 'a profile key switched to' : by === 'rule' ? 'a focused-app rule switched to' : 'this run was switched to';

export interface ConfirmCopy {
  title: string; confirm: string; message: string; risk: RiskLevel;
  // The capabilities in plain language, then every distinct text snippet and multi-step sequence.
  lines: string[]; snippets: { text: string; more: number; pressEnter: boolean }[]; sequences: string[][];
  // Set for a high-risk profile: the warning shown above the details and the label of the box that must be ticked.
  warning: string | null; acknowledge: string | null;
}

const unknownContents = 'This app cannot read the contents of this profile here, so it cannot list what will run. It may paste text, press Return, open web pages, capture the screen or run multi-step sequences. Open Manage profiles in the header to review it before you continue.';
// After a profile key switch the run holds its own copy, which the saved profile it came from may no longer match.
const unknownKeyContents = 'This app could not load the profile this run holds, so it cannot list what will run. It may paste text, press Return, open web pages, capture the screen or run multi-step sequences. Cancel and choose Resume again to retry.';

// Copy for the Execute confirmation, used when starting a run and when resuming a paused one. It discloses what the
// profile will actually do (profileCapabilities over the whole profile). A null profile is one whose contents the
// frontend does not hold (a run it did not start, or one a backend switch moved to another profile): that is treated
// as high risk. A run a focused-app rule or a profile key switched resumes with the saved profile it switched to, not the
// original snapshot. A profile key switch carries no profile body, so its resume is treated as high risk too.
export function confirmCopy(kind: 'start' | 'resume', profileName: string, deviceLabel: string, switched = false, profile: Profile | null = null, switchedBy?: ServiceRun['profileSwitchedBy'], profileId?: string | null): ConfirmCopy {
  const key = kind === 'resume' && switched && switchedBy === 'key';
  // A key switch names the saved profile by id (two profiles can share a name) and says the run holds its own copy.
  const keyNote = key ? ` ${profileId ? `Saved profile "${profileName}", id ${shortId(profileId)}. ` : ''}This is the copy the run took when it switched; later edits to the saved profile do not apply until the next switch.` : '';
  const head = kind === 'start'
    ? { title: 'Run actions on this Mac?', confirm: 'Run actions', message: `Profile "${profileName}" will run on ${deviceLabel}. Everything listed here will really run on this Mac whenever you use the module, without asking again.` }
    : { title: 'Resume actions on this Mac?', confirm: 'Resume actions', message: `Profile "${profileName}" (${switched ? `the saved profile ${switchedTo(switchedBy)}` : 'as it was when this run started'}) will resume on ${deviceLabel}. Everything listed here will really run on this Mac again whenever you use the module, without asking again.${keyNote}` };
  const capabilities = profile ? profileCapabilities(profile) : null;
  const risk: RiskLevel = capabilities?.risk ?? 'high';
  const reasons = capabilities ? capabilities.riskReasons : ['The profile contents could not be checked.'];
  return {
    ...head, risk,
    lines: capabilities ? capabilityLines(capabilities) : [key ? unknownKeyContents : unknownContents],
    snippets: capabilities?.texts.map(snippet => ({ ...snippetPreview(snippet.text), pressEnter: snippet.pressEnter })) ?? [],
    sequences: capabilities?.sequences.map(steps => steps.map(stepLabel)) ?? [],
    warning: risk === 'high' ? `High risk. ${reasons.join(' ')} With one press of a control, a profile like this can type and run commands, or send what is on screen or in the clipboard to a web page. Only continue if you wrote this profile or reviewed every snippet and step below and trust where it came from.` : null,
    acknowledge: risk === 'high' ? 'I reviewed this profile and trust its source' : null,
  };
}

// Resume copy for the run resumeSession holds (by its session id), never the run the device selection shows now:
// the selection can move while the dialog is open (a module plugged in or out). Null once that run is gone.
export function resumeCopy(runs: Record<string, ServiceRun>, sessionId: string | null) {
  const run = sessionId === null ? undefined : runs[sessionId];
  // The run's own snapshot, never the draft: resuming restarts the profile the run holds.
  return run ? confirmCopy('resume', run.profileName, run.session ? sessionDeviceName(run.session) : 'the module', run.profileSwitched === true, snapshotProfile(run.snapshot), run.profileSwitchedBy, run.profileId) : null;
}

// The copy of the open confirmation. Null for a resume whose run disappeared while its dialog was open: the panel then
// answers false and closes the dialog instead of showing start-a-run text for a resume.
export function dialogCopy(kind: 'start' | 'resume', runs: Record<string, ServiceRun>, resumeId: string | null, startCopy: ReturnType<typeof confirmCopy>) {
  return kind === 'resume' ? resumeCopy(runs, resumeId) : startCopy;
}

export function relativeTime(at: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 5) return 'now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  return minutes < 60 ? `${minutes}m ago` : `${Math.floor(minutes / 60)}h ago`;
}

// Answers an open Execute confirmation with false and forgets it.
export function settlePendingConfirm(pending: { current: ((ok: boolean) => void) | null }) {
  const resolve = pending.current;
  pending.current = null;
  resolve?.(false);
}

const noopSubscribe = () => () => {};
const idleFeed = () => idleFeedState;
const idleRunner = () => idleServiceState;
const idleDisplay = () => idleDisplaySettingsState;
const deviceName = (device: LiveDevice) => `${modules.find(entry => entry.id === device.module)?.shortName ?? device.module} ${device.serial_number ?? device.path_hex.slice(-6)}`;

// Retain the selected identity and stable run key while inventory paths disappear or change.
export interface ServiceSelection { device: LiveDevice; sessionId?: string }
export const disconnectedServiceValue = 'disconnected:selected-session';
export function resolveServiceSelection(devices: LiveDevice[], runs: Record<string, ServiceRun>, chosen: ServiceSelection | null) {
  const serial = (value: string | null | undefined) => value?.trim() || null;
  const matches = (identity: { module: ServiceModule; serialNumber: string | null }, device: LiveDevice) =>
    identity.module === device.module && serial(identity.serialNumber) !== null && serial(identity.serialNumber) === serial(device.serial_number);
  const open = Object.values(runs).filter(item => isActiveRun(item) && item.session);
  // A remounted panel can recover one unambiguous session, even with no connected device.
  if (!chosen && open.length === 1) {
    const session = open[0].session!;
    chosen = { device: { module: session.module, serial_number: session.serialNumber,
      path_hex: (session.serviceId ?? open[0].serviceId).replace(/^service:/, '') }, sessionId: open[0].serviceId };
  }
  const selected = chosen?.device ?? devices[0];
  let run = chosen?.sessionId ? runs[chosen.sessionId] : undefined;
  const ended = run && !isActiveRun(run) ? run : undefined;
  if (ended) run = undefined;
  let device: LiveDevice | undefined;
  if (run?.session) {
    const session = run.session;
    // The current process path is authoritative; otherwise require exact serial or one module candidate.
    device = devices.find(item => item.module === session.module && session.serviceId === serviceIdFor(item.path_hex));
    if (!device) {
      const candidates = devices.filter(item => item.module === session.module);
      device = serial(session.serialNumber) === null
        ? candidates.length === 1 ? candidates[0] : undefined
        : candidates.find(item => matches(session, item));
    }
  } else if (selected) {
    device = devices.find(item => item.path_hex === selected.path_hex && item.module === selected.module
      && serial(item.serial_number) === serial(selected.serial_number));
    if (!device && serial(selected.serial_number) !== null) device = devices.find(item => matches({ module: selected.module, serialNumber: selected.serial_number }, item));
  }
  const target = device ?? selected;
  if (!run && target) {
    const id = serviceIdFor(target.path_hex);
    // The runner replaces a pending key when the first session event adopts it. A serial-less
    // module may already be gone by then: follow the original stable key, or one unambiguous
    // active serial-less session, rather than losing Stop or guessing among several sessions.
    const adopted = chosen?.sessionId && !runs[chosen.sessionId] && serial(target.serial_number) === null
      ? open.filter(item => item.session!.module === target.module && serial(item.session!.serialNumber) === null) : [];
    const adoptedRun = adopted.find(item => item.serviceId === id) ?? (adopted.length === 1 ? adopted[0] : undefined);
    run = Object.values(runs).find(item => isActiveRun(item) && item.session && item.session.module === target.module &&
      (item.session.serviceId === id || matches(item.session, target)
        || (serial(item.session.serialNumber) === null && device !== undefined
          && devices.filter(candidate => candidate.module === target.module).length === 1)))
      ?? Object.values(runs).find(item => isActiveRun(item) && !item.session &&
        ((item.processId ?? item.serviceId) === id || (item.startedOn && matches(item.startedOn, target))))
      ?? adoptedRun
      ?? ended
      ?? Object.values(runs).find(item => item.session?.phase === 'stopped' && matches(item.session, target))
      ?? Object.values(runs).find(item => !item.session && (item.processId ?? item.serviceId) === id);
  }
  run ??= ended;
  const selection: ServiceSelection | null = selected ? { device: device ?? selected, sessionId: run?.serviceId ?? chosen?.sessionId } : null;
  return { device, run, selection };
}

// dryRunOnly: the open profile (saved, or a draft of it) was imported from a .mhz file and not yet allowed to execute
// actions. Execute is unavailable; the session manager refuses it too.
export const dryRunOnlyRunNote = 'This profile was imported from a MasterHUB .mhz file, so it runs only as a dry run. Open Manage profiles (next to the profile switcher) to review it and allow it to execute actions first.';
export const runBlockNotes = {
  imported: dryRunOnlyRunNote,
  deleted: 'The saved profile behind these unsaved changes was deleted. Save it to run it with actions.',
  unreadable: 'The list of imported profiles cannot be read, so every profile runs only as a dry run. Open Manage profiles (next to the profile switcher) to fix it first.',
} as const;

export interface RunSessionOptions {
  runner?: ServiceRunner; feed?: DeviceFeed; profile: Profile; profileId?: string | null; sourceProfileId?: string | null;
  desktop: boolean; unsaved: boolean; displaySettings?: DisplaySettingsStore; dryRunOnly?: boolean; dryRunNote?: string;
  platform?: PlatformInfo; now?: number; initialExecute?: boolean;
  // Settles the draft before a start (the pending automatic save), so the run
  // uses the last saved version. It returns the draft identity to start with;
  // without a result the start uses the values this hook was rendered with.
  // The start still goes ahead when settling fails, as it would have without it.
  beforeStart?(): Promise<SettledDraft | void>;
}

// The draft values a start uses after beforeStart settled it. profileId is
// null exactly when the draft is still unsaved, matching the props.
export interface SettledDraft { profile: Profile; profileId: string | null; sourceProfileId?: string | null; unsaved: boolean }

// All state and behaviour of starting, stopping and resuming a run. Lives once in App, so the header
// Run control works on every page and shares its selection and mode with the Devices-page panel.
export function useRunSession({ runner, feed, profile, profileId = null, sourceProfileId = null, desktop, unsaved, displaySettings, dryRunOnly = false, dryRunNote = dryRunOnlyRunNote, platform = typeof navigator === 'undefined' ? undefined : navigator, now: fixedNow, initialExecute = false, beforeStart }: RunSessionOptions) {
  const service = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner);
  const devices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleFeed, feed?.getState ?? idleFeed).devices;
  const display = useSyncExternalStore(displaySettings?.subscribe ?? noopSubscribe, displaySettings?.getState ?? idleDisplay, displaySettings?.getState ?? idleDisplay);
  useEffect(() => { if (displaySettings && display.desktop && !display.loaded) void displaySettings.load(); }, [displaySettings, display.desktop, display.loaded]);
  const [chosen, setChosen] = useState<ServiceSelection | null>(null);
  const [execute, setExecute] = useState(initialExecute);
  const [softwareMode, setSoftwareMode] = useState(false);
  const [confirm, setConfirm] = useState<((ok: boolean) => void) | null>(null);
  const [confirmKind, setConfirmKind] = useState<'start' | 'resume'>('start');
  // The run being resumed. The dialog names it, not whatever run the device selection shows by the time it renders.
  const [resumeId, setResumeId] = useState<string | null>(null);
  // The high-risk acknowledgement. Cleared with every answer, so each confirmation asks again.
  const [acknowledged, setAcknowledged] = useState(false);
  const [clock, setClock] = useState(() => Date.now());
  // A confirmation still open when the app unmounts answers false, so the start or resume never hangs.
  const pendingConfirm = useRef(confirm);
  pendingConfirm.current = confirm;
  useEffect(() => () => settlePendingConfirm(pendingConfirm), []);
  const mac = isMacPlatform(platform);
  const enabled = desktop && runner !== undefined;

  const { device, run, selection } = resolveServiceSelection(devices, service.runs, chosen);
  useEffect(() => {
    if (selection && (chosen?.device.path_hex !== selection.device.path_hex || chosen?.sessionId !== selection.sessionId)) setChosen(selection);
  }, [chosen, selection]);
  const session = run?.session;
  const phase = session?.phase ?? (isActiveRun(run) ? 'running' : undefined);
  const active = isActiveRun(run);
  const blocker = runBlocker({ desktop: enabled, device, profile, moduleControl: display.settings.module_control, settingsLoaded: display.loaded, settingsError: display.error });
  const box = softwareModeBox({ module: device?.module, loaded: display.loaded, moduleControl: display.settings.module_control, ticked: softwareMode });
  const stale = isStale(run, profile);
  const now = fixedNow ?? clock;
  const executeSelected = execute && mac && !dryRunOnly;

  useEffect(() => {
    if (fixedNow !== undefined || !run?.entries.length) return;
    const timer = setInterval(() => setClock(Date.now()), 5000);
    return () => clearInterval(timer);
  }, [run?.entries.length, fixedNow]);
  useEffect(() => {
    if (fixedNow !== undefined || session?.nextAttemptAt === undefined) return;
    const timer = setInterval(() => setClock(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [session?.nextAttemptAt, fixedNow]);

  // A start that settles the draft first must use the settled identity, not
  // the values captured when this closure was created (an automatic save can
  // save the draft while it waits, and the run then carries the profile id).
  const latest = useRef({ runner, profile, profileId, sourceProfileId, unsaved, executeSelected, box, device });
  latest.current = { runner, profile, profileId, sourceProfileId, unsaved, executeSelected, box, device };
  const begin = async () => {
    let settled: SettledDraft | void = undefined;
    try { settled = await beforeStart?.(); } catch { /* the start uses the draft as it is */ }
    const atStart = settled ?? latest.current;
    const current = latest.current;
    if (!current.runner || !current.device) return;
    const confirmExecute = current.executeSelected ? () => new Promise<boolean>(resolve => { setConfirmKind('start'); setConfirm(() => resolve); }) : undefined;
    void current.runner.start({
      profile: atStart.profile, profileId: atStart.profileId, sourceProfileId: atStart.sourceProfileId,
      pathHex: current.device.path_hex, execute: current.executeSelected, softwareMode: current.box.requested,
      draftUnsaved: atStart.unsaved, device: current.device,
    }, confirmExecute);
  };
  const resume = () => {
    if (!runner || !run) return;
    const sessionId = run.serviceId;
    void runner.resumeSession(sessionId, () => new Promise<boolean>(resolve => { setConfirmKind('resume'); setResumeId(sessionId); setConfirm(() => resolve); }));
  };
  const answer = (ok: boolean) => { confirm?.(ok); setConfirm(null); setAcknowledged(false); };

  const startCopy = confirmCopy('start', profile.name, device ? deviceName(device) : 'the selected module', false, profile);
  const dialog = dialogCopy(confirmKind, service.runs, resumeId, startCopy);
  const resumeGone = confirm !== null && dialog === null;
  useEffect(() => { if (resumeGone) answer(false); });

  return {
    service, devices, display, mac, enabled, dryRunOnly, dryRunNote, chosen, selection, setExecute, setSoftwareMode, softwareMode,
    chooseDevice: (next: LiveDevice) => setChosen({ device: next }),
    device, run, session, phase, active, blocker, box, stale, now, executeSelected,
    begin, resume, stop: () => { if (runner && run) void runner.stop(run.serviceId); },
    confirm, confirmKind, resumeGone, acknowledged, setAcknowledged, answer,
    copy: dialog ?? startCopy, dialogOpen: confirm !== null && !resumeGone,
  };
}

export type RunSession = ReturnType<typeof useRunSession>;

const noop = () => {};
// A run session for a panel or header control mounted without stores (tests, the browser preview):
// everything is stopped and Run stays disabled with the desktop-app blocker.
export const idleRunSession: RunSession = {
  service: idleServiceState, devices: [], display: idleDisplaySettingsState, mac: false, enabled: false,
  dryRunOnly: false, dryRunNote: dryRunOnlyRunNote, chosen: null, selection: null, setExecute: noop, setSoftwareMode: noop, softwareMode: false,
  chooseDevice: noop,
  device: undefined, run: undefined, session: undefined, phase: undefined, active: false,
  blocker: runBlocker({ desktop: false, device: undefined, profile: { schema_version: 1, name: '', bindings: [] } }),
  box: softwareModeBox({ module: undefined, loaded: false, moduleControl: 'app', ticked: false }),
  stale: false, now: 0, executeSelected: false,
  begin: async () => {}, resume: noop, stop: noop,
  confirm: null, confirmKind: 'start', resumeGone: false, acknowledged: false, setAcknowledged: noop, answer: noop,
  copy: confirmCopy('start', '', 'the selected module'), dialogOpen: false,
};

// Compact module names for the header Run button ("Run on 15-key"); the tooltip carries the full device name.
const runTargetLabel: Record<ServiceModule, string> = { m131e: 'encoder', m4315: '15-key', m003k: 'knobs', m005s: 'faders', m032le: 'rollers' };

// The header Run control: the run state (Stopped / Dry run / Executing / Paused / …) next to the one
// primary action — Run, Stop, or, for a paused run, Resume. The button names the module a start would
// use and turns danger when it would run with actions. The Execute confirmation lives here too, so
// Run works from every page.
export function RunHeaderControl({ run }: { run: RunSession }) {
  const { run: activeRun, active, phase, blocker, begin, resume, stop, executeSelected, device, dialogOpen, copy, answer, acknowledged, setAcknowledged } = run;
  const pill = runStatePill(activeRun);
  const needsConfirmation = phase === 'needs_confirmation';
  const stopping = activeRun?.state === 'stopping';
  const starting = activeRun?.state === 'starting';
  const target = device ? runTargetLabel[device.module] : null;
  const runTitle = blocker ?? `${executeSelected ? 'Run with actions' : 'Dry run'}${device ? ` on ${deviceName(device)}` : ''}`;
  return <div className="run-control" role="group" aria-label="Run control">
    <span className={`mode-pill run-state-pill ${pill.className}`} role="status">{pill.label}</span>
    {needsConfirmation ? <>
      <button type="button" className="button compact danger" title="Resume actions on this Mac" onClick={resume}>Resume…</button>
      <button type="button" className="button compact" title="Stop without resuming" disabled={stopping} onClick={stop}>Stop</button>
    </> : active
      ? <button type="button" className="button" disabled={stopping} onClick={stop}>{stopping ? 'Stopping…' : <><Square size={16} />Stop</>}</button>
      : <button type="button" className={`button ${executeSelected ? 'danger' : 'primary'}`} disabled={blocker !== null} title={runTitle} onClick={begin}>{starting ? <LoaderCircle size={16} className="spinning" /> : <Play size={16} />}{executeSelected ? target ? `Run with actions on ${target}` : 'Run with actions' : target ? `Run on ${target}` : 'Run'}</button>}
    <ExecuteDialog run={run} />
  </div>;
}

// Why Run is unavailable, as a full-width strip under the header row: visible on every page, never
// tooltip-only. The browser preview omits it — its own banner already says the desktop app is needed.
export function RunBlockerNotice({ run }: { run: RunSession }) {
  const { blocker, active, enabled } = run;
  if (!blocker || active || !enabled) return null;
  return <div className="run-blocker-strip" role="status"><TriangleAlert size={15} aria-hidden="true" /><span>{blocker}</span></div>;
}

// The Execute confirmation with the capability disclosure. Shown for a start and for a resume.
export function ExecuteDialog({ run }: { run: RunSession }) {
  const { copy, dialogOpen, answer, acknowledged, setAcknowledged } = run;
  return <ModalDialog open={dialogOpen} role="alertdialog" labelledBy="execute-title" describedBy={copy.warning ? 'execute-warning execute-message' : 'execute-message'} onCancel={() => answer(false)} className="execute-dialog">
    <h2 id="execute-title">{copy.title}</h2>
    {copy.warning && <p id="execute-warning" className="execute-warning"><TriangleAlert size={18} aria-hidden="true" /><span>{copy.warning}</span></p>}
    <p id="execute-message">{copy.message}</p>
    <div className="execute-details" tabIndex={0} role="region" aria-label="What this profile will do">
      <ul className="execute-capabilities">{copy.lines.map(line => <li key={line}>{line}</li>)}</ul>
      {copy.snippets.length > 0 && <>
        <h3>Text snippets</h3>
        <p className="fine-print">Shown exactly as they will be pasted: ⏎ is a line break, ⇥ a tab and \u{'{…}'} an invisible character or a stack of combining marks.</p>
        <ol className="execute-snippets">{copy.snippets.map((snippet, index) => <li key={index}>
          <code>{snippet.text}</code>
          {snippet.more > 0 && <span className="fine-print"> … and {snippet.more} more characters not shown</span>}
          {snippet.pressEnter && <strong className="execute-enter"> Then presses Return.</strong>}
        </li>)}</ol>
      </>}
      {copy.sequences.length > 0 && <>
        <h3>Multi-step sequences</h3>
        {copy.sequences.map((steps, index) => <ol key={index} className="execute-sequence" aria-label={`Sequence ${index + 1}`}>{steps.map((step, stepIndex) => <li key={stepIndex}>{step}</li>)}</ol>)}
      </>}
    </div>
    {copy.acknowledge && <label className="service-option execute-acknowledge"><input type="checkbox" checked={acknowledged} onChange={event => setAcknowledged(event.target.checked)} />{copy.acknowledge}</label>}
    <div className="dialog-actions"><button type="button" className="button" autoFocus onClick={() => answer(false)}>Cancel</button><button type="button" className="button danger" disabled={copy.acknowledge !== null && !acknowledged} onClick={() => answer(true)}>{copy.confirm}</button></div>
  </ModalDialog>;
}

// The Devices-page run section: which device and mode the header Run control uses, plus the live
// banners, counters and activity of the selected run. Run and Stop live in the header.
export function ServicePanel({ run = idleRunSession }: { run?: RunSession }) {
  const { service, devices, enabled, mac, dryRunOnly, dryRunNote, selection, setExecute, setSoftwareMode, softwareMode, chooseDevice, device, run: activeRun, session, phase, active, box, stale, now, executeSelected, resume } = run;
  const log = useRef<HTMLOListElement>(null);
  useEffect(() => { if (log.current) log.current.scrollTop = log.current.scrollHeight; }, [activeRun?.entries.length]);
  const pill = runStatePill(activeRun);

  return <section className="service-panel" aria-label="Run profile">
    <div className="section-heading"><h2>Run profile</h2><span className={`service-badge ${pill.className}`} role="status">{pill.label}</span></div>
    {activeRun && active && phase === 'running' && activeRun.execute && <div className="service-banner execute" role="alert"><TriangleAlert size={20} /><div><strong>Actions are running on this Mac</strong><span>Every action in this profile, including pasted text, web pages and multi-step sequences, really runs when you use the module. Stop to end it.</span></div></div>}
    {activeRun && active && phase === 'running' && !activeRun.execute && <div className="service-banner" role="status"><CircleHelp size={18} /><span>Dry run: actions are logged only. Nothing changes on this Mac.</span></div>}
    {activeRun && session && phase === 'suspended' && <div className="service-banner paused" role="status"><MoonStar size={18} /><div><strong>Paused while your Mac sleeps</strong><span>{activeRun.execute ? session.autoStart ? 'Actions resume automatically after wake.' : 'Actions stay off. You will be asked before they resume.' : 'It resumes automatically in dry-run mode.'}</span></div></div>}
    {activeRun && session && phase === 'waiting_for_device' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>Waiting for {sessionDeviceName(session)} to reconnect…</strong><span>{activeRun.execute ? session.autoStart ? 'Actions stay paused until the module reconnects, then resume automatically.' : 'Actions are paused and will not run until you confirm.' : 'The dry run continues when the module is back.'}</span></div></div>}
    {activeRun && session && phase === 'reconnecting' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>Reconnecting to {sessionDeviceName(session)} (attempt {session.attempt}/{maxReconnectAttempts})</strong><span>{countdownSeconds(session.nextAttemptAt, now) === null ? 'Trying now.' : `Next attempt in ${countdownSeconds(session.nextAttemptAt, now)}s.`}{activeRun.execute ? session.autoStart ? ' Actions are paused until the session restarts; they resume automatically.' : ' Actions are paused until you confirm.' : ''}</span></div></div>}
    {activeRun && session && phase === 'needs_confirmation' && <div className="service-banner paused attention" role="alert"><TriangleAlert size={20} /><div><strong>Actions are paused. Nothing will run until you resume.</strong><span>{session.reason ? `Paused because ${stoppedReason[session.reason]}. ` : ''}{session.detail ?? ''} {activeRun.profileSwitched ? `Resuming starts the saved profile "${activeRun.profileName}" ${switchedTo(activeRun.profileSwitchedBy)}.` : `Resuming restarts "${activeRun.profileName}" exactly as it was started.`}</span><button type="button" id={serviceResumeId} className="button danger" onClick={resume}>Resume actions…</button></div></div>}
    {activeRun && session && phase === 'stopped' && <div className="service-banner stopped" role="status"><CircleHelp size={18} /><div><strong>Session stopped{session.reason ? `: ${stoppedReason[session.reason]}` : ''}</strong>{session.detail && <span>{session.detail}</span>}</div></div>}
    {session?.autoStart && session.phase !== 'stopped' && <p className="fine-print service-auto-start" role="status">{autoStartNote(session.autoStart, activeRun?.execute ?? false, session.autoDryRunReason)}</p>}
    {active && phase === 'running' && activeRun?.state !== 'stopping' && <p className="fine-print" role="status">{runPreviewNote}</p>}
    <div className="service-controls">
      <label className="sr-only" htmlFor="service-device">Device to run on</label>
      <SelectField id="service-device" value={selection && !device ? disconnectedServiceValue : device?.path_hex ?? ''} disabled={!enabled || active || devices.length === 0} onChange={event => {
        const next = devices.find(item => item.path_hex === event.target.value);
        if (next) chooseDevice(next);
      }}>
        {selection && !device && <option value={disconnectedServiceValue}>{deviceName(selection.device)} (disconnected)</option>}
        {devices.length === 0 && !selection ? <option value="">No module connected</option> : devices.map(item => <option key={item.path_hex} value={item.path_hex}>{deviceName(item)}</option>)}
      </SelectField>
      <div className="mode-toggle" role="group" aria-label="Run mode">
        <button type="button" aria-pressed={!executeSelected} disabled={!enabled || active} onClick={() => setExecute(false)}>Dry run (default)</button>
        <button type="button" aria-pressed={executeSelected} disabled={!enabled || active || !mac || dryRunOnly} onClick={() => setExecute(true)}>Execute actions</button>
      </div>
    </div>
    <label className="service-option"><input type="checkbox" checked={box.checked} disabled={!enabled || active || box.locked} aria-describedby={box.appDriven ? 'service-app-control' : undefined} onChange={event => setSoftwareMode(event.target.checked)} />Request device software mode (encoder and 15-key modules)</label>
    {box.appDriven && <p id="service-app-control" className="fine-print">{appControlNote}</p>}
    {dryRunOnly && !active && <p className="service-stale" role="note"><TriangleAlert size={15} />{dryRunNote}</p>}
    {!mac && enabled && <p className="fine-print">Executing actions is only available on macOS. Dry runs log what would happen.</p>}
    {activeRun && <p className="fine-print">Profile "{activeRun.profileName}" ({activeRun.draftUnsaved ? 'unsaved changes' : 'the saved version'}) was used when this run started.</p>}
    {stale && <p className="service-stale" role="status"><TriangleAlert size={15} />Profile changed since start. Restart to apply.</p>}
    {activeRun?.state === 'failed' && <div className="service-failure" role="alert"><strong>{activeRun.error ?? 'The run failed.'}</strong>{activeRun.stderrTail && <pre>{activeRun.stderrTail}</pre>}</div>}
    {activeRun?.state === 'exited' && activeRun.exitCode !== undefined && activeRun.exitCode !== 0 && <p className="fine-print">The background helper exited with code {activeRun.exitCode}.</p>}
    {service.error && <p className="service-failure" role="alert">{service.error}</p>}
    {activeRun && <>
      <dl className="service-counters" aria-label="Run counters">
        <div><dt>Events</dt><dd>{activeRun.counters.events}</dd></div><div><dt>Would run</dt><dd>{activeRun.counters.planned}</dd></div>
        <div><dt>Executed</dt><dd>{activeRun.counters.executed}</dd></div><div><dt>Failed</dt><dd>{activeRun.counters.failed}</dd></div>
        {activeRun.counters.timerAlertsFailed > 0 && <div><dt>Timer alerts failed</dt><dd>{activeRun.counters.timerAlertsFailed}</dd></div>}
      </dl>
      <ol className="service-log" ref={log} role="log" aria-label="Run activity" tabIndex={0}>
        {activeRun.entries.length === 0 ? <li className="info">Waiting for activity…</li> : activeRun.entries.slice(-100).map((entry, index) =>
          <li key={entry.seq ?? index} className={entry.kind}><time>{relativeTime(entry.at, now)}</time><span>{entry.text}</span></li>)}
      </ol>
    </>}
  </section>;
}
