// What the workspace tooltip says about a control: each bound action's name (actionLabel) and a one-line description of
// what that kind of action does. Pure, so it is tested without a DOM.
import type { Action, BindingAction, Gesture, KeypadPage, KeypadSlot, Profile } from './contracts';
import { english } from './i18n-react';
import type { MessageKey } from './locales/en';
import { actionLabel, controlLabel, gestureLabels, gestures } from './ui-model';
import type { ModuleId } from './ui-model';

export type ActionType = BindingAction['type'];

// A key per action type: the compiler fails here when a new action type has no description.
const descriptionKeys: Record<ActionType, MessageKey> = {
  volume: 'tip.desc.volume', set_volume: 'tip.desc.set_volume', toggle_mute: 'tip.desc.toggle_mute', shortcut: 'tip.desc.shortcut',
  launch_app: 'tip.desc.launch_app', open_url: 'tip.desc.open_url', text: 'tip.desc.text', media: 'tip.desc.media', system: 'tip.desc.system',
  volume_from_position: 'tip.desc.volume_from_position', mic_input_volume_from_position: 'tip.desc.mic_input_volume_from_position',
  obs_input_volume_from_position: 'tip.desc.obs_input_volume_from_position', multi_action: 'tip.desc.multi_action', conditional: 'tip.desc.conditional',
  macro: 'tip.desc.macro', profile: 'tip.desc.profile', timer: 'tip.desc.timer', terminal_command: 'tip.desc.terminal_command',
  set_variable: 'tip.desc.set_variable', toggle_variable: 'tip.desc.toggle_variable', add_to_variable: 'tip.desc.add_to_variable',
  obs_scene: 'tip.desc.obs_scene', obs_virtual_camera: 'tip.desc.obs_virtual_camera', obs_replay_buffer: 'tip.desc.obs_replay_buffer',
  obs_stream: 'tip.desc.obs_stream', obs_record: 'tip.desc.obs_record', obs_studio_mode: 'tip.desc.obs_studio_mode', obs_transition: 'tip.desc.obs_transition',
  obs_screenshot: 'tip.desc.obs_screenshot', obs_filter: 'tip.desc.obs_filter', obs_source_visibility: 'tip.desc.obs_source_visibility',
  obs_scene_collection: 'tip.desc.obs_scene_collection', obs_profile: 'tip.desc.obs_profile', obs_set_input_volume: 'tip.desc.obs_set_input_volume',
  obs_input_mute: 'tip.desc.obs_input_mute', obs_adjust_input_volume: 'tip.desc.obs_adjust_input_volume', obs_media_source: 'tip.desc.obs_media_source',
  streamlabs_record: 'tip.desc.streamlabs_record', streamlabs_stream: 'tip.desc.streamlabs_stream', streamlabs_studio_mode: 'tip.desc.streamlabs_studio_mode',
  streamlabs_transition: 'tip.desc.streamlabs_transition', streamlabs_scene: 'tip.desc.streamlabs_scene',
  streamlabs_source_visibility: 'tip.desc.streamlabs_source_visibility', streamlabs_audio: 'tip.desc.streamlabs_audio',
  run_program: 'tip.desc.run_program', plugin: 'tip.desc.plugin',
};

export const actionDescription = (action: BindingAction | Action): string => english(descriptionKeys[action.type]);

// One line of the tooltip: the action, and the gesture it answers when the control has several.
export interface TipRow { gesture: string | null; name: string; description: string }
export interface ControlTip { title: string; rows: TipRow[] }

const row = (gesture: string | null, action: BindingAction | Action): TipRow => ({ gesture, name: actionLabel(action), description: actionDescription(action) });
const unassigned = (title: string): ControlTip => ({ title, rows: [{ gesture: null, name: english('tip.unassigned'), description: english('tip.unassignedHint') }] });

// A control's bindings, one per gesture in the module's gesture order (an exact-serial binding shadows the any-serial one in the service).
function gestureRows(items: ReadonlyArray<{ gesture: Gesture; action: BindingAction | Action }>, order: readonly Gesture[]): TipRow[] {
  const rows: TipRow[] = [];
  const seen = new Set<Gesture>();
  const sorted = [...items].filter(item => order.includes(item.gesture)).sort((a, b) => order.indexOf(a.gesture) - order.indexOf(b.gesture));
  for (const item of sorted) {
    if (seen.has(item.gesture)) continue;
    seen.add(item.gesture);
    rows.push(row(gestureLabels[item.gesture], item.action));
  }
  // A single gesture needs no name: a key's press, a fader's position.
  return rows.length === 1 ? [{ ...rows[0], gesture: null }] : rows;
}

// A tooltip is a glance, not a list: past this many rows (a dial with many functions) the rest is summed up in one line.
export const maxTipRows = 6;
function capRows(tip: ControlTip): ControlTip {
  if (tip.rows.length <= maxTipRows) return tip;
  const hidden = tip.rows.length - (maxTipRows - 1);
  return { ...tip, rows: [...tip.rows.slice(0, maxTipRows - 1), { gesture: null, name: english('tip.more', { count: hidden }), description: english('tip.moreHint') }] };
}

export interface TipContext {
  // The keypad page on screen and the pages it can open (keys live on pages, not in profile.bindings).
  keypadPage?: KeypadPage;
  keypadPages?: ReadonlyArray<KeypadPage>;
  keypadBack?: boolean;
}

export const controlTip = (profile: Profile, module: ModuleId, control: number, context: TipContext = {}): ControlTip => capRows(buildTip(profile, module, control, context));

function buildTip(profile: Profile, module: ModuleId, control: number, context: TipContext): ControlTip {
  const title = controlLabel(module, control);
  if (module === 'm4315') return keypadTip(title, context.keypadPage?.slots[control] ?? null, context);
  if (module === 'm131e' && profile.dial_functions?.length) {
    const rows = profile.dial_functions.flatMap(item => item.bindings.map(binding => row(english('tip.dialFunction', { name: item.name, gesture: gestureLabels[binding.gesture] }), binding.action)));
    return rows.length ? { title, rows } : unassigned(title);
  }
  const rows = gestureRows(profile.bindings.filter(binding => binding.module === module && binding.control === control), gestures[module]);
  return rows.length ? { title, rows } : unassigned(title);
}

function keypadTip(title: string, slot: KeypadSlot, context: TipContext): ControlTip {
  if (context.keypadBack) return { title, rows: [{ gesture: null, name: english('tip.back'), description: english('tip.backHint') }] };
  if (slot?.type === 'folder') {
    const page = context.keypadPages?.find(item => item.id === slot.page_id);
    return { title, rows: [{ gesture: null, name: english('tip.folder', { name: page?.name ?? slot.page_id }), description: english('tip.folderHint', { count: (page?.slots ?? []).filter(inner => inner !== null).length }) }] };
  }
  if (slot?.type !== 'action') return unassigned(title);
  const rows = gestureRows(slot.bindings, ['pressed', 'released']);
  return rows.length ? { title: slot.label || title, rows } : unassigned(title);
}
