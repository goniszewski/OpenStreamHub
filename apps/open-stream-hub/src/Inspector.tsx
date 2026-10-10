import { useState } from 'react';
import { CheckCircle2, CircleHelp, Folder, LoaderCircle, Play, Trash2 } from 'lucide-react';
import type { Gesture, Profile } from './contracts';
import { DialDisplayPanel } from './DialDisplayPanel';
import { ActionFields } from './ActionFields';
import { IntegerField, LabelError, SelectField } from './controls';
import { keypadDepth, keypadMaximumDepth, keypadMaximumPages, renameKeypadAction, renameKeypadPage } from './keypad-model';
import { useObsCatalog } from './obs-catalog';
import type { ObsCatalogStore } from './obs-catalog';
import type { KeypadPreview, Invoke, ProfileSession, SessionState } from './profile-session';
import { KeyArtworkEditor } from './KeyArtworkEditor';
import { actionLabel, controlLabel, faderMaximum, gestureLabels, gestures } from './ui-model';
import type { Page } from './ui-model';
import type { DeviceEditor, SerialScope } from './useDeviceEditor';
import { ControlWarnings } from './BindingsPanel';
import { editorGestures } from './binding-model';
import { DialFunctionsPanel, bindingPreviewText } from './DialFunctionsPanel';


