import { ArrowDownToLine, Check, CheckCircle2, ChevronRight, CircleHelp, FileText, History, LayoutTemplate, LoaderCircle, Plus, RotateCcw, ShieldAlert, Trash2, TriangleAlert, X } from 'lucide-react';
import type { ProfileSummary } from './contracts';
import { LabelError } from './controls';
import { isUnsaved } from './profile-session';
import type { ProfileSession, ReplaySession, SessionState } from './profile-session';
import { actionLabel, controlLabel, dialSwitchLabel, gestureLabels, versionSummary, versionTimeLabel } from './ui-model';
import type { ConfirmRequest } from './useDeviceEditor';
import { ProfileTransfer, useImportDrop } from './ProfileTransfer';
import { FocusRulesPanel } from './FocusRulesPanel';
import { AutoSwitchRow } from './AutoSwitchRow';
import type { FocusRulesStore } from './focus-rules';
import { translateIn, useT } from './i18n-react';
import type { Translate } from './i18n-react';

// The profile manager: a sheet opened by the header's Manage profiles button (next to the
// profile switcher). It holds the saved profile list, import/export, the open profile's name,
// validation, earlier versions, the dry-run flag and the focused-app rules. The workspace stays
// on screen behind it.
export function ProfileManagerSheet({ session, state, desktop, disabled, confirm, mac = false, focusRules, onAllow, onStarters, onClose }: {
  session: ProfileSession; state: SessionState; desktop: boolean; disabled: boolean; confirm(request: ConfirmRequest): void; mac?: boolean;
  focusRules?: FocusRulesStore; onAllow(): void; onStarters?(): void; onClose(): void;
}) {
  const t = useT();
  const { profile, profileId } = state.draft;
  const unsaved = isUnsaved(state.draft);
  const drop = useImportDrop(session, disabled);
  const flagged = state.profiles.find(item => item.id === profileId && item.dry_run_only);
  const { contentHash } = state.draft;
  // The last import's report is kept for this session; the banner reopens it for a profile it created.
  const reportFor = flagged && state.mhzImport?.profiles.some(item => item.stored.id === flagged.id);
  const deleteProfile = (item: ProfileSummary) => confirm({
    title: t('profiles.deleteTitle', { name: item.name }),
    description: t(item.id === profileId ? 'profiles.deleteOpen' : 'profiles.deleteSaved'),
    confirmLabel: t('profiles.delete'),
    run: () => { void session.deleteProfile(item); },
  });
  return <section id="profile-manager-sheet" className={`profile-manager-sheet panel${drop.dragging ? ' drop-target' : ''}`} aria-label={t('profiles.title')} {...drop.props}>
    <div className="section-heading"><h2>{t('profiles.title')}</h2><div className="manager-heading-actions">
      <button className="icon-button" disabled={disabled} aria-label={t('profiles.refresh')} onClick={() => { void session.refreshProfiles(); }}><RotateCcw size={18} /></button>
      <button className="icon-button" aria-label={t('profiles.close')} onClick={onClose}><X size={17} /></button>
    </div></div>

    <h3>{t('profiles.current')}</h3>
    <div className="current-profile-heading"><label htmlFor="profile-name">{t('profiles.name')}</label><span className={`small-label ${unsaved ? 'draft-label' : ''}`}>{t(unsaved ? 'profiles.unsaved' : 'profiles.saved')}</span></div>
    <input id="profile-name" disabled={disabled} value={profile.name} aria-describedby="profile-name-error" onChange={event => session.edit({ ...profile, name: event.target.value }, 'profile-name')} /><LabelError id="profile-name-error" value={profile.name} field="Profile name" />
    <div className="profile-actions"><button className="button" disabled={disabled || state.validation === 'running'} onClick={() => { void session.validate(); }}>{state.validation === 'running' ? <LoaderCircle size={16} className="spinning" /> : <Check size={17} />}{t('profiles.validate')}</button><button className="button" disabled={disabled || !profileId} onClick={() => { if (profileId) session.guard('reload the saved profile', () => { void session.load(profileId); }); }}><ArrowDownToLine size={17} />{t('profiles.reload')}</button></div>
    {state.validation === 'valid' && <p className="validation-status"><CheckCircle2 size={17} />{t('profiles.valid')}</p>}
    <AutoSwitchRow store={focusRules} profileId={profileId} profileName={profile.name} disabled={disabled || !desktop || !state.ready} />
    <VersionHistory session={session} state={state} disabled={disabled} now={Date.now()} t={t} />
    {flagged && <div className="service-banner paused attention dry-run-notice" role="status"><ShieldAlert size={20} /><div>
      <strong>{t('profiles.dryRunTitle')}</strong>
      <span>{t('profiles.dryRunNote')}</span>
      <button type="button" className="button danger" disabled={disabled || unsaved || !contentHash} onClick={onAllow}>{t('profiles.allow')}</button>
      {reportFor && <button type="button" className="button" onClick={() => session.showMhzImport()}>{t('profiles.importReport')}</button>}
      {unsaved && <small>{t('profiles.saveFirst')}</small>}
    </div></div>}
    {state.importFlagsError && <div className="service-banner paused attention dry-run-notice" role="alert"><ShieldAlert size={20} /><div>
      <strong>{t('profiles.allDryTitle')}</strong>
      <span>{t('profiles.allDryNote', { error: state.importFlagsError })}</span>
      <button type="button" className="button danger" disabled={disabled} onClick={() => { void session.markAllForReview(); }}>{t('profiles.markAll')}</button>
    </div></div>}

    <h3>{t('profiles.saved.title')}</h3>
    <div className="profile-actions manager-actions"><button className="button" disabled={disabled} onClick={() => session.guard('create a new profile', () => { session.newProfile(); })}><Plus size={18} />{t('profiles.new')}</button>
      {onStarters && <button className="button" disabled={disabled} onClick={onStarters}><LayoutTemplate size={18} />{t('profiles.newStarter')}</button>}</div>
    {state.profiles.length ? state.profiles.map(item => <div key={item.id} className="saved-profile-row">
      <button className={`saved-profile ${item.id === profileId ? 'active' : ''}`} disabled={disabled} aria-pressed={item.id === profileId} onClick={() => session.guard('switch profiles', () => { void session.load(item.id); })}><FileText size={20} /><span><strong>{item.name}</strong><small>{t('profiles.bindings', { count: item.binding_count })}</small>{item.dry_run_only && <small className="dry-run-badge" title={t('profiles.importedTip')}>{t('profiles.imported')}</small>}</span><ChevronRight size={16} /></button>
      <button className="icon-button" disabled={disabled} aria-label={t('profiles.deleteNamed', { name: item.name })} title={t('profiles.delete')} onClick={() => deleteProfile(item)}><Trash2 size={17} /></button>
    </div>) : <p className="fine-print">{t(desktop ? 'profiles.none' : 'profiles.desktopOnly')}</p>}
    {state.unreadable.length > 0 && <div className="unreadable-profiles" role="status"><h4><TriangleAlert size={17} />{t('profiles.unreadable')} <span className="small-label">{state.unreadable.length}</span></h4>
      <p className="fine-print">{t('profiles.unreadableNote')} {t('profiles.unreadableWhere')}{state.storagePath ? <>: <code>{state.storagePath}</code></> : ''}.</p>
      <ul>{state.unreadable.map(item => <li key={item.id}><code>{item.id}.json</code><span>{item.error}</span></li>)}</ul></div>}

    <h3>{t('profiles.transfer')}</h3>
    <ProfileTransfer session={session} state={state} disabled={disabled} mac={mac} />

    <FocusRulesPanel store={focusRules} profiles={state.profiles} disabled={!desktop || !state.ready} />
  </section>;
}

