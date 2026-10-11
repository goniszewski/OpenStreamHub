import { useEffect, useState, useSyncExternalStore } from 'react';
import { AppWindow, Pencil, Trash2, TriangleAlert, X } from 'lucide-react';
import type { FocusRule, ProfileSummary } from './contracts';
import { SelectField } from './controls';
import { bundleIdIssue, idleFocusRulesState, maxFocusRules } from './focus-rules';
import type { FocusRulesStore } from './focus-rules';
import { useT } from './i18n-react';

const noopSubscribe = () => () => {};
const idle = () => idleFocusRulesState;
const bundleExample = 'com.apple.Safari';
export const focusBundleInputId = 'focus-rule-bundle-id';

// The profile the rule form submits. A new rule defaults to the first profile; an edited rule whose profile is missing
// gets no profile ('') until the user picks one, so it is never silently retargeted.
export function ruleFormProfile(profiles: ProfileSummary[], profileId: string, editing: boolean): string {
  if (profiles.some(item => item.id === profileId)) return profileId;
  return editing ? '' : profiles[0]?.id ?? '';
}

// Opt-in rules that switch live sessions to a saved profile when an app comes to the front.
export function FocusRulesPanel({ store, profiles, disabled = false }: { store?: FocusRulesStore; profiles: ProfileSummary[]; disabled?: boolean }) {
  const t = useT();
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const [editing, setEditing] = useState<string | null>(null);
  const [bundleId, setBundleId] = useState('');
  const [profileId, setProfileId] = useState('');
  const [touched, setTouched] = useState(false);
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);

  const off = disabled || !store || !state.desktop || state.busy;
  const chosenProfile = ruleFormProfile(profiles, profileId, editing !== null);
  const issue = bundleIdIssue(bundleId);
  const showIssue = touched && issue !== null;
  const full = editing === null && state.rules.length >= maxFocusRules;
  const profileName = (id: string) => profiles.find(item => item.id === id)?.name;
  const reset = () => { setEditing(null); setBundleId(''); setTouched(false); };
  const edit = (rule: FocusRule) => { setEditing(rule.id); setBundleId(rule.app_bundle_id); setProfileId(rule.profile_id); setTouched(false); document.getElementById(focusBundleInputId)?.focus(); };
  const submit = async () => {
    setTouched(true);
    if (!store || issue || !chosenProfile || full) return;
    if (await store.save({ id: editing, app_bundle_id: bundleId, profile_id: chosenProfile })) reset();
  };
  const useCurrent = async () => {
    const current = await store?.currentApp();
    if (current) { setBundleId(current); setTouched(true); }
  };

  return <section className="panel focus-rules-panel" aria-labelledby="focus-rules-title">
    <div className="section-heading"><div><h2 id="focus-rules-title">{t('focus.title')}</h2>
      <p className="fine-print" id="focus-rules-description">{t('focus.description')}</p></div>
      <span className="small-label">{state.rules.length}/{maxFocusRules}</span></div>
    {!state.desktop && <p className="fine-print">{t('focus.needsDesktop')}</p>}
    <label className="service-option"><input type="checkbox" checked={state.enabled} disabled={off} aria-describedby="focus-rules-description" onChange={event => { void store?.setEnabled(event.target.checked); }} />{t('focus.enable')}</label>
    {state.enabled && state.focusedApp && <p className="fine-print" role="status">{t('focus.lastApp')} <code>{state.focusedApp}</code></p>}
    {state.error && <div className="service-failure focus-rules-error" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label={t('focus.dismiss')} onClick={() => store?.dismissError()}><X size={15} /></button></div>}
    {state.rules.length > 0
      ? <div className="table-scroll"><table className="events-table focus-rules-table"><caption className="sr-only">{t('focus.caption')}</caption>
        <thead><tr><th scope="col">{t('focus.bundleId')}</th><th scope="col">{t('focus.profile')}</th><th scope="col"><span className="sr-only">{t('focus.actions')}</span></th></tr></thead>
        <tbody>{state.rules.map(rule => {
          const name = profileName(rule.profile_id);
          return <tr key={rule.id} aria-current={editing === rule.id ? 'true' : undefined}>
            <td><code>{rule.app_bundle_id}</code></td>
            <td>{name ?? <span className="focus-rule-missing"><TriangleAlert size={14} aria-hidden="true" />{t('focus.missing')}</span>}</td>
            <td className="focus-rule-actions">
              <button type="button" className="icon-button" disabled={off} aria-label={t('focus.editNamed', { app: rule.app_bundle_id })} title={t('focus.edit')} onClick={() => edit(rule)}><Pencil size={16} /></button>
              <button type="button" className="icon-button" disabled={off} aria-label={t('focus.deleteNamed', { app: rule.app_bundle_id })} title={t('focus.delete')} onClick={() => { if (editing === rule.id) reset(); void store?.remove(rule.id); }}><Trash2 size={16} /></button>
            </td></tr>;
        })}</tbody></table></div>
      : <p className="empty-state">{t('focus.empty')}</p>}
    <form className="focus-rule-form" aria-labelledby="focus-rule-form-title" noValidate onSubmit={event => { event.preventDefault(); void submit(); }}>
      <h3 id="focus-rule-form-title">{t(editing ? 'focus.edit' : 'focus.add')}</h3>
      <label htmlFor={focusBundleInputId}>{t('focus.bundleId')}</label>
      <div className="focus-bundle-row">
        <input id={focusBundleInputId} value={bundleId} disabled={off} placeholder={bundleExample} autoComplete="off" spellCheck={false} maxLength={300}
          aria-invalid={showIssue} aria-describedby={`${focusBundleInputId}-hint${showIssue ? ` ${focusBundleInputId}-error` : ''}`}
          onChange={event => setBundleId(event.target.value)} onBlur={() => { if (bundleId) setTouched(true); }} />
        <button type="button" className="button" disabled={off} onClick={() => { void useCurrent(); }}><AppWindow size={16} />{t('focus.useCurrent')}</button>
      </div>
      <p id={`${focusBundleInputId}-hint`} className="fine-print">{t('focus.useCurrentHint')}</p>
      {showIssue && <p id={`${focusBundleInputId}-error`} className="field-error" role="alert">{issue}</p>}
      <label htmlFor="focus-rule-profile">{t('focus.profile')}</label>
      <SelectField id="focus-rule-profile" value={chosenProfile} disabled={off || profiles.length === 0} onChange={event => setProfileId(event.target.value)}>
        {profiles.length === 0 ? <option value="">{t('focus.saveProfileFirst')}</option> : <>
          {chosenProfile === '' && <option value="" disabled>{t('focus.chooseProfile')}</option>}
          {profiles.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </>}
      </SelectField>
      {editing && chosenProfile === '' && profiles.length > 0 && <p className="fine-print" role="status">{t('focus.profileGone')}</p>}
      {full && <p className="fine-print" role="status">{t('focus.full', { max: maxFocusRules })}</p>}
      <div className="profile-actions">
        <button type="submit" className="button primary" disabled={off || !chosenProfile || full}>{t(editing ? 'focus.save' : 'focus.add')}</button>
        {editing && <button type="button" className="button" onClick={reset}>{t('focus.cancel')}</button>}
      </div>
    </form>
  </section>;
}