export function Inspector({ editor, session, state, disabled, desktop = false, invoke, setPage, catalog: catalogStore }: { editor: DeviceEditor; session: ProfileSession; state: SessionState; disabled: boolean; desktop?: boolean; invoke?: Invoke; setPage?: (page: Page) => void; catalog?: ObsCatalogStore }) {
  const { selected, selection, isKeypad, isBack, keypadPath, currentKeypadPage, keypadSlot, folderPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  // While the profile uses dial functions, the encoder's inspector is the dial functions editor
  // (every encoder binding belongs to a function there); the other modules keep the flat editor.
  const dialFunctionsActive = selection.module === 'm131e' && !!profile.dial_functions;
  const previewRunning = state.preview.status === 'running' || state.keypadPreview.status === 'running';
  const serialMissing = serialScope === 'exact' && !selection.serial;
  const resultFailed = isKeypad ? state.keypadPreview.status === 'failed' : state.preview.status === 'failed';
  const resultDone = isKeypad ? state.keypadPreview.status === 'completed' : state.preview.status === 'completed';
  // Number fields remount when the edited selector changes, discarding typed text.
  const selectorKey = `${selection.module}:${keypadPath.join('/')}:${selection.control}:${selection.gesture}:${selection.serial}`;
  // OBS's current listings for the name fields; asked once per app session, refreshed on demand.
  const catalog = useObsCatalog(catalogStore);
  // Which tab of the dial editor (functions or dial screen) is open; view state only.
  const [dialTab, setDialTab] = useState<'functions' | 'display'>('functions');
  return <aside className="inspector" aria-label="Selected control settings">
    {selected.supported ? <>
      {isKeypad && <><label htmlFor="keypad-page-name">{keypadDepth(keypadPath) ? `Folder · L${keypadDepth(keypadPath)}` : 'Root page'}</label><input id="keypad-page-name" disabled={disabled} value={currentKeypadPage.name} aria-describedby="keypad-page-name-error" onChange={event => session.edit(renameKeypadPage(profile, keypadPath, event.target.value), `page-name:${keypadPath.join('/')}`)} /><LabelError id="keypad-page-name-error" value={currentKeypadPage.name} field="Page name" />{keypadPath.length > 1 && <button className="button compact" disabled={disabled} onClick={editor.deleteCurrentFolder}><Trash2 size={16} />Delete folder</button>}<div className="inspector-divider" /></>}
      <h2>{isBack ? `Back · L${keypadDepth(keypadPath)}` : controlLabel(selection.module, selection.control)}</h2>
      <ControlWarnings profile={profile} editor={editor} selected={{ module: selection.module, control: selection.control, keypadPageId: isKeypad ? currentKeypadPage.id : undefined }}
        excludeKinds={dialFunctionsActive ? ['dial-flat-conflict'] : undefined} />
      {isKeypad && !isBack && <KeyArtworkEditor key={`${state.draftGeneration}:${currentKeypadPage.id}:${selection.control}`} session={session} path={keypadPath} control={selection.control} artwork={currentKeypadPage.artwork?.[String(selection.control)]} action={keypadSlot?.type === 'action' ? keypadSlot.bindings[0]?.action : null} initialText={keypadSlot?.type === 'action' ? keypadSlot.label : ''} disabled={disabled} invoke={invoke} />}
      {isKeypad && <>{isBack ? <p className="fine-print">The reserved Back key returns one level. Its badge shows the current nesting level.</p> : folderPage ? <><button className="button" onClick={() => editor.navigateKeypad([...keypadPath, folderPage.id])}><Folder size={17} />Open {folderPage.name}</button><button className="button" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={17} />Delete folder</button></> : <>{keypadSlot?.type === 'action' ? <><label htmlFor="keypad-label">Key label</label><input id="keypad-label" disabled={disabled} value={keypadSlot.label} aria-describedby="keypad-label-error" onChange={event => session.edit(renameKeypadAction(profile, keypadPath, selection.control, event.target.value), `key-label:${keypadPath.join('/')}:${selection.control}`)} /><LabelError id="keypad-label-error" value={keypadSlot.label} field="Key label" /><button className="button compact" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={16} />Clear key</button></> : <button className="button" disabled={disabled || keypadDepth(keypadPath) >= keypadMaximumDepth || profile.keypad!.pages.length >= keypadMaximumPages} onClick={editor.createFolder}><Folder size={17} />Create folder</button>}</>}</>}
      {dialFunctionsActive ? <DialEditorTabs tab={dialTab} onTab={setDialTab} profile={profile} session={session} state={state} disabled={disabled} invoke={invoke} setPage={setPage} catalog={catalogStore} />
      : !(isKeypad && (isBack || folderPage)) && <>
      <label htmlFor="binding-gesture">Gesture</label><SelectField id="binding-gesture" disabled={!editorGestures(profile, selection.module).length} value={selection.gesture} onChange={event => editor.changeSelection({ ...selection, gesture: event.target.value as Gesture })}>
        {editorGestures(profile, selection.module).map(gesture => <option key={gesture} value={gesture}>{gestureLabels[gesture]}</option>)}
      </SelectField>
      <div className="inspector-divider" />
      <fieldset disabled={disabled || serialMissing || !editorGestures(profile, selection.module).length}>
        <ActionFields action={binding?.action ?? null} fader={selection.module === 'm005s'} selectorKey={selectorKey} onChange={editor.mutateAction} onSetUpObs={setPage ? () => setPage('settings') : undefined} catalog={catalog} />
      </fieldset>
      <div className="inspector-divider" />
      <details className="advanced">
        <summary>Advanced</summary>
        <label htmlFor="serial-scope">Limit to one device</label><SelectField id="serial-scope" value={serialScope} onChange={event => editor.changeSerialScope(event.target.value as SerialScope)}><option value="any">Any device of this type</option><option value="exact">One device by serial number</option></SelectField>
        {serialScope === 'exact' && <><label className="spaced-label" htmlFor="serial-number">Serial number</label><input id="serial-number" value={selection.serial} spellCheck={false} onChange={event => editor.changeSelection({ ...selection, serial: event.target.value })} /><p className="fine-print">Matched exactly, including upper and lower case. Enter a serial before adding the binding.</p></>}
        <p className="fine-print">If you connect more than one module of the same type, a serial number keeps this binding on the one you chose.</p>
      </details>
      {selection.module === 'm005s' && <div className="position-preview"><label htmlFor="fader-position">Simulated fader position</label><div className="position-inputs"><input id="fader-position" type="range" min={0} max={faderMaximum} step={1} value={editor.faderPosition} disabled={disabled || state.preview.status === 'running'} onChange={event => editor.changeFaderPosition(Number(event.target.value))} /><label className="sr-only" htmlFor="fader-position-value">Fader position value</label><IntegerField id="fader-position-value" value={editor.faderPosition} minimum={0} maximum={faderMaximum} disabled={disabled || state.preview.status === 'running'} onCommit={editor.changeFaderPosition} /></div><p className="fine-print">Position scale: 0–100. A test sets the volume for the position you choose; the physical fader's endpoints still need checking.</p></div>}
      <div className="inspector-test">
        <button className="button compact" disabled={disabled || previewRunning || serialMissing} onClick={() => { void editor.testBinding(); }}><Play size={15} />{selection.module === 'm005s' ? 'Test position' : isKeypad ? 'Simulate key click' : 'Test gesture'}</button>
        <p className={`preview-line${resultDone ? ' done' : ''}`} role={resultFailed ? 'alert' : 'status'}><TestResult desktop={desktop} preview={state.preview} keypadPreview={state.keypadPreview} isKeypad={isKeypad} /></p>
        <p className="fine-print">Tests never execute anything. Your changes are not saved until you press Save profile.</p>
      </div>
      </>}
      {/* The dial functions editor always lives here: while functions are off it offers to add
          them (flat bindings keep working until then), and with functions on it is the only
          editor, since every encoder binding then belongs to a function. */}
      {!dialFunctionsActive && selection.module === 'm131e' && <DialEditorTabs tab={dialTab} onTab={setDialTab} profile={profile} session={session} state={state} disabled={disabled} invoke={invoke} setPage={setPage} catalog={catalogStore} />}
    </> : <><h2>{selected.name}</h2><div className="status offline"><span className="status-dot" />Not available yet</div><p className="inspector-note">Binding editing is available for the encoder and three-knob modules. See Compatibility for what this module supports.</p></>}
  </aside>;
}

// The dial editor's two tabs. Functions is the dial functions editor (or its
// offer to add them); Display sets what the dial screen shows for this
// profile. The tab choice is view state; it survives nothing.
export function DialEditorTabs({ tab, onTab, profile, session, state, disabled, invoke, setPage, catalog: catalogStore }: {
  tab: 'functions' | 'display'; onTab(tab: 'functions' | 'display'): void; profile: Profile; session: ProfileSession; state: SessionState; disabled: boolean; invoke?: Invoke; setPage?: (page: Page) => void; catalog?: ObsCatalogStore;
}) {
  return <section className="dial-editor" aria-label="Dial editor">
    <div role="tablist" aria-label="Dial editor sections" className="dial-tabs">
      <button type="button" role="tab" id="dial-tab-functions" aria-selected={tab === 'functions'} aria-controls="dial-panel-functions" className={tab === 'functions' ? 'active' : ''} onClick={() => onTab('functions')}>Functions</button>
      <button type="button" role="tab" id="dial-tab-display" aria-selected={tab === 'display'} aria-controls="dial-panel-display" className={tab === 'display' ? 'active' : ''} onClick={() => onTab('display')}>Display</button>
    </div>
    {tab === 'functions'
      ? <div role="tabpanel" id="dial-panel-functions" aria-labelledby="dial-tab-functions"><DialFunctionsPanel profile={profile} session={session} state={state} disabled={disabled} setPage={setPage} catalog={catalogStore} /></div>
      : <div role="tabpanel" id="dial-panel-display" aria-labelledby="dial-tab-display"><DialDisplayPanel profile={profile} session={session} disabled={disabled} invoke={invoke} /></div>}
  </section>;
}

// The one-line result of the inspector's Test button, for a gesture, a fader position or a key click.
export function TestResult({ desktop, preview, keypadPreview, isKeypad }: { desktop: boolean; preview: SessionState['preview']; keypadPreview: KeypadPreview; isKeypad: boolean }) {
  if (isKeypad) {
    if (keypadPreview.status === 'idle') return <><CircleHelp size={15} /><span>Run a test to see what this key click would do.</span></>;
    if (keypadPreview.status === 'running') return <><LoaderCircle size={15} className="spinning" /><span>Resolving key click…</span></>;
    if (keypadPreview.status === 'failed') return <><CircleHelp size={15} /><span className="error-text">{keypadPreview.error}</span></>;
    const navigation = keypadPreview.result.events.find(event => event.navigation)?.navigation;
    const actions = keypadPreview.result.events.filter(event => event.action);
    return <><CheckCircle2 size={15} /><span>{navigation ? `Navigated ${navigation.depth ? `to L${navigation.depth}` : 'to root'}.` : actions.length ? actions.map(event => `${gestureLabels[event.gesture]}: would ${actionLabel(event.action).toLowerCase()}`).join('; ') : 'No action is bound to this key click.'}</span></>;
  }
  if (preview.status === 'idle') return <><CircleHelp size={15} /><span>{desktop ? 'Run a test to preview what the binding would do.' : 'Launch the desktop app to preview bindings.'}</span></>;
  if (preview.status === 'running') return <><LoaderCircle size={15} className="spinning" /><span>Resolving binding…</span></>;
  if (preview.status === 'failed') return <><CircleHelp size={15} /><span className="error-text">{preview.error}</span></>;
  return <><CheckCircle2 size={15} /><span>{bindingPreviewText(preview.event)}</span></>;
}
