import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { CircleHelp, FlaskConical, Gamepad2, LoaderCircle, MoonStar, Pause, Play, TriangleAlert, Zap } from 'lucide-react';
import type { LiveDevice, ModuleControl, Profile, ServiceModule, ServiceState } from './contracts';
import { SelectField, ModalDialog } from './controls';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { autoStartLabel, idleServiceState, isActiveRun, isStale, maxReconnectAttempts, serviceIdFor, staleNote, unknownSnapshot } from './service-runner';
import type { ServiceRun, ServiceRunner } from './service-runner';
import { modules } from './ui-model';
import { capabilityLines, macroLabel, macroStepLabel, profileCapabilities, snapshotProfile, snippetPreview, stepLabel } from './profile-capabilities';import type { RiskLevel } from './profile-capabilities';
import type { RunMode } from './contracts';
import { usesKeypadPages } from './contracts';
import { appControls, idleDisplaySettingsState } from './display-settings';
import type { DisplaySettingsStore } from './display-settings';
import { endEditingHold, restoreNeedsConfirm } from './editing-hold';
import { useT } from './i18n-react';
import type { Translate } from './i18n-react';
import type { MessageKey } from './locales/en';

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
  if (input.device.module === 'm4315' && usesKeypadPages(input.profile) && input.moduleControl === 'built_in_animation') return builtInKeypadBlocker;
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
// only while the header mode is Live; in Test mode, running another profile needs Stop and then the user's own Run. A dry
// run that should have executed says why.
export function autoStartNote(autoStart: NonNullable<ServiceRun['session']>['autoStart'], execute = false, dryRunReason?: string) {
  if (!autoStart) return null;
  const label = `${autoStartLabel(autoStart, execute)}.`;
  if (execute) return `${label} Actions run without asking. Stop it, or switch the header mode to Test, to end them.`;
  const why = dryRunReason ? ` Actions are off for this run: ${dryRunReason}` : '';
  return `${label}${why} Stop it to run another profile on this module.`;
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
// The state pill's label in the reader's language. The labels themselves stay English in the model (tests and the CSS
// classes read them); an unknown one is shown as it is.
const pillKeys: Record<string, MessageKey> = {
  Paused: 'run.pill.paused', Waiting: 'run.pill.waiting', Reconnecting: 'run.pill.reconnecting', 'Needs confirmation': 'run.pill.needsConfirmation',
  Failed: 'run.pill.failed', Stopped: 'run.pill.stopped', Stopping: 'run.pill.stopping', Executing: 'run.pill.executing', 'Dry run': 'run.pill.dryRun',
  Starting: 'run.pill.starting', Running: 'run.pill.running',
};
export const pillText = (t: Translate, label: string) => label in pillKeys ? t(pillKeys[label]) : label;
const badgeKeys: Record<string, MessageKey> = { 'Test (imported profile)': 'run.badge.testImported', 'Test (editing)': 'run.badge.testEditing', Live: 'run.badge.live', Test: 'run.badge.test' };
const moduleName = (module: ServiceModule) => modules.find(entry => entry.id === module)?.shortName ?? module;
const sessionDeviceName = (session: { module: ServiceModule; serialNumber: string | null }) => `${moduleName(session.module)}${session.serialNumber ? ` …${session.serialNumber.slice(-6)}` : ''}`;
export const countdownSeconds = (nextAttemptAt: number | undefined, now: number) => nextAttemptAt === undefined ? null : Math.max(0, Math.ceil((nextAttemptAt - now) / 1000));
const stoppedReason = { sleep: 'the Mac went to sleep', device_removed: 'the module was unplugged', service_exited: 'the background helper exited', start_refused: 'the profile could not start', user: 'you stopped it', profile_switch: 'a focused-app rule switched the profile', profile_action: 'a profile key switched the profile', profile_saved: 'the profile was saved', module_control: 'OpenStreamHub no longer drives the display', run_mode: 'the run mode was switched', demo: 'a demo is using the module' } as const;
// The first 8 characters of a saved profile id, enough to tell two profiles of the same name apart.
const shortId = (id: string) => id.length > 8 ? `${id.slice(0, 8)}…` : id;
// Who switched the run, as one phrase for the banner and the Resume dialog.
const switchedTo = (by: ServiceRun['profileSwitchedBy']) => by === 'key' ? 'a profile key switched to' : by === 'rule' ? 'a focused-app rule switched to' : 'this run was switched to';

export interface ConfirmCopy {
  title: string; confirm: string; message: string; risk: RiskLevel;
  // The capabilities in plain language, then every distinct text snippet,
  // multi-step sequence and recorded macro.
  lines: string[]; snippets: { text: string; more: number; pressEnter: boolean }[]; sequences: string[][]; macros: { label: string; steps: string[] }[];
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
export function confirmCopy(kind: 'start' | 'resume', profileName: string, deviceLabel: string, switched = false, profile: Profile | null = null, switchedBy?: ServiceRun['profileSwitchedBy'], profileId?: string | null, saved = false): ConfirmCopy {
  const key = kind === 'resume' && switched && switchedBy === 'key';
  // A key switch names the saved profile by id (two profiles can share a name) and says the run holds its own copy.
  const keyNote = key ? ` ${profileId ? `Saved profile "${profileName}", id ${shortId(profileId)}. ` : ''}This is the copy the run took when it switched; later edits to the saved profile do not apply until the next switch.` : '';
  // A save applied the saved content to the run; the resume restarts that.
  const version = switched ? `(the saved profile ${switchedTo(switchedBy)})`
    : saved ? '(as you saved it)' : '(as it was when this run started)';
  const head = kind === 'start'
    ? { title: 'Run actions on this Mac?', confirm: 'Run actions', message: `Profile "${profileName}" will run on ${deviceLabel}. Everything listed here will really run on this Mac whenever you use the module, without asking again.` }
    : { title: 'Resume actions on this Mac?', confirm: 'Resume actions', message: `Profile "${profileName}" ${version} will resume on ${deviceLabel}. Everything listed here will really run on this Mac again whenever you use the module, without asking again.${keyNote}` };
  const capabilities = profile ? profileCapabilities(profile) : null;
  const risk: RiskLevel = capabilities?.risk ?? 'high';
  const reasons = capabilities ? capabilities.riskReasons : ['The profile contents could not be checked.'];
  return {
    ...head, risk,
    lines: capabilities ? capabilityLines(capabilities) : [key ? unknownKeyContents : unknownContents],
    snippets: capabilities?.texts.map(snippet => ({ ...snippetPreview(snippet.text), pressEnter: snippet.pressEnter })) ?? [],
    sequences: capabilities?.sequences.map(steps => steps.map(stepLabel)) ?? [],
    macros: capabilities?.macros.map(steps => ({ label: macroLabel(steps), steps: steps.map(macroStepLabel) })) ?? [],
    warning: risk === 'high' ? `High risk. ${reasons.join(' ')} With one press of a control, a profile like this can type and run commands, or send what is on screen or in the clipboard to a web page. Only continue if you wrote this profile or reviewed every snippet and step below and trust where it came from.` : null,
    acknowledge: risk === 'high' ? 'I reviewed this profile and trust its source' : null,
  };
}

// Resume copy for the run resumeSession holds (by its session id), never the run the device selection shows now:
// the selection can move while the dialog is open (a module plugged in or out). Null once that run is gone.
export function resumeCopy(runs: Record<string, ServiceRun>, sessionId: string | null) {
  const run = sessionId === null ? undefined : runs[sessionId];
  if (!run) return null;
  // The run's own snapshot, never the draft: resuming restarts the profile the run holds.
  const copy = confirmCopy('resume', run.profileName, run.session ? sessionDeviceName(run.session) : 'the module', run.profileSwitched === true, snapshotProfile(run.snapshot), run.profileSwitchedBy, run.profileId, run.session?.reason === 'profile_saved');
  // A save-paused run whose previous body the frontend holds lists only what the save added over it; the dialog is
  // never emptier than the full disclosure.
  const prior = run.session?.reason === 'profile_saved' && run.savedSwap ? snapshotProfile(run.savedSwap) : null;
  const body = snapshotProfile(run.snapshot);
  if (!prior || !body) return copy;
  const previous = profileCapabilities(prior);
  const current = profileCapabilities(body);
  const previousLines = new Set(capabilityLines(previous));
  const lines = capabilityLines(current).filter(line => !previousLines.has(line));
  const previousTexts = new Set(previous.texts.map(snippet => JSON.stringify([snippet.text, snippet.pressEnter])));
  const previousSequences = new Set(previous.sequences.map(steps => JSON.stringify(steps)));
  const previousMacros = new Set(previous.macros.map(steps => JSON.stringify(steps)));
  return {
    ...copy,
    lines: lines.length ? lines : copy.lines,
    snippets: current.texts.filter(snippet => !previousTexts.has(JSON.stringify([snippet.text, snippet.pressEnter]))).map(snippet => ({ ...snippetPreview(snippet.text), pressEnter: snippet.pressEnter })),
    sequences: current.sequences.filter(steps => !previousSequences.has(JSON.stringify(steps))).map(steps => steps.map(stepLabel)),
    macros: current.macros.filter(steps => !previousMacros.has(JSON.stringify(steps))).map(steps => ({ label: macroLabel(steps), steps: steps.map(macroStepLabel) })),
  };
}

// The copy of the open confirmation. Null for a resume whose run disappeared while its dialog was open: the panel then
// answers false and closes the dialog instead of showing start-a-run text for a resume.
export function dialogCopy(kind: 'start' | 'resume', runs: Record<string, ServiceRun>, resumeId: string | null, startCopy: ReturnType<typeof confirmCopy>) {
  return kind === 'resume' ? resumeCopy(runs, resumeId) : startCopy;
}

// The header badge for the run mode. Three holds read as Test whatever the
// mode is, because with any of them in place a press will not run actions: an
// imported profile not yet allowed to execute, the one-time confirmation
// still owed on a fresh install (automatic sessions run dry until it), and
// the editing hold ("Switch to Test mode while editing").
export const runModeBadge = (runMode: RunMode, dryRunOnly: boolean, liveConfirmed = true, editingHeld = false): string =>
  dryRunOnly ? 'Test (imported profile)' : editingHeld ? 'Test (editing)'
    : runMode === 'live' && !dryRunOnly && liveConfirmed ? 'Live' : 'Test';

// Why the badge reads "Test (editing)", as its tooltip.
export const editingHoldNote = 'You are editing a binding, so presses are held in Test mode. Save, undo or close the editor to go back to the mode you chose.';

// The copy of the run-mode switch to Live: one dialog for every running
// session that would start executing. A profile whose body the frontend does
// not hold (or that could not be fetched) counts as unknown contents, which
// is high risk — a confirmation never covers what it could not show.
export function modeSwitchCopy(runs: ServiceRun[], bodies: (Profile | null)[]): ConfirmCopy {
  const entries = runs.map((item, index) => ({ name: item.profileName, body: bodies[index] ?? null }));
  const capabilities = entries.map(entry => ({ name: entry.name, caps: entry.body ? profileCapabilities(entry.body) : null }));
  const checked = capabilities.filter(entry => entry.caps !== null).map(entry => entry.caps!);
  const unknown = capabilities.length - checked.length;
  const risk: RiskLevel = unknown > 0 || checked.some(caps => caps.risk === 'high') ? 'high' : 'elevated';
  // Several modules often run the same profile; name it once.
  const nameList = [...new Set(entries.map(entry => entry.name))];
  const names = nameList
    .map(name => {
      const count = entries.filter(entry => entry.name === name).length;
      return count > 1 ? `"${name}" (${count} modules)` : `"${name}"`;
    })
    .join(', ');
  const subject = nameList.length === 1 ? `the running profile ${names}` : `the running profiles ${names}`;
  const lines = [...new Set(checked.flatMap(caps => capabilityLines(caps)))];
  const reasons = checked.filter(caps => caps.risk === 'high').flatMap(caps => caps.riskReasons);
  return {
    title: 'Switch the mode to Live?',
    confirm: 'Switch to Live',
    message: `In Live mode, everything in ${subject} runs on this Mac whenever the module is used, without asking again.`,
    risk,
    lines: lines.length ? lines : [unknownContents],
    snippets: checked.flatMap(caps => caps.texts.map(snippet => ({ ...snippetPreview(snippet.text), pressEnter: snippet.pressEnter }))),
    sequences: checked.flatMap(caps => caps.sequences.map(steps => steps.map(stepLabel))),
    macros: checked.flatMap(caps => caps.macros.map(steps => ({ label: macroLabel(steps), steps: steps.map(macroStepLabel) }))),
    warning: risk === 'high' ? `High risk.${unknown > 0 ? ` ${unknown === 1 ? 'One profile' : `${unknown} profiles`} could not be checked here and may do anything.` : reasons.length ? ` ${reasons.join(' ')}` : ''} With one press of a control, a profile like this can type and run commands, or send what is on screen or in the clipboard to a web page. Only continue if you wrote these profiles or reviewed every snippet and step below and trust where they came from.` : null,
    acknowledge: risk === 'high' ? 'I reviewed these profiles and trust their sources' : null,
  };
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
export const dryRunOnlyRunNote = 'This profile was imported (from a MasterHUB .mhz file or an app preset), so it runs only as a dry run. Open Manage profiles (next to the profile switcher) to review it and allow it to execute actions first.';
export const runBlockNotes = {
  imported: dryRunOnlyRunNote,
  deleted: 'The saved profile behind these unsaved changes was deleted. Save it to run it with actions.',
  unreadable: 'The list of imported profiles cannot be read, so every profile runs only as a dry run. Open Manage profiles (next to the profile switcher) to fix it first.',
} as const;

export interface RunSessionOptions {
  runner?: ServiceRunner; feed?: DeviceFeed; profile: Profile; profileId?: string | null; sourceProfileId?: string | null;
  desktop: boolean; unsaved: boolean; displaySettings?: DisplaySettingsStore; dryRunOnly?: boolean; dryRunNote?: string;
  platform?: PlatformInfo; now?: number;
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
export function useRunSession({ runner, feed, profile, profileId = null, sourceProfileId = null, desktop, unsaved, displaySettings, dryRunOnly = false, dryRunNote = dryRunOnlyRunNote, platform = typeof navigator === 'undefined' ? undefined : navigator, now: fixedNow, beforeStart }: RunSessionOptions) {
  const service = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner);
  const devices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleFeed, feed?.getState ?? idleFeed).devices;
  const display = useSyncExternalStore(displaySettings?.subscribe ?? noopSubscribe, displaySettings?.getState ?? idleDisplay, displaySettings?.getState ?? idleDisplay);
  useEffect(() => { if (displaySettings && display.desktop && !display.loaded) void displaySettings.load(); }, [displaySettings, display.desktop, display.loaded]);
  const [chosen, setChosen] = useState<ServiceSelection | null>(null);
  const [softwareMode, setSoftwareMode] = useState(false);
  const [confirm, setConfirm] = useState<((ok: boolean) => void) | null>(null);
  const [confirmKind, setConfirmKind] = useState<'start' | 'resume'>('start');
  // The run-mode switch to Live with running sessions: the dialog lists what
  // they would start doing; null while Test needs no confirmation. The ref
  // carries the open copy to the answer, which may run from a closure made
  // before the dialog opened (the same reason pendingConfirm exists).
  const [modeCopy, setModeCopyState] = useState<ConfirmCopy | null>(null);
  const modeCopyRef = useRef<ConfirmCopy | null>(null);
  const setModeCopy = (copy: ConfirmCopy | null) => { modeCopyRef.current = copy; setModeCopyState(copy); };
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
  // The one mode decides what a press does: the badge, the Run button and the
  // banners all read it. An imported profile not yet allowed keeps the badge
  // on Test for the open profile whatever the mode is, and the editing hold
  // keeps it on "Test (editing)" for as long as the user edits — including
  // at the launch after a quit or crash mid-edit, until the restore runs.
  const runMode: RunMode = display.settings.run_mode;
  const editingHeld = display.settings.held_from === 'live';
  const executeSelected = runMode === 'live' && !editingHeld && mac && !dryRunOnly;
  // The switch saves the mode; the backend applies it to every running
  // module. Test needs no confirmation. Live with running sessions that
  // would start executing asks once, listing their profiles; Live with
  // nothing running just saves (a later Run still confirms, as always).
  const modeBusy = display.busy || !display.loaded;
  // The switch-to-Live dialog for the dry runs that would start executing:
  // the manual switch and the editing hold's restore share it.
  const askLive = (switching: ServiceRun[]) => {
    void Promise.all(switching.map(async item => {
      if (item.snapshot !== unknownSnapshot) return snapshotProfile(item.snapshot);
      const heldProfile = await runner?.heldProfile(item.serviceId, item.profileGeneration ?? 0);
      return heldProfile?.profile ?? null;
    })).then(bodies => setModeCopy(modeSwitchCopy(switching, bodies)));
  };
  const switchMode = (next: RunMode) => {
    if (!displaySettings || modeBusy || next === runMode) return;
    if (next === 'test') { void displaySettings.setRunMode('test'); return; }
    const switching = Object.values(service.runs).filter(item => isActiveRun(item) && !item.execute);
    if (!switching.length) { void displaySettings.setRunMode('live'); return; }
    askLive(switching);
  };

  // The editing hold ("Switch to Test mode while editing"): the first edit
  // holds the mode at Test — the save reaches the backend, so a press on any
  // module is logged, never run — and ending the edit restores the user's
  // own mode. Restoring asks through the same dialog as a manual switch only
  // when a dry run started during the hold would begin executing; the runs
  // the hold itself turned dry were running with actions before it, so the
  // user's mode goes back to them in silence — as does the launch after a
  // quit or crash mid-edit, which restores the stored Live without this
  // session having seen anything execute. Both saves keep live_offered as it
  // is: the hold never answers the one-time confirmation.
  const applyEditingHold = (editing: boolean, whileEditing: boolean) => {
    if (!displaySettings || !display.loaded) return;
    if (editing && whileEditing) {
      displaySettings.beginEditingHold(whileEditing, Object.values(service.runs).filter(item => item.execute && isActiveRun(item)).map(item => item.serviceId));
      return;
    }
    const step = endEditingHold(displaySettings.editingHeld);
    if (!step.save) return;
    const switching = Object.values(service.runs).filter(item => isActiveRun(item) && !item.execute && item.session);
    if (!restoreNeedsConfirm(displaySettings.editingSnapshotRuns, switching)) { void displaySettings.setHeldMode(step.save, null); return; }
    displaySettings.dropEditingHold();
    askLive(switching);
  };

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
  const latest = useRef({ runner, profile, profileId, sourceProfileId, unsaved, executeSelected, box, device, displaySettings });
  latest.current = { runner, profile, profileId, sourceProfileId, unsaved, executeSelected, box, device, displaySettings };
  const begin = async () => {
    let settled: SettledDraft | void = undefined;
    try { settled = await beforeStart?.(); } catch { /* the start uses the draft as it is */ }
    const atStart = settled ?? latest.current;
    const current = latest.current;
    if (!current.runner || !current.device) return;
    const confirmExecute = current.executeSelected ? async () => {
      const ok = await new Promise<boolean>(resolve => { setConfirmKind('start'); setConfirm(() => resolve); });
      // Running actions once confirms the mode on a fresh install.
      if (ok && current.displaySettings && !current.displaySettings.getState().settings.live_offered) void current.displaySettings.markLiveConfirmed();
      return ok;
    } : undefined;
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
  const answer = (ok: boolean) => {
    if (modeCopyRef.current !== null) {
      setModeCopy(null);
      setAcknowledged(false);
      if (ok && displaySettings) void displaySettings.setRunMode('live');
      return;
    }
    confirm?.(ok);
    setConfirm(null);
    setAcknowledged(false);
  };

  const startCopy = confirmCopy('start', profile.name, device ? deviceName(device) : 'the selected module', false, profile);
  const dialog = dialogCopy(confirmKind, service.runs, resumeId, startCopy);
  const resumeGone = confirm !== null && dialog === null;
  useEffect(() => { if (resumeGone) answer(false); });

  return {
    service, devices, display, mac, enabled, dryRunOnly, dryRunNote, chosen, selection, runMode, setRunMode: switchMode, setSoftwareMode, softwareMode,
    chooseDevice: (next: LiveDevice) => setChosen({ device: next }),
    device, run, session, phase, active, blocker, box, stale, now, executeSelected, editingHeld, applyEditingHold,
    // The open draft, for the panel's stale note (what the run holds against it).
    draftProfile: profile,
    begin, resume, stop: () => { if (runner && run) void runner.stop(run.serviceId); },
    confirm, confirmKind, resumeGone, acknowledged, setAcknowledged, answer,
    copy: modeCopy ?? dialog ?? startCopy, dialogOpen: modeCopy !== null || (confirm !== null && !resumeGone),
  };
}

export type RunSession = ReturnType<typeof useRunSession>;

const noop = () => {};
// A run session for a panel or header control mounted without stores (tests, the browser preview):
// everything is stopped and Run stays disabled with the desktop-app blocker.
export const idleRunSession: RunSession = {
  service: idleServiceState, devices: [], display: idleDisplaySettingsState, mac: false, enabled: false,
  dryRunOnly: false, dryRunNote: dryRunOnlyRunNote, chosen: null, selection: null, runMode: 'test', setRunMode: noop, setSoftwareMode: noop, softwareMode: false,
  chooseDevice: noop,
  device: undefined, run: undefined, session: undefined, phase: undefined, active: false,
  blocker: runBlocker({ desktop: false, device: undefined, profile: { schema_version: 1, name: '', bindings: [] } }),
  box: softwareModeBox({ module: undefined, loaded: false, moduleControl: 'app', ticked: false }),
  stale: false, now: 0, executeSelected: false, editingHeld: false, applyEditingHold: noop,
  draftProfile: { schema_version: 1, name: '', bindings: [] },
  begin: async () => {}, resume: noop, stop: noop,
  confirm: null, confirmKind: 'start', resumeGone: false, acknowledged: false, setAcknowledged: noop, answer: noop,
  copy: confirmCopy('start', '', 'the selected module'), dialogOpen: false,
};

// Compact module names for the header Run button ("Run on 15-key"); the tooltip carries the full device name.
const runTargetKey: Record<ServiceModule, MessageKey> = { m131e: 'run.target.m131e', m4315: 'run.target.m4315', m003k: 'run.target.m003k', m005s: 'run.target.m005s', m032le: 'run.target.m032le' };

// The one run-mode badge and its switch: the only place that shows and
// changes what a press does. The switch saves the mode; with running sessions
// that would start executing, Live asks first (the dialog below). While the
// editing hold is on, the badge reads "Test (editing)" and its tooltip says
// why.
export function ModeControl({ run }: { run: RunSession }) {
  const t = useT();
  const { runMode, setRunMode, display, enabled, mac, dryRunOnly, editingHeld } = run;
  const live = runMode === 'live' && !editingHeld;
  const confirmed = display.settings.live_offered;
  const busy = display.busy || !display.loaded;
  // The switch itself shows the mode; the badge text stays for screen readers, because three holds
  // (imported profile, unconfirmed Live, editing) read as Test whatever the mode is.
  return <div className={`mode-control${run.active ? '' : ' idle'}`} role="group" aria-label={t('run.mode')}>
    <span className={`mode-pill run-mode-pill sr-only ${live && !dryRunOnly && confirmed ? 'live' : 'test'}`} role="status">{(badge => badgeKeys[badge] ? t(badgeKeys[badge]) : badge)(runModeBadge(runMode, dryRunOnly, confirmed, editingHeld))}</span>
    <button type="button" className="mode-choice" aria-pressed={!live} disabled={!enabled || busy} aria-label={t('run.testMode')}
      data-tip={editingHeld ? t('run.editingNote') : t('run.testTip')} onClick={() => setRunMode('test')}><FlaskConical size={16} aria-hidden="true" /><span>{t('run.badge.test')}</span></button>
    <button type="button" className="mode-choice live" aria-pressed={live} disabled={!enabled || busy || !mac} aria-label={t('run.liveMode')}
      data-tip={mac ? t('run.liveTip') : t('run.macOnly')}
      onClick={() => setRunMode('live')}><Zap size={16} aria-hidden="true" /><span>{t('run.badge.live')}</span></button>
  </div>;
}

// The header Run control: the run state (Paused / Dry run / Executing / Paused / …), the run-mode
// badge and switch, then the one primary action — Run, Stop, or, for a paused run, Resume. The
// button names the module a start would use and turns danger when the mode would run with actions.
// The confirmations live here too, so Run and the mode switch work from every page.
export function RunHeaderControl({ run }: { run: RunSession }) {
  const t = useT();
  const { run: activeRun, active, phase, blocker, begin, resume, stop, executeSelected, device } = run;
  const pill = runStatePill(activeRun);
  const needsConfirmation = phase === 'needs_confirmation';
  const stopping = activeRun?.state === 'stopping';
  const starting = activeRun?.state === 'starting';
  const target = device ? t(runTargetKey[device.module]) : null;
  const what = executeSelected ? t('run.runWithActions') : t('run.dryRun');
  const runTitle = blocker ?? (device ? t('run.on', { what, device: deviceName(device) }) : what);
  const runLabel = executeSelected ? target ? t('run.runWithActionsOn', { target }) : t('run.runWithActions') : target ? t('run.runOn', { target }) : t('run.run');
  // Normal operation says nothing visually: the Pause / Activate button is the state and the Test | Live switch is the
  // mode. Only a state that needs attention (waiting, reconnecting, paused by sleep, needs confirmation, failed) gets
  // a chip; the others stay for screen readers.
  const quiet = ['Stopped', 'Dry run', 'Executing', 'Running', 'Starting', 'Stopping'].includes(pill.label);
  return <div className="run-control" role="group" aria-label={t('run.control')}>
    <span className={`mode-pill run-state-pill ${pill.className}${quiet ? ' sr-only' : ''}`} role="status">{pillText(t, pill.label)}</span>
    <ModeControl run={run} />
    {needsConfirmation ? <>
      <button type="button" className="button compact danger" data-tip={t('run.resumeTip')} onClick={resume}>{t('run.resume')}</button>
      <button type="button" className="button icon-run" aria-label={t('run.stop')} data-tip={t('run.stopWithout')} disabled={stopping} onClick={stop}><Pause size={16} aria-hidden="true" /><span>{t('run.stop')}</span></button>
    </> : active
      ? <button type="button" className="button icon-run" aria-label={stopping ? t('run.stoppingLabel') : t('run.stop')} data-tip={stopping ? t('run.stoppingTip') : t('run.stopTip')} disabled={stopping} onClick={stop}>{stopping ? <LoaderCircle size={16} className="spinning" aria-hidden="true" /> : <Pause size={16} aria-hidden="true" />}<span>{stopping ? t('run.stoppingLabel') : t('run.stop')}</span></button>
      : <button type="button" className={`button icon-run ${executeSelected ? 'danger' : 'primary'}`} disabled={blocker !== null} aria-label={runLabel} data-tip={runTitle} onClick={begin}>{starting ? <LoaderCircle size={16} className="spinning" aria-hidden="true" /> : <Play size={16} aria-hidden="true" />}<span>{starting ? t('run.pill.starting') : t('run.run')}</span></button>}
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
  const t = useT();
  const { copy, dialogOpen, answer, acknowledged, setAcknowledged } = run;
  return <ModalDialog open={dialogOpen} role="alertdialog" labelledBy="execute-title" describedBy={copy.warning ? 'execute-warning execute-message' : 'execute-message'} onCancel={() => answer(false)} className="execute-dialog">
    <h2 id="execute-title">{copy.title}</h2>
    {copy.warning && <p id="execute-warning" className="execute-warning"><TriangleAlert size={18} aria-hidden="true" /><span>{copy.warning}</span></p>}
    <p id="execute-message">{copy.message}</p>
    <div className="execute-details" tabIndex={0} role="region" aria-label={t('execute.region')}>
      <ul className="execute-capabilities">{copy.lines.map(line => <li key={line}>{line}</li>)}</ul>
      {copy.snippets.length > 0 && <>
        <h3>{t('execute.snippets')}</h3>
        <p className="fine-print">{t('execute.snippetsNote')}</p>
        <ol className="execute-snippets">{copy.snippets.map((snippet, index) => <li key={index}>
          <code>{snippet.text}</code>
          {snippet.more > 0 && <span className="fine-print"> {t('execute.more', { count: snippet.more })}</span>}
          {snippet.pressEnter && <strong className="execute-enter"> {t('execute.enter')}</strong>}
        </li>)}</ol>
      </>}
      {copy.sequences.length > 0 && <>
        <h3>{t('execute.sequences')}</h3>
        {copy.sequences.map((steps, index) => <ol key={index} className="execute-sequence" aria-label={t('execute.sequence', { n: index + 1 })}>{steps.map((step, stepIndex) => <li key={stepIndex}>{step}</li>)}</ol>)}
      </>}
      {copy.macros.length > 0 && <>
        <h3>{t('execute.macros')}</h3>
        {copy.macros.map((macro, index) => <div key={index} className="execute-macro" aria-label={t('execute.macro', { n: index + 1 })}>
          <p className="fine-print">{macro.label}</p>
          <ol className="execute-sequence">{macro.steps.map((step, stepIndex) => <li key={stepIndex}>{step}</li>)}</ol>
        </div>)}
      </>}
    </div>
    {copy.acknowledge && <label className="service-option execute-acknowledge"><input type="checkbox" checked={acknowledged} onChange={event => setAcknowledged(event.target.checked)} />{copy.acknowledge}</label>}
    <div className="dialog-actions"><button type="button" className="button" autoFocus onClick={() => answer(false)}>{t('execute.cancel')}</button><button type="button" className="button danger" disabled={copy.acknowledge !== null && !acknowledged} onClick={() => answer(true)}>{copy.confirm}</button></div>
  </ModalDialog>;
}

// The Devices-page run section: which device and mode the header Run control uses, plus the live
// banners, counters and activity of the selected run. Run and Stop live in the header.
export function ServicePanel({ run = idleRunSession, variant = 'full' }: { run?: RunSession; variant?: 'full' | 'workspace' }) {
  const { service, devices, enabled, mac, dryRunOnly, dryRunNote, selection, setSoftwareMode, softwareMode, chooseDevice, device, run: activeRun, session, phase, active, box, stale, now, executeSelected, resume, draftProfile } = run;
  const log = useRef<HTMLOListElement>(null);
  useEffect(() => { if (log.current) log.current.scrollTop = log.current.scrollHeight; }, [activeRun?.entries.length]);
  const pill = runStatePill(activeRun);
  const t = useT();
  const reason = (value: keyof typeof stoppedReason) => t(`service.reason.${value}` as MessageKey);
  const by = (value: ServiceRun['profileSwitchedBy']) => t(value === 'key' ? 'service.by.key' : value === 'rule' ? 'service.by.rule' : 'service.by.other');
  const seconds = countdownSeconds(session?.nextAttemptAt, now);
  // On the workspace the panel only speaks up when something needs attention or a Test run has activity to read;
  // the header carries Run, Stop and the Live or Test badge. Everything else lives under Settings, Run details.
  const full = variant === 'full';
  const attention = !!activeRun && (activeRun.state === 'failed' || stale || (!!session && phase !== 'running' && phase !== undefined)
    || (activeRun.state === 'exited' && activeRun.exitCode !== undefined && activeRun.exitCode !== 0));
  const testActivity = !!activeRun && !activeRun.execute && active;
  if (!full && !attention && !testActivity && !service.error && !(dryRunOnly && !active)) return null;

  return <section className="service-panel" aria-label={t('service.heading')}>
    <div className="section-heading"><h2>{t(full ? 'service.heading' : 'service.activityHeading')}</h2><span className={`service-badge ${pill.className}`} role="status">{pillText(t, pill.label)}</span></div>
    {full && activeRun && active && phase === 'running' && activeRun.execute && <div className="service-banner execute" role="status"><Zap size={18} aria-hidden="true" /><div><strong>{t('service.executing')}</strong><span>{t('service.executingBody')}</span></div></div>}
    {full && activeRun && active && phase === 'running' && !activeRun.execute && <div className="service-banner" role="status"><CircleHelp size={18} /><span>{t('service.dryBanner')}</span></div>}
    {activeRun && session && phase === 'suspended' && session.reason === 'demo' && <div className="service-banner paused" role="status"><Gamepad2 size={18} /><div><strong>{t('service.demoTitle')}</strong><span>{t('service.demoBody')}</span></div></div>}
    {activeRun && session && phase === 'suspended' && session.reason !== 'demo' && <div className="service-banner paused" role="status"><MoonStar size={18} /><div><strong>{t('service.sleepTitle')}</strong><span>{activeRun.execute ? session.autoStart ? t('service.sleepAuto') : t('service.sleepAsk') : t('service.sleepDry')}</span></div></div>}
    {activeRun && session && phase === 'waiting_for_device' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>{t('service.waitTitle', { device: sessionDeviceName(session) })}</strong><span>{activeRun.execute ? session.autoStart ? t('service.waitAuto') : t('service.waitAsk') : t('service.waitDry')}</span></div></div>}
    {activeRun && session && phase === 'reconnecting' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>{t('service.reconTitle', { device: sessionDeviceName(session), attempt: session.attempt, max: maxReconnectAttempts })}</strong><span>{seconds === null ? t('service.tryingNow') : t('service.nextIn', { seconds })}{activeRun.execute ? session.autoStart ? t('service.reconAuto') : t('service.reconAsk') : ''}</span></div></div>}
    {activeRun && session && phase === 'needs_confirmation' && <div className="service-banner paused attention" role="alert"><TriangleAlert size={20} /><div><strong>{t('service.pausedTitle')}</strong><span>{session.reason ? `${t('service.pausedBecause', { reason: reason(session.reason) })} ` : ''}{session.detail ?? ''} {activeRun.profileSwitched ? t('service.resumeSwitched', { name: activeRun.profileName, by: by(activeRun.profileSwitchedBy) }) : session.reason === 'profile_saved' ? t('service.resumeSaved', { name: activeRun.profileName }) : t('service.resumeSame', { name: activeRun.profileName })}</span><button type="button" id={serviceResumeId} className="button danger" onClick={resume}>{t('service.resumeButton')}</button></div></div>}
    {activeRun && session && phase === 'stopped' && <div className="service-banner stopped" role="status"><CircleHelp size={18} /><div><strong>{session.reason ? t('service.stoppedBecause', { reason: reason(session.reason) }) : t('service.stoppedTitle')}</strong>{session.detail && <span>{session.detail}</span>}</div></div>}
    {full && session?.autoStart && session.phase !== 'stopped' && <p className="fine-print service-auto-start" role="status">{autoStartNote(session.autoStart, activeRun?.execute ?? false, session.autoDryRunReason)}</p>}
    {full && active && phase === 'running' && activeRun?.state !== 'stopping' && <p className="fine-print" role="status">{runPreviewNote}</p>}
    {full && <><div className="service-controls">
      <label className="sr-only" htmlFor="service-device">{t('service.deviceLabel')}</label>
      <SelectField id="service-device" value={selection && !device ? disconnectedServiceValue : device?.path_hex ?? ''} disabled={!enabled || active || devices.length === 0} onChange={event => {
        const next = devices.find(item => item.path_hex === event.target.value);
        if (next) chooseDevice(next);
      }}>
        {selection && !device && <option value={disconnectedServiceValue}>{t('service.disconnected', { device: deviceName(selection.device) })}</option>}
        {devices.length === 0 && !selection ? <option value="">{t('service.noModule')}</option> : devices.map(item => <option key={item.path_hex} value={item.path_hex}>{deviceName(item)}</option>)}
      </SelectField>
      <p className="fine-print service-mode-note">{t('service.modeNote', { mode: t(executeSelected ? 'run.badge.live' : 'run.badge.test') })}</p>
    </div>
    <label className="service-option"><input type="checkbox" checked={box.checked} disabled={!enabled || active || box.locked} aria-describedby={box.appDriven ? 'service-app-control' : undefined} onChange={event => setSoftwareMode(event.target.checked)} />{t('service.softwareMode')}</label>
    {box.appDriven && <p id="service-app-control" className="fine-print">{appControlNote}</p>}</>}
    {dryRunOnly && !active && <p className="service-stale" role="note"><TriangleAlert size={15} />{dryRunNote}</p>}
    {full && !mac && enabled && <p className="fine-print">{t('service.macOnlyNote')}</p>}
    {full && activeRun && <p className="fine-print">{t('service.usedProfile', { name: activeRun.profileName, version: t(activeRun.draftUnsaved ? 'service.unsavedVersion' : 'service.savedVersion') })}</p>}
    {stale && <p className="service-stale" role="status"><TriangleAlert size={15} />{staleNote(activeRun, draftProfile)}</p>}
    {activeRun?.state === 'failed' && <div className="service-failure" role="alert"><strong>{activeRun.error ?? t('service.failed')}</strong>{activeRun.stderrTail && <pre>{activeRun.stderrTail}</pre>}</div>}
    {activeRun?.state === 'exited' && activeRun.exitCode !== undefined && activeRun.exitCode !== 0 && <p className="fine-print">{t('service.helperExit', { code: activeRun.exitCode ?? 0 })}</p>}
    {service.error && <p className="service-failure" role="alert">{service.error}</p>}
    {activeRun && (full || testActivity) && <>
      {full && <dl className="service-counters" aria-label={t('service.counters')}>
        <div><dt>{t('service.events')}</dt><dd>{activeRun.counters.events}</dd></div><div><dt>{t('service.wouldRun')}</dt><dd>{activeRun.counters.planned}</dd></div>
        <div><dt>{t('service.executed')}</dt><dd>{activeRun.counters.executed}</dd></div><div><dt>{t('service.failedCount')}</dt><dd>{activeRun.counters.failed}</dd></div>
        {activeRun.counters.timerAlertsFailed > 0 && <div><dt>{t('service.timerAlerts')}</dt><dd>{activeRun.counters.timerAlertsFailed}</dd></div>}
      </dl>}
      <ol className="service-log" ref={log} role="log" aria-label={t('service.activity')} tabIndex={0}>
        {activeRun.entries.length === 0 ? <li className="info">{t('service.waitingActivity')}</li> : activeRun.entries.slice(-100).map((entry, index) =>
          <li key={entry.seq ?? index} className={entry.kind}><time>{relativeTime(entry.at, now)}</time><span>{entry.text}</span></li>)}
      </ol>
    </>}
  </section>;
}
