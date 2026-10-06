import { describe, expect, it } from 'vitest';
import type { Binding, Profile } from './contracts';
import { bindingWarnings, entryOnControl, filterBindings, listBindings, warningsForControl } from './binding-model';
import { editKeypadBinding, keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { browserProfile } from './ui-model';

const mute = { type: 'toggle_mute' } as const;
const dial = (gesture: Binding['gesture'], serial?: string): Binding => ({ module: 'm131e', control: 0, gesture, action: mute, ...(serial ? { serial_number: serial } : {}) });
const profileWith = (...bindings: Binding[]): Profile => upgradeKeypadProfile({ schema_version: 1, name: 'P', bindings });
const keys = (...edits: [number, 'pressed' | 'released', string, string?][]) => edits.reduce((profile, [control, gesture, label, serial]) =>
  editKeypadBinding(profile, [keypadRoot], control, gesture, serial ?? '', mute, label), upgradeKeypadProfile({ schema_version: 1, name: 'K', bindings: [] }));

describe('listBindings and filterBindings', () => {
  const profile = editKeypadBinding(profileWith(
    { module: 'm003k', control: 1, gesture: 'clockwise', action: { type: 'volume', delta: 5 } },
    { module: 'm131e', control: 0, gesture: 'long_press', action: { type: 'launch_app', bundle_id: 'com.apple.finder' }, serial_number: 'SN-42' },
  ), [keypadRoot], 4, 'pressed', '', { type: 'toggle_mute' }, 'Mute mic');
  const entries = listBindings(profile);

  it('lists flat and keypad bindings', () => {
    expect(entries.map(entry => entry.title)).toEqual(['Knob 2 · Clockwise', 'Encoder · Long press', 'Root · Key 5 · Press']);
    expect(entries.map(entry => entry.id)).toEqual(['flat:0', 'flat:1', 'keypad:root:4:0']);
  });

  const find = (query: string) => filterBindings(entries, query).map(entry => entry.id);
  it('matches module name, control, gesture, action, key label, page name and serial case-insensitively', () => {
    expect(find('3x knob')).toEqual(['flat:0']);
    expect(find('KNOB 2')).toEqual(['flat:0']);
    expect(find('encoder')).toEqual(['flat:1']);
    expect(find('long press')).toEqual(['flat:1']);
    expect(find('finder')).toEqual(['flat:1']);
    expect(find('output volume')).toEqual(['flat:0']);
    expect(find('mute mic')).toEqual(['keypad:root:4:0']);
    expect(find('root')).toEqual(['keypad:root:4:0']);
    expect(find('key 5')).toEqual(['keypad:root:4:0']);
    expect(find('sn-42')).toEqual(['flat:1']);
  });

  it('requires every word, trims, and keeps everything for an empty query', () => {
    expect(find('encoder finder')).toEqual(['flat:1']);
    expect(find('encoder volume')).toEqual([]);
    expect(find('   ')).toHaveLength(3);
    expect(find('')).toHaveLength(3);
    expect(find('nothing like this')).toEqual([]);
  });
});

describe('bindingWarnings', () => {
  it('has no warnings for valid profiles, including the browser profile', () => {
    expect(bindingWarnings(upgradeKeypadProfile(browserProfile))).toEqual([]);
    expect(bindingWarnings(profileWith())).toEqual([]);
    // Press + release on the dial fire at different moments; short press alone is fine.
    expect(bindingWarnings(profileWith(dial('pressed'), dial('released'), dial('right'), dial('left')))).toEqual([]);
    expect(bindingWarnings(profileWith(dial('short_press'), dial('long_press'), dial('pressed')))).toEqual([]);
  });

  it('warns when released coexists with short or long press for the same control and serial scope', () => {
    const short = bindingWarnings(profileWith(dial('released'), dial('short_press')));
    expect(short).toHaveLength(1);
    expect(short[0]).toMatchObject({ kind: 'release-overlap', entryIds: ['flat:0', 'flat:1'] });
    expect(short[0].message).toContain('Encoder');
    expect(short[0].message).toContain('Release');
    expect(short[0].message).toContain('Short press');
    const both = bindingWarnings(profileWith(dial('long_press'), dial('released'), dial('short_press')));
    expect(both.map(warning => warning.entryIds)).toEqual([['flat:1', 'flat:0'], ['flat:1', 'flat:2']]);
    expect(both[0].message).toContain('Long press');
  });

  it('scopes the overlap rule by serial', () => {
    expect(bindingWarnings(profileWith(dial('released', 'A'), dial('short_press', 'B')))).toEqual([]);
    expect(bindingWarnings(profileWith(dial('released', 'A'), dial('short_press', 'A')))).toHaveLength(1);
    expect(bindingWarnings(profileWith(dial('released', 'A'), dial('short_press', 'A')))[0].message).toContain('serial A');
  });

  it('warns about identical selectors but not about a serial next to an any-serial binding', () => {
    const duplicate = bindingWarnings(profileWith(dial('right'), dial('right')));
    expect(duplicate).toHaveLength(1);
    expect(duplicate[0]).toMatchObject({ kind: 'duplicate-selector', entryIds: ['flat:0', 'flat:1'] });
    expect(bindingWarnings(profileWith(dial('right'), dial('right', 'A')))).toEqual([]);
    expect(bindingWarnings(profileWith(dial('right', 'A'), dial('right', 'B')))).toEqual([]);
    expect(bindingWarnings(profileWith(
      { module: 'm003k', control: 0, gesture: 'pressed', action: mute }, { module: 'm003k', control: 1, gesture: 'pressed', action: mute }))).toEqual([]);
  });

  it('allows keypad pressed + released on one slot and serial plus any-serial bindings, but flags a duplicate selector', () => {
    expect(bindingWarnings(keys([2, 'pressed', 'A'], [2, 'released', 'A']))).toEqual([]);
    expect(bindingWarnings(keys([2, 'pressed', 'A'], [2, 'pressed', 'A', 'SN1']))).toEqual([]);
    const duplicated = keys([2, 'pressed', 'A']);
    const slot = duplicated.keypad!.pages[0].slots[2]!;
    if (slot.type === 'action') slot.bindings.push({ gesture: 'pressed', action: mute });
    const warnings = bindingWarnings(duplicated);
    expect(warnings).toHaveLength(1);
    expect(warnings[0].kind).toBe('duplicate-selector');
    expect(warnings[0].message).toContain('Root · Key 3 · Press');
  });

  it('finds the warnings that involve a selected control', () => {
    const profile = profileWith(dial('released'), dial('short_press'), { module: 'm003k', control: 0, gesture: 'pressed', action: mute });
    const entries = listBindings(profile);
    const warnings = bindingWarnings(profile);
    expect(warningsForControl(warnings, entries, { module: 'm131e', control: 0 })).toHaveLength(1);
    expect(warningsForControl(warnings, entries, { module: 'm003k', control: 0 })).toEqual([]);
    expect(entryOnControl(entries[2], { module: 'm003k', control: 0 })).toBe(true);
    expect(entryOnControl(entries[2], { module: 'm003k', control: 1 })).toBe(false);
    const keypad = keys([2, 'pressed', 'A']);
    const keyEntries = listBindings(keypad);
    expect(entryOnControl(keyEntries[0], { module: 'm4315', control: 2, keypadPageId: 'root' })).toBe(true);
    expect(entryOnControl(keyEntries[0], { module: 'm4315', control: 2, keypadPageId: 'other' })).toBe(false);
  });
});
