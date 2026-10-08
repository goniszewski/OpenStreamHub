import { useEffect, useSyncExternalStore } from 'react';
import { X } from 'lucide-react';
import type { DialContent, ModuleControl } from './contracts';
import { SelectField } from './controls';
import { dialContentOptions, idleDisplaySettingsState, moduleControlOptions } from './display-settings';
import type { DisplaySettingsStore } from './display-settings';
import { idleServiceState, isActiveRun } from './service-runner';
import type { ServiceStoreState } from './service-runner';

const noopSubscribe = () => () => {};
const idle = () => idleDisplaySettingsState;
const idleRunner = () => idleServiceState;

// What each Module control value does, shown under the choice.
export const moduleControlNotes: Record<ModuleControl, string> = {
  app: 'OpenStreamHub drives the 15-key module and the encoder dial whenever a profile runs on them: it switches them to software mode and gives them back to their built-in mode when the run ends. On the 15-key module it draws placeholder key images and folder pages for profiles with 15-key pages; for a flat (older) 15-key profile it draws nothing on the keys, and what they show then has not been checked. The dial screen shows the content below (redrawn at most about once a second). Other modules are not affected.',
  built_in_animation: 'The 15-key module and the dial keep their built-in animation. On this setting their keys and dial send no input unless you tick "Request device software mode" in the Run panel, nothing is drawn on their displays, and profiles with 15-key pages (folders) cannot run live.',
};
// The honest limits of what was tested on hardware (docs/masterhub-hardware-results.md).
export const moduleControlCaveat = 'Tested on one 15-key module and one encoder dial, mostly with the service on its own. Through this app, only a dry-run dial session (and force-quitting the app during it) has been run on real hardware; a 15-key session, a crash while on the 15-key module, sleep and wake, reconnect, and unplugging a module while it is in software mode have not been tested. Key images are placeholders; only the first three keys have been confirmed to line up between what is drawn and what is pressed, and display latency has not been measured.';

// The note shown while profiles run: the setting is read when a profile starts, never mid-run.
export function appliesNote(activeRuns: number): string {
  const base = 'Changes apply the next time you start a profile.';
  if (activeRuns <= 0) return base;
  const rest = activeRuns === 1
    ? 'The running profile keeps the setting it started with until you stop and start it again'
    : `The ${activeRuns} running profiles keep the setting each started with until you stop and start them again`;
  return `${base} ${rest}; nothing is restarted for you.`;
}

// Settings panel for Module control (App by default) and the dial content.
export function DisplaySettingsPanel({ store, runner }: { store?: DisplaySettingsStore; runner?: { subscribe(listener: () => void): () => void; getState(): ServiceStoreState } }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const runs = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner).runs;
  const activeRuns = Object.values(runs).filter(isActiveRun).length;
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const off = !store || !state.desktop || state.busy || !state.loaded;
  const { module_control: moduleControl, dial_content: dialContent } = state.settings;

  return <section className="panel display-settings-panel" aria-labelledby="display-settings-title">
    <h2 id="display-settings-title">Module control</h2>
    <label htmlFor="display-module-control">Who controls the 15-key module and the encoder dial</label>
    <SelectField id="display-module-control" value={moduleControl} disabled={off} aria-describedby="display-module-control-note display-settings-applies" onChange={event => { void store?.setModuleControl(event.target.value as ModuleControl); }}>
      {moduleControlOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </SelectField>
    <p id="display-module-control-note" className="fine-print">{moduleControlNotes[moduleControl]}</p>
    <label htmlFor="display-dial-content">Dial screen shows</label>
    <SelectField id="display-dial-content" value={dialContent} disabled={off || moduleControl !== 'app'} onChange={event => { void store?.setDialContent(event.target.value as DialContent); }}>
      {dialContentOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </SelectField>
    <p className="fine-print">{moduleControlCaveat}</p>
    <p id="display-settings-applies" className="fine-print" role="status">{appliesNote(activeRuns)}</p>
    {!state.desktop && <p className="fine-print">Module control needs the desktop app.</p>}
    {state.error && <div className="service-failure" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss display settings error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
  </section>;
}
