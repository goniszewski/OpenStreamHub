import { useCallback, useEffect, useRef, useState } from 'react';
import type { CSSProperties, SelectHTMLAttributes } from 'react';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  Activity, ArrowDownToLine, Check, CheckCircle2, ChevronDown, ChevronRight, CircleHelp, FileText,
  Grid2X2, Keyboard, Link, LoaderCircle, Monitor, Moon, Palette, Play, Plus, RotateCcw,
  RotateCw, Save, Settings, Sun, Trash2, Volume2, VolumeX, Workflow, X,
} from 'lucide-react';
import type { Action, AppBootstrap, Binding, BindingAction, Gesture, Modifier, Profile, ProfileSummary, ReplayEvent, ReplayKind, ReplayResult, StoredProfile } from './contracts';
import { actionLabel, appearanceStorageKey, browserProfile, controlCounts, controlLabel, editBinding, errorMessage, faderMaximum, gestureLabels, gestures, moduleNote, modules, readAppearance, replayLabels, sameSelector } from './ui-model';
import type { Appearance, BindingSelection, ModuleId, Page } from './ui-model';
import { moduleArtwork } from './module-controls';

const desktop = isTauri();
type Session = { status: 'idle' } | { status: 'running'; name: string } | { status: 'failed'; name: string; error: string } |
  { status: 'completed'; name: string; result: ReplayResult };
type Preview = { status: 'idle' } | { status: 'running' } | { status: 'failed'; error: string } | { status: 'completed'; event: ReplayEvent };
type PendingIntent = { label: string; run: () => void };

function getAppearance(): Appearance {
  try { return readAppearance(localStorage); } catch { return 'system'; }
}

