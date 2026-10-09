import { describe, expect, it } from 'vitest';
import {
  actionDragMime, defaultBindingAction, entriesForScope, isValidForControl, pickerCategories, pickerEntries,
  pickerEntryLabel, pickerGroups, readActionDrag, readRecentActions, recentActionLimit, recentActionsStorageKey,
  recordRecentAction, searchPickerEntries, offersActionDrag, writeActionDrag,
} from './action-picker-model';
import type { PickerActionType, StorageLike } from './action-picker-model';

const memoryStorage = (initial: Record<string, string> = {}): StorageLike & { store: Map<string, string> } => {
  const store = new Map(Object.entries(initial));
  return { store, getItem: key => store.get(key) ?? null, setItem: (key, value) => { store.set(key, value); } };
};

const types = (entries: readonly { type: PickerActionType }[]) => entries.map(entry => entry.type);

describe('picker catalogue', () => {
  it('covers every assignable action type exactly once', () => {
    const expected: PickerActionType[] = ['volume', 'set_volume', 'toggle_mute', 'volume_from_position', 'shortcut', 'text',
      'launch_app', 'open_url', 'media', 'system', 'obs_scene', 'obs_virtual_camera', 'obs_replay_buffer', 'obs_studio_mode',
      'obs_transition', 'obs_filter', 'obs_source_visibility', 'obs_scene_collection', 'obs_profile', 'timer', 'multi_action', 'profile'];
    expect(types(pickerEntries).sort()).toEqual([...expected].sort());
    expect(new Set(types(pickerEntries)).size).toBe(pickerEntries.length);
  });

  it('keeps the wording of the flat select it replaces', () => {
    expect(pickerEntryLabel('open_url')).toBe('Open URL');
    expect(pickerEntryLabel('multi_action')).toBe('Multi-action');
    expect(pickerEntryLabel('profile')).toBe('Switch profile');
    expect(pickerEntryLabel('timer')).toBe('Timer');
    expect(pickerEntryLabel('volume_from_position')).toBe('Output volume follows position');
  });

  it('places every entry in a known category, in display order', () => {
    const known = new Set(pickerCategories.map(([category]) => category));
    for (const entry of pickerEntries) expect(known.has(entry.category), entry.type).toBe(true);
  });
});

describe('scope filtering', () => {
  it('offers faders only the position-driven action', () => {
    expect(types(entriesForScope('binding-fader'))).toEqual(['volume_from_position']);
    expect(isValidForControl('volume_from_position', 'm005s')).toBe(true);
    for (const type of ['open_url', 'timer', 'multi_action', 'profile', 'volume'] as PickerActionType[]) expect(isValidForControl(type, 'm005s')).toBe(false);
  });

  it('offers every other control all actions except the fader one', () => {
    const offered = types(entriesForScope('binding'));
    expect(offered).not.toContain('volume_from_position');
    for (const type of ['open_url', 'text', 'media', 'system', 'multi_action', 'shortcut', 'profile', 'timer', 'obs_scene']) expect(offered).toContain(type);
    for (const module of ['m003k', 'm131e', 'm4315', 'm032le'] as const) {
      expect(isValidForControl('volume_from_position', module)).toBe(false);
      expect(isValidForControl('timer', module)).toBe(true);
    }
  });

  it('offers multi-action steps the executable actions only', () => {
    const offered = types(entriesForScope('step'));
    expect(offered).not.toContain('multi_action');
    expect(offered).not.toContain('profile');
    expect(offered).not.toContain('timer');
    expect(offered).not.toContain('volume_from_position');
    expect(offered).toContain('toggle_mute');
    expect(offered).toContain('obs_scene');
  });

  it('groups the visible entries by category and drops empty groups', () => {
    const groups = pickerGroups('', 'binding');
    expect(groups.map(group => group.label)).toEqual(['Audio', 'Keyboard', 'Apps & web', 'Media', 'System', 'OBS', 'Timers & sequences', 'Navigation']);
    expect(groups.find(group => group.category === 'obs')!.entries).toHaveLength(9);
    expect(pickerGroups('', 'binding-fader').map(group => group.label)).toEqual(['Audio']);
  });
});

