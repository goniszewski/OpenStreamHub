import { describe, expect, it } from 'vitest';
import {
  actionDragMime, defaultBindingAction, entriesForScope, isValidForControl, pickerCategories, pickerEntries,
  pickerEntryIsCurrent, pickerEntryLabel, pickerGroups, readActionDrag, readRecentActions, recentActionLimit,
  recentActionsStorageKey, recordRecentAction, searchPickerEntries, offersActionDrag, writeActionDrag,
} from './action-picker-model';
import type { PickerActionType, StorageLike } from './action-picker-model';

const memoryStorage = (initial: Record<string, string> = {}): StorageLike & { store: Map<string, string> } => {
  const store = new Map(Object.entries(initial));
  return { store, getItem: key => store.get(key) ?? null, setItem: (key, value) => { store.set(key, value); } };
};

const types = (entries: readonly { type: PickerActionType }[]) => entries.map(entry => entry.type);

describe('picker catalogue', () => {
  it('covers every assignable action type exactly once', () => {
    const expected: PickerActionType[] = ['volume', 'set_volume', 'toggle_mute', 'volume_from_position', 'mic_input_volume_from_position', 'obs_input_volume_from_position', 'shortcut', 'text',
      'launch_app', 'open_url', 'media', 'system', 'obs', 'streamlabs', 'timer', 'multi_action', 'conditional', 'macro', 'profile', 'set_variable', 'toggle_variable', 'add_to_variable'];
    expect(types(pickerEntries).sort()).toEqual([...expected].sort());
    expect(new Set(types(pickerEntries)).size).toBe(pickerEntries.length);
  });

  it('keeps the wording of the flat select it replaces', () => {
    expect(pickerEntryLabel('open_url')).toBe('Open URL');
    expect(pickerEntryLabel('multi_action')).toBe('Multi-action');
    expect(pickerEntryLabel('profile')).toBe('Switch profile');
    expect(pickerEntryLabel('timer')).toBe('Timer');
    expect(pickerEntryLabel('volume_from_position')).toBe('Output volume follows position');
    // The one OBS entry names every obs_* action, so the trigger and the editor's
    // Operation dropdown never repeat each other.
    expect(pickerEntryLabel('obs')).toBe('OBS');
    for (const type of ['obs_scene', 'obs_filter', 'obs_profile'] as PickerActionType[]) expect(pickerEntryLabel(type)).toBe('OBS');
    expect(pickerEntryIsCurrent('obs', 'obs_filter')).toBe(true);
    expect(pickerEntryIsCurrent('obs', 'open_url')).toBe(false);
    expect(pickerEntryIsCurrent('open_url', 'open_url')).toBe(true);
    // The same for the one Streamlabs entry and the seven streamlabs_* actions.
    expect(pickerEntryLabel('streamlabs')).toBe('Streamlabs');
    for (const type of ['streamlabs_scene', 'streamlabs_audio', 'streamlabs_stream'] as PickerActionType[]) {
      expect(pickerEntryLabel(type)).toBe('Streamlabs');
      expect(pickerEntryIsCurrent('streamlabs', type)).toBe(true);
      expect(pickerEntryIsCurrent('obs', type)).toBe(false);
    }
    expect(pickerEntryIsCurrent('streamlabs', 'obs_scene')).toBe(false);
  });

  it('places every entry in a known category, in display order', () => {
    const known = new Set(pickerCategories.map(([category]) => category));
    for (const entry of pickerEntries) expect(known.has(entry.category), entry.type).toBe(true);
  });
});

describe('scope filtering', () => {
  it('offers faders only the position-driven actions', () => {
    expect(types(entriesForScope('binding-fader'))).toEqual(['volume_from_position', 'mic_input_volume_from_position', 'obs_input_volume_from_position']);
    expect(isValidForControl('mic_input_volume_from_position', 'm005s')).toBe(true);
    expect(isValidForControl('mic_input_volume_from_position', 'm003k')).toBe(false);
    expect(types(entriesForScope('binding'))).not.toContain('mic_input_volume_from_position');
    expect(isValidForControl('volume_from_position', 'm005s')).toBe(true);
    expect(isValidForControl('obs_input_volume_from_position', 'm005s')).toBe(true);
    expect(isValidForControl('obs_input_volume_from_position', 'm003k')).toBe(false);
    expect(types(entriesForScope('binding'))).not.toContain('obs_input_volume_from_position');
    expect(types(entriesForScope('step'))).not.toContain('obs_input_volume_from_position');
    for (const type of ['open_url', 'timer', 'multi_action', 'profile', 'volume', 'obs'] as PickerActionType[]) expect(isValidForControl(type, 'm005s')).toBe(false);
  });

  it('offers every other control all actions except the fader one', () => {
    const offered = types(entriesForScope('binding'));
    expect(offered).not.toContain('volume_from_position');
    for (const type of ['open_url', 'text', 'media', 'system', 'multi_action', 'shortcut', 'profile', 'timer', 'obs']) expect(offered).toContain(type);
    for (const module of ['m003k', 'm131e', 'm4315', 'm032le'] as const) {
      expect(isValidForControl('volume_from_position', module)).toBe(false);
      expect(isValidForControl('timer', module)).toBe(true);
      expect(isValidForControl('obs', module)).toBe(true);
    }
  });

  it('offers multi-action steps the executable actions only', () => {
    const offered = types(entriesForScope('step'));
    expect(offered).not.toContain('multi_action');
    expect(offered).not.toContain('conditional');
    expect(offered).not.toContain('profile');
    expect(offered).not.toContain('timer');
    expect(offered).not.toContain('volume_from_position');
    expect(offered).toContain('toggle_mute');
    expect(offered).toContain('obs');
    expect(offered).toContain('streamlabs');
  });

  it('groups the visible entries by category and drops empty groups', () => {
    const groups = pickerGroups('', 'binding');
    expect(groups.map(group => group.label)).toEqual(['Audio', 'Keyboard', 'Apps & web', 'Media', 'System', 'OBS', 'Streamlabs', 'Timers & sequences', 'Variables', 'Navigation']);
    expect(groups.find(group => group.category === 'obs')!.entries).toHaveLength(1);
    expect(groups.find(group => group.category === 'streamlabs')!.entries).toHaveLength(1);
    expect(pickerGroups('', 'binding-fader').map(group => group.label)).toEqual(['Audio', 'OBS']);
  });
});

