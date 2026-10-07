import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { CircleHelp, LoaderCircle, MoonStar, Play, Square, TriangleAlert } from 'lucide-react';
import type { LiveDevice, Profile, ServiceModule, ServiceState } from './contracts';
import { SelectField, ModalDialog } from './controls';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { findRun, idleServiceState, isActiveRun, isStale, maxReconnectAttempts } from './service-runner';
import type { ServiceRun, ServiceRunner } from './service-runner';
import { modules } from './ui-model';
import { capabilityLines, profileCapabilities, snapshotProfile, snippetPreview, stepLabel } from './profile-capabilities';
import type { RiskLevel } from './profile-capabilities';

// The Resume actions button, focused after Review on a paused-run notice.
export const serviceResumeId = 'service-resume';

export type PlatformInfo = { platform?: string; userAgent?: string };
export const isMacPlatform = (info: PlatformInfo | undefined) => /mac/i.test(`${info?.platform ?? ''} ${info?.userAgent ?? ''}`);

export const hasKeypadFolders = (profile: Profile) => profile.keypad?.pages.some(page => page.slots.some(slot => slot?.type === 'folder')) ?? false;

// Why Run is unavailable for this selection, or null. The service refuses nested M4315 profiles itself.
export function runBlocker(input: { desktop: boolean; device: LiveDevice | undefined; profile: Profile }): string | null {
  if (!input.desktop) return 'Running a profile needs the desktop app. Launch it to run actions.';
  if (!input.device) return 'No module connected. Connect a module to run this profile on it.';
  if (input.device.module === 'm4315' && hasKeypadFolders(input.profile)) return 'This profile uses keypad folders. The 15-key display only supports them offline, so the service will not run it live. Remove the folders to run it.';
  return null;
}

export const stateLabel = (state: ServiceState | undefined) => state === undefined ? 'Stopped' : state === 'exited' ? 'Stopped' : state[0].toUpperCase() + state.slice(1);

export const phaseLabel = (run: ServiceRun | undefined) => {
  switch (run?.session?.phase) {
    case 'suspended': return 'Paused';
    case 'waiting_for_device': return 'Waiting';
    case 'reconnecting': return 'Reconnecting';
    case 'needs_confirmation': return 'Needs confirmation';
    default: return stateLabel(run?.state);
  }
};
const phaseClass = (run: ServiceRun | undefined) => run?.session && run.session.phase !== 'running' && run.session.phase !== 'stopped' ? run.session.phase : run?.state ?? 'idle';
const moduleName = (module: ServiceModule) => modules.find(entry => entry.id === module)?.shortName ?? module;
const sessionDeviceName = (session: { module: ServiceModule; serialNumber: string | null }) => `${moduleName(session.module)}${session.serialNumber ? ` …${session.serialNumber.slice(-6)}` : ''}`;
export const countdownSeconds = (nextAttemptAt: number | undefined, now: number) => nextAttemptAt === undefined ? null : Math.max(0, Math.ceil((nextAttemptAt - now) / 1000));
const stoppedReason = { sleep: 'the Mac went to sleep', device_removed: 'the module was unplugged', service_exited: 'the service exited', user: 'you stopped it', profile_switch: 'a focused-app rule switched the profile' } as const;

export interface ConfirmCopy {
  title: string; confirm: string; message: string; risk: RiskLevel;
  // The capabilities in plain language, then every distinct text snippet and multi-step sequence.
  lines: string[]; snippets: { text: string; more: number; pressEnter: boolean }[]; sequences: string[][];
  // Set for a high-risk profile: the warning shown above the details and the label of the box that must be ticked.
  warning: string | null; acknowledge: string | null;
}

const unknownContents = 'This app cannot read the contents of this profile here, so it cannot list what will run. It may paste text, press Return, open web pages, capture the screen or run multi-step sequences. Review it on the Profiles page before you continue.';

