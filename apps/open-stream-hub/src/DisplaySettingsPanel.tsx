import { useEffect, useSyncExternalStore } from 'react';
import { X } from 'lucide-react';
import type { ModuleControl } from './contracts';
import { SelectField } from './controls';
import { idleDisplaySettingsState, moduleControlOptions } from './display-settings';
import type { DisplaySettingsStore } from './display-settings';
import { idleServiceState, isActiveRun } from './service-runner';
import type { ServiceStoreState } from './service-runner';

const noopSubscribe = () => () => {};
const idle = () => idleDisplaySettingsState;
const idleRunner = () => idleServiceState;

// What each Module control value does, shown under the choice.
export const moduleControlNotes: Record<ModuleControl, string> = {
  app: 'OpenStreamHub draws on the 15-key module and the dial, and starts a run on each module that is connected.',
  built_in_animation: 'The 15-key module and the dial keep their built-in animation, and nothing starts on them by itself. Their keys and dial send no input unless you tick "Request device software mode" in the Run panel, nothing is drawn on their displays, and profiles with 15-key pages (folders) cannot run live.',
};
// The details behind the App choice, folded away under it.
export const moduleControlDetails = [
  'A run starts with the default profile below, otherwise the profile last run on that module, or a blank one.',
  'The 15-key module and the dial use software mode and return to their built-in mode when a run stops normally or you quit. The knobs only have their input read.',
  'What a press does, Live or Test, is the run mode in the header, not this setting.',
  'Stopping a run leaves the module alone until it is connected again or the app is opened again.',
  'Switching to App starts the connected modules; switching away stops the runs that started by themselves.',
  'The dial shows the active function name, or the analog clock for a profile with nothing on the dial.',
  'The 15-key module shows placeholder key images and folder pages; a flat (older) profile is drawn as one page.',
];
// The honest limits of what was tested on hardware (docs/masterhub-hardware-results.md), folded away.
export const moduleControlCaveat = [
  'Tested on one 15-key module, one encoder dial and one knob module.',
  '15-key module, dry run through this app: Stop, Quit, force-quit, sleep and wake, unplugging and replugging.',
  'Dial, dry run: force-quit, a normal Stop, unplugging and replugging.',
  'Not yet tested: a run that starts by itself or runs actions by itself, a knob run, blank and flat (one-page) profiles on the 15-key module, a normal Quit on the dial, sleep and wake on the dial.',
  'Key images are placeholders; only the first three keys have been confirmed to line up between what is drawn and what is pressed, and display latency has not been measured.',
];

export const defaultProfileNote = 'Used by every module that starts by itself. With No default, each uses the profile last run on it, or a blank one. Applies the next time a module starts.';

// The note shown while profiles run: module control is read when a profile
// starts, never mid-run (the run mode, by contrast, reaches running modules
// at once through its own switch in the header).
export function appliesNote(activeRuns: number): string {
  const base = 'Changes apply the next time a profile starts.';
  if (activeRuns <= 0) return base;
  const rest = activeRuns === 1
    ? 'The running profile keeps the setting it started with until you stop and start it again'
    : `The ${activeRuns} running profiles keep the setting each started with until you stop and start them again`;
  return `${base} ${rest}; nothing is restarted for you.`;
}

// What the press feedback setting does, shown under the checkbox.
export const pressFeedbackNote = 'Draws a key dimmed while it is held, so you can see the press was received. It never delays the action, and switches itself off for the rest of a run if a key write is slow. Applies the next time a profile starts.';

// What "Switch to Test mode while editing" does, shown under the checkbox.
export const testWhileEditingNote = 'While you edit a binding or have unsaved changes, presses run in Test mode and the header badge reads “Test (editing)”. It returns to your chosen mode when you save, undo or close the editor.';

