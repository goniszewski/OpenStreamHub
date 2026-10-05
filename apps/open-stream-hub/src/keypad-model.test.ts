import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { clearKeypadSlot, createKeypadFolder, editKeypadBinding, keypadBackPath, keypadBindingAt, keypadPagesAtPath, keypadPathToPage, profileBindingCount, renameKeypadPage, upgradeKeypadProfile } from './keypad-model';

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
