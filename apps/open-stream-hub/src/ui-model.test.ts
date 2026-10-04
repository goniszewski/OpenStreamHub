import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { editBinding, readAppearance } from './ui-model';

const profile: Profile = {
  schema_version: 1, name: 'Selectors', bindings: [
    { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
    { module: 'm003k', control: 0, gesture: 'pressed', serial_number: 'Desk', action: { type: 'shortcut', key: 'c', modifiers: ['command'] } },
    { module: 'm003k', control: 1, gesture: 'pressed', action: { type: 'shortcut', key: 'v', modifiers: ['command'] } },
  ],
};

describe('profile draft selector editing', () => {
  it('keeps the wildcard and other controls when editing an exact serial selector', () => {
    const snapshot = structuredClone(profile);
    const edited = editBinding(profile, { module: 'm003k', control: 0, gesture: 'pressed', serial: 'Desk' }, { type: 'volume', delta: 8 });
    expect(edited.bindings[0]).toEqual(profile.bindings[0]);
    expect(edited.bindings[1]).toEqual({ ...profile.bindings[1], action: { type: 'volume', delta: 8 } });
    expect(edited.bindings[2]).toEqual(profile.bindings[2]);
    expect(profile).toEqual(snapshot);
  });

  it('treats serial casing as distinct without deciding runtime matching precedence', () => {
    const edited = editBinding(profile, { module: 'm003k', control: 0, gesture: 'pressed', serial: 'desk' }, { type: 'volume', delta: -5 });
    expect(edited.bindings).toHaveLength(4);
    expect(edited.bindings[1]).toEqual(profile.bindings[1]);
    expect(edited.bindings[3].serial_number).toBe('desk');
  });

  it('removes only the exact binding, leaving the wildcard available to the Rust resolver', () => {
    const edited = editBinding(profile, { module: 'm003k', control: 0, gesture: 'pressed', serial: 'Desk' }, null);
    expect(edited.bindings).toEqual([profile.bindings[0], profile.bindings[2]]);
  });
});

describe('appearance preference migration', () => {
  function preferences(values: Record<string, string>) {
    return { getItem: (key: string) => values[key] ?? null };
  }

  it('keeps the existing dark preference when starting under the new app name', () => {
    expect(readAppearance(preferences({ 'masterhub.appearance': 'dark' }))).toBe('dark');
  });

  it('keeps an explicitly chosen system appearance instead of restoring the old dark preference', () => {
    expect(readAppearance(preferences({ 'masterhub.appearance': 'dark', 'open-stream-hub.appearance': 'system' }))).toBe('system');
  });
});
