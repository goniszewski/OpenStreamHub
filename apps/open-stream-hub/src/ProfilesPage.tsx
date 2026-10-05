import { ArrowDownToLine, Check, CheckCircle2, ChevronRight, CircleHelp, FileText, LoaderCircle, Play, Plus, RotateCcw, Trash2, TriangleAlert } from 'lucide-react';
import type { ProfileSummary, ReplayKind } from './contracts';
import { LabelError, SelectField } from './controls';
import { profileBindingCount } from './keypad-model';
import { isUnsaved } from './profile-session';
import type { ProfileSession, ReplaySession, SessionState } from './profile-session';
import { actionLabel, controlLabel, gestureLabels, replayLabels } from './ui-model';
import type { Page } from './ui-model';
import type { ConfirmRequest, DeviceEditor } from './useDeviceEditor';

export function ProfilesPage({ session, state, editor, desktop, disabled, replayKind, setReplayKind, setPage, confirm }: {
  session: ProfileSession; state: SessionState; editor: DeviceEditor; desktop: boolean; disabled: boolean;
  replayKind: ReplayKind; setReplayKind(kind: ReplayKind): void; setPage(page: Page): void; confirm(request: ConfirmRequest): void;
}) {
  const { profile, profileId } = state.draft;
  const unsaved = isUnsaved(state.draft);
  const deleteProfile = (item: ProfileSummary) => confirm({
    title: `Delete ${item.name}?`,
    description: item.id === profileId ? 'This permanently removes the saved profile file. Its contents stay open as an unsaved draft until you discard them.' : 'This permanently removes the saved profile file. This cannot be undone.',
    confirmLabel: 'Delete profile',
    run: () => { void session.deleteProfile(item); },
  });
  return <section className="page-content profiles-page"><div className="page-title"><div><h1>Profiles</h1><p>Saved bindings and device-free replay.</p></div><button className="button" disabled={disabled} onClick={() => session.guard('create a new profile', () => { session.newProfile(); setPage('profiles'); })}><Plus size={18} />New profile</button></div>
    <div className="profiles-grid"><section className="panel saved-profiles"><div className="section-heading"><h2>Saved profiles</h2><button className="icon-button" disabled={disabled} aria-label="Refresh profile list" onClick={() => { void session.refreshProfiles(); }}><RotateCcw size={18} /></button></div>
      {state.profiles.length ? state.profiles.map(item => <div key={item.id} className="saved-profile-row">
        <button className={`saved-profile ${item.id === profileId ? 'active' : ''}`} disabled={disabled} aria-pressed={item.id === profileId} onClick={() => session.guard('switch profiles', () => { void session.load(item.id); })}><FileText size={20} /><span><strong>{item.name}</strong><small>{item.binding_count} bindings</small></span><ChevronRight size={16} /></button>
        <button className="icon-button" disabled={disabled} aria-label={`Delete profile ${item.name}`} title="Delete profile" onClick={() => deleteProfile(item)}><Trash2 size={17} /></button>
      </div>) : <p className="fine-print">{desktop ? 'No saved profiles are available yet.' : 'Saved profiles are available in the desktop app.'}</p>}
      {state.unreadable.length > 0 && <div className="unreadable-profiles" role="status"><h3><TriangleAlert size={17} />Unreadable profiles <span className="small-label">{state.unreadable.length}</span></h3>
        <p className="fine-print">These files could not be opened. They are kept unchanged for recovery. Inspect them in the storage folder{state.storagePath ? <>: <code>{state.storagePath}</code></> : ''}.</p>
        <ul>{state.unreadable.map(item => <li key={item.id}><code>{item.id}.json</code><span>{item.error}</span></li>)}</ul></div>}
    </section><section className="panel profile-editor"><div className="section-heading"><h2>Profile draft</h2><span className={`small-label ${unsaved ? 'draft-label' : ''}`}>{unsaved ? 'Unsaved changes' : 'Saved'}</span></div>
      <label htmlFor="profile-name">Name</label><input id="profile-name" disabled={disabled} value={profile.name} aria-describedby="profile-name-error" onChange={event => session.edit({ ...profile, name: event.target.value })} /><LabelError id="profile-name-error" value={profile.name} field="Profile name" />
      <div className="profile-actions"><button className="button" disabled={disabled || state.validation === 'running'} onClick={() => { void session.validate(); }}>{state.validation === 'running' ? <LoaderCircle size={16} className="spinning" /> : <Check size={17} />}Validate draft</button><button className="button" disabled={disabled || !profileId} onClick={() => { if (profileId) session.guard('reload the saved profile', () => { void session.load(profileId); }); }}><ArrowDownToLine size={17} />Reload saved</button></div>
      {state.validation === 'valid' && <p className="validation-status"><CheckCircle2 size={17} />Current draft is valid.</p>}
      <h3>Bindings <span className="small-label">{profileBindingCount(profile)}</span></h3>
      {profile.bindings.length ? <div className="binding-list">{profile.bindings.map((item, index) => <div key={`${item.module}:${item.control}:${item.gesture}:${item.serial_number ?? ''}:${index}`} className="binding-row"><div><strong>{controlLabel(item.module, item.control)} · {gestureLabels[item.gesture]}</strong><span>{actionLabel(item.action)}</span><small>{item.serial_number ? `Serial: ${item.serial_number}` : 'Any device serial'}</small></div><button className="button compact" onClick={() => editor.selectBinding(item)}>Edit</button><button className="icon-button" aria-label={`Remove ${controlLabel(item.module, item.control)} ${gestureLabels[item.gesture]} binding`} disabled={disabled} onClick={() => session.update(latest => ({ ...latest, bindings: latest.bindings.filter((_, position) => position !== index) }))}><Trash2 size={17} /></button></div>)}</div> : profileBindingCount(profile) === 0 && <p className="empty-state">No bindings yet. Select a module in Devices and choose an action to add one.</p>}
      <div className="binding-list">{profile.keypad!.pages.flatMap(keyPage => keyPage.slots.flatMap((slot, control) => slot?.type === 'action' ? slot.bindings.map((item, index) => <div className="binding-row" key={`${keyPage.id}:${control}:${index}`}><div><strong>{keyPage.name} · Key {control + 1} · {gestureLabels[item.gesture]}</strong><span>{actionLabel(item.action)}</span><small>{item.serial_number ? `Serial: ${item.serial_number}` : 'Any device serial'}</small></div><button className="button compact" onClick={() => editor.selectKeypadBinding(keyPage.id, control, item)}>Edit</button><button className="icon-button" aria-label={`Remove ${keyPage.name} Key ${control + 1} ${gestureLabels[item.gesture]} binding`} disabled={disabled} onClick={() => editor.removeKeypadBinding(keyPage.id, control, item)}><Trash2 size={17} /></button></div>) : []))}</div>
      <button className="button" onClick={() => setPage('devices')}><Plus size={17} />Edit device bindings</button>
    </section></div>
    <section className="panel replay-panel"><div className="section-heading"><div><h2>Replay a capture</h2><p className="fine-print">Bundled synthetic input reports pass through the Rust decoder and binding engine using the current draft.</p></div><span className="small-label">No hardware required</span></div><div className="replay-controls"><label htmlFor="replay-capture">Capture</label><SelectField id="replay-capture" value={replayKind} onChange={event => { session.clearReplay(); setReplayKind(event.target.value as ReplayKind); }}>{Object.entries(replayLabels).map(([kind, label]) => <option key={kind} value={kind}>{label}</option>)}</SelectField><button className="button primary" disabled={disabled || state.replay.status === 'running'} onClick={() => { void session.replayCapture(replayKind); }}><Play size={17} />{state.replay.status === 'running' ? 'Running…' : 'Replay capture'}</button></div>
      <SessionView session={state.replay} />
    </section>
  </section>;
}

