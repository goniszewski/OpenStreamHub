import { FileText, Folder, Play, Trash2, Workflow } from 'lucide-react';
import type { Gesture } from './contracts';
import { ActionFields } from './ActionFields';
import { LabelError, SelectField } from './controls';
import { keypadDepth, keypadMaximumDepth, keypadMaximumPages, renameKeypadAction, renameKeypadPage } from './keypad-model';
import type { ProfileSession, SessionState } from './profile-session';
import { controlLabel, gestureLabels, gestures } from './ui-model';
import type { DeviceEditor, SerialScope } from './useDeviceEditor';
import { ControlWarnings } from './BindingsPanel';
import { editorGestures } from './binding-model';

export function Inspector({ editor, session, state, disabled }: { editor: DeviceEditor; session: ProfileSession; state: SessionState; disabled: boolean }) {
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
      <label htmlFor="serial-scope">Device selector</label><SelectField id="serial-scope" value={serialScope} onChange={event => editor.changeSerialScope(event.target.value as SerialScope)}><option value="any">Any device serial</option><option value="exact">Exact device serial</option></SelectField>
      {serialScope === 'exact' && <><label className="spaced-label" htmlFor="serial-number">Exact serial number</label><input id="serial-number" value={selection.serial} spellCheck={false} onChange={event => editor.changeSelection({ ...selection, serial: event.target.value })} /><p className="fine-print">Case-sensitive. Enter a serial before adding an exact binding.</p></>}
      <p className="fine-print">The action editor targets this exact selector. The Rust service applies exact-serial precedence during preview and replay.</p>
      <button className="button full-width" disabled={disabled || previewRunning || serialMissing} onClick={() => { void editor.testBinding(); }}><Play size={18} />{selection.module === 'm005s' ? 'Test selected position' : isKeypad ? 'Simulate key click' : 'Test selected gesture'}</button>
      <p className="fine-print">Edits belong to the unsaved profile draft. Save to retain them after restart.</p>
      </>}
      <div className="planned-actions"><span><Workflow size={17} />Run flow<small>Planned</small></span><span><FileText size={17} />Run script<small>Planned</small></span></div>
    </> : <><h2>{selected.name}</h2><div className="status planned"><span className="status-dot" />Planned</div><p className="inspector-note">Binding editing is available for the encoder and three-knob modules. This module cannot be added to a service profile yet.</p></>}
  </aside>;
}
