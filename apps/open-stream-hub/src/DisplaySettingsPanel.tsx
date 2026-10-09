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
  app: 'OpenStreamHub drives the 15-key module, the encoder dial and the knobs whenever it is open: when the app opens and whenever one of them is connected, it starts a run on it with the default profile below (otherwise the profile last run on that module, or a blank one). The 15-key module and the dial switch to software mode and go back to their built-in mode when the run stops normally or you quit; the knobs only have their input read, and nothing is written to them. That run is a dry run unless Run actions automatically is on; to run actions without that, stop it and choose Run with Execute actions. Stopping it leaves the module alone until it is connected again or the app is opened again. Switching this setting to App starts the modules that are connected; switching away from App stops the runs that started by themselves. On the 15-key module, Stop, Quit, a force-quit during a run, sleep and wake, and unplugging and replugging (the run reconnects) have been checked on hardware; on the dial, a force-quit, a normal Stop and unplugging and replugging have. A normal Quit and sleep and wake on the dial have not been checked. On the 15-key module it draws placeholder key images and folder pages; a flat (older) 15-key profile is drawn as one page, which has not been checked on hardware. The dial screen shows the content below (redrawn at most about once a second); with "Active dial function name", a profile with nothing on the dial shows the analog clock. Other modules are not affected.',
  built_in_animation: 'The 15-key module and the dial keep their built-in animation, and nothing starts on them by itself. On this setting their keys and dial send no input unless you tick "Request device software mode" in the Run panel, nothing is drawn on their displays, and profiles with 15-key pages (folders) cannot run live.',
};
// The honest limits of what was tested on hardware (docs/masterhub-hardware-results.md).
export const moduleControlCaveat = 'Tested on one 15-key module, one encoder dial and one knob module. Through this app, dry-run sessions on the 15-key module have been checked with Stop, Quit, force-quitting the app, sleep and wake, and unplugging and replugging (the run reconnects); dry-run dial sessions with force-quitting the app, a normal Stop, and unplugging and replugging. Not yet tested on real hardware: a run that starts by itself, one that runs actions by itself, a knob run that only reads input, the blank and flat (one-page) profiles on the 15-key module, a normal Quit on the dial, and sleep and wake on the dial. Key images are placeholders; only the first three keys have been confirmed to line up between what is drawn and what is pressed, and display latency has not been measured.';

// "Run actions automatically": the disclosure stays next to the toggle, worded as agreed.
export const autoExecuteDisclosure = 'Key and knob presses will act on your Mac from the moment the app opens, and again after sleep, wake and reconnect, with no confirmation.';
export const autoExecuteNote = 'Off by default. Turning it on or off restarts the runs that started by themselves; turning it off ends their actions at once and they go on as dry runs. Imported profiles not yet allowed to execute (Profiles page) still only run as dry runs, and the Run panel says why. Actions run only on macOS.';
export const defaultProfileNote = 'The profile every module that starts by itself uses (15-key module, encoder dial and knobs; a profile holds the bindings of each module). With No default, each module uses the profile last run on it, or a blank one. A default that is deleted or cannot be read is skipped the same way. Applies the next time a module starts by itself.';

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
export function DisplaySettingsPanel({ store, runner, profiles = [] }: { store?: DisplaySettingsStore; runner?: { subscribe(listener: () => void): () => void; getState(): ServiceStoreState }; profiles?: { id: string; name: string }[] }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const runs = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner).runs;
  const activeRuns = Object.values(runs).filter(isActiveRun).length;
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const chosen = state.defaultProfile;
  useEffect(() => { if (store && state.desktop && !chosen.loaded && !chosen.busy && !chosen.error) void store.loadDefaultProfile(); }, [store, state.desktop, chosen.loaded, chosen.busy, chosen.error]);
  const off = !store || !state.desktop || state.busy || !state.loaded;
  const { module_control: moduleControl, dial_content: dialContent, auto_execute: autoExecute } = state.settings;
  const missingDefault = chosen.profileId !== null && !profiles.some(profile => profile.id === chosen.profileId);

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
    <label className="setting-toggle"><input type="checkbox" checked={autoExecute} disabled={off || moduleControl !== 'app'} aria-describedby="display-auto-execute-disclosure display-auto-execute-note" onChange={event => { void store?.setAutoExecute(event.target.checked); }} />Run actions automatically</label>
    <p id="display-auto-execute-disclosure" className="fine-print auto-execute-disclosure"><strong>{autoExecuteDisclosure}</strong></p>
    <p id="display-auto-execute-note" className="fine-print">{autoExecuteNote}</p>
    <label htmlFor="display-default-profile">Default profile</label>
    <SelectField id="display-default-profile" value={chosen.profileId ?? ''} disabled={!store || !state.desktop || chosen.busy || !chosen.loaded} aria-describedby="display-default-profile-note" onChange={event => { void store?.setDefaultProfile(event.target.value || null); }}>
      <option value="">No default</option>
      {profiles.map(profile => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
      {missingDefault && <option value={chosen.profileId!}>Missing profile (skipped)</option>}
    </SelectField>
    <p id="display-default-profile-note" className="fine-print">{defaultProfileNote}</p>
    {chosen.error && <div className="service-failure" role="alert"><span>{chosen.error}</span></div>}
    <p className="fine-print">{moduleControlCaveat}</p>
    <p id="display-settings-applies" className="fine-print" role="status">{appliesNote(activeRuns)}</p>
    {!state.desktop && <p className="fine-print">Module control needs the desktop app.</p>}
    {state.error && <div className="service-failure" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss display settings error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
  </section>;
}