describe('search', () => {
  const binding = entriesForScope('binding');

  it('matches every whitespace-separated word across label, keywords and type', () => {
    expect(types(searchPickerEntries('obs', binding))).toEqual(['obs']);
    expect(types(searchPickerEntries('spotify', binding))).toEqual(['media']);
    expect(types(searchPickerEntries('countdown', binding))).toEqual(['timer']);
    expect(types(searchPickerEntries('url', binding))).toEqual(['open_url']);
    // The single OBS entry stays findable by each of the nine operations.
    for (const word of ['scene', 'camera', 'replay', 'studio', 'transition', 'filter', 'visibility', 'collection', 'profile']) {
      expect(types(searchPickerEntries(word, binding))).toContain('obs');
    }
    expect(types(searchPickerEntries('scene switch', binding))).toContain('obs');
    // Words match in any order.
    expect(types(searchPickerEntries('switch scene', binding))).toContain('obs');
    expect(searchPickerEntries('  ', binding)).toEqual(binding);
    // The single Streamlabs entry too, and "obs" does not find it.
    expect(types(searchPickerEntries('streamlabs', binding))).toEqual(['streamlabs']);
    for (const word of ['record', 'studio', 'transition', 'visibility', 'collection', 'audio', 'mute']) {
      expect(types(searchPickerEntries(word, binding))).toContain('streamlabs');
    }
  });

  it('finds nothing for nonsense and never leaks the fader action into other scopes', () => {
    expect(searchPickerEntries('printer', binding)).toEqual([]);
    expect(types(searchPickerEntries('volume', entriesForScope('step')))).toEqual(['volume', 'set_volume']);
    expect(types(searchPickerEntries('follows position', binding))).toEqual([]);
  });
});

describe('default actions', () => {
  it('builds the same defaults the flat select built, for every type', () => {
    for (const entry of pickerEntries) expect(defaultBindingAction(entry.type).type).toBe(entry.type === 'obs' ? 'obs_scene' : entry.type === 'streamlabs' ? 'streamlabs_scene' : entry.type);
    expect(defaultBindingAction('shortcut')).toEqual({ type: 'shortcut', key: 'c', modifiers: ['command'] });
    expect(defaultBindingAction('multi_action')).toEqual({ type: 'multi_action', actions: [{ type: 'toggle_mute' }, { type: 'shortcut', key: 'c', modifiers: ['command'] }] });
    expect(defaultBindingAction('profile')).toEqual({ type: 'profile', direction: 'next' });
    expect(defaultBindingAction('timer')).toEqual({ type: 'timer', seconds: 300 });
    expect(defaultBindingAction('volume_from_position')).toEqual({ type: 'volume_from_position' });
    expect(defaultBindingAction('mic_input_volume_from_position')).toEqual({ type: 'mic_input_volume_from_position' });
    expect(defaultBindingAction('obs_input_volume_from_position')).toEqual({ type: 'obs_input_volume_from_position', input: 'Mic/Aux' });
    // The OBS entry starts from the scene operation, which the editor's Operation dropdown changes.
    expect(defaultBindingAction('obs')).toEqual({ type: 'obs_scene', scene: 'Scene' });
    expect(defaultBindingAction('streamlabs')).toEqual({ type: 'streamlabs_scene', scene: 'Scene' });
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
    recordRecentAction(storage, 'obs');
    expect(readRecentActions(storage)).toEqual(['obs', 'profile', 'shortcut', 'system']);
    // A stored obs_* type from before the one-entry picker is dropped, not shown.
    const legacy = memoryStorage({ [recentActionsStorageKey]: '["obs_filter","volume"]' });
    expect(readRecentActions(legacy)).toEqual(['volume']);
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
    writeActionDrag(dataTransfer as unknown as DataTransfer, 'obs');
    expect(stored[actionDragMime]).toBe('obs');
    expect(offersActionDrag(dataTransfer)).toBe(true);
    expect(readActionDrag(dataTransfer)).toBe('obs');
  });

  it('ignores foreign payloads', () => {
    expect(offersActionDrag({ types: ['text/plain'] })).toBe(false);
    expect(readActionDrag({ getData: () => 'text/plain' })).toBeNull();
    expect(readActionDrag({ getData: () => 'not-a-type' })).toBeNull();
  });
});
