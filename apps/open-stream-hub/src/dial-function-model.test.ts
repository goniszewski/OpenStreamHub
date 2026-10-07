import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { DialFunction, Profile } from './contracts';
import {
  addDialFunction, deleteDialFunction, dialBindingAt, dialFunctionNameIssue, dialSerialIssue, editDialBinding, flatDialBindings,
  maxDialFunctions, moveDialFunction, moveFlatDialBindings, removeDialBinding, renameDialFunction,
} from './dial-function-model';
import { stableStringify } from './draft-model';

const fixtureText = readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/dial-functions.json', import.meta.url), 'utf8');
const fixture = () => JSON.parse(fixtureText) as Profile;
const mute = { type: 'toggle_mute' } as const;
const plain: Profile = { schema_version: 1, name: 'P', bindings: [{ module: 'm003k', control: 0, gesture: 'clockwise', action: mute }] };

describe('dial function names mirror the service validator', () => {
  const functions: DialFunction[] = [{ name: 'Volume', bindings: [] }, { name: 'Scroll', bindings: [] }];
  // The same cases as invalid_dial_function_configs_are_rejected in crates/cooltek-sdk-service/src/dial.rs.
  it.each([
    ['', 'Enter'], ['  ', 'Enter'], [' Volume', 'start and end'], ['Volume ', 'start and end'], ['a\nb', 'control'],
    ['x'.repeat(65), '64 bytes'], ['é'.repeat(33), '64 bytes'], ['Vol‮ume', 'invisible'], ['Vol⁦ume', 'invisible'],
    ['Vol​ume', 'invisible'], ['Vol﻿ume', 'invisible'], ['Vol­ume', 'invisible'], ['Scroll', 'already named'],
  ])('rejects %j', (name, message) => {
    expect(dialFunctionNameIssue(name, functions, 0)).toContain(message);
  });

  it.each([['Volume'], ['Two words'], ['Łódź'], ['x'.repeat(64)], ['é'.repeat(32)]])('accepts %j', name => {
    expect(dialFunctionNameIssue(name, functions, 0)).toBeNull();
  });

  it('is case-sensitive and never compares a function with itself', () => {
    expect(dialFunctionNameIssue('scroll', functions, 0)).toBeNull();
    expect(dialFunctionNameIssue('Scroll', functions, 1)).toBeNull();
  });

  it('checks optional binding serials like the service', () => {
    expect(dialSerialIssue('')).toBeNull();
    expect(dialSerialIssue('desk-dial')).toBeNull();
    expect(dialSerialIssue('  ')).toContain('only spaces');
    expect(dialSerialIssue('a\tb')).toContain('control');
    expect(dialSerialIssue('x'.repeat(257))).toContain('256');
  });
});