// The built-in module samples that run through the open profile without hardware. Lives under the
// module canvas in the workspace.

// The kept earlier versions of the open profile, listed when the panel is
// opened. Hook-free and controlled by the session state, so tests can call
// it as a function; the details element holds its own open state, and every
// open fetches the list again, so versions kept while it was closed appear.
export function VersionHistory({ session, state, disabled, now, t = translateIn('en') }: { session: ProfileSession; state: SessionState; disabled: boolean; now: number; t?: Translate }) {
  const { profileId } = state.draft;
  const listed = profileId !== null && state.versionsId === profileId ? state.versions : null;
  // What the versions are compared with: the saved profile, not the unsaved draft on top of it.
  const saved = state.profiles.find(item => item.id === profileId);
  const toggle = (event: { currentTarget: { open: boolean } }) => {
    if (event.currentTarget.open && profileId && !disabled) void session.loadVersions(profileId);
  };
  return <details className="panel version-history" data-testid="version-history" onToggle={toggle}>
    <summary><History size={17} />{t('versions.title')}</summary>
    <p className="fine-print">{t('versions.note')}</p>
    {state.versionsError && <p className="fine-print" role="alert">{state.versionsError}</p>}
    {listed === null && !state.versionsError && <p className="fine-print" role="status">{t('versions.load')}</p>}
    {listed && listed.length === 0 && <p className="fine-print" role="status">{t('versions.none')}</p>}
    {listed && listed.length > 0 && <ul className="version-list">
      {listed.map(version => <li key={version.modified_at_ms}>
        <span>{t('versions.savedAt', { time: versionTimeLabel(version.modified_at_ms, now) })}<small>{versionSummary(version, { name: saved?.name ?? state.draft.profile.name, bindingCount: saved?.binding_count ?? state.draft.profile.bindings.length })}</small></span>
        <button className="button compact" disabled={disabled} onClick={() => { void session.restoreVersion(version.modified_at_ms); }}>{t('versions.restore')}</button>
      </li>)}
    </ul>}
  </details>;
}

