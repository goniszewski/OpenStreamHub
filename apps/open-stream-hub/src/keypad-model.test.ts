import { describe, expect, it } from 'vitest';
import type { KeyArtwork, Profile } from './contracts';
import { ProfileSession } from './profile-session';
import { clearKeypadSlot, copyKeypadSlot, createKeypadFolder, editKeypadBinding, emptyKeypadPage, keypadBackPath, keypadBindingAt, keypadFolderPages, keypadPagesAtPath, keypadPathToPage, moveKeypadSlot, pasteKeypadSlot, profileBindingCount, renameKeypadPage, upgradeKeypadProfile } from './keypad-model';

const legacy: Profile = { schema_version: 1, name: 'Desk', bindings: [
  { module: 'm4315', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
  { module: 'm4315', control: 0, gesture: 'released', serial_number: 'Desk', action: { type: 'volume', delta: 5 } },
  { module: 'm003k', control: 0, gesture: 'clockwise', action: { type: 'volume', delta: 5 } },
] };

describe('keypad folders', () => {
  it('migrates all key selectors without changing source or other module bindings', () => {
    const before = structuredClone(legacy), profile = upgradeKeypadProfile(legacy);
    expect(legacy).toEqual(before);
    expect(profile.bindings).toEqual([legacy.bindings[2]]);
    expect(profileBindingCount(profile)).toBe(3);
    expect(keypadBindingAt(profile, ['root'], 0, 'released', 'Desk')?.action).toEqual({ type: 'volume', delta: 5 });
    expect(profile.keypad!.pages[0].slots).toHaveLength(15);
  });
  it('rejects fractional wire controls during migration', () => {
    expect(() => upgradeKeypadProfile({ ...legacy, bindings: [{ ...legacy.bindings[0], control: 0.5 }] })).toThrow();
  });
  it('creates nested folders and reserves Back only below root', () => {
    let profile = createKeypadFolder(upgradeKeypadProfile(legacy), ['root'], 1, 'tools', 'Tools');
    profile = createKeypadFolder(profile, ['root', 'tools'], 1, 'git', 'Git');
    expect(keypadPathToPage(profile, 'git')).toEqual(['root', 'tools', 'git']);
    expect(keypadBackPath(['root', 'tools', 'git'])).toEqual(['root', 'tools']);
    expect(() => editKeypadBinding(profile, ['root', 'tools'], 0, 'pressed', '', { type: 'toggle_mute' }, 'Mute')).toThrow('reserved');
    expect(() => keypadPagesAtPath(profile, ['root', 'git'])).toThrow();
  });
  it('rejects invalid paths and stops at eight levels', () => {
    let profile = upgradeKeypadProfile(legacy), path = ['root'];
    for (let depth = 1; depth <= 8; depth++) { profile = createKeypadFolder(profile, path, 1, `level${depth}`); path = [...path, `level${depth}`]; }
    expect(() => createKeypadFolder(profile, path, 1, 'too-deep')).toThrow('eight');
    expect(() => keypadPagesAtPath(profile, ['root', 'level2'])).toThrow();
    expect(() => keypadPagesAtPath(profile, ['root', 'root'])).toThrow();
  });
  it('deletes the entire subtree while retaining unrelated actions', () => {
    let profile = createKeypadFolder(upgradeKeypadProfile(legacy), ['root'], 1, 'tools');
    profile = createKeypadFolder(profile, ['root', 'tools'], 2, 'git');
    const next = clearKeypadSlot(profile, ['root'], 1);
    expect(next.keypad!.pages.map(page => page.id)).toEqual(['root']);
    expect(profile.keypad!.pages).toHaveLength(3);
    expect(profileBindingCount(next)).toBe(3);
  });
  it('edits just one selector and renames a page without altering its keys', () => {
    const profile = upgradeKeypadProfile(legacy);
    const next = editKeypadBinding(profile, ['root'], 0, 'pressed', '', { type: 'volume', delta: 10 }, 'Volume');
    expect(keypadBindingAt(next, ['root'], 0, 'released', 'Desk')).toEqual(keypadBindingAt(profile, ['root'], 0, 'released', 'Desk'));
    expect(renameKeypadPage(next, ['root'], 'Home').keypad!.pages[0].slots).toEqual(next.keypad!.pages[0].slots);
  });
});

const frame = (mark: string): KeyArtwork => ({ frames: [{ jpeg_base64: mark, delay_ms: 100 }], loop_count: 0 });
const artworkOf = (profile: Profile, control: number) => profile.keypad!.pages[0].artwork?.[String(control)];
const counterIds = () => { let n = 0; return () => `copy-${++n}`; };

describe('copy and paste keys', () => {
  it('copies a key with its bindings and artwork, and pastes onto an empty key', () => {
    let profile = upgradeKeypadProfile(legacy);
    profile = { ...profile, keypad: { pages: [{ ...profile.keypad!.pages[0], artwork: { '0': frame('zero') } }] } };
    const clipboard = copyKeypadSlot(profile, ['root'], 0);
    expect(clipboard.type).toBe('action');
    const next = pasteKeypadSlot(profile, ['root'], 4, clipboard, counterIds());
    expect(keypadBindingAt(next, ['root'], 4, 'released', 'Desk')?.action).toEqual({ type: 'volume', delta: 5 });
    expect(next.keypad!.pages[0].slots[4]).toEqual(profile.keypad!.pages[0].slots[0]);
    expect(artworkOf(next, 4)).toEqual(frame('zero'));
    expect(artworkOf(next, 0)).toEqual(frame('zero'));
    // The clipboard is pure data: pasting again works unchanged.
    expect(keypadBindingAt(pasteKeypadSlot(profile, ['root'], 6, clipboard, counterIds()), ['root'], 6, 'pressed', '')?.action).toEqual({ type: 'toggle_mute' });
  });

  it('refuses pasting onto an occupied key and copying an empty one', () => {
    const profile = upgradeKeypadProfile(legacy);
    const clipboard = copyKeypadSlot(profile, ['root'], 0);
    expect(() => pasteKeypadSlot(profile, ['root'], 0, clipboard, counterIds())).toThrow('already holds content');
    expect(() => copyKeypadSlot(profile, ['root'], 1)).toThrow('nothing to copy');
  });

  it('copies a folder with its whole subtree and pastes it under fresh page ids', () => {
    let profile = createKeypadFolder(upgradeKeypadProfile(legacy), ['root'], 2, 'fx', 'FX');
    profile = createKeypadFolder(profile, ['root', 'fx'], 3, 'inner', 'Inner');
    const clipboard = copyKeypadSlot(profile, ['root'], 2);
    expect(clipboard.type).toBe('folder');
    const next = pasteKeypadSlot(profile, ['root'], 5, clipboard, counterIds());
    const ids = next.keypad!.pages.map(page => page.id);
    expect(ids).toHaveLength(5);
    expect(new Set(ids).size).toBe(5);
    expect(next.keypad!.pages[0].slots[5]).toEqual({ type: 'folder', page_id: 'copy-1' });
    const pasted = next.keypad!.pages.find(page => page.id === 'copy-1')!;
    expect(pasted.name).toBe('FX');
    expect(pasted.slots[3]).toEqual({ type: 'folder', page_id: 'copy-2' });
    expect(next.keypad!.pages.find(page => page.id === 'copy-2')!.name).toBe('Inner');
    // The source subtree is untouched and still resolvable.
    expect(keypadPagesAtPath(next, ['root', 'fx', 'inner']).at(-1)!.name).toBe('Inner');
  });

  it('refuses a paste deeper than eight levels with the folder message', () => {
    let profile = upgradeKeypadProfile(legacy), path = ['root'];
    for (let depth = 1; depth <= 7; depth++) { profile = createKeypadFolder(profile, path, 2, `l${depth}`); path = [...path, `l${depth}`]; }
    const clipboard = copyKeypadSlot(profile, ['root'], 2);
    // Seven nested folders reach exactly the limit at the root; one level lower they would not fit.
    expect(() => pasteKeypadSlot(profile, path, 3, clipboard, counterIds())).toThrow('eight');
    expect(keypadPagesAtPath(pasteKeypadSlot(profile, ['root'], 3, clipboard, counterIds()), ['root', 'copy-1', 'copy-2', 'copy-3', 'copy-4', 'copy-5', 'copy-6', 'copy-7'])).toHaveLength(8);
  });

  it('refuses a paste beyond the 64-page limit with the page message', () => {
    const pages = [emptyKeypadPage()];
    for (let n = 1; n < 64; n++) pages.push(emptyKeypadPage(`p${n}`, `P${n}`));
    const profile: Profile = { schema_version: 2, name: 'Full', bindings: [], keypad: { pages } };
    const donor = upgradeKeypadProfile(legacy);
    const clipboard = copyKeypadSlot(createKeypadFolder(donor, ['root'], 2, 'one'), ['root'], 2);
    expect(() => pasteKeypadSlot(profile, ['root'], 4, clipboard, counterIds())).toThrow('64-page limit');
  });

  it('lists a folder subtree in folder order and refuses a broken tree', () => {
    let profile = createKeypadFolder(upgradeKeypadProfile(legacy), ['root'], 2, 'fx', 'FX');
    profile = createKeypadFolder(profile, ['root', 'fx'], 3, 'inner', 'Inner');
    expect(keypadFolderPages(profile, 'fx').map(page => page.id)).toEqual(['fx', 'inner']);
    expect(() => keypadFolderPages(profile, 'root')).toThrow('Invalid folder tree.');
    expect(() => keypadFolderPages(profile, 'gone')).toThrow('unavailable');
  });
});

describe('move and swap keys', () => {
  const withArt: Profile = { ...upgradeKeypadProfile(legacy), keypad: { pages: [{ ...emptyKeypadPage(), slots: emptyKeypadPage().slots.map((_, at) => at === 0 || at === 1 ? { type: 'action', label: `Key ${at + 1}`, bindings: [] } : null), artwork: { '0': frame('zero'), '1': frame('one') } }] } };

  it('moves a key and its artwork onto an empty key, in one page or across pages', () => {
    const next = moveKeypadSlot(withArt, ['root'], 0, ['root'], 4);
    expect(next.keypad!.pages[0].slots[4]).toMatchObject({ label: 'Key 1' });
    expect(next.keypad!.pages[0].slots[0]).toBeNull();
    expect(artworkOf(next, 4)).toEqual(frame('zero'));
    expect(artworkOf(next, 0)).toBeUndefined();
    let profile = createKeypadFolder(withArt, ['root'], 6, 'fx', 'FX');
    const across = moveKeypadSlot(profile, ['root'], 1, ['root', 'fx'], 2);
    expect(keypadPagesAtPath(across, ['root', 'fx']).at(-1)!.slots[2]).toMatchObject({ label: 'Key 2' });
    expect(across.keypad!.pages[0].slots[1]).toBeNull();
    expect(keypadPagesAtPath(across, ['root', 'fx']).at(-1)!.artwork?.['2']).toEqual(frame('one'));
  });

  it('swaps two occupied keys including their artwork', () => {
    const next = moveKeypadSlot(withArt, ['root'], 0, ['root'], 1);
    expect(next.keypad!.pages[0].slots[0]).toMatchObject({ label: 'Key 2' });
    expect(next.keypad!.pages[0].slots[1]).toMatchObject({ label: 'Key 1' });
    expect(artworkOf(next, 0)).toEqual(frame('one'));
    expect(artworkOf(next, 1)).toEqual(frame('zero'));
  });

  it('returns the same profile for a drop on the same key and refuses empty or nested moves', () => {
    expect(moveKeypadSlot(withArt, ['root'], 0, ['root'], 0)).toBe(withArt);
    expect(() => moveKeypadSlot(withArt, ['root'], 4, ['root'], 5)).toThrow('nothing to move');
    const folder = createKeypadFolder(withArt, ['root'], 6, 'fx', 'FX');
    expect(() => moveKeypadSlot(folder, ['root'], 6, ['root', 'fx'], 4)).toThrow('inside itself');
    // A sibling folder swap is fine.
    const two = createKeypadFolder(folder, ['root'], 7, 'db', 'DB');
    const swapped = moveKeypadSlot(two, ['root'], 6, ['root'], 7);
    expect(swapped.keypad!.pages[0].slots[6]).toEqual({ type: 'folder', page_id: 'db' });
    expect(swapped.keypad!.pages[0].slots[7]).toEqual({ type: 'folder', page_id: 'fx' });
  });
});

describe('key operations go through the edit history', () => {
  it('undo and redo restore a paste and a move', () => {
    const session = new ProfileSession(() => new Promise(() => {}), upgradeKeypadProfile(legacy), false);
    const clipboard = copyKeypadSlot(session.getState().draft.profile, ['root'], 0);
    session.update(latest => pasteKeypadSlot(latest, ['root'], 4, clipboard, counterIds()));
    expect(keypadBindingAt(session.getState().draft.profile, ['root'], 4, 'pressed', '')).toBeTruthy();
    expect(session.undo()).toBe(true);
    expect(keypadBindingAt(session.getState().draft.profile, ['root'], 4, 'pressed', '')).toBeUndefined();
    expect(session.redo()).toBe(true);
    expect(keypadBindingAt(session.getState().draft.profile, ['root'], 4, 'pressed', '')).toBeTruthy();
    session.update(latest => moveKeypadSlot(latest, ['root'], 4, ['root'], 6));
    expect(keypadBindingAt(session.getState().draft.profile, ['root'], 6, 'pressed', '')).toBeTruthy();
    expect(session.undo()).toBe(true);
    expect(keypadBindingAt(session.getState().draft.profile, ['root'], 6, 'pressed', '')).toBeUndefined();
    expect(keypadBindingAt(session.getState().draft.profile, ['root'], 4, 'pressed', '')).toBeTruthy();
  });
});

describe('profile schema upgrade', () => {
  it('raises a schema-2 profile to the current schema and leaves a current one alone', async () => {
    const { upgradeKeypadProfile, emptyKeypadPage } = await import('./keypad-model');
    const { currentSchema } = await import('./contracts');
    const pages = { schema_version: 2, name: 'Old', bindings: [], keypad: { pages: [emptyKeypadPage()] } };
    const upgraded = upgradeKeypadProfile(pages);
    expect(upgraded.schema_version).toBe(currentSchema);
    expect(upgraded.keypad).toEqual(pages.keypad);
    expect(upgradeKeypadProfile(upgraded)).toBe(upgraded);
    expect(upgradeKeypadProfile({ schema_version: 1, name: 'Flat', bindings: [] }).schema_version).toBe(currentSchema);
  });

  it('names a profile from a newer version instead of calling it unsupported', async () => {
    const { upgradeKeypadProfile } = await import('./keypad-model');
    expect(() => upgradeKeypadProfile({ schema_version: 5, name: 'x', bindings: [] })).toThrow(/newer version/);
  });
});