// Copy for the Execute confirmation, used when starting a run and when resuming a paused one. It discloses what the
// profile will actually do (profileCapabilities over the whole profile). A null profile is one whose contents the
// frontend does not hold (a run it did not start, or one a backend switch moved to another profile): that is treated
// as high risk. A run a focused-app rule switched resumes with the saved profile it switched to, not the original snapshot.
export function confirmCopy(kind: 'start' | 'resume', profileName: string, deviceLabel: string, switched = false, profile: Profile | null = null): ConfirmCopy {
  const head = kind === 'start'
    ? { title: 'Run actions on this Mac?', confirm: 'Run actions', message: `Profile "${profileName}" will run on ${deviceLabel}. Everything listed here will really run on this Mac whenever you use the module, without asking again.` }
    : { title: 'Resume actions on this Mac?', confirm: 'Resume actions', message: `Profile "${profileName}" (${switched ? 'the saved profile a focused-app rule switched to' : 'as it was when this run started'}) will resume on ${deviceLabel}. Everything listed here will really run on this Mac again whenever you use the module, without asking again.` };
  const capabilities = profile ? profileCapabilities(profile) : null;
  const risk: RiskLevel = capabilities?.risk ?? 'high';
  const reasons = capabilities ? capabilities.riskReasons : ['The profile contents could not be checked.'];
  return {
    ...head, risk,
    lines: capabilities ? capabilityLines(capabilities) : [unknownContents],
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
  return run ? confirmCopy('resume', run.profileName, run.session ? sessionDeviceName(run.session) : 'the module', run.profileSwitched === true, snapshotProfile(run.snapshot)) : null;
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
const deviceName = (device: LiveDevice) => `${modules.find(entry => entry.id === device.module)?.shortName ?? device.module} ${device.serial_number ?? device.path_hex.slice(-6)}`;

export function ServicePanel({ runner, feed, profile, profileId = null, desktop, unsaved, platform = typeof navigator === 'undefined' ? undefined : navigator, now: fixedNow }: {
  runner?: ServiceRunner; feed?: DeviceFeed; profile: Profile; profileId?: string | null; desktop: boolean; unsaved: boolean; platform?: PlatformInfo; now?: number;
}) {
  const service = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner);
  const devices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleFeed, feed?.getState ?? idleFeed).devices;
  const [chosen, setChosen] = useState('');
  const [execute, setExecute] = useState(false);
  const [softwareMode, setSoftwareMode] = useState(false);
  const [confirm, setConfirm] = useState<((ok: boolean) => void) | null>(null);
  const [confirmKind, setConfirmKind] = useState<'start' | 'resume'>('start');
  // The run being resumed. The dialog names it, not whatever run the device selection shows by the time it renders.
  const [resumeId, setResumeId] = useState<string | null>(null);
  // The high-risk acknowledgement. Cleared with every answer, so each confirmation asks again.
  const [acknowledged, setAcknowledged] = useState(false);
  const [clock, setClock] = useState(() => Date.now());
  const log = useRef<HTMLOListElement>(null);
  // A confirmation still open when the panel unmounts (page change) answers false, so the start or resume never hangs.
  const pendingConfirm = useRef(confirm);
  pendingConfirm.current = confirm;
  useEffect(() => () => settlePendingConfirm(pendingConfirm), []);
  const mac = isMacPlatform(platform);
  const enabled = desktop && runner !== undefined;

  const device = devices.find(item => item.path_hex === chosen) ?? devices[0];
  const run: ServiceRun | undefined = findRun(service.runs, device);
  const session = run?.session;
  const phase = session?.phase ?? (isActiveRun(run) ? 'running' : undefined);
  const active = isActiveRun(run);
  const blocker = runBlocker({ desktop: enabled, device, profile });
  const stale = isStale(run, profile);
  const now = fixedNow ?? clock;
  const executeSelected = execute && mac;

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
  useEffect(() => { if (log.current) log.current.scrollTop = log.current.scrollHeight; }, [run?.entries.length]);

  const begin = () => {
    if (!runner || !device) return;
    const confirmExecute = executeSelected ? () => new Promise<boolean>(resolve => { setConfirmKind('start'); setConfirm(() => resolve); }) : undefined;
    void runner.start({ profile, profileId, pathHex: device.path_hex, execute: executeSelected, softwareMode, draftUnsaved: unsaved, device }, confirmExecute);
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
  const copy = dialog ?? startCopy;

  return <section className="service-panel" aria-label="Run profile">
    <div className="section-heading"><h2>Run profile</h2><span className={`service-badge ${phaseClass(run)}`} role="status">{phaseLabel(run)}</span></div>
    {run && active && phase === 'running' && run.execute && <div className="service-banner execute" role="alert"><TriangleAlert size={20} /><div><strong>Actions are running on this Mac</strong><span>Every action in this profile, including pasted text, web pages and multi-step sequences, really runs when you use the module. Stop to end it.</span></div></div>}
    {run && active && phase === 'running' && !run.execute && <div className="service-banner" role="status"><CircleHelp size={18} /><span>Dry run: actions are logged only. Nothing changes on this Mac.</span></div>}
    {run && session && phase === 'suspended' && <div className="service-banner paused" role="status"><MoonStar size={18} /><div><strong>Paused while your Mac sleeps</strong><span>{run.execute ? 'Actions stay off. You will be asked before they resume.' : 'It resumes automatically in dry-run mode.'}</span></div></div>}
    {run && session && phase === 'waiting_for_device' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>Waiting for {sessionDeviceName(session)} to reconnect…</strong><span>{run.execute ? 'Actions are paused and will not run until you confirm.' : 'The dry run continues when the module is back.'}</span></div></div>}
    {run && session && phase === 'reconnecting' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>Reconnecting to {sessionDeviceName(session)} (attempt {session.attempt}/{maxReconnectAttempts})</strong><span>{countdownSeconds(session.nextAttemptAt, now) === null ? 'Trying now.' : `Next attempt in ${countdownSeconds(session.nextAttemptAt, now)}s.`}{run.execute ? ' Actions are paused.' : ''}</span></div></div>}
    {run && session && phase === 'needs_confirmation' && <div className="service-banner paused attention" role="alert"><TriangleAlert size={20} /><div><strong>Actions are paused. Nothing will run until you resume.</strong><span>{session.reason ? `Paused because ${stoppedReason[session.reason]}. ` : ''}{session.detail ?? ''} {run.profileSwitched ? `Resuming starts the saved profile "${run.profileName}" a focused-app rule switched to.` : `Resuming restarts "${run.profileName}" exactly as it was started.`}</span><button type="button" id={serviceResumeId} className="button danger" onClick={resume}>Resume actions…</button></div></div>}
    {run && session && phase === 'stopped' && <div className="service-banner stopped" role="status"><CircleHelp size={18} /><div><strong>Session stopped{session.reason ? `: ${stoppedReason[session.reason]}` : ''}</strong>{session.detail && <span>{session.detail}</span>}</div></div>}
    {active && phase === 'running' && <p className="fine-print" role="status">Live preview paused while actions run.</p>}
    {blocker && !active && <p className="service-blocker" role="status">{blocker}</p>}
    <div className="service-controls">
      <label className="sr-only" htmlFor="service-device">Device to run on</label>
      <SelectField id="service-device" value={device?.path_hex ?? ''} disabled={!enabled || active || devices.length === 0} onChange={event => setChosen(event.target.value)}>
        {devices.length === 0 ? <option value="">No module connected</option> : devices.map(item => <option key={item.path_hex} value={item.path_hex}>{deviceName(item)}</option>)}
      </SelectField>
      <div className="mode-toggle" role="group" aria-label="Run mode">
        <button type="button" aria-pressed={!executeSelected} disabled={!enabled || active} onClick={() => setExecute(false)}>Dry run (default)</button>
        <button type="button" aria-pressed={executeSelected} disabled={!enabled || active || !mac} onClick={() => setExecute(true)}>Execute actions</button>
      </div>
      {run && active
        ? <button type="button" className="button" disabled={run.state === 'stopping'} onClick={() => { void runner?.stop(run.serviceId); }}><Square size={16} />{run.state === 'stopping' ? 'Stopping…' : 'Stop'}</button>
        : <button type="button" className={`button ${executeSelected ? 'danger' : 'primary'}`} disabled={blocker !== null} onClick={begin}>{run?.state === 'starting' ? <LoaderCircle size={16} className="spinning" /> : <Play size={16} />}{executeSelected ? 'Run with actions' : 'Run'}</button>}
    </div>
    <label className="service-option"><input type="checkbox" checked={softwareMode} disabled={!enabled || active} onChange={event => setSoftwareMode(event.target.checked)} />Request device software mode (encoder and 15-key modules)</label>
    {!mac && enabled && <p className="fine-print">Executing actions is only available on macOS. Dry runs log what would happen.</p>}
    {run && <p className="fine-print">Profile "{run.profileName}"{run.draftUnsaved ? ' (unsaved draft)' : ''} was used when this run started.</p>}
    {stale && <p className="service-stale" role="status"><TriangleAlert size={15} />Profile changed since start. Restart to apply.</p>}
    {run?.state === 'failed' && <div className="service-failure" role="alert"><strong>{run.error ?? 'The service failed.'}</strong>{run.stderrTail && <pre>{run.stderrTail}</pre>}</div>}
    {run?.state === 'exited' && run.exitCode !== undefined && run.exitCode !== 0 && <p className="fine-print">The service exited with code {run.exitCode}.</p>}
    {service.error && <p className="service-failure" role="alert">{service.error}</p>}
    {run && <>
      <dl className="service-counters" aria-label="Run counters">
        <div><dt>Events</dt><dd>{run.counters.events}</dd></div><div><dt>Planned</dt><dd>{run.counters.planned}</dd></div>
        <div><dt>Executed</dt><dd>{run.counters.executed}</dd></div><div><dt>Failed</dt><dd>{run.counters.failed}</dd></div>
      </dl>
      <ol className="service-log" ref={log} role="log" aria-label="Service activity" tabIndex={0}>
        {run.entries.length === 0 ? <li className="info">Waiting for activity…</li> : run.entries.slice(-100).map((entry, index) =>
          <li key={entry.seq ?? index} className={entry.kind}><time>{relativeTime(entry.at, now)}</time><span>{entry.text}</span></li>)}
      </ol>
    </>}
    <ModalDialog open={confirm !== null && !resumeGone} role="alertdialog" labelledBy="execute-title" describedBy={copy.warning ? 'execute-warning execute-message' : 'execute-message'} onCancel={() => answer(false)} className="execute-dialog">
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
    </ModalDialog>
  </section>;
}