describe('search', () => {
  const binding = entriesForScope('binding');

  it('matches every whitespace-separated word across label, keywords and type', () => {
    expect(types(searchPickerEntries('obs', binding))).toHaveLength(9);
    expect(types(searchPickerEntries('spotify', binding))).toEqual(['media']);
    expect(types(searchPickerEntries('countdown', binding))).toEqual(['timer']);
    expect(types(searchPickerEntries('url', binding))).toEqual(['open_url']);
    expect(types(searchPickerEntries('scene switch', binding))).toContain('obs_scene');
    // Words match in any order.
    expect(types(searchPickerEntries('switch scene', binding))).toContain('obs_scene');
    expect(searchPickerEntries('  ', binding)).toEqual(binding);
  });

  it('finds nothing for nonsense and never leaks the fader action into other scopes', () => {
    expect(searchPickerEntries('printer', binding)).toEqual([]);
    expect(types(searchPickerEntries('volume', entriesForScope('step')))).toEqual(['volume', 'set_volume']);
    expect(types(searchPickerEntries('follows position', binding))).toEqual([]);
  });
});

describe('default actions', () => {
  it('builds the same defaults the flat select built, for every type', () => {
    for (const entry of pickerEntries) expect(defaultBindingAction(entry.type).type).toBe(entry.type);
    expect(defaultBindingAction('shortcut')).toEqual({ type: 'shortcut', key: 'c', modifiers: ['command'] });
    expect(defaultBindingAction('multi_action')).toEqual({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'shortcut', key: 'c', modifiers: ['command'] }] });
    expect(defaultBindingAction('profile')).toEqual({ type: 'profile', direction: 'next' });
    expect(defaultBindingAction('timer')).toEqual({ type: 'timer', seconds: 300 });
    expect(defaultBindingAction('volume_from_position')).toEqual({ type: 'volume_from_position' });
  });
});

describe('recent actions', () => {
  it('is empty without storage or with damaged contents', () => {
    expect(readRecentActions(memoryStorage())).toEqual([]);
    expect(readRecentActions(memoryStorage({ [recentActionsStorageKey]: 'not json' }))).toEqual([]);
    expect(readRecentActions(memoryStorage({ [recentActionsStorageKey]: '"text"' }))).toEqual([]);
    expect(readRecentActions(memoryStorage({ [recentActionsStorageKey]: '["junk","volume"]' }))).toEqual(['volume']);
  });

  it('records newest first, deduplicates and keeps only four', () => {
    const storage = memoryStorage();
    for (const type of ['open_url', 'timer', 'open_url', 'media', 'system', 'shortcut', 'profile'] as PickerActionType[]) recordRecentAction(storage, type);
    expect(readRecentActions(storage)).toEqual(['profile', 'shortcut', 'system', 'media']);
    expect(readRecentActions(storage)).toHaveLength(recentActionLimit);
  });
});

describe('drag payload', () => {
  it('round-trips the action type through the custom MIME type', () => {
    const stored: Record<string, string> = {};
    const dataTransfer = {
      types: [] as string[],
      effectAllowed: '',
      setData(name: string, value: string) { stored[name] = value; this.types.push(name); },
      getData(name: string) { return stored[name] ?? ''; },
    };
    writeActionDrag(dataTransfer as unknown as DataTransfer, 'obs_scene');
    expect(stored[actionDragMime]).toBe('obs_scene');
    expect(offersActionDrag(dataTransfer)).toBe(true);
    expect(readActionDrag(dataTransfer)).toBe('obs_scene');
  });

  it('ignores foreign payloads', () => {
    expect(offersActionDrag({ types: ['text/plain'] })).toBe(false);
    expect(readActionDrag({ getData: () => 'text/plain' })).toBeNull();
    expect(readActionDrag({ getData: () => 'not-a-type' })).toBeNull();
  });
});