export function SessionView({ session, t = translateIn('en') }: { session: ReplaySession; t?: Translate }) {
  if (session.status === 'idle') return <p className="empty-state">{t('session.idle')}</p>;
  if (session.status === 'running') return <div className="session-status" role="status"><LoaderCircle size={21} className="spinning" /><span>{t('session.running', { name: session.name })}</span></div>;
  if (session.status === 'failed') return <div className="session-status failed" role="alert"><CircleHelp size={21} /><div><strong>{t('session.failed')}</strong><p>{session.error}</p></div></div>;
  const { result } = session;
  return <div className="session-results"><div className="session-status completed" role="status"><CheckCircle2 size={21} /><span>{t('session.completed', { name: session.name })}</span></div><div className="session-counts"><div><strong>{result.report_count}</strong><span>{t('session.reports')}</span></div><div><strong>{result.event_count}</strong><span>{t('session.events')}</span></div><div><strong>{result.planned_action_count}</strong><span>{t('session.wouldRun')}</span></div><div><strong>{result.events.filter(event => event.action === null && !event.navigation && !event.dial_function_switch).length}</strong><span>{t('session.noAction')}</span></div></div>
    <p className="fine-print">{t('sample.input')}: <code>{result.source}</code>. {t('session.nothingRan')}</p><div className="table-scroll"><table className="events-table"><caption>{t('session.events')}</caption><thead><tr><th>{t('session.time')}</th><th>{t('session.control')}</th><th>{t('session.gesture')}</th><th>{t('session.value')}</th><th>{t('session.wouldRun')}</th></tr></thead><tbody>{result.events.map((event, index) => <tr key={index}><td>{event.timestamp_us}</td><td>{controlLabel(event.module, event.control)}<small>{event.serial_number ? t('session.serial', { serial: event.serial_number }) : t('session.noSerial')}</small></td><td>{gestureLabels[event.gesture]}{event.dial_function && <small>{t('session.function', { name: event.dial_function })}</small>}</td><td>{event.value ? `${event.value.raw}/${event.value.maximum}` : '—'}</td><td className={event.action ? '' : 'muted'}>{event.dial_function_switch ? <span className="dial-switch">{t('session.dialFunction', { label: dialSwitchLabel(event.dial_function_switch) })}</span> : event.navigation ? t('session.navigate', { page: event.navigation.to_page, depth: event.navigation.depth }) : actionLabel(event.action)}</td></tr>)}</tbody></table></div><div className="diagnostics"><h3>{t('session.diagnostics')} <span className="small-label">{result.diagnostics.length}</span></h3>{result.diagnostics.length ? <ul>{result.diagnostics.map((diagnostic, index) => <li key={index}><code>{diagnostic.timestamp_us} µs</code><span>{diagnostic.message}</span></li>)}</ul> : <p className="fine-print">{t('session.noDiagnostics')}</p>}</div>
  </div>;
}
