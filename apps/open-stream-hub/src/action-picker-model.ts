import type { BindingAction, ServiceModule } from './contracts';
import { defaultMacro, defaultMultiAction, defaultProfileAction, defaultStepAction, defaultTimerAction, stepActionTypes } from './action-model';
import type { StepActionType } from './action-model';

// One file per picker entry: the step actions plus the binding-only ones, and the one
// OBS entry ('obs'), which stands in for the ten obs_* step action types.
export type PickerActionType = StepActionType | 'multi_action' | 'profile' | 'timer' | 'macro' | 'volume_from_position' | 'mic_input_volume_from_position' | 'obs_input_volume_from_position' | 'obs';

export type PickerCategory = 'audio' | 'keyboard' | 'apps' | 'media' | 'system' | 'obs' | 'sequences' | 'navigation';

// Category order is the picker's display order.
export const pickerCategories: ReadonlyArray<readonly [PickerCategory, string]> = [
  ['audio', 'Audio'], ['keyboard', 'Keyboard'], ['apps', 'Apps & web'], ['media', 'Media'], ['system', 'System'],
  ['obs', 'OBS'], ['sequences', 'Timers & sequences'], ['navigation', 'Navigation'],
];

// Labels stay identical to the flat list they replace, so existing profiles and tests keep their wording.
const pickerLabels = new Map<string, string>([...stepActionTypes, ['multi_action', 'Multi-action'], ['profile', 'Switch profile'],
  ['timer', 'Timer'], ['macro', 'Recorded macro'], ['volume_from_position', 'Output volume follows position'], ['mic_input_volume_from_position', 'Microphone volume follows position'],
  ['obs_input_volume_from_position', 'OBS input volume follows position'], ['obs', 'OBS']]);

export interface PickerEntry { type: PickerActionType; label: string; category: PickerCategory; keywords: string }

// Extra search terms beyond the label ("spotify" finds Media control, "countdown" finds Timer).
const pickerEntrySpecs: ReadonlyArray<readonly [PickerActionType, PickerCategory, string]> = [
  ['volume', 'audio', 'louder quieter softer up down'],
  ['set_volume', 'audio', 'percent level fixed'],
  ['toggle_mute', 'audio', 'mute unmute silence sound'],
  ['volume_from_position', 'audio', 'fader position volume follow'],
  ['mic_input_volume_from_position', 'audio', 'fader position microphone mic input gain level follow'],
  ['obs_input_volume_from_position', 'obs', 'fader position follow input volume level mixer mic audio'],
  ['shortcut', 'keyboard', 'hotkey key combination command press'],
  ['text', 'keyboard', 'type snippet clipboard string enter'],
  ['launch_app', 'apps', 'launch app program start bundle'],
  ['open_url', 'apps', 'website link address browser http web page'],
  ['media', 'media', 'play pause next previous track stop music spotify itunes'],
  ['system', 'system', 'undo redo screenshot clipboard'],
  // One OBS entry: the operation is chosen in the editor afterwards. The keywords keep the nine
  // operations findable ("camera", "filter", "replay" all land here).
  ['obs', 'obs', 'scene switch program virtual camera webcam replay buffer save output studio mode preview transition cut filter source enable disable visibility show hide collection profile settings stream'],
  ['timer', 'sequences', 'countdown alarm reminder notification wait'],
  ['multi_action', 'sequences', 'sequence steps batch several queue'],
  ['macro', 'sequences', 'record macro keystrokes timing timeline replay type'],
  ['profile', 'navigation', 'next previous saved profiles change'],
];

export const pickerEntries: ReadonlyArray<PickerEntry> = pickerEntrySpecs.map(([type, category, keywords]) => ({
  type, category, keywords, label: pickerLabels.get(type) ?? type,
}));

const entryFor = (type: PickerActionType) => pickerEntries.find(entry => entry.type === type);

// The catalogue entry for a type, for rendering the recent list.
export function pickerEntry(type: PickerActionType): PickerEntry | undefined {
  return entryFor(type);
}

// The trigger's text for a bound action: the one OBS entry stands in for every bound obs_* type,
// so the trigger and the editor's Operation dropdown never say the same thing twice.
export function pickerEntryLabel(type: PickerActionType): string {
  return entryFor(type)?.label ?? (type.startsWith('obs_') ? 'OBS' : type);
}

// Whether the catalogue entry is the one bound: the OBS entry matches every obs_* action.
export function pickerEntryIsCurrent(entry: PickerActionType, action: PickerActionType | undefined): boolean {
  return entry === action || (entry === 'obs' && !!action?.startsWith('obs_') && action !== 'obs_input_volume_from_position');
}

// Which catalogue the picker offers: faders take only the position-driven actions, multi-action
// steps cannot nest multi-actions, profiles or timers, and every other control offers all but the fader actions.
export type PickerScope = 'binding' | 'binding-fader' | 'step';

