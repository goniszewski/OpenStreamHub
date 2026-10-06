import { useEffect, useState, useSyncExternalStore } from 'react';
import { AppWindow, Pencil, Trash2, TriangleAlert, X } from 'lucide-react';
import type { FocusRule, ProfileSummary } from './contracts';
import { SelectField } from './controls';
import { bundleIdIssue, idleFocusRulesState, maxFocusRules } from './focus-rules';
import type { FocusRulesStore } from './focus-rules';

const noopSubscribe = () => () => {};
const idle = () => idleFocusRulesState;
export const focusBundleInputId = 'focus-rule-bundle-id';

// The profile the rule form submits. A new rule defaults to the first profile; an edited rule whose profile is missing
// gets no profile ('') until the user picks one, so it is never silently retargeted.
export function ruleFormProfile(profiles: ProfileSummary[], profileId: string, editing: boolean): string {
  if (profiles.some(item => item.id === profileId)) return profileId;
  return editing ? '' : profiles[0]?.id ?? '';
}

// Opt-in rules that switch live sessions to a saved profile when an app comes to the front.
export function FocusRulesPanel({ store, profiles, disabled = false }: { store?: FocusRulesStore; profiles: ProfileSummary[]; disabled?: boolean }) {
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
    <div className="section-heading"><div><h2 id="focus-rules-title">Focused-app rules</h2>
      <p className="fine-print" id="focus-rules-description">Switch a running profile when an app comes to the front. Dry runs switch at once. Runs that execute actions pause and wait for you to confirm before actions run with the new profile. The first matching rule wins.</p></div>
      <span className="small-label">{state.rules.length}/{maxFocusRules}</span></div>
    {!state.desktop && <p className="fine-print">Focused-app rules need the desktop app.</p>}
    <label className="service-option"><input type="checkbox" checked={state.enabled} disabled={off} aria-describedby="focus-rules-description" onChange={event => { void store?.setEnabled(event.target.checked); }} />Switch profiles by focused app (off by default)</label>
    {state.enabled && state.focusedApp && <p className="fine-print" role="status">Last focused app: <code>{state.focusedApp}</code></p>}
    {state.error && <div className="service-failure focus-rules-error" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss focused-app rules error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
    {state.rules.length > 0
      ? <div className="table-scroll"><table className="events-table focus-rules-table"><caption className="sr-only">Focused-app rules, first match wins</caption>
        <thead><tr><th scope="col">App bundle ID</th><th scope="col">Profile</th><th scope="col"><span className="sr-only">Actions</span></th></tr></thead>
        <tbody>{state.rules.map(rule => {
          const name = profileName(rule.profile_id);
          return <tr key={rule.id} aria-current={editing === rule.id ? 'true' : undefined}>
            <td><code>{rule.app_bundle_id}</code></td>
            <td>{name ?? <span className="focus-rule-missing"><TriangleAlert size={14} aria-hidden="true" />Missing profile (rule is skipped)</span>}</td>
            <td className="focus-rule-actions">
              <button type="button" className="icon-button" disabled={off} aria-label={`Edit rule for ${rule.app_bundle_id}`} title="Edit rule" onClick={() => edit(rule)}><Pencil size={16} /></button>
              <button type="button" className="icon-button" disabled={off} aria-label={`Delete rule for ${rule.app_bundle_id}`} title="Delete rule" onClick={() => { if (editing === rule.id) reset(); void store?.remove(rule.id); }}><Trash2 size={16} /></button>
            </td></tr>;
        })}</tbody></table></div>
      : <p className="empty-state">No rules yet. Add one below.</p>}
    <form className="focus-rule-form" aria-labelledby="focus-rule-form-title" noValidate onSubmit={event => { event.preventDefault(); void submit(); }}>
      <h3 id="focus-rule-form-title">{editing ? 'Edit rule' : 'Add rule'}</h3>
      <label htmlFor={focusBundleInputId}>App bundle ID</label>
      <div className="focus-bundle-row">
        <input id={focusBundleInputId} value={bundleId} disabled={off} placeholder="com.apple.Safari" autoComplete="off" spellCheck={false} maxLength={300}
          aria-invalid={showIssue} aria-describedby={`${focusBundleInputId}-hint${showIssue ? ` ${focusBundleInputId}-error` : ''}`}
          onChange={event => setBundleId(event.target.value)} onBlur={() => { if (bundleId) setTouched(true); }} />
        <button type="button" className="button" disabled={off} onClick={() => { void useCurrent(); }}><AppWindow size={16} />Use current app</button>
      </div>
      <p id={`${focusBundleInputId}-hint`} className="fine-print">"Use current app" fills in the last app you used before OpenStreamHub.</p>
      {showIssue && <p id={`${focusBundleInputId}-error`} className="field-error" role="alert">{issue}</p>}
      <label htmlFor="focus-rule-profile">Profile</label>
      <SelectField id="focus-rule-profile" value={chosenProfile} disabled={off || profiles.length === 0} onChange={event => setProfileId(event.target.value)}>
        {profiles.length === 0 ? <option value="">Save a profile first</option> : <>
          {chosenProfile === '' && <option value="" disabled>Choose a profile</option>}
          {profiles.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </>}
      </SelectField>
      {editing && chosenProfile === '' && profiles.length > 0 && <p className="fine-print" role="status">This rule's profile no longer exists. Choose a profile to save the rule.</p>}
      {full && <p className="fine-print" role="status">At most {maxFocusRules} rules. Delete one to add another.</p>}
      <div className="profile-actions">
        <button type="submit" className="button primary" disabled={off || !chosenProfile || full}>{editing ? 'Save rule' : 'Add rule'}</button>
        {editing && <button type="button" className="button" onClick={reset}>Cancel</button>}
      </div>
    </form>
  </section>;
}
