import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { clipKeyLabel, quickMap, wrapText } from './quick-map-model';
import { starters } from './starter-model';
import { actionLabel, browserProfile } from './ui-model';

const starter = (id: string) => starters.find(item => item.id === id)!.profile;
const empty: Profile = { schema_version: 5, name: 'Empty', bindings: [], keypad: { pages: [{ id: 'root', name: 'Root', slots: Array.from({ length: 15 }, () => null) }] } };

describe('quick map legend', () => {
  it('draws nothing for a profile with nothing bound', () => {
    const map = quickMap(empty);
    expect(map.modules).toEqual([]);
    expect(map.legend).toEqual([]);
  });

  it('numbers bound controls in drawing order and words them with the action labels', () => {
    const map = quickMap(browserProfile);
    expect(map.modules.map(item => item.module)).toEqual(['m131e', 'm003k']);
    expect(map.legend.map(line => line.number)).toEqual([1, 2]);
    expect(map.legend[0]).toMatchObject({ name: 'Encoder', text: `Right: ${actionLabel({ type: 'volume', delta: 5 })}` });
    expect(map.legend[1].name).toBe('Knob 1');
    expect(map.legend[1].text).toBe('Clockwise: Output volume +5 points; Counter-clockwise: Output volume -5 points; Press: Toggle output mute');
  });

  it('leaves unbound controls without a number', () => {
    const knobs = quickMap(browserProfile).modules.find(item => item.module === 'm003k')!;
    expect(knobs.controls.map(control => control.number)).toEqual([2, null, null]);
  });

  it('names a fader without a gesture and a key with its label', () => {
    const map = quickMap({
      ...empty, bindings: [{ module: 'm005s', control: 0, gesture: 'position_changed', action: { type: 'volume_from_position' } }],
      keypad: { pages: [{ id: 'root', name: 'Root', slots: [{ type: 'action', label: 'Mute', bindings: [{ gesture: 'pressed', action: { type: 'toggle_mute' } }] }, ...Array.from({ length: 14 }, () => null)] }] },
    });
    expect(map.legend).toEqual([
      { number: 1, name: 'Key 1 (Mute)', folder: false, text: 'Toggle output mute' },
      { number: 2, name: 'Fader 1', folder: false, text: 'Follow fader position for output volume' },
    ]);
  });

  it('marks a folder key and says it opens a sub-page', () => {
    const map = quickMap({
      ...empty, keypad: { pages: [
        { id: 'root', name: 'Root', slots: [{ type: 'folder', page_id: 'docs' }, ...Array.from({ length: 14 }, () => null)] },
        { id: 'docs', name: 'Docs', slots: [null, { type: 'action', label: 'A', bindings: [{ gesture: 'pressed', action: { type: 'toggle_mute' } }] }, ...Array.from({ length: 13 }, () => null)] },
      ] },
    });
    const key = map.modules[0].controls[0];
    expect(key.folder).toEqual({ name: 'Docs' });
    expect(map.legend[0]).toMatchObject({ folder: true, text: 'opens the “Docs” folder (a sub-page)' });
  });

  it('lists the dial per function and notes that a short press switches function', () => {
    const map = quickMap({
      ...empty, dial_functions: [
        { name: 'Volume', bindings: [{ gesture: 'right', action: { type: 'volume', delta: 5 } }] },
        { name: 'Zoom', bindings: [{ gesture: 'long_press', action: { type: 'toggle_mute' } }] },
      ],
    });
    expect(map.legend[0].text).toBe('Volume · Right: Output volume +5 points; Zoom · Long press: Toggle output mute; Short press switches to the next function');
  });

  it('only reads the root page of the keypad, and migrates a schema 1 profile', () => {
    const map = quickMap({ schema_version: 1, name: 'Old', bindings: [{ module: 'm4315', control: 2, gesture: 'pressed', action: { type: 'toggle_mute' } }] });
    expect(map.legend.map(line => line.name)).toEqual(['Key 3']);
  });

  it('describes the built-in profiles the user keeps in everyday use', () => {
    for (const id of ['mac-basics', 'terminal-agents']) {
      const profile = id === 'terminal-agents' ? starters.find(item => item.id.includes('terminal'))!.profile : starter(id);
      const map = quickMap(profile);
      expect(map.legend.length).toBeGreaterThan(0);
      expect(map.legend.every(line => line.text.length > 0)).toBe(true);
    }
  });
});

describe('text helpers', () => {
  it('wraps at word boundaries and keeps a long word whole', () => {
    expect(wrapText('one two three four', 9)).toEqual(['one two', 'three', 'four']);
    expect(wrapText('extraordinarily short', 5)).toEqual(['extraordinarily', 'short']);
    expect(wrapText('', 5)).toEqual([]);
  });
  it('clips a key label by code points', () => {
    expect(clipKeyLabel('Short', 11)).toBe('Short');
    expect(clipKeyLabel('A very long label', 8)).toBe('A very …');
    expect(clipKeyLabel('😀😀😀😀', 3)).toBe('😀😀…');
  });
});