describe('dial function edits', () => {
  it('starts from no functions without changing the profile, and the last delete restores that shape', () => {
    expect(plain).not.toHaveProperty('dial_functions');
    const added = addDialFunction(plain);
    expect(added.dial_functions).toEqual([{ name: 'Function 1', bindings: [] }]);
    expect(added.bindings).toBe(plain.bindings);
    const removed = deleteDialFunction(added, 0);
    expect(removed).not.toHaveProperty('dial_functions');
    expect(stableStringify(removed)).toBe(stableStringify(plain));
  });

  it('adds up to 16 functions with unique default names', () => {
    let profile = plain;
    for (let index = 0; index < maxDialFunctions; index += 1) profile = addDialFunction(profile);
    expect(new Set(profile.dial_functions!.map(item => item.name)).size).toBe(maxDialFunctions);
    expect(() => addDialFunction(profile)).toThrow('at most 16');
    const renamed = renameDialFunction(deleteDialFunction(profile, 0), 0, 'Function 3');
    expect(addDialFunction(renamed).dial_functions!.at(-1)!.name).not.toBe('Function 3');
  });

  it('renames, reorders and deletes without touching other functions', () => {
    const profile = fixture();
    const renamed = renameDialFunction(profile, 1, 'Scroll ');
    expect(renamed.dial_functions![1].name).toBe('Scroll ');
    expect(renamed.dial_functions![0]).toBe(profile.dial_functions![0]);
    expect(moveDialFunction(profile, 0, 1).dial_functions!.map(item => item.name)).toEqual(['Scroll', 'Volume', 'Apps']);
    expect(moveDialFunction(profile, 0, -1)).toBe(profile);
    expect(moveDialFunction(profile, 2, 1)).toBe(profile);
    expect(deleteDialFunction(profile, 1).dial_functions!.map(item => item.name)).toEqual(['Volume', 'Apps']);
    expect(() => renameDialFunction(profile, 5, 'x')).toThrow();
    expect(profile).toEqual(fixture());
  });

  it('edits exact gesture and serial selectors and never accepts Short press, Press or Release', () => {
    const profile = fixture();
    const volume = { type: 'volume', delta: 2 } as const;
    const edited = editDialBinding(profile, 0, 'right', 'desk-dial', volume);
    expect(dialBindingAt(edited, 0, 'right', 'desk-dial')!.action).toEqual(volume);
    expect(dialBindingAt(edited, 0, 'right', '')!.action).toEqual({ type: 'volume', delta: 5 });
    const added = editDialBinding(profile, 2, 'right', '', mute);
    expect(added.dial_functions![2].bindings.at(-1)).toEqual({ gesture: 'right', action: mute });
    expect(editDialBinding(profile, 1, 'left', '', null).dial_functions![1].bindings.map(item => item.gesture)).toEqual(['right']);
    expect(removeDialBinding(profile, 0, 1).dial_functions![0].bindings).toHaveLength(3);
    for (const gesture of ['short_press', 'pressed', 'released'] as const) {
      expect(() => editDialBinding(profile, 0, gesture as never, '', mute)).toThrow('Right, Left and Long press');
    }
  });

  it('moves flat encoder bindings into a function only on request and drops what functions cannot hold', () => {
    const profile: Profile = {
      ...addDialFunction({ ...plain, bindings: [...plain.bindings,
        { module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 5 } },
        { module: 'm131e', control: 0, gesture: 'long_press', serial_number: 'unit', action: mute },
        { module: 'm131e', control: 0, gesture: 'short_press', action: mute },
      ] }, 'Main'),
    };
    expect(flatDialBindings(profile).moved).toHaveLength(2);
    expect(flatDialBindings(profile).dropped.map(item => item.gesture)).toEqual(['short_press']);
    const moved = moveFlatDialBindings(profile, 0);
    expect(moved.bindings).toEqual(plain.bindings);
    expect(moved.dial_functions![0].bindings).toEqual([
      { gesture: 'right', action: { type: 'volume', delta: 5 } },
      { gesture: 'long_press', serial_number: 'unit', action: mute },
    ]);
    expect(() => moveFlatDialBindings(plain, 0)).toThrow('Add a dial function first');
  });

  it('round-trips the service fixture through JSON unchanged', () => {
    const profile = fixture();
    expect(JSON.parse(JSON.stringify(profile))).toEqual(JSON.parse(fixtureText));
    const back = renameDialFunction(renameDialFunction(profile, 0, 'Temp'), 0, 'Volume');
    expect(stableStringify(back)).toBe(stableStringify(JSON.parse(fixtureText)));
  });
});

describe('review follow-ups', () => {
  it('names flat bindings that collide with the target function and keeps the function binding', () => {
    const base = addDialFunction({ ...plain, bindings: [...plain.bindings,
      { module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 9 } },
      { module: 'm131e', control: 0, gesture: 'left', action: { type: 'volume', delta: -9 } },
    ] }, 'Main');
    const profile = editDialBinding(base, 0, 'right', '', mute);
    const plan = flatDialBindings(profile, 0);
    expect(plan.moved.map(item => item.gesture)).toEqual(['left']);
    expect(plan.conflicting.map(item => item.gesture)).toEqual(['right']);
    expect(plan.dropped).toEqual([]);
    // Without a target function nothing conflicts.
    expect(flatDialBindings(profile).conflicting).toEqual([]);
    const moved = moveFlatDialBindings(profile, 0);
    expect(moved.dial_functions![0].bindings).toEqual([{ gesture: 'right', action: mute }, { gesture: 'left', action: { type: 'volume', delta: -9 } }]);
    expect(moved.bindings).toEqual(plain.bindings);
  });

  it('rejects lone UTF-16 surrogates that the service cannot store', () => {
    expect(dialFunctionNameIssue('Vol\uD800ume', [], 0)).toBe('Remove invalid characters.');
    expect(dialFunctionNameIssue('Vol\uDC00', [], 0)).toBe('Remove invalid characters.');
    expect(dialFunctionNameIssue('Emoji 🎛', [], 0)).toBeNull();
    expect(dialSerialIssue('unit\uD800')).toBe('Remove invalid characters.');
  });
});
