import { Folder, Play, Trash2 } from 'lucide-react';
import type { Gesture } from './contracts';
import { ActionFields } from './ActionFields';
import { LabelError, SelectField } from './controls';
import { keypadDepth, keypadMaximumDepth, keypadMaximumPages, renameKeypadAction, renameKeypadPage } from './keypad-model';
import type { Invoke, ProfileSession, SessionState } from './profile-session';
import { KeyArtworkEditor } from './KeyArtworkEditor';
import { controlLabel, gestureLabels, gestures } from './ui-model';
import type { DeviceEditor, SerialScope } from './useDeviceEditor';
import { ControlWarnings } from './BindingsPanel';
import { editorGestures } from './binding-model';

export function Inspector({ editor, session, state, disabled, invoke }: { editor: DeviceEditor; session: ProfileSession; state: SessionState; disabled: boolean; invoke?: Invoke }) {
  const { selected, selection, isKeypad, isBack, keypadPath, currentKeypadPage, keypadSlot, folderPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  const previewRunning = state.preview.status === 'running' || state.keypadPreview.status === 'running';
  const serialMissing = serialScope === 'exact' && !selection.serial;
  // Number fields remount when the edited selector changes, discarding typed text.
  const selectorKey = `${selection.module}:${keypadPath.join('/')}:${selection.control}:${selection.gesture}:${selection.serial}`;
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
        <ActionFields action={binding?.action ?? null} fader={selection.module === 'm005s'} selectorKey={selectorKey} onChange={editor.mutateAction} />
      </fieldset>
      <div className="inspector-divider" />
      <details className="advanced">
        <summary>Advanced</summary>
        <label htmlFor="serial-scope">Limit to one device</label><SelectField id="serial-scope" value={serialScope} onChange={event => editor.changeSerialScope(event.target.value as SerialScope)}><option value="any">Any device of this type</option><option value="exact">One device by serial number</option></SelectField>
        {serialScope === 'exact' && <><label className="spaced-label" htmlFor="serial-number">Serial number</label><input id="serial-number" value={selection.serial} spellCheck={false} onChange={event => editor.changeSelection({ ...selection, serial: event.target.value })} /><p className="fine-print">Matched exactly, including upper and lower case. Enter a serial before adding the binding.</p></>}
        <p className="fine-print">If you connect more than one module of the same type, a serial number keeps this binding on the one you chose.</p>
      </details>
      <button className="button full-width" disabled={disabled || previewRunning || serialMissing} onClick={() => { void editor.testBinding(); }}><Play size={18} />{selection.module === 'm005s' ? 'Test selected position' : isKeypad ? 'Simulate key click' : 'Test selected gesture'}</button>
      <p className="fine-print">Your changes are not saved until you press Save profile.</p>
      </>}
    </> : <><h2>{selected.name}</h2><div className="status offline"><span className="status-dot" />Not available yet</div><p className="inspector-note">Binding editing is available for the encoder and three-knob modules. See Compatibility for what this module supports.</p></>}
  </aside>;
}
