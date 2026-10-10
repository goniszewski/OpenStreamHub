import { describe, expect, it } from 'vitest';
import type { Action, LiveDevice, Profile } from './contracts';
import { labelIssue } from './draft-model';
import { FakeStorage } from './fake-storage';
import { markStartersOffered, nextModulePrompt, shouldOfferStarters, starterIssue, starterModules, starters, startersForModule, startersStorageKey } from './starter-model';
import { controlCounts, gestures, modules } from './ui-model';

const device = (path: string, module: LiveDevice['module']): LiveDevice => ({ path_hex: path, module, serial_number: null });

// Walks every action in a starter profile, including the keypad tiles.
function* actionsOf(profile: Profile): Generator<{ action: Action | { type: 'volume_from_position' }; where: string }> {
  for (const binding of profile.bindings) yield { action: binding.action, where: `${binding.module}/${binding.control}/${binding.gesture}` };
  for (const page of profile.keypad?.pages ?? []) for (const [index, slot] of page.slots.entries()) {
    if (slot?.type !== 'action') continue;
    for (const keypadBinding of slot.bindings) yield { action: keypadBinding.action, where: `keypad/${page.id}/${index}` };
  }
}

describe('shipped starters', () => {
  it('are structurally valid profiles the service will accept', () => {
    expect(starters.map(item => item.name)).toEqual(['Mac basics', 'Streaming (OBS)', 'Editing and media']);
    for (const item of starters) {
      expect(starterIssue(item.profile), item.id).toBeNull();
      expect(labelIssue(item.profile.name, 'Profile name'), item.id).toBeNull();
      const page = item.profile.keypad!.pages[0];
      expect(labelIssue(page.name, 'Page name'), item.id).toBeNull();
      for (const slot of page.slots) {
        if (slot?.type !== 'action') continue;
        expect(labelIssue(slot.label, 'Key label'), `${item.id}: ${slot.label}`).toBeNull();
      }
    }
  });
  it('stay dry-run safe: nothing that types, launches, opens or names an OBS item', () => {
    const allowed = new Set(['volume', 'toggle_mute', 'media', 'system', 'timer', 'volume_from_position', 'obs_virtual_camera', 'obs_replay_buffer', 'obs_transition']);
    for (const item of starters) {
      for (const { action, where } of actionsOf(item.profile)) {
        expect(allowed.has(action.type), `${item.id} at ${where}`).toBe(true);
      }
    }
  });
  it('bind supported selectors without duplicates and keep the 15-key module on the keypad', () => {
    for (const item of starters) {
      const seen = new Set<string>();
      for (const binding of item.profile.bindings) {
        expect(controlCounts[binding.module], `${item.id} module`).toBeDefined();
        expect(binding.control, `${item.id} control`).toBeLessThan(controlCounts[binding.module]);
        expect(gestures[binding.module], `${item.id} gesture`).toContain(binding.gesture);
        const selector = `${binding.module}/${binding.control}/${binding.gesture}`;
        expect(seen.has(selector), `${item.id} duplicate ${selector}`).toBe(false);
        seen.add(selector);
        expect(binding.module, item.id).not.toBe('m4315');
      }
      const page = item.profile.keypad!.pages[0];
      expect(page.id).toBe('root');
      expect(page.slots).toHaveLength(15);
    }
  });
  it('cover every module together, so a connected module is always offered something', () => {
    const covered = new Set(starters.flatMap(item => starterModules(item.profile)));
    expect([...covered].sort()).toEqual(modules.map(item => item.id).sort());
  });
  it('derive their covered modules from the bindings and the keypad tiles', () => {
    expect(starterModules(starters[0].profile)).toEqual(['m003k', 'm131e', 'm005s', 'm032le', 'm4315']);
    expect(startersForModule('m032le')).toEqual([starters[0]]);
    expect(startersForModule('m4315')).toEqual(starters);
  });
});

describe('hot-plug prompt from a simulated inventory', () => {
  it('treats the first snapshot as the baseline', () => {
    expect(nextModulePrompt(null, [device('aa', 'm4315')])).toBeNull();
  });
  it('names the first added module after the baseline, and nothing on removals or swaps', () => {
    const baseline = [device('aa', 'm4315'), device('bb', 'm003k')];
    expect(nextModulePrompt(baseline, [...baseline, device('cc', 'm131e')])).toBe('m131e');
    expect(nextModulePrompt(baseline, [device('aa', 'm4315')])).toBeNull();
    expect(nextModulePrompt(baseline, [device('bb', 'm003k'), device('cc', 'm131e')])).toBe('m131e');
    // A module swap on one path is a removal plus an addition.
    expect(nextModulePrompt(baseline, [device('aa', 'm4315'), device('bb2', 'm005s')])).toBe('m005s');
    // An unchanged inventory names nobody.
    expect(nextModulePrompt(baseline, [...baseline])).toBeNull();
  });
});

describe('first-run flag', () => {
  it('offers the chooser once on a fresh desktop store and records the offer', () => {
    const storage = new FakeStorage();
    expect(shouldOfferStarters(storage, true)).toBe(true);
    expect(shouldOfferStarters(storage, true)).toBe(true); // deciding is not offering
    markStartersOffered(storage);
    expect(storage.getItem(startersStorageKey)).toBe('offered');
    expect(shouldOfferStarters(storage, true)).toBe(false);
  });
  it('never offers outside the desktop app, without storage, or when storage is blocked', () => {
    expect(shouldOfferStarters(new FakeStorage(), false)).toBe(false);
    expect(shouldOfferStarters(null, true)).toBe(false);
    const broken = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
    expect(shouldOfferStarters(broken, true)).toBe(false);
    expect(() => markStartersOffered(broken)).not.toThrow();
    expect(() => markStartersOffered(null)).not.toThrow();
  });
});