// Settings panel for Module control (App by default), the default profile and
// the run mode's editing hold.
export function DisplaySettingsPanel({ store, runner, profiles = [], testWhileEditing = true, onTestWhileEditing }: { store?: DisplaySettingsStore; runner?: { subscribe(listener: () => void): () => void; getState(): ServiceStoreState }; profiles?: { id: string; name: string }[]; testWhileEditing?: boolean; onTestWhileEditing?: (on: boolean) => void }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const runs = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner).runs;
  const activeRuns = Object.values(runs).filter(isActiveRun).length;
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const chosen = state.defaultProfile;
  useEffect(() => { if (store && state.desktop && !chosen.loaded && !chosen.busy && !chosen.error) void store.loadDefaultProfile(); }, [store, state.desktop, chosen.loaded, chosen.busy, chosen.error]);
  const off = !store || !state.desktop || state.busy || !state.loaded;
  const { module_control: moduleControl } = state.settings;
  const missingDefault = chosen.profileId !== null && !profiles.some(profile => profile.id === chosen.profileId);

  return <section className="panel display-settings-panel" aria-labelledby="display-settings-title">
    <h2 id="display-settings-title">Let OpenStreamHub drive the display</h2>
    <label htmlFor="display-module-control">Who controls the 15-key module and the encoder dial</label>
    <SelectField id="display-module-control" value={moduleControl} disabled={off} aria-describedby="display-module-control-note display-settings-applies" onChange={event => { void store?.setModuleControl(event.target.value as ModuleControl); }}>
      {moduleControlOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
    </SelectField>
    <p id="display-module-control-note" className="fine-print">{moduleControlNotes[moduleControl]}</p>
    {moduleControl === 'app' && <details className="learn-more"><summary>More about App control</summary><ul className="fine-print notes-list">{moduleControlDetails.map(line => <li key={line}>{line}</li>)}</ul></details>}
    <label className="service-option" htmlFor="display-press-feedback">
      <input id="display-press-feedback" type="checkbox" checked={state.settings.press_feedback} disabled={off || moduleControl !== 'app'}
        aria-describedby="display-press-feedback-note" onChange={event => { void store?.setPressFeedback(event.target.checked); }} />
      Dim a key while it is pressed
    </label>
    <p id="display-press-feedback-note" className="fine-print">{pressFeedbackNote}</p>
    <h3 className="display-settings-subtitle">Run mode</h3>
    <label className="service-option" htmlFor="run-mode-editing">
      <input id="run-mode-editing" type="checkbox" checked={testWhileEditing} disabled={!store || !state.desktop}
        aria-describedby="run-mode-editing-note" onChange={event => onTestWhileEditing?.(event.target.checked)} />
      Switch to Test mode while editing
    </label>
    <p id="run-mode-editing-note" className="fine-print">{testWhileEditingNote}</p>
    <label htmlFor="display-default-profile">Default profile</label>
    <SelectField id="display-default-profile" value={chosen.profileId ?? ''} disabled={!store || !state.desktop || chosen.busy || !chosen.loaded} aria-describedby="display-default-profile-note" onChange={event => { void store?.setDefaultProfile(event.target.value || null); }}>
      <option value="">No default</option>
      {profiles.map(profile => <option key={profile.id} value={profile.id}>{profile.name}</option>)}
      {missingDefault && <option value={chosen.profileId!}>Missing profile (skipped)</option>}
    </SelectField>
    <p id="display-default-profile-note" className="fine-print">{defaultProfileNote}</p>
    {chosen.error && <div className="service-failure" role="alert"><span>{chosen.error}</span></div>}
    <details className="learn-more"><summary>What has been tested on hardware</summary><ul className="fine-print notes-list">{moduleControlCaveat.map(line => <li key={line}>{line}</li>)}</ul></details>
    <p id="display-settings-applies" className="fine-print" role="status">{appliesNote(activeRuns)}</p>
    {!state.desktop && <p className="fine-print">This setting needs the desktop app.</p>}
    {state.error && <div className="service-failure" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss display settings error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
  </section>;
}
