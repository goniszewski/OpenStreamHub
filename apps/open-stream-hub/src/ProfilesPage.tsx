import { useState } from 'react';
import { ArrowDownToLine, Check, CheckCircle2, ChevronRight, CircleHelp, FileText, LoaderCircle, Play, Plus, RotateCcw, ShieldAlert, Trash2, TriangleAlert } from 'lucide-react';
import type { ProfileSummary, ReplayKind } from './contracts';
import { LabelError, SelectField } from './controls';
import { profileBindingCount } from './keypad-model';
import { isUnsaved } from './profile-session';
import type { ProfileSession, ReplaySession, SessionState } from './profile-session';
import { actionLabel, controlLabel, dialSwitchLabel, gestureLabels, replayLabels } from './ui-model';
import type { Page } from './ui-model';
import type { ConfirmRequest, DeviceEditor } from './useDeviceEditor';
import { ProfileTransfer, useImportDrop } from './ProfileTransfer';
import { BindingsPanel } from './BindingsPanel';
import { DialFunctionsPanel } from './DialFunctionsPanel';
import { FocusRulesPanel } from './FocusRulesPanel';
import type { FocusRulesStore } from './focus-rules';
import { AllowExecutionDialog, MhzImportReport } from './MhzImport';

export function ProfilesPage({ session, state, editor, desktop, disabled, replayKind, setReplayKind, setPage, confirm, mac = false, focusRules }: {
  session: ProfileSession; state: SessionState; editor: DeviceEditor; desktop: boolean; disabled: boolean;
  replayKind: ReplayKind; setReplayKind(kind: ReplayKind): void; setPage(page: Page): void; confirm(request: ConfirmRequest): void; mac?: boolean; focusRules?: FocusRulesStore;
}) {
  const { profile, profileId } = state.draft;
  const unsaved = isUnsaved(state.draft);
  const drop = useImportDrop(session, disabled);
  const [allowing, setAllowing] = useState(false);
  const flagged = state.profiles.find(item => item.id === profileId && item.dry_run_only);
  const { contentHash } = state.draft;
  // The last import's report is kept for this session; the banner reopens it for a profile it created.
  const reportFor = flagged && state.mhzImport?.profiles.some(item => item.stored.id === flagged.id);
  const deleteProfile = (item: ProfileSummary) => confirm({
    title: `Delete ${item.name}?`,
    description: item.id === profileId ? 'This permanently removes the saved profile file. Its contents stay open as unsaved changes until you discard them.' : 'This permanently removes the saved profile file. This cannot be undone.',
    confirmLabel: 'Delete profile',
    run: () => { void session.deleteProfile(item); },
  });
  return <section className={`page-content profiles-page${drop.dragging ? ' drop-target' : ''}`} {...drop.props}><div className="page-title"><div><h1>Profiles</h1><p>Saved bindings and sample tests without hardware.</p></div><button className="button" disabled={disabled} onClick={() => session.guard('create a new profile', () => { session.newProfile(); setPage('profiles'); })}><Plus size={18} />New profile</button></div>
    <div className="profiles-grid"><section className="panel saved-profiles"><div className="section-heading"><h2>Saved profiles</h2><button className="icon-button" disabled={disabled} aria-label="Refresh profile list" onClick={() => { void session.refreshProfiles(); }}><RotateCcw size={18} /></button></div>
      {state.profiles.length ? state.profiles.map(item => <div key={item.id} className="saved-profile-row">
        <button className={`saved-profile ${item.id === profileId ? 'active' : ''}`} disabled={disabled} aria-pressed={item.id === profileId} onClick={() => session.guard('switch profiles', () => { void session.load(item.id); })}><FileText size={20} /><span><strong>{item.name}</strong><small>{item.binding_count} bindings</small>{item.dry_run_only && <small className="dry-run-badge" title="Imported from a MasterHUB .mhz file. It runs only as a dry run until you allow actions.">Imported · dry run only</small>}</span><ChevronRight size={16} /></button>
        <button className="icon-button" disabled={disabled} aria-label={`Delete profile ${item.name}`} title="Delete profile" onClick={() => deleteProfile(item)}><Trash2 size={17} /></button>
      </div>) : <p className="fine-print">{desktop ? 'No saved profiles are available yet.' : 'Saved profiles are available in the desktop app.'}</p>}
      {state.unreadable.length > 0 && <div className="unreadable-profiles" role="status"><h3><TriangleAlert size={17} />Unreadable profiles <span className="small-label">{state.unreadable.length}</span></h3>
        <p className="fine-print">These files could not be opened. They are kept unchanged for recovery. Inspect them in the storage folder{state.storagePath ? <>: <code>{state.storagePath}</code></> : ''}.</p>
        <ul>{state.unreadable.map(item => <li key={item.id}><code>{item.id}.json</code><span>{item.error}</span></li>)}</ul></div>}
    </section><section className="panel profile-editor"><div className="section-heading"><h2>Profile</h2><span className={`small-label ${unsaved ? 'draft-label' : ''}`}>{unsaved ? 'Unsaved changes' : 'Saved'}</span></div>
      <label htmlFor="profile-name">Name</label><input id="profile-name" disabled={disabled} value={profile.name} aria-describedby="profile-name-error" onChange={event => session.edit({ ...profile, name: event.target.value }, 'profile-name')} /><LabelError id="profile-name-error" value={profile.name} field="Profile name" />
      <div className="profile-actions"><button className="button" disabled={disabled || state.validation === 'running'} onClick={() => { void session.validate(); }}>{state.validation === 'running' ? <LoaderCircle size={16} className="spinning" /> : <Check size={17} />}Validate changes</button><button className="button" disabled={disabled || !profileId} onClick={() => { if (profileId) session.guard('reload the saved profile', () => { void session.load(profileId); }); }}><ArrowDownToLine size={17} />Reload saved</button></div>
      {flagged && <div className="service-banner paused attention dry-run-notice" role="status"><ShieldAlert size={20} /><div>
        <strong>Imported profile: dry run only</strong>
        <span>This profile was imported from a MasterHUB .mhz file. It can run only as a dry run until you review its bindings and allow it to execute actions.</span>
        <button type="button" className="button danger" disabled={disabled || unsaved || !contentHash} onClick={() => setAllowing(true)}>Allow this imported profile to execute actions…</button>
        {reportFor && <button type="button" className="button" onClick={() => session.showMhzImport()}>Show import report</button>}
        {unsaved && <small>Save or discard your changes first, so you review exactly what will run.</small>}
      </div></div>}
      {state.importFlagsError && <div className="service-banner paused attention dry-run-notice" role="alert"><ShieldAlert size={20} /><div>
        <strong>Every profile runs only as a dry run</strong>
        <span>The list of imported profiles cannot be read ({state.importFlagsError}). Until it is restored, no profile can run with actions. Mark all profiles as needing review to continue: each one then runs only as a dry run until you review it and allow actions.</span>
        <button type="button" className="button danger" disabled={disabled} onClick={() => { void session.markAllForReview(); }}>Mark all profiles as needing review</button>
      </div></div>}
      <ProfileTransfer session={session} state={state} disabled={disabled} mac={mac} />
      {state.validation === 'valid' && <p className="validation-status"><CheckCircle2 size={17} />The current changes are valid.</p>}
      <h3>Bindings <span className="small-label">{profileBindingCount(profile)}</span></h3>
      <BindingsPanel profile={profile} session={session} editor={editor} disabled={disabled} />
      <DialFunctionsPanel profile={profile} session={session} state={state} disabled={disabled} />
      <button className="button" onClick={() => setPage('devices')}><Plus size={17} />Edit device bindings</button>
    </section></div>
    <MhzImportReport result={state.mhzReportOpen ? state.mhzImport : null} onClose={() => session.dismissMhzImport()} />
    <AllowExecutionDialog target={allowing && flagged && !unsaved ? { name: flagged.name, profile } : null} onCancel={() => setAllowing(false)}
      onConfirm={() => { setAllowing(false); if (flagged && contentHash) void session.allowExecution(flagged.id, flagged.name, contentHash); }} />
    <FocusRulesPanel store={focusRules} profiles={state.profiles} disabled={!desktop || !state.ready} />
    <details className="panel replay-panel"><summary className="replay-summary"><span>Test with sample input</span><span className="small-label">No hardware required</span></summary>
      <p className="fine-print">Built-in samples run through the current profile as if a module sent them, so you can see what each control would do.</p>
      <div className="replay-controls"><label htmlFor="replay-capture">Sample input</label><SelectField id="replay-capture" value={replayKind} onChange={event => { session.clearReplay(); setReplayKind(event.target.value as ReplayKind); }}>{Object.entries(replayLabels).map(([kind, label]) => <option key={kind} value={kind}>{label}</option>)}</SelectField><button className="button primary" disabled={disabled || state.replay.status === 'running'} onClick={() => { void session.replayCapture(replayKind); }}><Play size={17} />{state.replay.status === 'running' ? 'Running…' : 'Run test'}</button></div>
      <SessionView session={state.replay} />
    </details>
  </section>;
}

