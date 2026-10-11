import { useEffect, useState, useSyncExternalStore } from 'react';
import { AppWindow, X } from 'lucide-react';
import type { RunningApp } from './contracts';
import { SelectField } from './controls';
import { appChoices, appName, bundleIdIssue, idleFocusRulesState, maxFocusRules, rulesForProfile } from './focus-rules';
import type { FocusRulesStore } from './focus-rules';

const noopSubscribe = () => () => {};
const idle = () => idleFocusRulesState;
const other = '__other';

// "Switch automatically when [app] is in front", for the open profile: its rules as removable apps, and a picker of the
// running apps (or a typed bundle ID) to add one. It edits the same rules as the focused-app rules panel below it
// and shares its on/off switch, so the two always agree. A profile that has not been saved yet has nothing to switch to.
export function AutoSwitchRow({ store, profileId, profileName, disabled = false }: { store?: FocusRulesStore; profileId: string | null; profileName: string; disabled?: boolean }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const [apps, setApps] = useState<RunningApp[]>([]);
  const [choice, setChoice] = useState('');
  const [typed, setTyped] = useState('');
  const [touched, setTouched] = useState(false);
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const refresh = () => { if (store) void store.runningApps().then(setApps); };
  useEffect(refresh, [store]);

  const off = disabled || !store || !state.desktop || state.busy || profileId === null;
  const mine = rulesForProfile(state.rules, profileId);
  const choices = appChoices(apps, state.rules);
  const typing = choice === other;
  const bundleId = typing ? typed.trim() : choice;
  const issue = typing ? bundleIdIssue(typed) : null;
  const duplicate = bundleId !== '' && state.rules.some(rule => rule.app_bundle_id.toLowerCase() === bundleId.toLowerCase());
  const full = state.rules.length >= maxFocusRules;
  const canAdd = !off && bundleId !== '' && !issue && !duplicate && !full;
  const add = async () => {
    if (!store || !canAdd || profileId === null) return;
    setTouched(true);
    if (await store.save({ id: null, app_bundle_id: bundleId, profile_id: profileId })) { setChoice(''); setTyped(''); setTouched(false); refresh(); }
  };
  const useCurrent = async () => {
    const current = await store?.currentApp();
    if (current) { setChoice(other); setTyped(current); setTouched(true); }
  };

  return <div className="auto-switch-row" role="group" aria-label={`Switch automatically to ${profileName}`}>
    <div className="auto-switch-heading"><AppWindow size={16} aria-hidden="true" /><strong>Switch automatically when an app is in front</strong></div>
    {profileId === null
      ? <p className="fine-print">Save this profile first, then choose the apps that should switch to it.</p>
      : <>
        {mine.length > 0 && <ul className="auto-switch-apps" aria-label={`Apps that switch to ${profileName}`}>
          {mine.map(rule => <li key={rule.id}><span title={rule.app_bundle_id}>{appName(rule.app_bundle_id, apps)}</span>
            <button type="button" className="icon-button" disabled={off} aria-label={`Stop switching to ${profileName} for ${appName(rule.app_bundle_id, apps)}`} data-tip="Remove" onClick={() => { void store?.remove(rule.id); }}><X size={14} /></button></li>)}
        </ul>}
        <div className="auto-switch-add">
          <label className="sr-only" htmlFor="auto-switch-app">App to switch on</label>
          <SelectField id="auto-switch-app" value={choice} disabled={off} onFocus={refresh} onChange={event => { setChoice(event.target.value); setTouched(false); }}>
            <option value="">{mine.length ? 'Add another app…' : 'Choose an app…'}</option>
            {choices.map(item => <option key={item.bundle_id} value={item.bundle_id}>{item.name}</option>)}
            <option value={other}>Another app (type its bundle ID)…</option>
          </SelectField>
          <button type="button" className="button compact" disabled={!canAdd} onClick={() => { void add(); }}>Add</button>
        </div>
        {typing && <div className="auto-switch-typed">
          <label className="sr-only" htmlFor="auto-switch-bundle">App bundle ID</label>
          <input id="auto-switch-bundle" value={typed} disabled={off} placeholder="com.apple.Safari" autoComplete="off" spellCheck={false} maxLength={300} aria-invalid={touched && issue !== null}
            onChange={event => setTyped(event.target.value)} onBlur={() => { if (typed) setTouched(true); }} />
          <button type="button" className="button compact" disabled={off} onClick={() => { void useCurrent(); }}>Use current app</button>
        </div>}
        {typing && touched && issue && <p className="field-error" role="alert">{issue}</p>}
        {duplicate && <p className="fine-print" role="status">That app already has a rule; edit or remove it in the focused-app rules below.</p>}
        {full && <p className="fine-print" role="status">At most {maxFocusRules} rules. Delete one to add another.</p>}
        {state.desktop && !state.enabled && <p className="fine-print" role="status">Automatic switching is off, so these rules do nothing yet.
          {' '}<button type="button" className="link-button" disabled={off} onClick={() => { void store?.setEnabled(true); }}>Turn it on</button></p>}
        {state.error && <p className="field-error" role="alert">{state.error}</p>}
      </>}
  </div>;
}
