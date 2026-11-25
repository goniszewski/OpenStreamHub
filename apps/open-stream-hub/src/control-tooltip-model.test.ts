import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { actionDescription, controlTip } from './control-tooltip-model';
import { keypadRoot, upgradeKeypadProfile } from './keypad-model';
import { actionLabel } from './ui-model';

const profile = (bindings: unknown[], extra: object = {}) => ({ schema_version: 1, name: 'Tips', bindings, ...extra }) as unknown as Profile;

describe('controlTip', () => {
  it('names the action and describes it', () => {
    const tip = controlTip(profile([{ module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } }]), 'm003k', 0);
    expect(tip.title).toBe('Knob 1');
    expect(tip.rows).toEqual([{ gesture: null, name: 'Toggle output mute', description: 'Mutes or unmutes the Mac output.' }]);
  });
  it('lists every gesture in the module order, naming each', () => {
    const tip = controlTip(profile([
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
      { module: 'm003k', control: 0, gesture: 'clockwise', action: { type: 'volume', delta: 5 } },
      { module: 'm003k', control: 1, gesture: 'clockwise', action: { type: 'volume', delta: 1 } },
    ]), 'm003k', 0);
    expect(tip.rows.map(row => row.name)).toEqual(['Output volume +5 points', 'Toggle output mute']);
    expect(tip.rows.every(row => row.gesture)).toBe(true);
  });
  it('asks to assign an unbound control', () => {
    expect(controlTip(profile([]), 'm003k', 2).rows[0].name).toBe('Unassigned');
  });
  it('reads keypad keys from the page on screen', () => {
    const upgraded = upgradeKeypadProfile(profile([{ module: 'm4315', control: 3, gesture: 'pressed', action: { type: 'obs_scene', scene: 'Intro' } }]));
    const page = upgraded.keypad!.pages.find(item => item.id === keypadRoot)!;
    const tip = controlTip(upgraded, 'm4315', 3, { keypadPage: page, keypadPages: upgraded.keypad!.pages });
    expect(tip.rows).toEqual([{ gesture: null, name: 'OBS scene “Intro”', description: 'Switches OBS to a scene.' }]);
    expect(controlTip(upgraded, 'm4315', 4, { keypadPage: page }).rows[0].name).toBe('Unassigned');
    expect(controlTip(upgraded, 'm4315', 0, { keypadPage: page, keypadBack: true }).rows[0].name).toBe('Back');
  });
  it('describes a keypad folder', () => {
    const upgraded = upgradeKeypadProfile(profile([]));
    const pages = [
      { ...upgraded.keypad!.pages[0], slots: upgraded.keypad!.pages[0].slots.map((slot, index) => index === 1 ? { type: 'folder' as const, page_id: 'f' } : slot) },
      { id: 'f', name: 'Scenes', slots: [{ type: 'action' as const, label: 'A', bindings: [] }, ...Array(14).fill(null)] },
    ];
    const tip = controlTip(upgraded, 'm4315', 1, { keypadPage: pages[0], keypadPages: pages });
    expect(tip.rows[0].name).toBe('Folder: Scenes');
    expect(tip.rows[0].description).toContain('1 key');
  });
  it('shows each dial function and its gesture', () => {
    const tip = controlTip(profile([], { dial_functions: [{ name: 'Volume', bindings: [{ gesture: 'right', action: { type: 'volume', delta: 5 } }] }] }), 'm131e', 0);
    expect(tip.rows[0].gesture).toBe('Volume · Right');
  });
  it('describes every kind of action', () => {
    expect(actionDescription({ type: 'obs_stream', op: 'toggle' } as never)).toMatch(/stream/);
    expect(actionLabel(null)).toBe('Unbound');
  });
});
