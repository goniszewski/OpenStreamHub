import { useEffect, useSyncExternalStore } from 'react';
import { TriangleAlert, X } from 'lucide-react';
import type { DialContent } from './contracts';
import { SelectField } from './controls';
import { dialContentOptions, idleDisplaySettingsState } from './display-settings';
import type { DisplaySettingsStore } from './display-settings';
import { idleServiceState, isActiveRun } from './service-runner';
import type { ServiceStoreState } from './service-runner';

const noopSubscribe = () => () => {};
const idle = () => idleDisplaySettingsState;
const idleRunner = () => idleServiceState;

export const displayWarning = 'This has never been tested on real hardware. Images may not appear, may be rotated, in the wrong order or partly drawn, and the module may need to be unplugged to recover. Keep it off unless you are testing.';

// The note shown while profiles run: the setting is read when a profile starts, never mid-run.
export function appliesNote(activeRuns: number): string {
  const base = 'Changes apply the next time you start a profile.';
  if (activeRuns <= 0) return base;
  const rest = activeRuns === 1
    ? 'The running profile keeps the setting it started with until you stop and start it again'
    : `The ${activeRuns} running profiles keep the setting each started with until you stop and start them again`;
  return `${base} ${rest}; nothing is restarted for you.`;
}

// Settings panel for the opt-in, experimental module display images. Off by default.
export function DisplaySettingsPanel({ store, runner }: { store?: DisplaySettingsStore; runner?: { subscribe(listener: () => void): () => void; getState(): ServiceStoreState } }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const runs = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner).runs;
  const activeRuns = Object.values(runs).filter(isActiveRun).length;
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const off = !store || !state.desktop || state.busy || !state.loaded;
  const { enabled, dial_content: dialContent } = state.settings;

  return <section className="panel display-settings-panel" aria-labelledby="display-settings-title">
    <h2 id="display-settings-title">Module displays</h2>
    <label className="setting-toggle"><input type="checkbox" checked={enabled} disabled={off} aria-describedby="display-settings-warning display-settings-applies" onChange={event => { void store?.setEnabled(event.target.checked); }} />Experimental: send images to module displays</label>
    <p id="display-settings-warning" className="service-blocker display-warning" role="note"><TriangleAlert size={15} aria-hidden="true" /><span>{displayWarning}</span></p>
    <p className="fine-print">When on, a profile started with "Request device software mode" draws placeholder key images and folder pages on the 15-key module, and the content below on the encoder's dial screen, at most about once a second. Off by default.</p>
    <label htmlFor="display-dial-content">Dial screen shows</label>
    <SelectField id="display-dial-content" value={dialContent} disabled={off} onChange={event => { void store?.setDialContent(event.target.value as DialContent); }}>
      {dialContentOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </SelectField>
    <p id="display-settings-applies" className="fine-print" role="status">{appliesNote(activeRuns)}</p>
    {!state.desktop && <p className="fine-print">Module displays need the desktop app.</p>}
    {state.error && <div className="service-failure" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss display settings error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
  </section>;
}
