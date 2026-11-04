import { useEffect, useState, useSyncExternalStore } from 'react';
import { AppWindow, X } from 'lucide-react';
import type { RunningApp } from './contracts';
import { SelectField } from './controls';
import { appChoices, appName, idleFocusRulesState, maxFocusRules, rulesForProfile } from './focus-rules';
import type { FocusRulesStore } from './focus-rules';
import { useT } from './i18n-react';

const noopSubscribe = () => () => {};
const idle = () => idleFocusRulesState;
const other = '__other';

// "Switch automatically when [app] is in front", for the open profile: its rules as removable apps, and a picker of the
// running apps (or any installed app, chosen in the Finder dialog on Applications) to add one.
// It edits the same rules as the focused-app rules panel below it
// and shares its on/off switch, so the two always agree. A profile that has not been saved yet has nothing to switch to.
export function AutoSwitchRow({ store, profileId, profileName, disabled = false }: { store?: FocusRulesStore; profileId: string | null; profileName: string; disabled?: boolean }) {
  const t = useT();
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const [apps, setApps] = useState<RunningApp[]>([]);
  const [choice, setChoice] = useState('');
  const [picked, setPicked] = useState<RunningApp | null>(null);
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const refresh = () => { if (store) void store.runningApps().then(setApps); };
  useEffect(refresh, [store]);

  const off = disabled || !store || !state.desktop || state.busy || profileId === null;
  const mine = rulesForProfile(state.rules, profileId);
  const known = picked && !apps.some(app => app.bundle_id.toLowerCase() === picked.bundle_id.toLowerCase()) ? [...apps, picked] : apps;
  const choices = appChoices(known, state.rules);
  const duplicate = choice !== '' && state.rules.some(rule => rule.app_bundle_id.toLowerCase() === choice.toLowerCase());
  const full = state.rules.length >= maxFocusRules;
  const canAdd = !off && choice !== '' && !duplicate && !full;
  const add = async () => {
    if (!store || !canAdd || profileId === null) return;
    if (await store.save({ id: null, app_bundle_id: choice, profile_id: profileId })) { setChoice(''); setPicked(null); refresh(); }
  };
  const browse = async () => {
    const app = await store?.chooseApp();
    if (app) { setPicked(app); setChoice(app.bundle_id); }
  };

  return <div className="auto-switch-row" role="group" aria-label={t('auto.group', { name: profileName })}>
    <div className="auto-switch-heading"><AppWindow size={16} aria-hidden="true" /><strong>{t('auto.heading')}</strong></div>
    {profileId === null
      ? <p className="fine-print">{t('auto.saveFirst')}</p>
      : <>
        {mine.length > 0 && <ul className="auto-switch-apps" aria-label={t('auto.apps', { name: profileName })}>
          {mine.map(rule => <li key={rule.id}><span title={rule.app_bundle_id}>{appName(rule.app_bundle_id, known)}</span>
            <button type="button" className="icon-button" disabled={off} aria-label={t('auto.stop', { name: profileName, app: appName(rule.app_bundle_id, known) })} data-tip={t('auto.remove')} onClick={() => { void store?.remove(rule.id); }}><X size={14} /></button></li>)}
        </ul>}
        <div className="auto-switch-add">
          <label className="sr-only" htmlFor="auto-switch-app">{t('auto.appLabel')}</label>
          <SelectField id="auto-switch-app" value={choice} disabled={off} onFocus={refresh} onChange={event => { if (event.target.value === other) void browse(); else setChoice(event.target.value); }}>
            <option value="">{t(mine.length ? 'auto.addAnother' : 'auto.choose')}</option>
            {choices.map(item => <option key={item.bundle_id} value={item.bundle_id}>{item.name}</option>)}
            <option value={other}>{t('auto.other')}</option>
          </SelectField>
          <button type="button" className="button compact" disabled={!canAdd} onClick={() => { void add(); }}>{t('auto.add')}</button>
        </div>
        {duplicate && <p className="fine-print" role="status">{t('auto.duplicate')}</p>}
        {full && <p className="fine-print" role="status">{t('focus.full', { max: maxFocusRules })}</p>}
        {state.desktop && !state.enabled && <p className="fine-print" role="status">{t('auto.off')}
          {' '}<button type="button" className="link-button" disabled={off} onClick={() => { void store?.setEnabled(true); }}>{t('auto.turnOn')}</button></p>}
        {state.error && <p className="field-error" role="alert">{state.error}</p>}
      </>}
  </div>;
}