export default function App() {
  const [page, setPage] = useState<Page>('devices');
  const [appearance, setAppearance] = useState<Appearance>(getAppearance);
  const [systemDark, setSystemDark] = useState(() => matchMedia('(prefers-color-scheme: dark)').matches);
  const [profile, setProfile] = useState<Profile>(browserProfile);
  const [profileId, setProfileId] = useState<string | null>(null);
  const [savedSnapshot, setSavedSnapshot] = useState(JSON.stringify(browserProfile));
  const [profiles, setProfiles] = useState<ProfileSummary[]>([]);
  const [storagePath, setStoragePath] = useState('');
  const [ready, setReady] = useState(!desktop);
  const [busy, setBusy] = useState<string | null>(desktop ? 'Opening profiles' : null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [validation, setValidation] = useState<'idle' | 'running' | 'valid' | 'failed'>('idle');
  const [selectedModule, setSelectedModule] = useState<ModuleId>('m003k');
  const [selection, setSelection] = useState<BindingSelection>({ module: 'm003k', control: 0, gesture: 'clockwise', serial: '' });
  const [serialScope, setSerialScope] = useState<'any' | 'exact'>('any');
  const [replayKind, setReplayKind] = useState<ReplayKind>('knobs');
  const [faderPosition, setFaderPosition] = useState(50);
  const [session, setSession] = useState<Session>({ status: 'idle' });
  const [preview, setPreview] = useState<Preview>({ status: 'idle' });
  const [pendingIntent, setPendingIntent] = useState<PendingIntent | null>(null);
  const dialog = useRef<HTMLDialogElement>(null);
  const revision = useRef(0);
  const requestSequence = useRef(0);
  const previewSequence = useRef(0);
  const validationSequence = useRef(0);
  const loadSequence = useRef(0);
  const listSequence = useRef(0);
  const unsaved = JSON.stringify(profile) !== savedSnapshot;
  const unsavedRef = useRef(unsaved);
  const busyRef = useRef(busy);
  unsavedRef.current = unsaved;
  busyRef.current = busy;
  const disabled = !desktop || !ready || busy !== null;
  const selected = modules.find(module => module.id === selectedModule)!;
  const binding = profile.bindings.find(item => sameSelector(item, selection));

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

  const invalidateResults = useCallback(() => {
    revision.current += 1;
    requestSequence.current += 1;
    previewSequence.current += 1;
    validationSequence.current += 1;
    listSequence.current += 1;
    setSession({ status: 'idle' });
    setPreview({ status: 'idle' });
    setValidation('idle');
    setNotice(null);
    setError(null);
  }, []);

  const adopt = useCallback((stored: StoredProfile) => {
    invalidateResults();
    setProfile(stored.profile);
    setProfileId(stored.id);
    setSavedSnapshot(JSON.stringify(stored.profile));
  }, [invalidateResults]);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    invoke<AppBootstrap>('bootstrap').then(value => {
      if (disposed) return;
      adopt(value.profile);
      setProfiles(value.profiles);
      setStoragePath(value.storage_path);
      setReady(true);
      setBusy(null);
    }).catch(reason => {
      if (disposed) return;
      setError(`Could not open profile storage: ${errorMessage(reason)}`);
      setBusy(null);
    });
    return () => { disposed = true; };
  }, [adopt]);

  useEffect(() => {
    if (!pendingIntent) return;
    dialog.current?.showModal();
  }, [pendingIntent]);

  useEffect(() => {
    if (!desktop) return;
    let disposed = false;
    let unlisten: (() => void) | undefined;
    getCurrentWindow().onCloseRequested(event => {
      if (busyRef.current && unsavedRef.current) {
        event.preventDefault();
        setError('Wait for the profile operation to finish before closing the window.');
      } else if (unsavedRef.current) {
        event.preventDefault();
        setPendingIntent({ label: 'close the window', run: () => {
          getCurrentWindow().destroy().catch(reason => setError(`Could not close the window: ${errorMessage(reason)}`));
        } });
      }
    }).then(remove => { if (disposed) remove(); else unlisten = remove; }).catch(reason => {
      if (!disposed) setError(`Could not enable the unsaved-draft close guard: ${errorMessage(reason)}`);
    });
    return () => { disposed = true; unlisten?.(); };
  }, []);

  function edit(next: Profile) { invalidateResults(); setProfile(next); }
  function mutateAction(action: BindingAction | null) { edit(editBinding(profile, selection, action)); }
  function guard(label: string, run: () => void) {
    if (unsaved) setPendingIntent({ label, run }); else run();
  }

  async function load(id: string) {
    const sequence = ++loadSequence.current;
    listSequence.current += 1;
    setBusy('Loading profile');
    setError(null);
    try {
      const stored = await invoke<StoredProfile>('load_profile', { id });
      if (sequence !== loadSequence.current) return;
      adopt(stored);
      setNotice(`Loaded ${stored.profile.name}.`);
    } catch (reason) {
      if (sequence === loadSequence.current) setError(`Could not load profile: ${errorMessage(reason)}`);
    } finally { if (sequence === loadSequence.current) setBusy(null); }
  }

  async function refreshProfiles() {
    const sequence = ++listSequence.current;
    try {
      const listed = await invoke<ProfileSummary[]>('list_profiles');
      if (sequence !== listSequence.current) return;
      setProfiles(listed);
      setNotice('Profile list refreshed. Your draft is unchanged.');
    } catch (reason) { if (sequence === listSequence.current) setError(`Could not list profiles: ${errorMessage(reason)}`); }
  }

  async function save() {
    if (disabled) return;
    const snapshot = structuredClone(profile);
    listSequence.current += 1;
    setBusy('Saving profile');
    setError(null);
    setNotice(null);
    try {
      const stored = await invoke<StoredProfile>('save_profile', { id: profileId, profile: snapshot });
      adopt(stored);
      setValidation('valid');
      setNotice(`Saved ${stored.profile.name}.`);
      const sequence = ++listSequence.current;
      try {
        const listed = await invoke<ProfileSummary[]>('list_profiles');
        if (sequence === listSequence.current) setProfiles(listed);
      } catch (reason) {
        if (sequence === listSequence.current) setError(`Profile saved, but the profile list could not refresh: ${errorMessage(reason)}`);
      }
    } catch (reason) { setError(`Could not save profile: ${errorMessage(reason)}`); setValidation('failed'); }
    finally { setBusy(null); }
  }

  async function validate() {
    if (disabled) return;
    const sequence = ++validationSequence.current;
    const snapshot = structuredClone(profile);
    setValidation('running'); setError(null); setNotice(null);
    try {
      await invoke<{ valid: true }>('validate_profile', { profile: snapshot });
      if (sequence !== validationSequence.current) return;
      setValidation('valid'); setNotice('The current draft passes service validation.');
    } catch (reason) {
      if (sequence !== validationSequence.current) return;
      setValidation('failed'); setError(`Profile validation failed: ${errorMessage(reason)}`);
    }
  }

  async function testBinding(gesture = selection.gesture) {
    if (disabled) return;
    const sequence = ++previewSequence.current;
    const snapshot = structuredClone(profile);
    const capturedSelection = { ...selection, gesture };
    setPreview({ status: 'running' });
    try {
      const event = await invoke<ReplayEvent>('preview_binding', {
        profile: snapshot, module: capturedSelection.module, control: capturedSelection.control,
        gesture: capturedSelection.gesture, serialNumber: capturedSelection.serial || null,
        value: capturedSelection.module === 'm005s' ? { type: 'position', raw: faderPosition, maximum: faderMaximum } : null,
      });
      if (sequence === previewSequence.current) setPreview({ status: 'completed', event });
    } catch (reason) {
      if (sequence === previewSequence.current) setPreview({ status: 'failed', error: errorMessage(reason) });
    }
  }

  async function replay() {
    if (disabled) return;
    const sequence = ++requestSequence.current;
    const snapshot = structuredClone(profile);
    const name = replayLabels[replayKind];
    setSession({ status: 'running', name });
    try {
      const result = await invoke<ReplayResult>('replay_demo', { profile: snapshot, kind: replayKind });
      if (sequence === requestSequence.current) setSession({ status: 'completed', name, result });
    } catch (reason) {
      if (sequence === requestSequence.current) setSession({ status: 'failed', name, error: errorMessage(reason) });
    }
  }

  function chooseModule(id: ModuleId) {
    setSelectedModule(id);
    previewSequence.current += 1;
    setPreview({ status: 'idle' });
    setSelection({ module: id, control: 0, gesture: gestures[id][0], serial: '' });
    setSerialScope('any');
  }

  function selectBinding(item: Binding) {
    setSelectedModule(item.module);
    setSelection({ module: item.module, control: item.control, gesture: item.gesture, serial: item.serial_number ?? '' });
    setSerialScope(item.serial_number ? 'exact' : 'any');
    setPage('devices');
    previewSequence.current += 1;
    setPreview({ status: 'idle' });
  }

  function changeSelection(next: BindingSelection) {
    setSelection(next);
    previewSequence.current += 1;
    setPreview({ status: 'idle' });
  }

  function changeFaderPosition(value: number) {
    setFaderPosition(value);
    previewSequence.current += 1;
    setPreview({ status: 'idle' });
  }

  function newProfile() {
    guard('create a new profile', () => {
      invalidateResults();
      setProfile({ schema_version: 1, name: 'New profile', bindings: [] });
      setProfileId(null); setSavedSnapshot(''); setPage('profiles');
    });
  }

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
          const id = event.target.value; if (id !== '__new') guard('switch profiles', () => { void load(id); });
        }}>
          {!profileId && <option value="__new">{profile.name}</option>}
          {profileId && !profiles.some(item => item.id === profileId) && <option value={profileId}>{profile.name}</option>}
          {profiles.map(item => <option key={item.id} value={item.id}>{item.name}</option>)}
        </SelectField> : <span>{profile.name}</span>}
        {unsaved && <span className="draft-dot" title="Unsaved changes" aria-label="Unsaved changes" />}
      </div>
      <div className="header-actions"><span className="mode-pill"><Activity size={16} />Device-free</span>
        <button className="button primary" disabled={disabled || !unsaved} onClick={() => { void save(); }}><Save size={17} />{busy === 'Saving profile' ? 'Saving…' : 'Save profile'}</button>
      </div>
    </header>
    <div className="app-body">
      <aside className="sidebar">
        <nav aria-label="Primary navigation">
          {([{ id: 'devices', label: 'Devices', icon: Grid2X2 }, { id: 'profiles', label: 'Profiles', icon: FileText }] as const).map(item =>
            <button key={item.id} className={page === item.id ? 'active' : ''} aria-current={page === item.id ? 'page' : undefined} onClick={() => setPage(item.id)}><item.icon size={22} />{item.label}</button>)}
          <button disabled title="Flow editor is planned"><Workflow size={22} /><span>Flows<small>Planned</small></span></button>
          <button disabled title="Display uploads require further implementation and hardware validation"><Palette size={22} /><span>Display artwork<small>Planned</small></span></button>
          <button className={page === 'compatibility' ? 'active' : ''} aria-current={page === 'compatibility' ? 'page' : undefined} onClick={() => setPage('compatibility')}><Link size={22} />Compatibility</button>
        </nav>
        <div className="sidebar-bottom"><button className={`settings-link ${page === 'settings' ? 'active' : ''}`} aria-current={page === 'settings' ? 'page' : undefined} onClick={() => setPage('settings')}><Settings size={22} />Settings</button>{themePicker}</div>
      </aside>
      <main className="main-content">
        {!desktop && <div className="browser-banner" role="status"><Monitor size={19} /><div><strong>Desktop app required</strong><span>This browser preview is read-only. Profile storage, validation and replay run in the native app.</span></div></div>}
        {busy && <div className="operation-status" role="status"><LoaderCircle size={16} className="spinning" />{busy}…</div>}
        {error && <div className="alert error" role="alert"><CircleHelp size={19} /><span>{error}</span><button aria-label="Dismiss error" onClick={() => setError(null)}><X size={17} /></button></div>}
        {notice && <div className="alert notice" role="status"><CheckCircle2 size={18} /><span>{notice}</span><button aria-label="Dismiss notification" onClick={() => setNotice(null)}><X size={17} /></button></div>}

        {page === 'devices' && <div className="device-layout">
          <aside className="catalog" aria-label="Module catalog"><h2>Modules</h2>
            {modules.map(module => <button key={module.id} className={`module-card ${selectedModule === module.id ? 'selected' : ''}`} aria-pressed={selectedModule === module.id} onClick={() => chooseModule(module.id)}>
              <span className="module-photo"><img className={module.art === 'encoder' || module.art === 'knob' ? 'art-square' : 'art-wide'} src={`/modules/${module.art}.png`} alt="" /></span>
              <span className="module-copy"><span className="module-name">{module.shortName}</span><small>{module.supported ? 'Device test pending' : 'Planned'}</small></span>
            </button>)}
          </aside>
          <section className="workspace" aria-label="Device controls">
            <div className="page-heading"><h1>Your controls</h1><p className="status"><span className="status-dot" />No hardware connected</p></div>
            <div className={`hero ${selected.art === 'knob' || selected.art === 'encoder' ? 'square' : 'wide'}${selected.id === 'm005s' || selected.id === 'm032le' ? ` art-${selected.art}` : ''}`}>
              <div className="module-image" style={{ '--art-aspect': moduleArtwork[selected.id].aspect } as CSSProperties} role="group" aria-label={`${selected.name} image controls`}>
                <img src={`/modules/${selected.art}.png`} alt="" draggable={false} />
                {selected.supported && moduleArtwork[selected.id].controls.map(area => <button key={area.control} type="button"
                  className={`image-control${area.round ? ' round' : ''}${selection.control === area.control ? ' selected' : ''}`}
                  style={{ left: `${area.left}%`, top: `${area.top}%`, width: `${area.width}%`, height: `${area.height}%` }}
                  aria-label={`Select ${controlLabel(selected.id, area.control)} on module image`}
                  aria-pressed={selection.control === area.control} title={controlLabel(selected.id, area.control)}
                  onClick={() => changeSelection({ ...selection, control: area.control })} />)}
              </div>
            </div>
            {selected.supported ? <>
              <div className={`control-picker ${selection.module === 'm4315' ? 'key-picker' : ''}`} role="group" aria-label="Control selection">
                {Array.from({ length: controlCounts[selection.module] }, (_, index) => index).map(control => <button key={control} className={selection.control === control ? 'selected' : ''} aria-pressed={selection.control === control} onClick={() => changeSelection({ ...selection, control })}>{controlLabel(selection.module, control)}</button>)}
              </div>
              <p className="hero-note">{moduleNote(selection.module)}</p>
              <section className="simulation-panel"><div className="section-heading"><h2>Simulation</h2><span className="small-label">Planned actions only</span></div>
                {selection.module === 'm005s' ? <div className="position-preview"><label htmlFor="fader-position">Simulated fader position</label><div className="position-inputs"><input id="fader-position" type="range" min={0} max={faderMaximum} step={1} value={faderPosition} disabled={disabled || preview.status === 'running'} onChange={event => changeFaderPosition(Number(event.target.value))} /><label className="sr-only" htmlFor="fader-position-value">Fader position value</label><input id="fader-position-value" type="number" min={0} max={faderMaximum} step={1} value={faderPosition} disabled={disabled || preview.status === 'running'} onChange={event => changeFaderPosition(Number(event.target.value))} /></div><p className="fine-print">Position scale: 0–100. Preview resolves the volume through the service; physical endpoints still need checking.</p><button className="button" disabled={disabled || preview.status === 'running' || serialScope === 'exact' && !selection.serial} onClick={() => { void testBinding(); }}><Play size={16} />Preview position</button></div> :
                  <div className="event-buttons">{gestures[selection.module].map(gesture => <button className="button" key={gesture} disabled={disabled || preview.status === 'running' || serialScope === 'exact' && !selection.serial} onClick={() => { void testBinding(gesture); }}>{gesture === 'clockwise' || gesture === 'right' ? <RotateCw size={17} /> : gesture === 'counter_clockwise' || gesture === 'left' ? <RotateCcw size={17} /> : <Play size={16} />}{gestureLabels[gesture]}</button>)}</div>}
                <div className="preview-result" role={preview.status === 'failed' ? 'alert' : 'status'}>
                  {preview.status === 'idle' ? <><CircleHelp size={20} /><span>{desktop ? 'Choose a gesture to preview its binding through the service.' : 'Launch the desktop app to preview bindings.'}</span></> :
                    preview.status === 'running' ? <><LoaderCircle size={20} className="spinning" /><span>Resolving binding…</span></> :
                    preview.status === 'failed' ? <><CircleHelp size={20} /><span className="error-text">{preview.error}</span></> :
                    <><CheckCircle2 size={22} /><span>{gestureLabels[preview.event.gesture]}{preview.event.value ? ` ${preview.event.value.raw}/${preview.event.value.maximum}` : ''}: {preview.event.action ? `Would ${actionLabel(preview.event.action).toLowerCase()}.` : 'No action is bound to this event.'}</span></>}
                </div>
                <p className="fine-print">No volume changes, shortcuts or application launches are executed.</p>
              </section>
            </> : <div className="planned-panel"><h2>Input support is planned</h2><p>{selected.name} is included in the module catalog. Its input decoder and binding support are not implemented in this app.</p><p>Display uploads and physical device behavior still require validation.</p><button className="button" onClick={() => setPage('compatibility')}>See compatibility<ChevronRight size={17} /></button></div>}
          </section>
          <aside className="inspector" aria-label="Selected control settings">
            {selected.supported ? <>
              <h2>{controlLabel(selection.module, selection.control)}</h2>
              <label htmlFor="binding-gesture">Gesture</label><SelectField id="binding-gesture" value={selection.gesture} onChange={event => changeSelection({ ...selection, gesture: event.target.value as Gesture })}>
                {gestures[selection.module].map(gesture => <option key={gesture} value={gesture}>{gestureLabels[gesture]}</option>)}
              </SelectField>
              <div className="inspector-divider" />
              <fieldset disabled={disabled || serialScope === 'exact' && !selection.serial}><label htmlFor="binding-action">Action</label>
                <SelectField id="binding-action" value={binding?.action.type ?? 'unbound'} onChange={event => {
                  const type = event.target.value;
                  mutateAction(type === 'unbound' ? null : type === 'volume_from_position' ? { type } : type === 'set_volume' ? { type, percent: 50 } : type === 'volume' ? { type, delta: 5 } : type === 'toggle_mute' ? { type } : type === 'shortcut' ? { type, key: 'c', modifiers: ['command'] } : { type: 'launch_app', bundle_id: 'com.apple.finder' });
                }}><option value="unbound">Unbound</option>{selection.module === 'm005s' ? <option value="volume_from_position">Output volume follows position</option> : <><option value="volume">Change output volume</option><option value="set_volume">Set output volume</option><option value="toggle_mute">Toggle output mute</option><option value="shortcut">Keyboard shortcut</option><option value="launch_app">Open application</option></>}</SelectField>
                {binding?.action.type === 'volume_from_position' && <p className="fine-print">Each position change plans an absolute output volume from 0% to 100%. It does not add a volume delta.</p>}
                {binding?.action.type === 'set_volume' && <div className="action-fields"><label htmlFor="volume-percent">Output volume (%)</label><input id="volume-percent" type="number" min={0} max={100} step={1} value={binding.action.percent} onChange={event => mutateAction({ type: 'set_volume', percent: Number(event.target.value) })} /><p className="fine-print">Plans an absolute output volume from 0% to 100%.</p></div>}
                {binding?.action.type === 'volume' && <div className="action-fields"><label htmlFor="volume-delta">Volume change (points)</label><input id="volume-delta" type="number" min={-100} max={100} step={1} value={binding.action.delta} onChange={event => mutateAction({ type: 'volume', delta: Number(event.target.value) })} /><p className="fine-print">Signed percentage points, −100 to +100, excluding zero.</p></div>}
                {binding?.action.type === 'shortcut' && <div className="action-fields"><label htmlFor="shortcut-key">Key</label><input id="shortcut-key" value={binding.action.key} maxLength={1} onChange={event => mutateAction({ ...(binding.action as Extract<Action, { type: 'shortcut' }>), key: event.target.value })} /><p className="fine-print">One ASCII letter, digit or space. Named keys and sequences are not supported.</p><fieldset className="modifiers"><legend>Modifiers</legend>{(['command', 'control', 'option', 'shift'] as Modifier[]).map(modifier => <label key={modifier}><input type="checkbox" checked={(binding.action as Extract<Action, { type: 'shortcut' }>).modifiers.includes(modifier)} onChange={event => { const action = binding.action as Extract<Action, { type: 'shortcut' }>; mutateAction({ ...action, modifiers: event.target.checked ? [...action.modifiers, modifier] : action.modifiers.filter(item => item !== modifier) }); }} />{modifier[0].toUpperCase() + modifier.slice(1)}</label>)}</fieldset></div>}
                {binding?.action.type === 'launch_app' && <div className="action-fields"><label htmlFor="app-bundle-id">Application bundle ID</label><input id="app-bundle-id" spellCheck={false} value={binding.action.bundle_id} onChange={event => mutateAction({ type: 'launch_app', bundle_id: event.target.value })} /><p className="fine-print">For example, com.apple.finder. The service validates the identifier; replay does not open it.</p></div>}
              </fieldset>
              <div className="inspector-divider" />
              <label htmlFor="serial-scope">Device selector</label><SelectField id="serial-scope" value={serialScope} onChange={event => { const scope = event.target.value as 'any' | 'exact'; setSerialScope(scope); changeSelection({ ...selection, serial: '' }); }}><option value="any">Any device serial</option><option value="exact">Exact device serial</option></SelectField>
              {serialScope === 'exact' && <><label className="spaced-label" htmlFor="serial-number">Exact serial number</label><input id="serial-number" value={selection.serial} spellCheck={false} onChange={event => changeSelection({ ...selection, serial: event.target.value })} /><p className="fine-print">Case-sensitive. Enter a serial before adding an exact binding.</p></>}
              <p className="fine-print">The action editor targets this exact selector. The Rust service applies exact-serial precedence during preview and replay.</p>
              <button className="button full-width" disabled={disabled || preview.status === 'running' || serialScope === 'exact' && !selection.serial} onClick={() => { void testBinding(); }}><Play size={18} />{selection.module === 'm005s' ? 'Test selected position' : 'Test selected gesture'}</button>
              <p className="fine-print">Edits belong to the unsaved profile draft. Save to retain them after restart.</p>
              <div className="planned-actions"><span><Workflow size={17} />Run flow<small>Planned</small></span><span><FileText size={17} />Run script<small>Planned</small></span></div>
            </> : <><h2>{selected.name}</h2><div className="status planned"><span className="status-dot" />Planned</div><p className="inspector-note">Binding editing is available for the encoder and three-knob modules. This module cannot be added to a service profile yet.</p></>}
          </aside>
        </div>}

        {page === 'profiles' && <section className="page-content profiles-page"><div className="page-title"><div><h1>Profiles</h1><p>Saved bindings and device-free replay.</p></div><button className="button" disabled={disabled} onClick={newProfile}><Plus size={18} />New profile</button></div>
          <div className="profiles-grid"><section className="panel saved-profiles"><div className="section-heading"><h2>Saved profiles</h2><button className="icon-button" disabled={disabled} aria-label="Refresh profile list" onClick={() => { void refreshProfiles(); }}><RotateCcw size={18} /></button></div>
            {profiles.length ? profiles.map(item => <button key={item.id} className={`saved-profile ${item.id === profileId ? 'active' : ''}`} disabled={disabled} aria-pressed={item.id === profileId} onClick={() => guard('switch profiles', () => { void load(item.id); })}><FileText size={20} /><span><strong>{item.name}</strong><small>{item.binding_count} bindings</small></span><ChevronRight size={16} /></button>) : <p className="fine-print">{desktop ? 'No saved profiles are available yet.' : 'Saved profiles are available in the desktop app.'}</p>}
          </section><section className="panel profile-editor"><div className="section-heading"><h2>Profile draft</h2><span className={`small-label ${unsaved ? 'draft-label' : ''}`}>{unsaved ? 'Unsaved changes' : 'Saved'}</span></div>
            <label htmlFor="profile-name">Name</label><input id="profile-name" disabled={disabled} value={profile.name} onChange={event => edit({ ...profile, name: event.target.value })} />
            <div className="profile-actions"><button className="button" disabled={disabled || validation === 'running'} onClick={() => { void validate(); }}>{validation === 'running' ? <LoaderCircle size={16} className="spinning" /> : <Check size={17} />}Validate draft</button><button className="button" disabled={disabled || !profileId} onClick={() => { if (profileId) guard('reload the saved profile', () => { void load(profileId); }); }}><ArrowDownToLine size={17} />Reload saved</button></div>
            {validation === 'valid' && <p className="validation-status"><CheckCircle2 size={17} />Current draft is valid.</p>}
            <h3>Bindings <span className="small-label">{profile.bindings.length}</span></h3>
            {profile.bindings.length ? <div className="binding-list">{profile.bindings.map((item, index) => <div key={`${item.module}:${item.control}:${item.gesture}:${item.serial_number ?? ''}:${index}`} className="binding-row"><div><strong>{controlLabel(item.module, item.control)} · {gestureLabels[item.gesture]}</strong><span>{actionLabel(item.action)}</span><small>{item.serial_number ? `Serial: ${item.serial_number}` : 'Any device serial'}</small></div><button className="button compact" onClick={() => selectBinding(item)}>Edit</button><button className="icon-button" aria-label={`Remove ${controlLabel(item.module, item.control)} ${gestureLabels[item.gesture]} binding`} disabled={disabled} onClick={() => edit({ ...profile, bindings: profile.bindings.filter((_, position) => position !== index) })}><Trash2 size={17} /></button></div>)}</div> : <p className="empty-state">No bindings yet. Select a module in Devices and choose an action to add one.</p>}
            <button className="button" onClick={() => setPage('devices')}><Plus size={17} />Edit device bindings</button>
          </section></div>
          <section className="panel replay-panel"><div className="section-heading"><div><h2>Replay a capture</h2><p className="fine-print">Bundled synthetic input reports pass through the Rust decoder and binding engine using the current draft.</p></div><span className="small-label">No hardware required</span></div><div className="replay-controls"><label htmlFor="replay-capture">Capture</label><SelectField id="replay-capture" value={replayKind} onChange={event => { requestSequence.current += 1; setSession({ status: 'idle' }); setReplayKind(event.target.value as ReplayKind); }}>{Object.entries(replayLabels).map(([kind, label]) => <option key={kind} value={kind}>{label}</option>)}</SelectField><button className="button primary" disabled={disabled || session.status === 'running'} onClick={() => { void replay(); }}><Play size={17} />{session.status === 'running' ? 'Running…' : 'Replay capture'}</button></div>
            <SessionView session={session} />
          </section>
        </section>}

        {page === 'compatibility' && <section className="page-content compatibility-page"><div className="page-title"><div><h1>Compatibility</h1><p>Cooler Master MasterHUB modules: implemented foundations and remaining device checks.</p></div><span className="mode-pill">Device-free build</span></div><section className="panel table-panel"><h2>Modules</h2><div className="table-scroll"><table className="compatibility-table"><thead><tr><th>Module</th><th>Binding and replay</th><th>Hardware</th><th>Displays</th></tr></thead><tbody>{modules.map(module => <tr key={module.id}><th scope="row"><div className="compatibility-module"><span className="compatibility-photo"><img src={`/modules/${module.art}.png`} alt="" /></span><span>{module.name}</span></div></th><td><span className={`capability ${module.supported ? 'available' : 'planned'}`}>{module.supported ? 'Available' : 'Planned'}</span><small>{module.input}</small></td><td>Unverified</td><td>{module.id === 'm131e' || module.id === 'm4315' ? 'Not available in this app' : '—'}</td></tr>)}</tbody></table></div><p className="fine-print">Available means the existing service accepts these bindings and synthetic reports. Physical control order, timing and behavior remain unverified.</p></section>
          <div className="feature-grid"><section className="panel"><h2>Actions</h2><ul className="feature-list"><li><Volume2 size={18} /><span>Output volume<small>Deltas for buttons and rotations; absolute position for faders</small></span><span className="capability available">Preview</span></li><li><VolumeX size={18} /><span>Toggle output mute</span><span className="capability available">Preview</span></li><li><Keyboard size={18} /><span>Keyboard shortcut<small>Single ASCII letter, digit or space</small></span><span className="capability available">Preview</span></li><li><Monitor size={18} /><span>Open application<small>Application bundle ID</small></span><span className="capability available">Preview</span></li></ul><p className="fine-print">This desktop build plans actions. It does not execute them or request macOS action permissions.</p></section><section className="panel"><h2>Software and modes</h2><ul className="feature-list"><li><CheckCircle2 size={18} /><span>Manual profile selection</span><span className="capability available">Available</span></li><li><Monitor size={18} /><span>Focused-app profiles</span><span className="capability planned">Planned</span></li><li><Grid2X2 size={18} /><span>Browser tabs on keys</span><span className="capability planned">Planned</span></li><li><Workflow size={18} /><span>Flows, scripts and plugins</span><span className="capability planned">Planned</span></li></ul><p className="fine-print">App focus, browser adapters, live HID sessions and display uploads are outside this first build.</p></section></div>
        </section>}

        {page === 'settings' && <section className="page-content settings-page"><div className="page-title"><div><h1>Settings</h1><p>Appearance and local profile storage.</p></div></div><section className="panel"><h2>Appearance</h2><p className="fine-print">System follows the macOS appearance setting. Your choice is remembered locally.</p>{themePicker}</section><section className="panel"><h2>Profile storage</h2><p>{desktop ? 'Profiles are validated and saved by the native Rust backend.' : 'The browser preview has no profile storage access.'}</p>{storagePath && <><label>Storage directory</label><code className="storage-path">{storagePath}</code></>}<p className="fine-print">Unsaved drafts are not persisted. Save a profile before closing to keep your changes. Dock Quit and system termination can bypass the draft prompt.</p></section><section className="panel"><h2>About OpenStreamHub</h2><p>Open-source control and automation for modular hardware.</p><p>An independent app for Cooler Master MasterHUB.</p><p className="fine-print">Not affiliated with or endorsed by Cooler Master.</p></section><section className="panel"><h2>Current mode</h2><p>Device-free profile editing and synthetic replay.</p><p className="fine-print">No HID interface is opened. No shortcuts, volume changes, application launches or arbitrary scripts are executed by this build.</p></section></section>}
      </main>
    </div>
    <dialog ref={dialog} className="unsaved-dialog" aria-labelledby="unsaved-title" aria-describedby="unsaved-message" onCancel={event => { event.preventDefault(); dialog.current?.close(); setPendingIntent(null); }}><h2 id="unsaved-title">Keep your changes?</h2><p id="unsaved-message">You have an unsaved profile draft. Discard it to {pendingIntent?.label}, or cancel and save your changes first.</p><div className="dialog-actions"><button className="button" autoFocus onClick={() => { dialog.current?.close(); setPendingIntent(null); }}>Cancel</button><button className="button danger" onClick={() => { const intent = pendingIntent; dialog.current?.close(); setPendingIntent(null); intent?.run(); }}>Discard changes</button></div></dialog>
  </div>;
}

