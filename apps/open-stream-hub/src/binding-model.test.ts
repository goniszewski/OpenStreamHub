import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Binding, DialFunction, Profile } from './contracts';
import { bindingWarnings, dialEditorGestures, editorGestures, entryOnControl, filterBindings, listBindings, warningsForControl } from './binding-model';
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
    expect(bindingWarnings(profileWith(dial('released', 'A'), dial('long_press', 'B')))).toEqual([]);
    expect(bindingWarnings(profileWith(dial('released', 'A'), dial('short_press', 'A')))).toHaveLength(1);
    expect(bindingWarnings(profileWith(dial('released', 'A'), dial('short_press', 'A')))[0].message).toContain('serial A');
  });

  it('warns when an any-serial binding overlaps a serial-specific one and names the serial', () => {
    const anyRelease = bindingWarnings(profileWith(dial('released'), dial('short_press', 'A')));
    expect(anyRelease).toHaveLength(1);
    expect(anyRelease[0]).toMatchObject({ kind: 'release-overlap', entryIds: ['flat:0', 'flat:1'] });
    expect(anyRelease[0].message).toContain('serial A');
    const anyPress = bindingWarnings(profileWith(dial('released', 'B'), dial('long_press')));
    expect(anyPress).toHaveLength(1);
    expect(anyPress[0].message).toContain('Long press');
    expect(anyPress[0].message).toContain('serial B');
    expect(bindingWarnings(profileWith(dial('released'), dial('short_press')))[0].message).not.toContain('serial');
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

describe('dial functions in listBindings and bindingWarnings', () => {
  const fixture = JSON.parse(readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/dial-functions.json', import.meta.url), 'utf8')) as Profile;
  const withFunctions = (functions: DialFunction[], ...bindings: Binding[]): Profile => ({ ...profileWith(...bindings), dial_functions: functions });

  it('lists function bindings under their function after the flat bindings', () => {
    const entries = listBindings(fixture);
    const dialEntries = entries.filter(entry => entry.dialFunction);
    expect(entries[0].id).toBe('flat:0');
    expect(dialEntries.map(entry => entry.id)).toEqual(['dial:0:0', 'dial:0:1', 'dial:0:2', 'dial:0:3', 'dial:1:0', 'dial:1:1', 'dial:2:0']);
    expect(dialEntries[1]).toMatchObject({ module: 'm131e', control: 0, gesture: 'right', serial: 'desk-dial', title: 'Encoder · Volume · Right', dialFunction: { index: 0, name: 'Volume' } });
    expect(dialEntries.at(-1)!.title).toBe('Encoder · Apps · Long press');
    expect(filterBindings(entries, 'scroll').map(entry => entry.id)).toEqual(['dial:1:0', 'dial:1:1']);
  });

  it('keeps a valid fixture free of warnings, including serial and wildcard on one gesture', () => {
    expect(bindingWarnings(fixture)).toEqual([]);
  });

  it('warns about flat encoder bindings next to dial functions, which the service rejects', () => {
    const warnings = bindingWarnings(withFunctions([{ name: 'A', bindings: [{ gesture: 'right', action: mute }] }], dial('short_press'), dial('released')));
    const conflict = warnings.find(warning => warning.kind === 'dial-flat-conflict')!;
    expect(conflict.entryIds).toEqual(['flat:0', 'flat:1']);
    expect(conflict.message).toContain('rejects flat encoder bindings');
    // Flat Release + Short press overlap is still reported, but never for function bindings.
    expect(warnings.filter(warning => warning.kind === 'release-overlap')).toHaveLength(1);
  });

  it('warns about functions without bindings, duplicate and invalid names, and duplicate selectors per function', () => {
    const right = { gesture: 'right', action: mute } as const;
    const warnings = bindingWarnings(withFunctions([
      { name: 'Same', bindings: [right, right] }, { name: 'Same', bindings: [right] }, { name: ' Padded', bindings: [] }, { name: 'Zero​width', bindings: [right] },
    ]));
    expect(warnings.map(warning => warning.kind).sort()).toEqual(['dial-function-empty', 'dial-function-name', 'dial-function-name', 'dial-function-name', 'duplicate-selector'].sort());
    expect(warnings.find(warning => warning.kind === 'duplicate-selector')!.entryIds).toEqual(['dial:0:0', 'dial:0:1']);
    const names = warnings.filter(warning => warning.kind === 'dial-function-name').map(warning => warning.id);
    // The duplicate is reported once, at the second occurrence.
    expect(names).toEqual(['dial-function-name:1', 'dial-function-name:2', 'dial-function-name:3']);
    expect(warnings.find(warning => warning.kind === 'dial-function-empty')!.message).toContain('does nothing');
    // The same selector in two different functions is not a duplicate.
    expect(bindingWarnings(withFunctions([{ name: 'A', bindings: [right] }, { name: 'B', bindings: [right] }]))).toEqual([]);
  });

  it('never offers encoder gestures, including Short press, to the flat editor while dial functions exist', () => {
    expect(editorGestures(fixture, 'm131e')).toEqual([]);
    expect(editorGestures(fixture, 'm003k')).toContain('clockwise');
    expect(editorGestures(profileWith(), 'm131e')).toContain('short_press');
    expect(dialEditorGestures).toEqual(['right', 'left', 'long_press']);
  });
});

describe('Inspector warnings and dial functions', () => {
  it('keeps warnings about dial function bindings out of the Inspector, where Show could not reach them', () => {
    const right = { gesture: 'right', action: mute } as const;
    const profile: Profile = { ...profileWith(dial('short_press')), dial_functions: [{ name: 'A', bindings: [right, right] }, { name: 'A', bindings: [right] }] };
    const warnings = bindingWarnings(profile);
    const shown = warningsForControl(warnings, listBindings(profile), { module: 'm131e', control: 0 });
    expect(warnings.some(warning => warning.kind === 'duplicate-selector')).toBe(true);
    expect(shown.map(warning => warning.kind)).toEqual(['dial-flat-conflict']);
  });
});