export function SessionView({ session }: { session: ReplaySession }) {
  if (session.status === 'idle') return <p className="empty-state">Results appear here. Editing the profile clears previous results.</p>;
  if (session.status === 'running') return <div className="session-status" role="status"><LoaderCircle size={21} className="spinning" /><span>Running {session.name}…</span></div>;
  if (session.status === 'failed') return <div className="session-status failed" role="alert"><CircleHelp size={21} /><div><strong>Test failed</strong><p>{session.error}</p></div></div>;
  const { result } = session;
  return <div className="session-results"><div className="session-status completed" role="status"><CheckCircle2 size={21} /><span>Completed · {session.name}</span></div><div className="session-counts"><div><strong>{result.report_count}</strong><span>Input reports</span></div><div><strong>{result.event_count}</strong><span>Events</span></div><div><strong>{result.planned_action_count}</strong><span>Would run</span></div><div><strong>{result.events.filter(event => event.action === null && !event.navigation && !event.dial_function_switch).length}</strong><span>No action bound</span></div></div>
    <p className="fine-print">Sample input: <code>{result.source}</code>. Nothing was executed on your Mac.</p><div className="table-scroll"><table className="events-table"><caption>Events</caption><thead><tr><th>Time (µs)</th><th>Control</th><th>Gesture</th><th>Value</th><th>Would run</th></tr></thead><tbody>{result.events.map((event, index) => <tr key={index}><td>{event.timestamp_us}</td><td>{controlLabel(event.module, event.control)}<small>{event.serial_number ? `Serial: ${event.serial_number}` : 'No serial'}</small></td><td>{gestureLabels[event.gesture]}{event.dial_function && <small>Function: {event.dial_function}</small>}</td><td>{event.value ? `${event.value.raw}/${event.value.maximum}` : '—'}</td><td className={event.action ? '' : 'muted'}>{event.dial_function_switch ? <span className="dial-switch">Dial function: {dialSwitchLabel(event.dial_function_switch)}</span> : event.navigation ? `Navigate to ${event.navigation.to_page} · L${event.navigation.depth}` : actionLabel(event.action)}</td></tr>)}</tbody></table></div><div className="diagnostics"><h3>Diagnostics <span className="small-label">{result.diagnostics.length}</span></h3>{result.diagnostics.length ? <ul>{result.diagnostics.map((diagnostic, index) => <li key={index}><code>{diagnostic.timestamp_us} µs</code><span>{diagnostic.message}</span></li>)}</ul> : <p className="fine-print">No parser diagnostics.</p>}</div>
  </div>;
}
