import { describe, expect, it } from 'vitest';
import type { Action, Binding, BindingAction, KeypadPage, Profile } from './contracts';
import { describeEdit } from './edit-labels';

const profile = (patch: Partial<Profile> = {}): Profile => ({ schema_version: 1, name: 'Everyday', bindings: [], ...patch });
const knob = (control: number, action: Binding['action']): Binding => ({ module: 'm003k', control, gesture: 'clockwise', action });
const page = (patch: Partial<KeypadPage> = {}): KeypadPage => ({ id: 'root', name: 'Main', slots: [null, null, null], ...patch });
const key = (label: string, action: Action): KeypadPage['slots'][number] => ({ type: 'action', label, bindings: [{ gesture: 'pressed', action }] });

describe('describeEdit', () => {
  it('names a rename', () => {
    expect(describeEdit(profile(), profile({ name: 'Streaming' }))).toBe('Renamed the profile to “Streaming”');
  });

  it('names a binding added, changed and removed by its control and gesture', () => {
    const volume = { type: 'volume', delta: 5 } as const;
    expect(describeEdit(profile(), profile({ bindings: [knob(0, volume)] }))).toBe('Knob 1 clockwise: Output volume +5 points');
    expect(describeEdit(profile({ bindings: [knob(0, volume)] }), profile({ bindings: [knob(0, { type: 'volume', delta: 8 })] }))).toBe('Knob 1 clockwise: Output volume +8 points');
    expect(describeEdit(profile({ bindings: [knob(2, volume)] }), profile())).toBe('Removed the Knob 3 clockwise binding');
  });

  it('counts several binding changes instead of listing them', () => {
    const volume = { type: 'volume', delta: 5 } as const;
    expect(describeEdit(profile(), profile({ bindings: [knob(0, volume), knob(1, volume)] }))).toBe('Changed 2 bindings');
  });

  it('names a key by its place, a folder or empty slot, artwork alone, and several keys by count', () => {
    const mute = { type: 'toggle_mute' } as const;
    const before = profile({ keypad: { pages: [page()] } });
    expect(describeEdit(before, profile({ keypad: { pages: [page({ slots: [null, key('Mute', mute), null] })] } }))).toBe('Key 2: Toggle output mute');
    expect(describeEdit(before, profile({ keypad: { pages: [page({ slots: [{ type: 'folder', page_id: 'f' }, null, null] })] } }))).toBe('Key 1: a folder');
    expect(describeEdit(before, profile({ keypad: { pages: [page({ slots: [key('A', mute), key('B', mute), null] })] } }))).toBe('Changed 2 keys');
    const art = { jpeg_base64: 'x', delay_ms: 0 };
    expect(describeEdit(before, profile({ keypad: { pages: [page({ artwork: { '2': { frames: [art] } as never } })] } }))).toBe('Changed the artwork of Key 3');
  });

  it('names key pages added and removed', () => {
    const one = profile({ keypad: { pages: [page()] } });
    const two = profile({ keypad: { pages: [page(), page({ id: 'f', name: 'Folder' })] } });
    expect(describeEdit(one, two)).toBe('Added the key page “Folder”');
    expect(describeEdit(two, one)).toBe('Removed the key page “Folder”');
  });

  it('names dial changes, and says several changes as the first plus a count', () => {
    expect(describeEdit(profile(), profile({ dial_display: { content: 'clock' } }))).toBe('Changed the dial display');
    expect(describeEdit(profile(), profile({ dial_functions: [{ name: 'Volume', bindings: [] }] }))).toBe('Changed the dial functions');
    expect(describeEdit(profile(), profile({ name: 'B', dial_display: { content: 'clock' } }))).toBe('Renamed the profile to “B” and 1 more change');
    expect(describeEdit(profile(), profile({ name: 'B', dial_display: { content: 'clock' }, dial_functions: [{ name: 'V', bindings: [] }] }))).toBe('Renamed the profile to “B” and 2 more changes');
  });

  it('keeps a macro’s keys and long names out of the text', () => {
    const macro: BindingAction = { type: 'macro', steps: [{ key: 'h', modifiers: [], delay_before_ms: 0, hold_ms: 40 }] };
    expect(describeEdit(profile(), profile({ bindings: [knob(0, macro)] }))).not.toMatch(/\bh\b/);
    expect(describeEdit(profile(), profile({ name: 'x'.repeat(60) }))).toBe(`Renamed the profile to “${'x'.repeat(28)}…”`);
  });

  it('falls back to a plain phrase when nothing it knows changed', () => {
    expect(describeEdit(profile(), profile({ schema_version: 2 }))).toBe('Edited the profile');
  });
});
