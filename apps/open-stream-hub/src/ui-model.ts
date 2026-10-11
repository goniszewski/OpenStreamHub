import type { Action, Binding, BindingAction, DialFunctionSwitch, Gesture, Profile, ReplayKind, ServiceModule } from './contracts';
import { keyLabel, mediaLabels, obsOutputOpLabels, obsReplayBufferOpLabels, obsSwitchOpLabels, profileDirectionLabels, systemLabels, timerDuration } from './action-model';

export type ModuleId = ServiceModule;
export type Appearance = 'system' | 'light' | 'dark';
export type Page = 'workspace' | 'artwork' | 'lighting' | 'compatibility' | 'settings' | 'about';

export const appearanceStorageKey = 'open-stream-hub.appearance';

export function readAppearance(storage: Pick<Storage, 'getItem'>): Appearance {
  try {
    const saved = storage.getItem(appearanceStorageKey) ?? storage.getItem('masterhub.appearance');
    return saved === 'dark' || saved === 'light' ? saved : 'system';
  } catch { return 'system'; }
}

// Automatic saving is on unless it was turned off: an edit should survive a
// quit without a save step, and the earlier versions make it reversible.
export const autosaveStorageKey = 'open-stream-hub.autosave';

export function readAutosaveSetting(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(autosaveStorageKey) !== 'off'; } catch { return true; }
}

export function writeAutosaveSetting(storage: Pick<Storage, 'setItem'> | null, on: boolean) {
  try { storage?.setItem(autosaveStorageKey, on ? 'on' : 'off'); } catch { /* the toggle still works for this session */ }
}

// "Switch to Test mode while editing" is on unless it was turned off: it is
// the safe choice, and the badge always says when it is holding the mode.
export const testWhileEditingStorageKey = 'open-stream-hub.test-while-editing';

export function readTestWhileEditingSetting(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(testWhileEditingStorageKey) !== 'off'; } catch { return true; }
}

export function writeTestWhileEditingSetting(storage: Pick<Storage, 'setItem'> | null, on: boolean) {
  try { storage?.setItem(testWhileEditingStorageKey, on ? 'on' : 'off'); } catch { /* the toggle still works for this session */ }
}

// When a version was saved, for the "Earlier versions" list: a clock time
// today, otherwise the date and the clock time.
export function versionTimeLabel(modifiedAtMs: number, now = Date.now()): string {
  const time = (date: Date) => `${date.getHours()}:${String(date.getMinutes()).padStart(2, '0')}`;
  const saved = new Date(modifiedAtMs);
  const current = new Date(now);
  if (saved.toDateString() === current.toDateString()) return time(saved);
  const day = saved.getDate();
  const month = saved.toLocaleString('en', { month: 'short' });
  return `${day} ${month}, ${time(saved)}`;
}

// What a kept earlier version holds, against the open profile: its binding count with the difference, and its name when that
// differs ("12 bindings (2 more than now)", "Everyday · 9 bindings (same as now)"). A file that cannot be read says so.
export function versionSummary(version: { name: string | null; binding_count: number | null }, current: { name: string; bindingCount: number }): string {
  if (version.binding_count === null) return 'Cannot be read as a profile';
  const count = version.binding_count;
  const plural = (value: number) => `${value} ${value === 1 ? 'binding' : 'bindings'}`;
  const gap = count - current.bindingCount;
  const against = gap === 0 ? 'same as now' : `${Math.abs(gap)} ${gap > 0 ? 'more' : 'fewer'} than now`;
  const named = version.name !== null && version.name !== current.name ? `“${version.name}” · ` : '';
  return `${named}${plural(count)} (${against})`;
}

export const modules: { id: ModuleId; name: string; shortName: string; art: string; supported: boolean; input: string }[] = [
  { id: 'm131e', name: 'Encoder with IPS Display', shortName: 'Encoder IPS display', art: 'encoder', supported: true, input: 'Rotate · Press · Release' },
  { id: 'm4315', name: '15x Key IPS Display', shortName: '15-key IPS display', art: 'keys', supported: true, input: 'Press · Release' },
  { id: 'm003k', name: '3x Knob', shortName: '3-knob module', art: 'knob', supported: true, input: 'Rotate · Press · Release' },
  { id: 'm005s', name: '5x Fader Precision', shortName: '5-fader precision module', art: 'faders', supported: true, input: 'Position · 0–100' },
  { id: 'm032le', name: '2x Roller Scroll', shortName: '2-roller scroll module', art: 'rollers', supported: true, input: 'Up · Down' },
];

export const gestureLabels: Record<Gesture, string> = {
  clockwise: 'Clockwise', counter_clockwise: 'Counter-clockwise', pressed: 'Press', released: 'Release',
  right: 'Right', left: 'Left', short_press: 'Short press', long_press: 'Long press',
  position_changed: 'Position', up: 'Up', down: 'Down',
};

export const gestures: Record<ServiceModule, Gesture[]> = {
  m003k: ['clockwise', 'counter_clockwise', 'pressed', 'released'],
  m131e: ['right', 'left', 'pressed', 'released', 'short_press', 'long_press'],
  m4315: ['pressed', 'released'],
  m005s: ['position_changed'],
  m032le: ['up', 'down'],
};

export const controlCounts: Record<ServiceModule, number> = { m003k: 3, m131e: 1, m4315: 15, m005s: 5, m032le: 2 };
export const faderMaximum = 100;
export const replayLabels: Record<ReplayKind, string> = {
  knobs: '3-knob module sample', dial: 'Encoder module sample', keys: '15-key module sample',
  faders: '5-fader module sample', rollers: '2-roller module sample',
};

