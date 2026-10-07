import type { Action, KeypadBinding, KeypadPage, KeypadSlot, Profile } from './contracts';

export const keypadRoot = 'root';
export const keypadSlotCount = 15;
export const keypadMaximumDepth = 8;
export const keypadMaximumPages = 64;

export function emptyKeypadPage(id = keypadRoot, name = 'Root'): KeypadPage {
  return { id, name, slots: Array.from({ length: keypadSlotCount }, () => null) };
}

// Migration only: action resolution, serial precedence and execution stay in Rust.
export function upgradeKeypadProfile(profile: Profile): Profile {
  if (profile.schema_version === 2) {
    if (!profile.keypad) throw new Error('This profile is missing its keypad pages.');
    return profile;
  }
  if (profile.schema_version !== 1) throw new Error('Unsupported profile version.');
  const root = emptyKeypadPage();
  for (const binding of profile.bindings.filter(item => item.module === 'm4315')) {
    if (!Number.isInteger(binding.control) || binding.control < 0 || binding.control >= keypadSlotCount || !['pressed', 'released'].includes(binding.gesture) || binding.action.type === 'volume_from_position') {
      throw new Error('The profile contains an unsupported keypad binding.');
    }
    let slot = root.slots[binding.control];
    if (!slot) root.slots[binding.control] = slot = { type: 'action', label: `Key ${binding.control + 1}`, bindings: [] };
    if (slot.type !== 'action') throw new Error('Invalid keypad migration.');
    slot.bindings.push({ gesture: binding.gesture as KeypadBinding['gesture'], ...(binding.serial_number === undefined ? {} : { serial_number: binding.serial_number }), action: structuredClone(binding.action) });
  }
  return { ...profile, schema_version: 2, bindings: profile.bindings.filter(item => item.module !== 'm4315'), keypad: { pages: [root] } };
}

export function keypadPagesAtPath(profile: Profile, path: readonly string[]): KeypadPage[] {
  const upgraded = upgradeKeypadProfile(profile);
  if (path[0] !== keypadRoot || path.length > keypadMaximumDepth + 1 || new Set(path).size !== path.length) throw new Error('Invalid keypad folder path.');
  const pages: KeypadPage[] = [];
  for (const id of path) {
    const page = upgraded.keypad!.pages.find(item => item.id === id);
    if (!page || pages.length && !pages.at(-1)!.slots.some(slot => slot?.type === 'folder' && slot.page_id === id)) throw new Error('This keypad folder is unavailable.');
    pages.push(page);
  }
  return pages;
}

export function keypadDepth(path: readonly string[]) { return path.length - 1; }
export function keypadBackPath(path: readonly string[]) { return path.length > 1 ? path.slice(0, -1) : [...path]; }
export function keypadIsBack(path: readonly string[], control: number) { return path.length > 1 && control === 0; }

function editablePage(profile: Profile, path: readonly string[], control?: number) {
  const upgraded = upgradeKeypadProfile(profile);
  const page = keypadPagesAtPath(upgraded, path).at(-1)!;
  if (control !== undefined && (!Number.isInteger(control) || control < 0 || control >= keypadSlotCount || keypadIsBack(path, control))) throw new Error('The Back key is reserved on folder pages.');
  return { profile: upgraded, page };
}

function replacePage(profile: Profile, page: KeypadPage): Profile {
  return { ...profile, keypad: { pages: profile.keypad!.pages.map(item => item.id === page.id ? page : item) } };
}

export function setKeypadSlot(profile: Profile, path: readonly string[], control: number, slot: KeypadSlot): Profile {
  const current = editablePage(profile, path, control);
  const slots = [...current.page.slots];
  slots[control] = slot;
  return replacePage(current.profile, { ...current.page, slots });
}

export function createKeypadFolder(profile: Profile, path: readonly string[], control: number, id: string, name = 'New folder'): Profile {
  const current = editablePage(profile, path, control);
  if (keypadDepth(path) >= keypadMaximumDepth) throw new Error('Folders can contain up to eight levels.');
  if (current.profile.keypad!.pages.length >= keypadMaximumPages) throw new Error('This profile has reached its 64-page limit.');
  if (current.page.slots[control]) throw new Error('Clear this key before creating a folder.');
  if (!id || current.profile.keypad!.pages.some(page => page.id === id)) throw new Error('The new folder ID must be unique.');
  const edited = setKeypadSlot(current.profile, path, control, { type: 'folder', page_id: id });
  return { ...edited, keypad: { pages: [...edited.keypad!.pages, emptyKeypadPage(id, name)] } };
}

