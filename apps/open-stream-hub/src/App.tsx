import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { DeviceFeed, readAutoDetect } from './device-feed';
import { ServiceRunner } from './service-runner';
import { NoticeStore } from './notices';
import { FocusRulesStore } from './focus-rules';
import { DisplaySettingsStore } from './display-settings';
import { DisplaySettingsPanel } from './DisplaySettingsPanel';
import { ObsSettingsStore } from './obs-settings';
import { ObsSettingsPanel } from './ObsSettingsPanel';
import { focusWhenRendered, NoticeRegion, noticeFallbackFocusId } from './NoticeRegion';
import { serviceResumeId } from './ServicePanel';
import { OnboardingDialog } from './OnboardingDialog';
import { markOnboardingDone, shouldAutoOpenOnboarding } from './onboarding-model';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { Activity, CheckCircle2, CircleHelp, FileText, Grid2X2, Link, LoaderCircle, Monitor, Moon, Palette, Redo2, RotateCcw, Save, Settings, Sun, Undo2, Workflow, X } from 'lucide-react';
import type { ReplayKind } from './contracts';
import { ModalDialog, SelectField } from './controls';
import { DeviceWorkspace, ModuleCatalog } from './DevicesPage';
import { ArtworkPage } from './ArtworkPage';
import { Inspector } from './Inspector';
import { DiagnosticsPanel } from './DiagnosticsPanel';
import { CompatibilityPage, SettingsPage } from './InfoPages';
import { DraftPersister, DraftStore } from './draft-store';
import { isUnsaved, ProfileSession } from './profile-session';
import { ProfilesPage } from './ProfilesPage';
import { installShortcuts, isMacPlatform, redoHint, shortcutPages, withHint } from './shortcuts';
import { bindingSearchId } from './BindingsPanel';
import { appearanceStorageKey, browserProfile, errorMessage, readAppearance } from './ui-model';
import type { Appearance, Page } from './ui-model';
import { useDeviceEditor } from './useDeviceEditor';
import type { ConfirmRequest } from './useDeviceEditor';

const desktop = isTauri();
const invokeCommand = <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);
function webStorage(): Storage | null {
  try { return localStorage; } catch { return null; }
}
const mac = isMacPlatform(typeof navigator === 'undefined' ? undefined : navigator);

// Crash-recovery drafts work without storage; the store then does nothing.
function draftPersister() {
  let storage: Storage | null = null;
  try { storage = localStorage; } catch { /* Drafts are simply not kept. */ }
  return new DraftPersister(new DraftStore(storage));
}

function getAppearance(): Appearance {
  try { return readAppearance(localStorage); } catch { return 'system'; }
}