function SelectField({ children, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return <div className={`select-field${props.disabled ? ' disabled' : ''}`}><select {...props}>{children}</select><ChevronDown size={17} aria-hidden="true" /></div>;
}

function SessionView({ session }: { session: Session }) {
  if (session.status === 'idle') return <p className="empty-state">Replay results appear here. Editing the profile clears previous results.</p>;
  if (session.status === 'running') return <div className="session-status" role="status"><LoaderCircle size={21} className="spinning" /><span>Running {session.name}…</span></div>;
  if (session.status === 'failed') return <div className="session-status failed" role="alert"><CircleHelp size={21} /><div><strong>Replay failed</strong><p>{session.error}</p></div></div>;
  const { result } = session;
  return <div className="session-results"><div className="session-status completed" role="status"><CheckCircle2 size={21} /><span>Completed · {session.name}</span></div><div className="session-counts"><div><strong>{result.report_count}</strong><span>Reports</span></div><div><strong>{result.event_count}</strong><span>Normalized events</span></div><div><strong>{result.planned_action_count}</strong><span>Planned actions</span></div><div><strong>{result.events.filter(event => event.action === null).length}</strong><span>Unbound events</span></div></div>
    <p className="fine-print">Synthetic source: <code>{result.source}</code>. Planned actions were not executed.</p><div className="table-scroll"><table className="events-table"><caption>Normalized events</caption><thead><tr><th>Time (µs)</th><th>Control</th><th>Gesture</th><th>Value</th><th>Planned action</th></tr></thead><tbody>{result.events.map((event, index) => <tr key={index}><td>{event.timestamp_us}</td><td>{controlLabel(event.module, event.control)}<small>{event.serial_number ? `Serial: ${event.serial_number}` : 'No serial'}</small></td><td>{gestureLabels[event.gesture]}</td><td>{event.value ? `${event.value.raw}/${event.value.maximum}` : '—'}</td><td className={event.action ? '' : 'muted'}>{actionLabel(event.action)}</td></tr>)}</tbody></table></div><div className="diagnostics"><h3>Diagnostics <span className="small-label">{result.diagnostics.length}</span></h3>{result.diagnostics.length ? <ul>{result.diagnostics.map((diagnostic, index) => <li key={index}><code>{diagnostic.timestamp_us} µs</code><span>{diagnostic.message}</span></li>)}</ul> : <p className="fine-print">No parser diagnostics.</p>}</div>
  </div>;
}
