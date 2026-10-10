import type { Action, KeyArtwork, KeypadBinding, KeypadPage, KeypadSlot, Profile } from './contracts';

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

// A copied key: its slot (label and every binding), its artwork, and — for a folder —
// the folder's page and every page inside it, in folder order. Pure data, so it can be
// pasted any number of times, in any folder or profile. `type` mirrors the slot type.
export type KeyClipboard =
  | { type: 'action'; slot: { type: 'action'; label: string; bindings: KeypadBinding[] }; artwork?: KeyArtwork }
  | { type: 'folder'; slot: { type: 'folder'; page_id: string }; artwork?: KeyArtwork; pages: KeypadPage[] };

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
  const artwork = { ...current.page.artwork };
  if (slot === null) delete artwork[String(control)];
  return replacePage(current.profile, { ...current.page, slots, ...(current.page.artwork ? { artwork: Object.keys(artwork).length ? artwork : undefined } : {}) });
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

// The folder's page followed by every page inside it, parents before children. Throws on a
// broken tree, like the folder checks of the service do.
export function keypadFolderPages(profile: Profile, pageId: string): KeypadPage[] {
  const pages: KeypadPage[] = [];
  const visit = (id: string) => {
    if (id === keypadRoot || pages.some(page => page.id === id)) throw new Error('Invalid folder tree.');
    const page = profile.keypad!.pages.find(item => item.id === id);
    if (!page) throw new Error('This folder is unavailable.');
    pages.push(page);
    for (const child of page.slots) if (child?.type === 'folder') visit(child.page_id);
  };
  visit(pageId);
  return pages;
}

// Writes or removes one key's artwork on a page, dropping the record when the last entry goes.
function withArtwork(page: KeypadPage, control: number, artwork: KeyArtwork | undefined): KeypadPage {
  const record = { ...page.artwork };
  if (artwork) record[String(control)] = artwork; else delete record[String(control)];
  const next = { ...page };
  if (Object.keys(record).length) next.artwork = record; else delete next.artwork;
  return next;
}

function placedPage(page: KeypadPage, control: number, slot: KeypadSlot, artwork: KeyArtwork | undefined): KeypadPage {
  return withArtwork({ ...page, slots: page.slots.map((item, at) => at === control ? slot : item) }, control, artwork ?? undefined);
}

// Deep-copies the slot on a key: its label and bindings, its artwork and, for a folder, the
// whole folder subtree. Throws for an empty key.
export function copyKeypadSlot(profile: Profile, path: readonly string[], control: number): KeyClipboard {
  const current = editablePage(profile, path, control);
  const slot = current.page.slots[control];
  if (!slot) throw new Error('This key is empty. There is nothing to copy.');
  const artwork = current.page.artwork?.[String(control)];
  if (slot.type !== 'folder') return { type: 'action', slot: structuredClone(slot), ...(artwork ? { artwork: structuredClone(artwork) } : {}) };
  return { type: 'folder', slot: { type: 'folder', page_id: slot.page_id }, ...(artwork ? { artwork: structuredClone(artwork) } : {}), pages: structuredClone(keypadFolderPages(current.profile, slot.page_id)) };
}

// Depth of a folder clipboard: how many folder levels its deepest page sits below the pasted
// folder itself. The pages arrive parents before children, so one pass is enough.
function folderDepth(pages: readonly KeypadPage[]): number {
  const depth = new Map<string, number>([[pages[0].id, 0]]);
  for (const page of pages) {
    const base = depth.get(page.id) ?? 0;
    for (const slot of page.slots) if (slot?.type === 'folder') depth.set(slot.page_id, base + 1);
  }
  return Math.max(...depth.values());
}

// Writes a copied key onto an empty key: the slot and artwork, and for a folder the subtree with
// fresh page ids (the source folder keeps existing). Depth and page limits refuse with the
// folder messages, an occupied key refuses instead of overwriting.
export function pasteKeypadSlot(profile: Profile, path: readonly string[], control: number, clipboard: KeyClipboard, newId: () => string): Profile {
  const current = editablePage(profile, path, control);
  if (current.page.slots[control]) throw new Error('This key already holds content. Clear it before pasting.');
  if (clipboard.type === 'folder') {
    const pages = clipboard.pages;
    if (keypadDepth(path) + 1 + folderDepth(pages) > keypadMaximumDepth) throw new Error('Folders can contain up to eight levels.');
    if (current.profile.keypad!.pages.length + pages.length > keypadMaximumPages) throw new Error('This profile has reached its 64-page limit.');
    const idFor = new Map(pages.map(page => [page.id, newId()]));
    const slot: KeypadSlot = { type: 'folder', page_id: idFor.get(clipboard.slot.page_id)! };
    const grown = pages.map(page => ({
      ...structuredClone(page),
      id: idFor.get(page.id)!,
      slots: page.slots.map(child => child?.type === 'folder' ? { ...child, page_id: idFor.get(child.page_id)! } : child),
    }));
    const edited = replacePage(current.profile, placedPage(current.page, control, slot, clipboard.artwork));
    return { ...edited, keypad: { pages: [...edited.keypad!.pages, ...grown] } };
  }
  return replacePage(current.profile, placedPage(current.page, control, structuredClone(clipboard.slot), clipboard.artwork));
}

// Moves a key to another key of any page: onto an empty key it moves (artwork follows), onto an
// occupied one it swaps. A folder refuses to move into its own subtree.
export function moveKeypadSlot(profile: Profile, fromPath: readonly string[], fromControl: number, toPath: readonly string[], toControl: number): Profile {
  const sameKey = fromControl === toControl && fromPath.length === toPath.length && fromPath.every((id, at) => id === toPath[at]);
  if (sameKey) return profile;
  const source = editablePage(profile, fromPath, fromControl);
  const target = editablePage(source.profile, toPath, toControl);
  const moved = source.page.slots[fromControl];
  if (!moved) throw new Error('This key is empty. There is nothing to move.');
  if (moved.type === 'folder' && keypadFolderPages(target.profile, moved.page_id).some(page => page.id === target.page.id)) throw new Error('A folder cannot be moved inside itself.');
  const swapped = target.page.slots[toControl] ?? null;
  const movedArtwork = source.page.artwork?.[String(fromControl)];
  const swappedArtwork = swapped ? target.page.artwork?.[String(toControl)] : undefined;
  if (source.page.id === target.page.id) {
    const page = placedPage(placedPage(source.page, fromControl, swapped, swapped ? swappedArtwork : undefined), toControl, moved, movedArtwork);
    return replacePage(source.profile, page);
  }
  const afterSource = replacePage(source.profile, placedPage(source.page, fromControl, swapped, swapped ? swappedArtwork : undefined));
  const afterTarget = replacePage(afterSource, placedPage(keypadPagesAtPath(afterSource, toPath).at(-1)!, toControl, moved, movedArtwork));
  return afterTarget;
}

export function clearKeypadSlot(profile: Profile, path: readonly string[], control: number): Profile {
  const current = editablePage(profile, path, control);
  const slot = current.page.slots[control];
  if (slot?.type !== 'folder') return setKeypadSlot(current.profile, path, control, null);
  const removed = new Set(keypadFolderPages(current.profile, slot.page_id).map(page => page.id));
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