const faderTypes: ReadonlyArray<PickerActionType> = ['volume_from_position', 'mic_input_volume_from_position', 'obs_input_volume_from_position'];
// A macro is a binding-level sequence like a multi-action: never a step.
const stepExcluded: ReadonlyArray<PickerActionType> = ['multi_action', 'macro', 'profile', 'timer', ...faderTypes];

export function entriesForScope(scope: PickerScope): ReadonlyArray<PickerEntry> {
  if (scope === 'binding-fader') return pickerEntries.filter(entry => faderTypes.includes(entry.type));
  if (scope === 'step') return pickerEntries.filter(entry => !stepExcluded.includes(entry.type));
  return pickerEntries.filter(entry => !faderTypes.includes(entry.type));
}

// Whether a type may be dropped on (or assigned to) a control of this module.
export function isValidForControl(type: PickerActionType, module: ServiceModule): boolean {
  return module === 'm005s' ? faderTypes.includes(type) : !faderTypes.includes(type);
}

// Case-insensitive; every whitespace-separated word of the query must appear in the label,
// the keywords or the action type, in any order. An empty query keeps everything.
export function searchPickerEntries(query: string, entries: ReadonlyArray<PickerEntry>): ReadonlyArray<PickerEntry> {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return entries;
  return entries.filter(entry => {
    const text = `${entry.label} ${entry.keywords} ${entry.type}`.toLowerCase();
    return words.every(word => text.includes(word));
  });
}

export interface PickerGroup { category: PickerCategory; label: string; entries: ReadonlyArray<PickerEntry> }

// The visible entries grouped by category, in display order; empty groups are dropped.
export function pickerGroups(query: string, scope: PickerScope): PickerGroup[] {
  const visible = searchPickerEntries(query, entriesForScope(scope));
  return pickerCategories
    .map(([category, label]) => ({ category, label, entries: visible.filter(entry => entry.category === category) }))
    .filter(group => group.entries.length > 0);
}

// The action a picker choice starts from: the type's default parameters, as the flat
// select's onChange built them. The OBS entry starts from the scene operation; the editor's
// Operation dropdown changes it from there.
export function defaultBindingAction(type: PickerActionType): BindingAction {
  return type === 'volume_from_position' || type === 'mic_input_volume_from_position' ? { type } : type === 'obs_input_volume_from_position' ? { type, input: 'Mic/Aux' } : type === 'multi_action' ? defaultMultiAction()
    : type === 'profile' ? defaultProfileAction() : type === 'timer' ? defaultTimerAction()
    : type === 'macro' ? defaultMacro()
    : type === 'obs' ? defaultStepAction('obs_scene') : defaultStepAction(type);
}

// Recently assigned types, newest first, shown at the top of the picker. The list is a
// nicety: unreadable or missing storage yields an empty list and a failed write is dropped.
export type StorageLike = Pick<Storage, 'getItem' | 'setItem'>;

export const recentActionsStorageKey = 'open-stream-hub.recent-actions';
export const recentActionLimit = 4;

export const pickerWebStorage: StorageLike = {
  getItem: key => globalThis.localStorage?.getItem(key) ?? null,
  setItem: (key, value) => globalThis.localStorage?.setItem(key, value),
};

export function readRecentActions(storage: StorageLike): PickerActionType[] {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(recentActionsStorageKey) ?? 'null');
    if (!Array.isArray(parsed)) return [];
    return parsed.filter((item): item is PickerActionType => typeof item === 'string' && !!entryFor(item as PickerActionType));
  } catch { return []; }
}

export function recordRecentAction(storage: StorageLike, type: PickerActionType): void {
  try {
    const next = [type, ...readRecentActions(storage).filter(item => item !== type)].slice(0, recentActionLimit);
    storage.setItem(recentActionsStorageKey, JSON.stringify(next));
  } catch { /* The recent list must never block assigning. */ }
}

// The picker's entries carry their type to a drop target as a custom payload, so unrelated
// page drags (text, images) are ignored by the controls.
export const actionDragMime = 'application/x-open-stream-hub-action';

export function writeActionDrag(dataTransfer: DataTransfer, type: PickerActionType): void {
  dataTransfer.setData(actionDragMime, type);
  dataTransfer.effectAllowed = 'copy';
}

// During dragover, data is unreadable but the offered types are visible.
export function offersActionDrag(dataTransfer: { types: readonly string[] }): boolean {
  return dataTransfer.types.includes(actionDragMime);
}

export function readActionDrag(dataTransfer: { getData(name: string): string }): PickerActionType | null {
  const value = dataTransfer.getData(actionDragMime);
  return !!entryFor(value as PickerActionType) ? value as PickerActionType : null;
}
