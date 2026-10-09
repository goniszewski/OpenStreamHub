import { useState } from 'react';
import type { Action, Binding, BindingAction, KeypadBinding, Profile, ServiceModule } from './contracts';
import { defaultBindingAction, isValidForControl, pickerEntryLabel, pickerWebStorage, recordRecentAction } from './action-picker-model';
import type { PickerActionType } from './action-picker-model';
import { clampUtf8 } from './draft-model';
import { clearKeypadSlot, createKeypadFolder, editKeypadBinding, keypadBackPath, keypadBindingAt, keypadIsBack, keypadPagesAtPath, keypadPathToPage, keypadRoot } from './keypad-model';
import type { ProfileSession, SessionState } from './profile-session';
import { actionLabel, controlLabel, editBinding, errorMessage, gestures, modules, sameSelector } from './ui-model';
import type { BindingSelection, ModuleId, Page } from './ui-model';

export interface ConfirmRequest { title: string; description: string; confirmLabel: string; run: () => void }
export type SerialScope = 'any' | 'exact';

// Falls back to the root page if an edit or reload removed the open folder.
export function usablePath(state: SessionState, keypad: { generation: number; path: string[] }) {
  if (keypad.generation !== state.draftGeneration) return [keypadRoot];
  try { keypadPagesAtPath(state.draft.profile, keypad.path); return keypad.path; } catch { return [keypadRoot]; }
}

// The stored keypad view state, reset to the root page once the open folder is gone.
// Returns the same object while the path is usable. Without the reset, undoing
// Create folder and then redoing it would jump back into the folder.
export function syncKeypad(state: SessionState, keypad: { generation: number; path: string[] }) {
  const path = usablePath(state, keypad);
  return path === keypad.path ? keypad : { generation: state.draftGeneration, path };
}

// The draft a dropped picker entry writes: the action's default parameters on the control's
// primary gesture, or on the key's press, in the keypad folder being edited. Throws for slots
// that cannot take a binding (a folder key, the reserved Back key).
export function dropProfileEdit(profile: Profile, keypadPath: readonly string[], module: ServiceModule, control: number, type: PickerActionType): Profile {
  const action = defaultBindingAction(type);
  if (module === 'm4315') return editKeypadBinding(profile, keypadPath, control, 'pressed', '', action as Action, clampUtf8(actionLabel(action)));
  return editBinding(profile, { module, control, gesture: gestures[module][0], serial: '' }, action);
}

// What a drop finds on the target's primary gesture: the any-serial action it replaces, and the
// serial of an exact-serial binding that stays in place beside the new any-serial one.
export function dropTargetState(profile: Profile, keypadPath: readonly string[], module: ServiceModule, control: number): { previous?: BindingAction; keptSerial?: string } {
  const gesture = gestures[module][0];
  if (module === 'm4315') {
    const slot = keypadPagesAtPath(profile, keypadPath).at(-1)!.slots[control];
    if (slot?.type !== 'action') return {};
    return {
      previous: slot.bindings.find(item => item.gesture === 'pressed' && (item.serial_number ?? '') === '')?.action,
      keptSerial: slot.bindings.find(item => item.gesture === 'pressed' && item.serial_number)?.serial_number,
    };
  }
  return {
    previous: profile.bindings.find(item => sameSelector(item, { module, control, gesture, serial: '' }))?.action,
    keptSerial: profile.bindings.find(item => item.module === module && item.control === control && item.gesture === gesture && item.serial_number)?.serial_number,
  };
}

// The one-line result of a drop, in catalogue names: what replaced what, or what was added and
// what an exact-serial binding keeps doing.
export function dropNoticeText(found: { previous?: BindingAction; keptSerial?: string }, type: PickerActionType, control: string): string {
  if (found.previous) return `Replaced ${pickerEntryLabel(found.previous.type as PickerActionType)} with ${pickerEntryLabel(type)} on ${control}.`;
  if (found.keptSerial) return `Added ${pickerEntryLabel(type)} to ${control}; the binding for serial ${found.keptSerial} is unchanged.`;
  return `Added ${pickerEntryLabel(type)} to ${control}.`;
}

