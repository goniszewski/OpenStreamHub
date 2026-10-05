import { FileText, Folder, Play, Trash2, Workflow } from 'lucide-react';
import type { Action, Gesture, Modifier } from './contracts';
import { IntegerField, LabelError, SelectField } from './controls';
import { keypadDepth, keypadMaximumDepth, keypadMaximumPages, renameKeypadAction, renameKeypadPage } from './keypad-model';
import type { ProfileSession, SessionState } from './profile-session';
import { controlLabel, gestureLabels, gestures } from './ui-model';
import type { DeviceEditor, SerialScope } from './useDeviceEditor';

export function Inspector({ editor, session, state, disabled }: { editor: DeviceEditor; session: ProfileSession; state: SessionState; disabled: boolean }) {
  const { selected, selection, isKeypad, isBack, keypadPath, currentKeypadPage, keypadSlot, folderPage, binding, serialScope } = editor;
  const profile = state.draft.profile;
  const previewRunning = state.preview.status === 'running' || state.keypadPreview.status === 'running';
  const serialMissing = serialScope === 'exact' && !selection.serial;
  // Number fields remount when the edited selector changes, discarding typed text.
  const selectorKey = `${selection.module}:${keypadPath.join('/')}:${selection.control}:${selection.gesture}:${selection.serial}`;
  return <aside className="inspector" aria-label="Selected control settings">
    {selected.supported ? <>
      {isKeypad && <><label htmlFor="keypad-page-name">{keypadDepth(keypadPath) ? `Folder · L${keypadDepth(keypadPath)}` : 'Root page'}</label><input id="keypad-page-name" disabled={disabled} value={currentKeypadPage.name} aria-describedby="keypad-page-name-error" onChange={event => session.edit(renameKeypadPage(profile, keypadPath, event.target.value))} /><LabelError id="keypad-page-name-error" value={currentKeypadPage.name} field="Page name" />{keypadPath.length > 1 && <button className="button compact" disabled={disabled} onClick={editor.deleteCurrentFolder}><Trash2 size={16} />Delete folder</button>}<div className="inspector-divider" /></>}
      <h2>{isBack ? `Back · L${keypadDepth(keypadPath)}` : controlLabel(selection.module, selection.control)}</h2>
      {isKeypad && <>{isBack ? <p className="fine-print">The reserved Back key returns one level. Its badge shows the current nesting level.</p> : folderPage ? <><button className="button" onClick={() => editor.navigateKeypad([...keypadPath, folderPage.id])}><Folder size={17} />Open {folderPage.name}</button><button className="button" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={17} />Delete folder</button></> : <>{keypadSlot?.type === 'action' ? <><label htmlFor="keypad-label">Key label</label><input id="keypad-label" disabled={disabled} value={keypadSlot.label} aria-describedby="keypad-label-error" onChange={event => session.edit(renameKeypadAction(profile, keypadPath, selection.control, event.target.value))} /><LabelError id="keypad-label-error" value={keypadSlot.label} field="Key label" /><button className="button compact" disabled={disabled} onClick={() => editor.clearKey()}><Trash2 size={16} />Clear key</button></> : <button className="button" disabled={disabled || keypadDepth(keypadPath) >= keypadMaximumDepth || profile.keypad!.pages.length >= keypadMaximumPages} onClick={editor.createFolder}><Folder size={17} />Create folder</button>}</>}</>}
      {!(isKeypad && (isBack || folderPage)) && <>
      <label htmlFor="binding-gesture">Gesture</label><SelectField id="binding-gesture" value={selection.gesture} onChange={event => editor.changeSelection({ ...selection, gesture: event.target.value as Gesture })}>
        {gestures[selection.module].map(gesture => <option key={gesture} value={gesture}>{gestureLabels[gesture]}</option>)}
      </SelectField>
      <div className="inspector-divider" />
      <fieldset disabled={disabled || serialMissing}><label htmlFor="binding-action">Action</label>
        <SelectField id="binding-action" value={binding?.action.type ?? 'unbound'} onChange={event => {
          const type = event.target.value;
          editor.mutateAction(type === 'unbound' ? null : type === 'volume_from_position' ? { type } : type === 'set_volume' ? { type, percent: 50 } : type === 'volume' ? { type, delta: 5 } : type === 'toggle_mute' ? { type } : type === 'shortcut' ? { type, key: 'c', modifiers: ['command'] } : { type: 'launch_app', bundle_id: 'com.apple.finder' });
        }}><option value="unbound">Unbound</option>{selection.module === 'm005s' ? <option value="volume_from_position">Output volume follows position</option> : <><option value="volume">Change output volume</option><option value="set_volume">Set output volume</option><option value="toggle_mute">Toggle output mute</option><option value="shortcut">Keyboard shortcut</option><option value="launch_app">Open application</option></>}</SelectField>
        {binding?.action.type === 'volume_from_position' && <p className="fine-print">Each position change plans an absolute output volume from 0% to 100%. It does not add a volume delta.</p>}
        {binding?.action.type === 'set_volume' && <div className="action-fields"><label htmlFor="volume-percent">Output volume (%)</label><IntegerField key={selectorKey} id="volume-percent" value={binding.action.percent} minimum={0} maximum={100} onCommit={percent => editor.mutateAction({ type: 'set_volume', percent })} /><p className="fine-print">Plans an absolute output volume from 0% to 100%.</p></div>}
        {binding?.action.type === 'volume' && <div className="action-fields"><label htmlFor="volume-delta">Volume change (points)</label><IntegerField key={selectorKey} id="volume-delta" value={binding.action.delta} minimum={-100} maximum={100} nonZero onCommit={delta => editor.mutateAction({ type: 'volume', delta })} /><p className="fine-print">Signed percentage points, −100 to +100, excluding zero.</p></div>}
        {binding?.action.type === 'shortcut' && <div className="action-fields"><label htmlFor="shortcut-key">Key</label><input id="shortcut-key" value={binding.action.key} maxLength={1} onChange={event => editor.mutateAction({ ...(binding.action as Extract<Action, { type: 'shortcut' }>), key: event.target.value })} /><p className="fine-print">One ASCII letter, digit or space. Named keys and sequences are not supported.</p><fieldset className="modifiers"><legend>Modifiers</legend>{(['command', 'control', 'option', 'shift'] as Modifier[]).map(modifier => <label key={modifier}><input type="checkbox" checked={(binding.action as Extract<Action, { type: 'shortcut' }>).modifiers.includes(modifier)} onChange={event => { const action = binding.action as Extract<Action, { type: 'shortcut' }>; editor.mutateAction({ ...action, modifiers: event.target.checked ? [...action.modifiers, modifier] : action.modifiers.filter(item => item !== modifier) }); }} />{modifier[0].toUpperCase() + modifier.slice(1)}</label>)}</fieldset></div>}
        {binding?.action.type === 'launch_app' && <div className="action-fields"><label htmlFor="app-bundle-id">Application bundle ID</label><input id="app-bundle-id" spellCheck={false} value={binding.action.bundle_id} onChange={event => editor.mutateAction({ type: 'launch_app', bundle_id: event.target.value })} /><p className="fine-print">For example, com.apple.finder. The service validates the identifier; replay does not open it.</p></div>}
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
