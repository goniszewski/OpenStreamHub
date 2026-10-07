import { useRef, useState } from 'react';
import { Check, Copy, LifeBuoy, LoaderCircle } from 'lucide-react';
import type { Profile } from './contracts';
import { SelectField } from './controls';
import {
  buildDiagnosticsClient, buildDiagnosticsRequest, clampCaptureSeconds, defaultDiagnosticsOptions, formatBytes, maxNotesChars, runDiagnostics,
} from './diagnostics';
import type { DiagnosticsOptions, DiagnosticsResult } from './diagnostics';
import type { DeviceFeed } from './device-feed';
import { idleFeedState } from './device-feed';
import type { Invoke } from './profile-session';
import type { ServiceRunner } from './service-runner';
import { idleServiceState } from './service-runner';

export function DiagnosticsPanel({ desktop, invoke, profile, runner, feed, initialResult, initialOptions }: {
  desktop: boolean; invoke: Invoke; profile: Profile; runner?: ServiceRunner; feed?: DeviceFeed;
  initialResult?: DiagnosticsResult; initialOptions?: Partial<DiagnosticsOptions>;
}) {
  const [options, setOptions] = useState<DiagnosticsOptions>({ ...defaultDiagnosticsOptions, ...initialOptions });
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<DiagnosticsResult | null>(initialResult ?? null);
  const [error, setError] = useState<string | null>(null);
  const [copy, setCopy] = useState<string | null>(null);
  const inFlight = useRef(false);
  const set = <K extends keyof DiagnosticsOptions>(key: K, value: DiagnosticsOptions[K]) => setOptions(current => ({ ...current, [key]: value }));
  const locked = !desktop || busy;

  const create = async () => {
    if (inFlight.current || !desktop) return;
    inFlight.current = true; setBusy(true); setError(null); setCopy(null);
    const client = buildDiagnosticsClient(runner?.getState() ?? idleServiceState, feed?.getState() ?? idleFeedState, options.notes);
    const outcome = await runDiagnostics(invoke, buildDiagnosticsRequest(options, profile, client));
    if (outcome.ok) setResult(outcome.result); else setError(outcome.error);
    inFlight.current = false; setBusy(false);
  };

  const copyPath = async () => {
    if (!result) return;
    try { await navigator.clipboard.writeText(result.path); setCopy('Path copied.'); }
    catch { setCopy('Could not copy automatically. Select the path above and copy it.'); }
  };

  return <section className="panel diagnostics-panel" aria-labelledby="diagnostics-title">
    <h2 id="diagnostics-title"><LifeBuoy size={18} />Diagnostics</h2>
    <p>Create a local file that helps explain how this Mac and your modules behave. It contains app and OS information, the USB device list with HID descriptors, an optional short passive input capture, and recent service and device-feed activity.</p>
    <p className="fine-print"><strong>Nothing is uploaded or sent.</strong> The file stays on this Mac. You decide whether to share it.</p>
    {!desktop && <p className="fine-print" role="status">Creating a bundle needs the desktop app.</p>}
    <div className="diagnostics-options">
      <label className="service-option"><input type="checkbox" checked={options.redactSerials} disabled={locked} onChange={event => set('redactSerials', event.target.checked)} />Hide serial numbers</label>
      <label className="service-option"><input type="checkbox" checked={options.includeCapture} disabled={locked} onChange={event => set('includeCapture', event.target.checked)} />Include a short input capture</label>
      <div className="diagnostics-seconds">
        <label htmlFor="diagnostics-seconds">Capture length</label>
        <SelectField id="diagnostics-seconds" value={options.captureSeconds} disabled={locked || !options.includeCapture} onChange={event => set('captureSeconds', clampCaptureSeconds(Number(event.target.value)))}>
          {[1, 2, 3, 4, 5].map(value => <option key={value} value={value}>{value} {value === 1 ? 'second' : 'seconds'}</option>)}
        </SelectField>
      </div>
      <p className="fine-print">The capture only listens. Modules in use by a device feed or a running service are skipped, so stop those first. Move or press controls on the modules during the capture.</p>
      <label className="service-option"><input type="checkbox" checked={options.includeProfile} disabled={locked} onChange={event => set('includeProfile', event.target.checked)} />Include current profile</label>
      <p className="fine-print">The profile draft is included as it is now. Application bundle IDs, keyboard shortcuts, text snippets and web addresses in it would be part of the file. With Hide serial numbers on, each text snippet is replaced by its length and each web address is reduced to its site address (no path or query).</p>
      <label htmlFor="diagnostics-notes">Notes (optional)</label>
      <textarea id="diagnostics-notes" rows={3} maxLength={maxNotesChars} value={options.notes} disabled={locked} placeholder="What were you trying to do?" onChange={event => set('notes', event.target.value)} />
      <p className="fine-print diagnostics-counter">{options.notes.length} / {maxNotesChars}</p>
    </div>
    <button type="button" className="button primary" disabled={locked} onClick={() => { void create(); }}>
      {busy ? <LoaderCircle size={16} className="spinning" /> : <LifeBuoy size={16} />}{busy ? 'Creating bundle…' : 'Create diagnostic bundle'}
    </button>
    {error && <div className="service-failure" role="alert"><strong>Could not create the bundle.</strong> {error}</div>}
    {result && <div className="diagnostics-result" role="status">
      <strong>Bundle created</strong>
      <code className="storage-path">{result.path}</code>
      <p className="fine-print">{formatBytes(result.bytes)} · {result.summary.devices} {result.summary.devices === 1 ? 'device' : 'devices'} · {result.summary.descriptors_read} descriptors read · {result.summary.descriptor_errors} descriptor errors · {result.summary.capture_reports} capture reports · serial numbers {result.summary.redacted ? 'hidden' : 'included'}</p>
      <div className="diagnostics-actions"><button type="button" className="button" onClick={() => { void copyPath(); }}>{copy === 'Path copied.' ? <Check size={16} /> : <Copy size={16} />}Copy path</button>{copy && <span className="fine-print">{copy}</span>}</div>
      <p className="fine-print">Nothing was sent anywhere. If you choose to share it, attach the file manually.</p>
    </div>}
  </section>;
}
