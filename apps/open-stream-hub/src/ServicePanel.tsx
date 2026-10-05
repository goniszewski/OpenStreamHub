import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { CircleHelp, LoaderCircle, MoonStar, Play, Square, TriangleAlert } from 'lucide-react';
import type { LiveDevice, Profile, ServiceModule, ServiceState } from './contracts';
import { SelectField, ModalDialog } from './controls';
import { idleFeedState } from './device-feed';
import type { DeviceFeed } from './device-feed';
import { findRun, idleServiceState, isActiveRun, isStale, maxReconnectAttempts } from './service-runner';
import type { ServiceRun, ServiceRunner } from './service-runner';
import { modules } from './ui-model';

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
const stoppedReason = { sleep: 'the Mac went to sleep', device_removed: 'the module was unplugged', service_exited: 'the service exited', user: 'you stopped it' } as const;

// Copy for the Execute confirmation, used when starting a run and when resuming a paused one.
export function confirmCopy(kind: 'start' | 'resume', profileName: string, deviceLabel: string) {
  return kind === 'start'
    ? { title: 'Run actions on this Mac?', confirm: 'Run actions', message: `Profile "${profileName}" will run on ${deviceLabel}. Volume, shortcut and app-launch actions from the profile will really run on this Mac whenever you use the module.` }
    : { title: 'Resume actions on this Mac?', confirm: 'Resume actions', message: `Profile "${profileName}" (as it was when this run started) will resume on ${deviceLabel}. Volume, shortcut and app-launch actions will really run on this Mac again whenever you use the module.` };
}

export function relativeTime(at: number, now: number): string {
  const seconds = Math.max(0, Math.round((now - at) / 1000));
  if (seconds < 5) return 'now';
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.floor(seconds / 60);
  return minutes < 60 ? `${minutes}m ago` : `${Math.floor(minutes / 60)}h ago`;
}

const noopSubscribe = () => () => {};
const idleFeed = () => idleFeedState;
const idleRunner = () => idleServiceState;
const deviceName = (device: LiveDevice) => `${modules.find(entry => entry.id === device.module)?.shortName ?? device.module} ${device.serial_number ?? device.path_hex.slice(-6)}`;

export function ServicePanel({ runner, feed, profile, desktop, unsaved, platform = typeof navigator === 'undefined' ? undefined : navigator, now: fixedNow }: {
  runner?: ServiceRunner; feed?: DeviceFeed; profile: Profile; desktop: boolean; unsaved: boolean; platform?: PlatformInfo; now?: number;
}) {
  const service = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner);
  const devices = useSyncExternalStore(feed?.subscribe ?? noopSubscribe, feed?.getState ?? idleFeed, feed?.getState ?? idleFeed).devices;
  const [chosen, setChosen] = useState('');
  const [execute, setExecute] = useState(false);
  const [softwareMode, setSoftwareMode] = useState(false);
  const [confirm, setConfirm] = useState<((ok: boolean) => void) | null>(null);
  const [confirmKind, setConfirmKind] = useState<'start' | 'resume'>('start');
  const [clock, setClock] = useState(() => Date.now());
  const log = useRef<HTMLOListElement>(null);
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
    void runner.start({ profile, pathHex: device.path_hex, execute: executeSelected, softwareMode, draftUnsaved: unsaved }, confirmExecute);
  };
  const resume = () => {
    if (!runner || !run) return;
    void runner.resumeSession(run.serviceId, () => new Promise<boolean>(resolve => { setConfirmKind('resume'); setConfirm(() => resolve); }));
  };
  const answer = (ok: boolean) => { confirm?.(ok); setConfirm(null); };

  const copy = confirmKind === 'resume' && run
    ? confirmCopy('resume', run.profileName, session ? sessionDeviceName(session) : 'the module')
    : confirmCopy('start', profile.name, device ? deviceName(device) : 'the selected module');

  return <section className="service-panel" aria-label="Run profile">
    <div className="section-heading"><h2>Run profile</h2><span className={`service-badge ${phaseClass(run)}`} role="status">{phaseLabel(run)}</span></div>
    {run && active && phase === 'running' && run.execute && <div className="service-banner execute" role="alert"><TriangleAlert size={20} /><div><strong>Actions are running on this Mac</strong><span>Volume, shortcut and app-launch actions from this profile really run when you use the module. Stop to end it.</span></div></div>}
    {run && active && phase === 'running' && !run.execute && <div className="service-banner" role="status"><CircleHelp size={18} /><span>Dry run: actions are logged only. Nothing changes on this Mac.</span></div>}
    {run && session && phase === 'suspended' && <div className="service-banner paused" role="status"><MoonStar size={18} /><div><strong>Paused while your Mac sleeps</strong><span>{run.execute ? 'Actions stay off. You will be asked before they resume.' : 'It resumes automatically in dry-run mode.'}</span></div></div>}
    {run && session && phase === 'waiting_for_device' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>Waiting for {sessionDeviceName(session)} to reconnect…</strong><span>{run.execute ? 'Actions are paused and will not run until you confirm.' : 'The dry run continues when the module is back.'}</span></div></div>}
    {run && session && phase === 'reconnecting' && <div className="service-banner paused" role="status"><LoaderCircle size={18} className="spinning" /><div><strong>Reconnecting to {sessionDeviceName(session)} (attempt {session.attempt}/{maxReconnectAttempts})</strong><span>{countdownSeconds(session.nextAttemptAt, now) === null ? 'Trying now.' : `Next attempt in ${countdownSeconds(session.nextAttemptAt, now)}s.`}{run.execute ? ' Actions are paused.' : ''}</span></div></div>}
    {run && session && phase === 'needs_confirmation' && <div className="service-banner paused attention" role="alert"><TriangleAlert size={20} /><div><strong>Actions are paused. Nothing will run until you resume.</strong><span>{session.reason ? `Paused because ${stoppedReason[session.reason]}. ` : ''}{session.detail ?? ''} Resuming restarts "{run.profileName}" exactly as it was started.</span><button type="button" className="button danger" onClick={resume}>Resume actions…</button></div></div>}
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
          <li key={`${run.entries.length}-${index}`} className={entry.kind}><time>{relativeTime(entry.at, now)}</time><span>{entry.text}</span></li>)}
      </ol>
    </>}
    <ModalDialog open={confirm !== null} labelledBy="execute-title" describedBy="execute-message" onCancel={() => answer(false)}>
      <h2 id="execute-title">{copy.title}</h2>
      <p id="execute-message">{copy.message}</p>
      <div className="dialog-actions"><button type="button" className="button" autoFocus onClick={() => answer(false)}>Cancel</button><button type="button" className="button danger" onClick={() => answer(true)}>{copy.confirm}</button></div>
    </ModalDialog>
  </section>;
}