export function renameKeypadPage(profile: Profile, path: readonly string[], name: string): Profile {
  const current = editablePage(profile, path);
  return replacePage(current.profile, { ...current.page, name });
}

export function keypadBindingAt(profile: Profile, path: readonly string[], control: number, gesture: KeypadBinding['gesture'], serial: string): KeypadBinding | undefined {
  if (keypadIsBack(path, control)) return undefined;
  const page = keypadPagesAtPath(profile, path).at(-1)!;
  const slot = page.slots[control];
  return slot?.type === 'action' ? slot.bindings.find(binding => binding.gesture === gesture && (binding.serial_number ?? '') === serial) : undefined;
}

export function editKeypadBinding(profile: Profile, path: readonly string[], control: number, gesture: KeypadBinding['gesture'], serial: string, action: Action | null, defaultLabel: string): Profile {
  const current = editablePage(profile, path, control);
  const slot = current.page.slots[control];
  if (slot?.type === 'folder') throw new Error('Open the folder to configure its keys.');
  const bindings = slot?.type === 'action' ? [...slot.bindings] : [];
  const index = bindings.findIndex(item => item.gesture === gesture && (item.serial_number ?? '') === serial);
  if (action) {
    const binding: KeypadBinding = { gesture, ...(serial ? { serial_number: serial } : {}), action };
    if (index < 0) bindings.push(binding); else bindings[index] = binding;
  } else if (index >= 0) bindings.splice(index, 1);
  return setKeypadSlot(current.profile, path, control, bindings.length || slot ? { type: 'action', label: slot?.type === 'action' ? slot.label : defaultLabel, bindings } : null);
}

export function renameKeypadAction(profile: Profile, path: readonly string[], control: number, label: string): Profile {
  const current = editablePage(profile, path, control);
  const slot = current.page.slots[control];
  if (slot?.type !== 'action') throw new Error('Choose an action before naming this key.');
  return setKeypadSlot(current.profile, path, control, { ...slot, label });
}

export function clearKeypadSlot(profile: Profile, path: readonly string[], control: number): Profile {
  const current = editablePage(profile, path, control);
  const slot = current.page.slots[control];
  if (slot?.type !== 'folder') return setKeypadSlot(current.profile, path, control, null);
  const removed = new Set<string>();
  const visit = (id: string) => {
    if (id === keypadRoot || removed.has(id)) throw new Error('Invalid folder tree.');
    const page = current.profile.keypad!.pages.find(item => item.id === id);
    if (!page) throw new Error('This folder is unavailable.');
    removed.add(id);
    for (const child of page.slots) if (child?.type === 'folder') visit(child.page_id);
  };
  visit(slot.page_id);
  const cleared = setKeypadSlot(current.profile, path, control, null);
  return { ...cleared, keypad: { pages: cleared.keypad!.pages.filter(page => !removed.has(page.id)) } };
}

export function keypadPathToPage(profile: Profile, target: string): string[] {
  const pages = upgradeKeypadProfile(profile).keypad!.pages;
  const visit = (path: string[]): string[] | undefined => {
    const id = path.at(-1)!;
    if (path.length > keypadMaximumDepth + 1) return undefined;
    if (id === target) return path;
    const page = pages.find(item => item.id === id);
    if (!page) return undefined;
    for (const slot of page.slots) if (slot?.type === 'folder' && !path.includes(slot.page_id)) {
      const found = visit([...path, slot.page_id]); if (found) return found;
    }
    return undefined;
  };
  const found = visit([keypadRoot]);
  if (!found) throw new Error('This keypad folder is unavailable.');
  return found;
}

// Mirrors Profile::binding_count: flat, keypad and dial function bindings.
export function profileBindingCount(profile: Profile) {
  return profile.bindings.length + (profile.dial_functions ?? []).reduce((total, item) => total + item.bindings.length, 0) + (profile.keypad?.pages.reduce((total, page) => total + page.slots.reduce((count, slot) => count + (slot?.type === 'action' ? slot.bindings.length : 0), 0), 0) ?? 0);
}