// View state for the Devices page and inspector: which module, control,
// gesture and keypad folder is selected. The draft itself lives in ProfileSession.
export function useDeviceEditor(session: ProfileSession, state: SessionState, setPage: (page: Page) => void, confirm: (request: ConfirmRequest) => void) {
  const profile = state.draft.profile;
  const [selectedModule, setSelectedModule] = useState<ModuleId>('m003k');
  const [selection, setSelection] = useState<BindingSelection>({ module: 'm003k', control: 0, gesture: 'clockwise', serial: '' });
  const [serialScope, setSerialScope] = useState<SerialScope>('any');
  const [faderPosition, setFaderPosition] = useState(50);
  const [keypad, setKeypad] = useState({ generation: state.draftGeneration, path: [keypadRoot] });
  const synced = syncKeypad(state, keypad);
  // Adjusting state during render (React's derived-state pattern); it settles in one extra render.
  if (synced !== keypad) setKeypad(synced);
  const keypadPath = synced.path;
  const setKeypadPath = (path: string[]) => setKeypad({ generation: session.getState().draftGeneration, path });

  const selected = modules.find(module => module.id === selectedModule)!;
  const keypadPages = keypadPagesAtPath(profile, keypadPath);
  const currentKeypadPage = keypadPages.at(-1)!;
  const isKeypad = selection.module === 'm4315';
  const isBack = isKeypad && keypadIsBack(keypadPath, selection.control);
  const keypadSlot = isKeypad ? currentKeypadPage.slots[selection.control] : null;
  const folderPage = keypadSlot?.type === 'folder' ? profile.keypad!.pages.find(item => item.id === keypadSlot.page_id) : undefined;
  const binding = isKeypad ? keypadBindingAt(profile, keypadPath, selection.control, selection.gesture as KeypadBinding['gesture'], selection.serial) : profile.bindings.find(item => sameSelector(item, selection));

  // `field` names a typed field, so rapid edits of it share one undo step.
  function mutateAction(action: BindingAction | null, field?: string) {
    const key = field && `${field}:${selection.module}:${keypadPath.join('/')}:${selection.control}:${selection.gesture}:${selection.serial}`;
    try {
      if (isKeypad) {
        if (action?.type === 'volume_from_position') throw new Error('This action is available only for faders.');
        session.edit(editKeypadBinding(profile, keypadPath, selection.control, selection.gesture as KeypadBinding['gesture'], selection.serial, action, clampUtf8(actionLabel(action))), key);
      } else session.edit(editBinding(profile, selection, action), key);
    } catch (reason) { session.setError(errorMessage(reason)); }
  }

  function changeSelection(next: BindingSelection) {
    setSelection(next);
    session.clearPreview();
  }

  // Dropping a picker entry on a control selects it and assigns the action's default parameters
  // to its primary gesture, as if it had been picked in the inspector. Like the inspector, the
  // encoder refuses flat bindings while dial functions exist. The notice names what changed,
  // with Undo, since a replaced binding is otherwise silent.
  function dropAction(module: ModuleId, control: number, type: PickerActionType) {
    if (!isValidForControl(type, module)) return;
    if (module === 'm131e' && profile.dial_functions) {
      session.setError('This profile uses dial functions, so encoder bindings are edited under Profiles · Dial functions.');
      return;
    }
    try {
      const found = dropTargetState(profile, keypadPath, module, control);
      session.edit(dropProfileEdit(profile, keypadPath, module, control, type), `drop:${type}:${module}:${control}`);
      recordRecentAction(pickerWebStorage, type);
      session.setNotice(dropNoticeText(found, type, controlLabel(module, control)), { label: 'Undo', run: () => session.undo() });
    } catch (reason) { session.setError(errorMessage(reason)); }
    chooseModule(module);
    changeSelection({ module, control, gesture: gestures[module][0], serial: '' });
  }

  function chooseModule(id: ModuleId) {
    setSelectedModule(id);
    session.clearPreview();
    setSelection({ module: id, control: 0, gesture: gestures[id][0], serial: '' });
    setSerialScope('any');
  }

  function selectBinding(item: Binding) {
    setSelectedModule(item.module);
    setSelection({ module: item.module, control: item.control, gesture: item.gesture, serial: item.serial_number ?? '' });
    setSerialScope(item.serial_number ? 'exact' : 'any');
    setPage('devices');
    session.clearPreview();
  }

  function navigateKeypad(path: string[]) {
    try {
      keypadPagesAtPath(profile, path);
      session.clearPreview();
      setKeypadPath(path);
      setSelection(previous => ({ ...previous, module: 'm4315', control: path.length > 1 ? 1 : 0, gesture: 'pressed', serial: '' }));
      setSelectedModule('m4315'); setSerialScope('any');
    } catch (reason) { session.setError(errorMessage(reason)); }
  }

  function selectKeypadBinding(pageId: string, control: number, item: KeypadBinding) {
    navigateKeypad(keypadPathToPage(profile, pageId));
    setSelection({ module: 'm4315', control, gesture: item.gesture, serial: item.serial_number ?? '' });
    setSerialScope(item.serial_number ? 'exact' : 'any');
    setPage('devices');
  }

  function removeKeypadBinding(pageId: string, control: number, item: KeypadBinding) {
    try {
      session.update(latest => editKeypadBinding(latest, keypadPathToPage(latest, pageId), control, item.gesture, item.serial_number ?? '', null, ''));
    } catch (reason) { session.setError(errorMessage(reason)); }
  }

  function createFolder() {
    try {
      const id = `page-${crypto.randomUUID()}`;
      session.edit(createKeypadFolder(profile, keypadPath, selection.control, id));
      setKeypadPath([...keypadPath, id]);
      setSelection(previous => ({ ...previous, control: 1, gesture: 'pressed', serial: '' })); setSerialScope('any');
    } catch (reason) { session.setError(errorMessage(reason)); }
  }

  function clearKey(control = selection.control, path = keypadPath) {
    const slot = keypadPagesAtPath(profile, path).at(-1)!.slots[control];
    if (!slot) return;
    const folder = slot.type === 'folder';
    confirm({
      title: folder ? 'Delete this folder?' : 'Clear this key?',
      description: folder ? 'This removes the folder and every key and folder inside it from the draft.' : 'This removes the key label and all its press, release and serial-specific bindings from the draft.',
      confirmLabel: 'Delete',
      run: () => {
        // Applied to the draft as it is when confirmed, not when the dialog opened.
        try { session.update(latest => clearKeypadSlot(latest, path, control)); setKeypadPath([...path]); setSelection(previous => ({ ...previous, control })); }
        catch (reason) { session.setError(errorMessage(reason)); }
      },
    });
  }

  function deleteCurrentFolder() {
    const parentPath = keypadBackPath(keypadPath);
    const parent = keypadPagesAtPath(profile, parentPath).at(-1)!;
    const control = parent.slots.findIndex(slot => slot?.type === 'folder' && slot.page_id === currentKeypadPage.id);
    if (control >= 0) clearKey(control, parentPath);
  }

  function changeFaderPosition(value: number) {
    setFaderPosition(value);
    session.clearPreview();
  }

  function changeSerialScope(scope: SerialScope) {
    setSerialScope(scope);
    changeSelection({ ...selection, serial: '' });
  }

  async function testKeypad(control = selection.control) {
    const result = await session.previewKeypad({ path: keypadPath, control, serial: selection.serial });
    if (!result) return;
    setKeypadPath(result.path);
    if (JSON.stringify(keypadPath) !== JSON.stringify(result.path)) setSelection(previous => ({ ...previous, control: result.depth ? 1 : 0 }));
  }

  async function testBinding(gesture = selection.gesture) {
    if (isKeypad) { await testKeypad(); return; }
    await session.previewBinding({ ...selection, gesture, faderPosition });
  }

  return {
    selectedModule, selected, selection, serialScope, faderPosition, keypadPath, keypadPages, currentKeypadPage,
    isKeypad, isBack, keypadSlot, folderPage, binding,
    mutateAction, changeSelection, chooseModule, selectBinding, navigateKeypad, selectKeypadBinding, removeKeypadBinding,
    createFolder, clearKey, deleteCurrentFolder, changeFaderPosition, changeSerialScope, testKeypad, testBinding, dropAction,
  };
}

export type DeviceEditor = ReturnType<typeof useDeviceEditor>;