export default function App() {
  const [session] = useState(() => new ProfileSession((command, args) => invoke(command, args), browserProfile, desktop, desktop ? draftPersister() : null));
  const [feed] = useState(() => {
    const created = new DeviceFeed((command, args) => invoke(command, args), (event, handler) => listen(event, handler), desktop);
    try { created.setAutoDetect(readAutoDetect(localStorage)); } catch { /* auto-detect stays on by default */ }
    return created;
  });
  const [runner] = useState(() => new ServiceRunner((command, args) => invoke(command, args), (event, handler) => listen(event, handler), desktop));
  const [notices] = useState(() => new NoticeStore());
  const [focusRules] = useState(() => new FocusRulesStore((command, args) => invoke(command, args), (event, handler) => listen(event, handler), desktop, runner));
  const [displaySettings] = useState(() => new DisplaySettingsStore((command, args) => invoke(command, args), desktop));
  const [obsSettings] = useState(() => new ObsSettingsStore((command, args) => invoke(command, args), desktop));
  useEffect(() => { feed.setBlocker(runner.isActivePath); runner.setLiveSink(feed); feed.connect(); runner.connect(); const stopNotices = notices.connect(feed, runner); const stopFocus = focusRules.connect(); return () => { stopFocus(); stopNotices(); runner.setLiveSink(null); feed.dispose(); runner.dispose(); }; }, [feed, runner, notices, focusRules]);
  const noticesEnabled = useSyncExternalStore(notices.subscribe, () => notices.getState().enabled);
  // The first-run guide opens itself once in the desktop app. Elsewhere it opens only on request.
  const [guide, setGuide] = useState<{ open: boolean; step: number }>(() => ({ open: shouldAutoOpenOnboarding(webStorage(), desktop), step: 0 }));
  const closeGuide = () => { markOnboardingDone(webStorage()); setGuide({ open: false, step: 0 }); };
  const state = useSyncExternalStore(session.subscribe, session.getState);
  const [page, setPage] = useState<Page>('devices');
  const [appearance, setAppearance] = useState<Appearance>(getAppearance);
  const [systemDark, setSystemDark] = useState(() => matchMedia('(prefers-color-scheme: dark)').matches);
  const [replayKind, setReplayKind] = useState<ReplayKind>('knobs');
  const [confirmation, setConfirmation] = useState<ConfirmRequest | null>(null);
  const editor = useDeviceEditor(session, state, setPage, setConfirmation);
  const { draft, busy, error, notice, pendingIntent, bootstrapError } = state;
  const { profile, profileId } = draft;
  const unsaved = isUnsaved(draft);
  const disabled = !desktop || !state.ready || busy !== null;

  useEffect(() => {
    const query = matchMedia('(prefers-color-scheme: dark)');
    const listener = () => setSystemDark(query.matches);
    query.addEventListener('change', listener);
    return () => query.removeEventListener('change', listener);
  }, []);

  useEffect(() => {
    document.documentElement.dataset.theme = appearance === 'system' ? systemDark ? 'dark' : 'light' : appearance;
    try { localStorage.setItem(appearanceStorageKey, appearance); } catch { /* Appearance still works without storage. */ }
  }, [appearance, systemDark]);

  useEffect(() => { if (desktop) void session.bootstrap(); }, [session]);

  // Write any pending crash-recovery copy before the page goes away.
  useEffect(() => {
    const flush = () => session.flushDraft();
    const hidden = () => { if (document.visibilityState === 'hidden') flush(); };
    document.addEventListener('visibilitychange', hidden);
    window.addEventListener('pagehide', flush);
    window.addEventListener('beforeunload', flush);
    return () => { document.removeEventListener('visibilitychange', hidden); window.removeEventListener('pagehide', flush); window.removeEventListener('beforeunload', flush); flush(); };
  }, [session]);

  const latest = useRef({ desktop, ready: state.ready, page });
  latest.current = { desktop, ready: state.ready, page };
  useEffect(() => installShortcuts(document, () => ({
    mac,
    isPageEnabled: page => shortcutPages.includes(page),
    setPage,
    save: () => { if (!latest.current.desktop) session.setNotice('Saving needs the desktop app.'); else if (latest.current.ready) void session.saveOrNote(); },
    duplicate: () => { if (!latest.current.desktop) session.setNotice('Duplicating needs the desktop app.'); else session.requestDuplicate(); },
    undo: () => { session.undo(); },
    redo: () => { session.redo(); },
    // Search belongs to the Profiles page; elsewhere the browser keeps Cmd/Ctrl+F.
    find: () => {
      const input = latest.current.page === 'profiles' ? document.getElementById(bindingSearchId) as HTMLInputElement | null : null;
      input?.focus(); input?.select();
      return input !== null;
    },
  })), [session]);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWindow().onCloseRequested(event => {
      const allowed = session.requestClose(() => {
        getCurrentWindow().destroy().catch(reason => session.setError(`Could not close the window: ${errorMessage(reason)}`));
      });
      if (!allowed) event.preventDefault();
    }).then(remove => { if (disposed) remove(); else unlisten = remove; }).catch(reason => {
      if (!disposed) session.setError(`Could not enable the unsaved-draft close guard: ${errorMessage(reason)}`);
    });
    return () => { disposed = true; unlisten?.(); };
  }, [session]);

  const themePicker = <div className="theme-picker" role="group" aria-label="Appearance">
    {(['system', 'light', 'dark'] as Appearance[]).map(value => <button key={value} aria-pressed={appearance === value}
      className={appearance === value ? 'active' : ''} onClick={() => setAppearance(value)}>
      {value === 'system' ? <Monitor size={15} /> : value === 'light' ? <Sun size={15} /> : <Moon size={15} />}
      {value[0].toUpperCase() + value.slice(1)}
    </button>)}
  </div>;

  return <div className="app-shell">
    <header className="app-header">
      <div className="brand"><Grid2X2 size={23} /><span>OpenStreamHub</span><span className="version-label">Desktop</span></div>
      <div className="profile-switcher"><FileText size={18} /><label className="sr-only" htmlFor="active-profile">Active profile</label>
        {desktop ? <SelectField id="active-profile" value={profileId ?? '__new'} disabled={disabled} onChange={event => {
          const id = event.target.value; if (id !== '__new') session.guard('switch profiles', () => { void session.load(id); });
        }}>
          {!profileId && <option value="__new">{profile.name}</option>}
          {profileId && !state.profiles.some(item => item.id === profileId) && <option value={profileId}>{profile.name}</option>}
          {state.profiles.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </SelectField> : <span>{profile.name}</span>}
        {unsaved && <span className="draft-dot" title="Unsaved changes" aria-label="Unsaved changes" />}
      </div>
      <div className="header-actions"><span className="mode-pill"><Activity size={16} />Device-free</span>
        <div className="history-buttons" role="group" aria-label="Draft history">
          <button className="icon-button" disabled={disabled || !state.canUndo} aria-label="Undo" title={withHint('Undo', 'Z', mac)} onClick={() => { session.undo(); }}><Undo2 size={18} /></button>
          <button className="icon-button" disabled={disabled || !state.canRedo} aria-label="Redo" title={`Redo (${redoHint(mac)})`} onClick={() => { session.redo(); }}><Redo2 size={18} /></button>
        </div>
        <button className="button primary" disabled={disabled || !unsaved} title={withHint('Save profile', 'S', mac)} onClick={() => { void session.save(); }}><Save size={17} />{busy === 'Saving profile' ? 'Saving…' : 'Save profile'}</button>
      </div>
    </header>
    <div className="app-body">
      <aside className="sidebar">
        <nav aria-label="Primary navigation">
          {([{ id: 'devices', label: 'Devices', icon: Grid2X2 }, { id: 'profiles', label: 'Profiles', icon: FileText }] as const).map(item =>
            <button key={item.id} className={page === item.id ? 'active' : ''} aria-current={page === item.id ? 'page' : undefined} onClick={() => setPage(item.id)}><item.icon size={22} />{item.label}</button>)}
          <button disabled title="Flow editor is planned"><Workflow size={22} /><span>Flows<small>Planned</small></span></button>
          <button className={page === 'artwork' ? 'active' : ''} aria-current={page === 'artwork' ? 'page' : undefined} title="Prepare images for module displays offline" onClick={() => setPage('artwork')}><Palette size={22} />Display artwork</button>
          <button className={page === 'compatibility' ? 'active' : ''} aria-current={page === 'compatibility' ? 'page' : undefined} onClick={() => setPage('compatibility')}><Link size={22} />Compatibility</button>
        </nav>
        <div className="sidebar-bottom"><button className="help-link" onClick={() => setGuide({ open: true, step: 0 })}><CircleHelp size={22} />Getting started</button><button className={`settings-link ${page === 'settings' ? 'active' : ''}`} aria-current={page === 'settings' ? 'page' : undefined} onClick={() => setPage('settings')}><Settings size={22} />Settings</button>{themePicker}</div>
      </aside>
      <main className="main-content" id={noticeFallbackFocusId} tabIndex={-1}>
        {!desktop && <div className="browser-banner" role="status"><Monitor size={19} /><div><strong>Desktop app required</strong><span>This browser preview is read-only. Profile storage, validation and replay run in the native app.</span></div></div>}
        {busy && <div className="operation-status" role="status"><LoaderCircle size={16} className="spinning" />{busy}…</div>}
        {bootstrapError && <div className="alert error" role="alert"><CircleHelp size={19} /><span><strong>Could not open profile storage.</strong> {bootstrapError.message}
          <small className="alert-hint">Check that the profile storage folder exists, is a real folder rather than a link, and is readable by your account{bootstrapError.storagePath ? <>: <code>{bootstrapError.storagePath}</code></> : '.'} Files in it are never changed by a failed start.</small></span>
          <button className="button compact" disabled={busy !== null} onClick={() => { void session.bootstrap(); }}><RotateCcw size={16} />Retry</button></div>}
        {error && <div className="alert error" role="alert"><CircleHelp size={19} /><span>{error}</span><button aria-label="Dismiss error" onClick={() => session.setError(null)}><X size={17} /></button></div>}
        {notice && <div className="alert notice" role="status"><CheckCircle2 size={18} /><span>{notice}</span><button aria-label="Dismiss notification" onClick={() => session.setNotice(null)}><X size={17} /></button></div>}

        {page === 'devices' && <div className="device-layout">
          <ModuleCatalog editor={editor} />
          <DeviceWorkspace editor={editor} state={state} desktop={desktop} disabled={disabled} setPage={setPage} feed={feed} runner={runner} displaySettings={displaySettings} />
          <Inspector editor={editor} session={session} state={state} disabled={disabled} invoke={desktop ? invokeCommand : undefined} />
        </div>}
        {page === 'profiles' && <ProfilesPage session={session} state={state} editor={editor} desktop={desktop} disabled={disabled} replayKind={replayKind} setReplayKind={setReplayKind} setPage={setPage} confirm={setConfirmation} mac={mac} focusRules={focusRules} />}
        {page === 'artwork' && <ArtworkPage desktop={desktop} invoke={invokeCommand} />}
        {page === 'compatibility' && <CompatibilityPage />}
        {page === 'settings' && <SettingsPage mac={mac} desktop={desktop} noticesEnabled={noticesEnabled} onNoticesEnabled={on => notices.setEnabled(on)} onShowGuide={() => setGuide({ open: true, step: 0 })} storagePath={state.storagePath} themePicker={themePicker} display={<DisplaySettingsPanel store={displaySettings} runner={runner} />} obs={<ObsSettingsPanel store={obsSettings} runner={runner} />} diagnostics={<DiagnosticsPanel desktop={desktop} invoke={(command, args) => invoke(command, args)} profile={profile} runner={runner} feed={feed} />} />}
      </main>
    </div>
    <NoticeRegion store={notices} onReview={() => { setPage('devices'); focusWhenRendered(() => document.getElementById(serviceResumeId)); }} />
    <OnboardingDialog open={guide.open} step={guide.step} mac={mac} onStep={step => setGuide({ open: true, step })} onSkip={closeGuide} onDone={closeGuide} />
    <ModalDialog open={confirmation !== null} labelledBy="delete-title" describedBy="delete-message" onCancel={() => setConfirmation(null)}>
      <h2 id="delete-title">{confirmation?.title}</h2><p id="delete-message">{confirmation?.description}</p>
      <div className="dialog-actions"><button className="button" autoFocus onClick={() => setConfirmation(null)}>Cancel</button><button className="button danger" onClick={() => { const request = confirmation; setConfirmation(null); request?.run(); }}>{confirmation?.confirmLabel}</button></div>
    </ModalDialog>
    <ModalDialog open={state.duplicatePrompt} labelledBy="duplicate-title" describedBy="duplicate-message" onCancel={() => { void session.resolveDuplicate(null); }}>
      <h2 id="duplicate-title">Duplicate which version?</h2><p id="duplicate-message">{profile.name} has unsaved changes. The copy replaces the open draft, so the other version is not kept open. Your saved file is never changed.</p>
      <div className="dialog-actions wrap"><button className="button" onClick={() => { void session.resolveDuplicate(null); }}>Cancel</button><button className="button" onClick={() => { void session.resolveDuplicate('saved'); }}>Saved version</button><button className="button primary" autoFocus onClick={() => { void session.resolveDuplicate('draft'); }}>Current draft</button></div>
    </ModalDialog>
    <ModalDialog open={state.restorePrompt !== null} labelledBy="restore-title" describedBy="restore-message" onCancel={() => { /* Escape never discards recovered changes. */ }}>
      <h2 id="restore-title">Restore unsaved changes to {state.restorePrompt?.name}?</h2>
      <p id="restore-message">OpenStreamHub closed before these changes were saved. Restore them to continue editing, or discard them and keep the saved profile.</p>
      {state.restorePrompt?.changedOnDisk && <p className="fine-print" role="note">The saved profile changed on disk after these changes were made. You can still restore them; saving will then ask you to reload first, so nothing on disk is overwritten.</p>}
      <div className="dialog-actions"><button className="button danger" onClick={() => session.resolveRestore(false)}>Discard</button><button className="button primary" autoFocus onClick={() => session.resolveRestore(true)}>Restore</button></div>
    </ModalDialog>
    <ModalDialog open={pendingIntent !== null} labelledBy="unsaved-title" describedBy="unsaved-message" onCancel={() => session.resolveIntent(false)}>
      <h2 id="unsaved-title">Keep your changes?</h2><p id="unsaved-message">You have an unsaved profile draft. Discard it to {pendingIntent?.label}, or cancel and save your changes first.</p>
      <div className="dialog-actions"><button className="button" autoFocus onClick={() => session.resolveIntent(false)}>Cancel</button><button className="button danger" onClick={() => session.resolveIntent(true)}>Discard changes</button></div>
    </ModalDialog>
  </div>;
}