export function moduleNote(module: ServiceModule) {
  switch (module) {
    case 'm003k': return 'Knob order verified on hardware: with the single knob on top, Knob 3 is on top, Knob 1 bottom-left and Knob 2 bottom-right.';
    case 'm4315': case 'm005s': return 'Control positions will be checked with the device.';
    case 'm131e': return 'Physical Right/Left rotation will be checked with the device.';
    case 'm032le': return 'Roller order and physical Up/Down direction will be checked with the device.';
  }
}

export function controlLabel(module: ServiceModule, control: number) {
  switch (module) {
    case 'm003k': return `Knob ${control + 1}`;
    case 'm131e': return 'Encoder';
    case 'm4315': return `Key ${control + 1}`;
    case 'm005s': return `Fader ${control + 1}`;
    case 'm032le': return `Roller ${control + 1}`;
  }
}

// 'Volume → Brightness (2/3)': index is the zero-based new current function.
export function dialSwitchLabel(change: DialFunctionSwitch): string {
  return `${change.from_function} → ${change.to_function} (${change.index + 1}/${change.count})`;
}

// By code points (never splits an emoji). The diagnostics scrub removes this same 24-character prefix (OBS_LABEL_PREFIX_CHARS).
const clipLabel = (name: string) => { const characters = Array.from(name); return characters.length > 24 ? `${characters.slice(0, 24).join('')}…` : name; };

export function actionLabel(action: Action | BindingAction | null): string {
  if (!action) return 'Unbound';
  switch (action.type) {
    case 'volume': return `Output volume ${action.delta > 0 ? '+' : ''}${action.delta} points`;
    case 'toggle_mute': return 'Toggle output mute';
    case 'shortcut': return `${action.modifiers.map(m => m[0].toUpperCase() + m.slice(1)).join(' + ')}${action.modifiers.length ? ' + ' : ''}${keyLabel(action.key)}`;
    case 'launch_app': return `Open ${action.bundle_id}`;
    case 'volume_from_position': return 'Follow fader position for output volume';
    case 'obs_input_volume_from_position': return `Follow fader position for OBS input “${clipLabel(action.input)}”`;
    case 'obs_set_input_volume': return `Set OBS input “${clipLabel(action.input)}” to ${action.percent}%`;
    case 'set_volume': return `Set output volume to ${action.percent}%`;
    case 'open_url': return `Open ${action.url}`;
    case 'media': return mediaLabels[action.command];
    case 'system': return systemLabels[action.command];
    case 'text': {
      const flat = action.text.replace(/\s+/g, ' ').trim();
      return `Paste “${flat.length > 24 ? `${flat.slice(0, 24)}…` : flat}”${action.press_enter ? ' + Return' : ''}`;
    }
    case 'multi_action': return `${action.actions.length} actions: ${action.actions.map(step => actionLabel(step)).join(', then ')}`;
    case 'profile': return profileDirectionLabels[action.direction] ?? 'Switch profile';
    case 'timer': return `Timer ${timerDuration(action.seconds)}${action.label ? ` “${action.label}”` : ''}`;
    case 'obs_virtual_camera': return `OBS virtual camera: ${obsOutputOpLabels[action.op] ?? action.op}`;
    case 'obs_replay_buffer': return `OBS replay buffer: ${obsReplayBufferOpLabels[action.op] ?? action.op}`;
    case 'obs_studio_mode': return `OBS studio mode: ${obsSwitchOpLabels[action.op] ?? action.op}`;
    case 'obs_transition': return 'OBS studio transition';
    case 'obs_filter': return `OBS filter “${clipLabel(action.filter)}”: ${obsSwitchOpLabels[action.op] ?? action.op}`;
    case 'obs_source_visibility': return `OBS source “${clipLabel(action.source)}”: ${obsSwitchOpLabels[action.op] ?? action.op}`;
    case 'obs_scene_collection': return `OBS scene collection “${clipLabel(action.name)}”`;
    case 'obs_profile': return `OBS profile “${clipLabel(action.name)}”`;
    case 'obs_scene': return `OBS scene “${clipLabel(action.scene)}”`;
  }
}

export interface BindingSelection { module: ServiceModule; control: number; gesture: Gesture; serial: string }

export function sameSelector(binding: Binding, selection: BindingSelection) {
  return binding.module === selection.module && binding.control === selection.control &&
    binding.gesture === selection.gesture && (binding.serial_number ?? '') === selection.serial;
}

// This edits the exact selector in a draft. Matching precedence and validation stay in Rust.
export function editBinding(profile: Profile, selection: BindingSelection, action: BindingAction | null): Profile {
  const index = profile.bindings.findIndex(binding => sameSelector(binding, selection));
  const bindings = [...profile.bindings];
  if (!action) {
    if (index !== -1) bindings.splice(index, 1);
  } else {
    const binding: Binding = {
      module: selection.module, control: selection.control, gesture: selection.gesture,
      ...(selection.serial ? { serial_number: selection.serial } : {}), action,
    };
    if (index === -1) bindings.push(binding); else bindings[index] = binding;
  }
  return { ...profile, bindings };
}

export const browserProfile: Profile = {
  schema_version: 1, name: 'Everyday', bindings: [
    { module: 'm003k', control: 0, gesture: 'clockwise', action: { type: 'volume', delta: 5 } },
    { module: 'm003k', control: 0, gesture: 'counter_clockwise', action: { type: 'volume', delta: -5 } },
    { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
    { module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 5 } },
  ],
};

export function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : String(error);
}
