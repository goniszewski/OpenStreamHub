import { CheckCircle2, CircleHelp, Folder, LoaderCircle, Play, Trash2 } from 'lucide-react';
import type { Gesture } from './contracts';
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
import { bindingPreviewText } from './DialFunctionsPanel';

export function Inspector({ editor, session, state, disabled, desktop = false, invoke, setPage, catalog: catalogStore }: { editor: DeviceEditor; session: ProfileSession; state: SessionState; disabled: boolean; desktop?: boolean; invoke?: Invoke; setPage?: (page: Page) => void; catalog?: ObsCatalogStore }) {
  const { selected, selection, isKeypad, isBack, keypadPath, currentKeypadPage, keypadSlot, folderPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  const previewRunning = state.preview.status === 'running' || state.keypadPreview.status === 'running';
  const serialMissing = serialScope === 'exact' && !selection.serial;
  const resultFailed = isKeypad ? state.keypadPreview.status === 'failed' : state.preview.status === 'failed';
  const resultDone = isKeypad ? state.keypadPreview.status === 'completed' : state.preview.status === 'completed';
  // Number fields remount when the edited selector changes, discarding typed text.
  const selectorKey = `${selection.module}:${keypadPath.join('/')}:${selection.control}:${selection.gesture}:${selection.serial}`;
  // OBS's current listings for the name fields; asked once per app session, refreshed on demand.
  const catalog = useObsCatalog(catalogStore);
  return <aside className="inspector" aria-label="Selected control settings">
    {selected.supported ? <>
      {isKeypad && <><label htmlFor="keypad-page-name">{keypadDepth(keypadPath) ? `Folder · L${keypadDepth(keypadPath)}` : 'Root page'}</label><input id="keypad-page-name" disabled={disabled} value={currentKeypadPage.name} aria-describedby="keypad-page-name-error" onChange={event => session.edit(renameKeypadPage(profile, keypadPath, event.target.value), `page-name:${keypadPath.join('/')}`)} /><LabelError id="keypad-page-name-error" value={currentKeypadPage.name} field="Page name" />{keypadPath.length > 1 && <button className="button compact" disabled={disabled} onClick={editor.deleteCurrentFolder}><Trash2 size={16} />Delete folder</button>}<div className="inspector-divider" /></>}
      <h2>{isBack ? `Back · L${keypadDepth(keypadPath)}` : controlLabel(selection.module, selection.control)}</h2>
      <ControlWarnings profile={profile} editor={editor} selected={{ module: selection.module, control: selection.control, keypadPageId: isKeypad ? currentKeypadPage.id : undefined }} />
      {isKeypad && !isBack && <KeyArtworkEditor key={`${state.draftGeneration}:${currentKeypadPage.id}:${selection.control}`} session={session} path={keypadPath} control={selection.control} artwork={currentKeypadPage.artwork?.[String(selection.control)]} disabled={disabled} invoke={invoke} />}
      {isKeypad && <>{isBack ? <p className="fine-print">The reserved Back key returns one level. Its badge shows the current nesting level.</p> : folderPage ? <><button className="button" onClick={() => editor.navigateKeypad([...keypadPath, folderPage.id])}><Folder size={17} />Open {folderPage.name}</button><button className="button" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={17} />Delete folder</button></> : <>{keypadSlot?.type === 'action' ? <><label htmlFor="keypad-label">Key label</label><input id="keypad-label" disabled={disabled} value={keypadSlot.label} aria-describedby="keypad-label-error" onChange={event => session.edit(renameKeypadAction(profile, keypadPath, selection.control, event.target.value), `key-label:${keypadPath.join('/')}:${selection.control}`)} /><LabelError id="keypad-label-error" value={keypadSlot.label} field="Key label" /><button className="button compact" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={16} />Clear key</button></> : <button className="button" disabled={disabled || keypadDepth(keypadPath) >= keypadMaximumDepth || profile.keypad!.pages.length >= keypadMaximumPages} onClick={editor.createFolder}><Folder size={17} />Create folder</button>}</>}</>}
      {!(isKeypad && (isBack || folderPage)) && <>
      {!editorGestures(profile, selection.module).length && <p className="fine-print" role="note">This profile uses dial functions, so every encoder binding belongs to a function and a short press switches functions. Edit them under Profiles · Dial functions.</p>}
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
        {selection.module === 'm131e' && profile.dial_functions && <p className="fine-print">Tests use the first dial function, where a run starts; a short press shows the function it would switch to.</p>}
        <p className="fine-print">Tests never execute anything. Your changes are not saved until you press Save profile.</p>
      </div>
      </>}
    </> : <><h2>{selected.name}</h2><div className="status offline"><span className="status-dot" />Not available yet</div><p className="inspector-note">Binding editing is available for the encoder and three-knob modules. See Compatibility for what this module supports.</p></>}
  </aside>;
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