function SessionView({ session }: { session: ReplaySession }) {
  if (session.status === 'idle') return <p className="empty-state">Replay results appear here. Editing the profile clears previous results.</p>;
  if (session.status === 'running') return <div className="session-status" role="status"><LoaderCircle size={21} className="spinning" /><span>Running {session.name}…</span></div>;
  if (session.status === 'failed') return <div className="session-status failed" role="alert"><CircleHelp size={21} /><div><strong>Replay failed</strong><p>{session.error}</p></div></div>;
  const { result } = session;
  return <div className="session-results"><div className="session-status completed" role="status"><CheckCircle2 size={21} /><span>Completed · {session.name}</span></div><div className="session-counts"><div><strong>{result.report_count}</strong><span>Reports</span></div><div><strong>{result.event_count}</strong><span>Normalized events</span></div><div><strong>{result.planned_action_count}</strong><span>Planned actions</span></div><div><strong>{result.events.filter(event => event.action === null && !event.navigation).length}</strong><span>Unbound events</span></div></div>
    <p className="fine-print">Synthetic source: <code>{result.source}</code>. Planned actions were not executed.</p><div className="table-scroll"><table className="events-table"><caption>Normalized events</caption><thead><tr><th>Time (µs)</th><th>Control</th><th>Gesture</th><th>Value</th><th>Planned action</th></tr></thead><tbody>{result.events.map((event, index) => <tr key={index}><td>{event.timestamp_us}</td><td>{controlLabel(event.module, event.control)}<small>{event.serial_number ? `Serial: ${event.serial_number}` : 'No serial'}</small></td><td>{gestureLabels[event.gesture]}</td><td>{event.value ? `${event.value.raw}/${event.value.maximum}` : '—'}</td><td className={event.action ? '' : 'muted'}>{event.navigation ? `Navigate to ${event.navigation.to_page} · L${event.navigation.depth}` : actionLabel(event.action)}</td></tr>)}</tbody></table></div><div className="diagnostics"><h3>Diagnostics <span className="small-label">{result.diagnostics.length}</span></h3>{result.diagnostics.length ? <ul>{result.diagnostics.map((diagnostic, index) => <li key={index}><code>{diagnostic.timestamp_us} µs</code><span>{diagnostic.message}</span></li>)}</ul> : <p className="fine-print">No parser diagnostics.</p>}</div>
  </div>;
}
