import { groupItems, groupMode, isGroup } from './flow-model';
import { english } from './i18n-react';
import type { Action, BindingAction, Condition, LeafCondition, Variable, VariableValue, FaderTemplate, MacroStep, MediaCommand, MediaKey, ObsNameList, ObsMediaOp, ObsMuteOp, ObsOutputOp, ObsRecordOp, ObsReplayBufferOp, ObsSwitchOp, ProfileDirection, StepAction, StreamlabsAudioOp, StreamlabsVisibilityOp, SystemCommand, TerminalApp, TimerSound } from './contracts';

// Mirrors NAMED_KEYS in crates/cooltek-sdk-service/src/profile.rs. The Rust
// service holds the authoritative allow-list and rejects anything else; the
// shared fixture test keeps both tables identical, in order.
export const namedKeys: ReadonlyArray<readonly [string, string]> = [
  ['return', 'Return'], ['tab', 'Tab'], ['space', 'Space'], ['delete', 'Delete'], ['escape', 'Escape'],
  ['forward_delete', 'Forward Delete'], ['home', 'Home'], ['end', 'End'], ['page_up', 'Page Up'], ['page_down', 'Page Down'],
  ['arrow_left', 'Left Arrow'], ['arrow_right', 'Right Arrow'], ['arrow_down', 'Down Arrow'], ['arrow_up', 'Up Arrow'],
  ...Array.from({ length: 20 }, (_, index) => [`f${index + 1}`, `F${index + 1}`] as const),
  // The Apple extended keypad (kVK_ANSI_Keypad*). F13–F20 are the last function keys macOS gives
  // virtual key codes; F21–F24 have none, so no key can send them.
  ...Array.from({ length: 10 }, (_, index) => [`keypad_${index}`, `Keypad ${index}`] as const),
  ['keypad_decimal', 'Keypad .'], ['keypad_multiply', 'Keypad *'], ['keypad_plus', 'Keypad +'],
  ['keypad_clear', 'Keypad Clear'], ['keypad_divide', 'Keypad /'], ['keypad_enter', 'Keypad Enter'],
  ['keypad_minus', 'Keypad −'], ['keypad_equals', 'Keypad ='],
];

// Mirrors MEDIA_KEYS in crates/cooltek-sdk-service/src/profile.rs: shortcut keys sent as hardware
// media keys. They have no virtual key code, cannot combine with modifiers, and cannot be macro
// keys (a recording cannot see them; the media action sends them on its own).
export const mediaKeyLabels: Record<MediaKey, string> = {
  play_pause: english('labels.mediaKey.playPause'), next_track: english('labels.mediaKey.nextTrack'), previous_track: english('labels.mediaKey.previousTrack'),
};

export const isMediaKey = (key: string): key is MediaKey => key in mediaKeyLabels;

export function keyLabel(key: string) {
  if (isMediaKey(key)) return mediaKeyLabels[key];
  return namedKeys.find(([name]) => name === key)?.[1] ?? (key === ' ' ? 'Space' : key.toUpperCase());
}

export const mediaLabels: Record<MediaCommand, string> = {
  play_pause: english('labels.media.playPause'), next_track: english('labels.media.nextTrack'), previous_track: english('labels.media.previousTrack'), stop: english('labels.media.stop'),
};

export const systemLabels: Record<SystemCommand, string> = {
  undo: english('labels.system.undo'), redo: english('labels.system.redo'), screenshot: english('labels.system.screenshot'),
};

// Mirrors ProfileDirection in crates/cooltek-sdk-service/src/profile.rs (a closed set).
export const profileDirectionLabels: Record<ProfileDirection, string> = {
  next: english('labels.profileDirection.next'), previous: english('labels.profileDirection.previous'),
};

// Mirrors TimerSound in crates/cooltek-sdk-service/src/profile.rs (a closed set, in order). Each maps to a fixed
// /System/Library/Sounds file in the service; a profile can never name a path.
export const timerSoundLabels: Record<TimerSound, string> = {
  basso: english('labels.timerSound.basso'), blow: english('labels.timerSound.blow'), bottle: english('labels.timerSound.bottle'),
  frog: english('labels.timerSound.frog'), funk: english('labels.timerSound.funk'), glass: english('labels.timerSound.glass'),
  hero: english('labels.timerSound.hero'), morse: english('labels.timerSound.morse'), ping: english('labels.timerSound.ping'),
  pop: english('labels.timerSound.pop'), purr: english('labels.timerSound.purr'), sosumi: english('labels.timerSound.sosumi'),
  submarine: english('labels.timerSound.submarine'), tink: english('labels.timerSound.tink'), none: english('labels.timerSound.none'),
};
export const defaultTimerSound: TimerSound = 'glass';

// Limits enforced by the Rust service (Action::MAX_* constants).
export const maxUrlBytes = 2048;
export const maxTextBytes = 2048;
export const multiActionSteps = { minimum: 2, maximum: 8 } as const;
// Conditional branches (Action::MAX_MULTI_ACTION_STEPS): 1 to 8 steps when the condition holds, 0 to 8 otherwise.
export const conditionalSteps = { thenMinimum: 1, maximum: 8 } as const;
export const timerSeconds = { minimum: 1, maximum: 86_400 } as const;
export const maxTimerLabelBytes = 64;
// TimerTable::MAX_RUNNING: more presses fail until one ends or is cancelled.
export const maxRunningTimers = 8;
export const maxObsSceneBytes = 128;
// A recorded macro: 1–64 keystrokes, per-step wait and hold bounds, and a
// total of waits and holds (Action::MAX_MACRO_* in profile.rs).
export const macroLimits = {
  steps: { minimum: 1, maximum: 64 },
  delayMs: { minimum: 0, maximum: 10_000 },
  holdMs: { minimum: 0, maximum: 5_000 },
  durationMs: 30_000,
} as const;

// The whole wall-clock time a macro plans to spend: waits plus holds.
export function macroDurationMs(steps: MacroStep[]): number {
  return steps.reduce((total, step) => total + step.delay_before_ms + step.hold_ms, 0);
}

// M:SS (m:ss.decimal below ten seconds) from milliseconds, for the macro's
// planned duration in the editor and the labels.
export function macroDurationLabel(ms: number): string {
  const total = Math.round(ms / 100) / 10;
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  const text = Number.isInteger(seconds) ? String(seconds).padStart(2, '0') : seconds.toFixed(1).padStart(4, '0');
  return `${minutes}:${text}`;
}

export function defaultMacro(): BindingAction {
  return { type: 'macro', steps: [{ key: 'c', modifiers: ['command'], delay_before_ms: 0, hold_ms: 80 }] };
}

// Mirrors the macro rules of Action::validate (profile.rs): inline hints only,
// the Rust validator is authoritative and runs on save.
export function macroIssue(steps: MacroStep[]): string | null {
  if (steps.length < macroLimits.steps.minimum || steps.length > macroLimits.steps.maximum) {
    return `A macro holds ${macroLimits.steps.minimum} to ${macroLimits.steps.maximum} keystrokes.`;
  }
  let total = 0;
  for (const [index, step] of steps.entries()) {
    const named = namedKeys.some(([name]) => name === step.key);
    const literal = step.key.length === 1 && /[a-z0-9 ]/i.test(step.key);
    if (!literal && !named) {
      return isMediaKey(step.key)
        ? `Step ${index + 1}: media keys cannot be macro keys. Use the Media control action instead.`
        : `Step ${index + 1}: pick a key the Mac can send.`;
    }
    if (step.modifiers.length > 4) return `Step ${index + 1}: at most four modifiers.`;
    if (new Set(step.modifiers).size !== step.modifiers.length) return `Step ${index + 1}: each modifier once.`;
    if (step.delay_before_ms < macroLimits.delayMs.minimum || step.delay_before_ms > macroLimits.delayMs.maximum) {
      return `Step ${index + 1}: the wait must be 0 to ${macroLimits.delayMs.maximum.toLocaleString('en-US')} ms.`;
    }
    if (step.hold_ms < macroLimits.holdMs.minimum || step.hold_ms > macroLimits.holdMs.maximum) {
      return `Step ${index + 1}: the hold must be 0 to ${macroLimits.holdMs.maximum.toLocaleString('en-US')} ms.`;
    }
    total += step.delay_before_ms + step.hold_ms;
    if (total > macroLimits.durationMs) {
      return `The waits and holds together must total at most ${macroDurationLabel(macroLimits.durationMs)} (m:ss). Trim the pauses.`;
    }
  }
  return null;
}

// Mirror ObsOutputOp and ObsReplayBufferOp in crates/cooltek-sdk-service/src/profile.rs (closed sets).
export const obsOutputOpLabels: Record<ObsOutputOp, string> = { toggle: english('labels.op.toggle'), start: english('labels.op.start'), stop: english('labels.op.stop') };
export const obsSwitchOpLabels: Record<ObsSwitchOp, string> = { toggle: english('labels.op.toggle'), on: english('labels.op.turnOn'), off: english('labels.op.turnOff') };
export const obsMuteOpLabels: Record<ObsMuteOp, string> = { toggle: english('labels.op.toggle'), mute: english('labels.op.mute'), unmute: english('labels.op.unmute') };
export const obsMediaOpLabels: Record<ObsMediaOp, string> = {
  play_pause: english('labels.op.playPause'), play_stop: english('labels.op.playStop'), play_restart: english('labels.op.playRestart'), stop: english('labels.op.stop'),
};
export const obsRecordOpLabels: Record<ObsRecordOp, string> = {
  ...obsOutputOpLabels, pause: english('labels.op.pause'), resume: english('labels.op.resume'), toggle_pause: english('labels.op.togglePause'),
};
export const obsReplayBufferOpLabels: Record<ObsReplayBufferOp, string> = { ...obsOutputOpLabels, save: english('labels.op.save') };

export type StepActionType = StepAction['type'];

// The fader-only templates: each position change resolves into an absolute action (Action::is_fader_template).
export const isFaderTemplate = (action: { type: string } | null | undefined): action is FaderTemplate =>
  action?.type === 'volume_from_position' || action?.type === 'mic_input_volume_from_position' || action?.type === 'obs_input_volume_from_position';

export const stepActionTypes: ReadonlyArray<readonly [StepActionType, string]> = [
  ['volume', 'Change output volume'], ['set_volume', 'Set output volume'], ['toggle_mute', 'Toggle output mute'],
  ['shortcut', 'Keyboard shortcut'], ['launch_app', 'Open application'], ['open_url', 'Open URL'],
  ['text', 'Paste text'], ['media', 'Media control'], ['system', 'System action'], ['obs_scene', 'Switch OBS scene'],
  ['obs_virtual_camera', 'OBS virtual camera'], ['obs_replay_buffer', 'OBS replay buffer'],
  ['obs_stream', 'OBS stream'], ['obs_record', 'OBS recording'],
  ['obs_studio_mode', 'OBS studio mode'], ['obs_transition', 'OBS studio transition'], ['obs_screenshot', 'OBS screenshot'], ['obs_filter', 'OBS filter'],
  ['obs_source_visibility', 'OBS source visibility'], ['obs_scene_collection', 'Switch OBS scene collection'],
  ['obs_profile', 'Switch OBS profile'], ['obs_set_input_volume', 'Set OBS input volume'],
  ['obs_input_mute', 'OBS input mute'], ['obs_adjust_input_volume', 'Change OBS input volume'], ['obs_media_source', 'OBS media source'],
  ['streamlabs_record', 'Streamlabs recording'], ['streamlabs_stream', 'Streamlabs stream'], ['streamlabs_studio_mode', 'Streamlabs studio mode'],
  ['streamlabs_transition', 'Streamlabs studio transition'], ['streamlabs_scene', 'Switch Streamlabs scene'],
  ['streamlabs_source_visibility', 'Streamlabs source visibility'], ['streamlabs_audio', 'Streamlabs audio source'],
  ['set_variable', 'Set a variable'], ['toggle_variable', 'Flip an on / off variable'], ['add_to_variable', 'Add to a number variable'],
];

export function defaultStepAction(type: StepActionType): StepAction {
  switch (type) {
    case 'volume': return { type, delta: 5 };
    case 'set_volume': return { type, percent: 50 };
    case 'toggle_mute': return { type };
    case 'shortcut': return { type, key: 'c', modifiers: ['command'] };
    case 'launch_app': return { type, bundle_id: 'com.apple.finder' };
    case 'open_url': return { type, url: 'https://example.com' };
    case 'text': return { type, text: 'Hello', press_enter: false };
    case 'media': return { type, command: 'play_pause' };
    case 'system': return { type, command: 'undo' };
    case 'obs_scene': return { type, scene: 'Scene' };
    case 'obs_virtual_camera': return { type, op: 'toggle' };
    case 'obs_replay_buffer': return { type, op: 'save' };
    case 'obs_stream': return { type, op: 'toggle' };
    case 'obs_record': return { type, op: 'toggle' };
    case 'obs_studio_mode': return { type, op: 'toggle' };
    case 'obs_transition': return { type };
    case 'obs_screenshot': return { type };
    case 'obs_filter': return { type, source: 'Source', filter: 'Filter', op: 'toggle' };
    case 'obs_source_visibility': return { type, scene: 'Scene', source: 'Source', op: 'toggle' };
    case 'obs_scene_collection': return { type, name: 'Scene collection' };
    case 'obs_profile': return { type, name: 'Profile' };
    case 'obs_set_input_volume': return { type, input: 'Mic/Aux', percent: 100 };
    case 'obs_input_mute': return { type, input: 'Mic/Aux', op: 'toggle' };
    case 'obs_adjust_input_volume': return { type, input: 'Mic/Aux', step_db: 3 };
    case 'obs_media_source': return { type, input: 'Media Source', op: 'play_pause' };
    case 'streamlabs_record': return { type };
    case 'streamlabs_stream': return { type };
    case 'streamlabs_studio_mode': return { type };
    case 'streamlabs_transition': return { type };
    case 'streamlabs_scene': return { type, scene: 'Scene' };
    case 'streamlabs_source_visibility': return { type, scene: 'Scene', source: 'Source', op: 'toggle' };
    case 'streamlabs_audio': return { type, source: 'Mic/Aux', op: { mute: 'toggle' } };
    case 'set_variable': return { type, name: '', value: true };
    case 'toggle_variable': return { type, name: '' };
    case 'add_to_variable': return { type, name: '', amount: 1 };
  }
}

export function defaultProfileAction(): Action {
  return { type: 'profile', direction: 'next' };
}

// The OBS operations of the one OBS editor, in the Operation list's order. The picker keeps
// one entry per operation; the editor groups them behind this dropdown.
export type ObsStepType = Extract<StepActionType, `obs_${string}`>;
export type ObsStepAction = Extract<StepAction, { type: ObsStepType }>;

export const obsOperationLabels: Record<ObsStepType, string> = {
  obs_scene: english('labels.operation.scene'), obs_virtual_camera: english('labels.operation.virtualCamera'),
  obs_replay_buffer: english('labels.operation.replayBuffer'), obs_stream: english('labels.operation.stream'),
  obs_record: english('labels.operation.recording'), obs_studio_mode: english('labels.operation.studioMode'),
  obs_transition: english('labels.operation.transition'), obs_screenshot: english('labels.operation.screenshot'),
  obs_filter: english('labels.operation.filter'), obs_source_visibility: english('labels.operation.sourceVisibility'),
  obs_scene_collection: english('labels.operation.sceneCollection'), obs_profile: english('labels.operation.profile'),
  obs_set_input_volume: english('labels.operation.inputVolume'), obs_input_mute: english('labels.operation.inputMute'),
  obs_adjust_input_volume: english('labels.operation.inputVolumeStep'), obs_media_source: english('labels.operation.mediaSource'),
};

// Switching operation starts from the new operation's defaults and keeps the names the two
// operations share (scene, source, filter, or the collection/profile name), so a scene switch
// becomes a source visibility for the same scene. Operation settings (toggle, start, save) reset.
export function switchObsOperation(action: ObsStepAction, next: ObsStepType): ObsStepAction {
  const target = { ...defaultStepAction(next) } as Record<string, unknown>;
  const previous = action as unknown as Record<string, unknown>;
  for (const field of ['scene', 'source', 'filter', 'name', 'input'] as const) {
    if (typeof previous[field] === 'string' && field in target) target[field] = previous[field];
  }
  return target as unknown as ObsStepAction;
}

// The Streamlabs operations of the one Streamlabs editor, in the Operation list's order, like the OBS ones.
export type StreamlabsStepType = Extract<StepActionType, `streamlabs_${string}`>;
export type StreamlabsStepAction = Extract<StepAction, { type: StreamlabsStepType }>;

export const streamlabsOperationLabels: Record<StreamlabsStepType, string> = {
  streamlabs_scene: english('labels.operation.scene'), streamlabs_record: english('labels.operation.recording'),
  streamlabs_stream: english('labels.operation.stream'), streamlabs_studio_mode: english('labels.operation.studioMode'),
  streamlabs_transition: english('labels.operation.transition'), streamlabs_source_visibility: english('labels.operation.sourceVisibility'),
  streamlabs_audio: english('labels.operation.audioSource'),
};

// Mirror StreamlabsVisibilityOp and StreamlabsAudioOp in crates/cooltek-sdk-service/src/profile.rs (closed sets).
export const streamlabsVisibilityOpLabels: Record<StreamlabsVisibilityOp, string> = { toggle: english('labels.op.toggle'), show: english('labels.op.show'), hide: english('labels.op.hide') };
export type StreamlabsAudioMode = 'set' | 'adjust' | 'mute';
export const streamlabsAudioModeLabels: Record<StreamlabsAudioMode, string> = {
  set: english('labels.audioMode.set'), adjust: english('labels.audioMode.adjust'), mute: english('labels.audioMode.mute'),
};
// The Rust bounds of each audio mode: set 0 to 100, adjust a nonzero -100 to 100 (percent of the fader's travel).
export const streamlabsAudioLimits = { set: { minimum: 0, maximum: 100 }, adjust: { minimum: -100, maximum: 100 } } as const;
export const streamlabsAudioMode = (op: StreamlabsAudioOp): StreamlabsAudioMode => 'set' in op ? 'set' : 'adjust' in op ? 'adjust' : 'mute';
export function defaultStreamlabsAudioOp(mode: StreamlabsAudioMode): StreamlabsAudioOp {
  return mode === 'set' ? { set: 100 } : mode === 'adjust' ? { adjust: 5 } : { mute: 'toggle' };
}
// "Set to 40%", "+5%", "Mute": the label tail of a streamlabs_audio action.
export function streamlabsAudioOpLabel(op: StreamlabsAudioOp): string {
  if ('set' in op) return `Set to ${op.set}%`;
  if ('adjust' in op) return `${op.adjust > 0 ? '+' : ''}${op.adjust}%`;
  return obsMuteOpLabels[op.mute] ?? String(op.mute);
}

// Switching operation keeps the scene and source names both operations have; the rest resets, like switchObsOperation.
export function switchStreamlabsOperation(action: StreamlabsStepAction, next: StreamlabsStepType): StreamlabsStepAction {
  const target = { ...defaultStepAction(next) } as Record<string, unknown>;
  const previous = action as unknown as Record<string, unknown>;
  for (const field of ['scene', 'source'] as const) {
    if (typeof previous[field] === 'string' && field in target) target[field] = previous[field];
  }
  return target as unknown as StreamlabsStepAction;
}

export function defaultTimerAction(): Action {
  return { type: 'timer', seconds: 300 };
}

// H:MM:SS from an hour, M:SS below (format_duration in timer.rs).
export function timerDuration(seconds: number): string {
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor(seconds / 60) % 60;
  const pad = (value: number) => String(value).padStart(2, '0');
  return hours > 0 ? `${hours}:${pad(minutes)}:${pad(seconds % 60)}` : `${minutes}:${pad(seconds % 60)}`;
}

export type ConditionKey = 'obs_streaming' | 'obs_recording' | 'obs_virtual_camera' | 'obs_replay_buffer' | 'output_muted' | 'output_volume_at_least' | 'timer_running' | 'focused_app' | 'variable_is' | 'variable_at_least';
// Picker order. The phrase pair is [true, false] for a yes/no condition and finishes "If …" in the editor and the key label.
export const conditionKeys: ReadonlyArray<readonly [ConditionKey, string]> = [
  ['output_muted', 'The output is muted'], ['output_volume_at_least', 'The output volume is at least'], ['timer_running', 'A timer is running'], ['focused_app', 'The front app is'],
  ['variable_is', 'A variable is'], ['variable_at_least', 'A number variable is at least'],
  ['obs_recording', 'OBS is recording'], ['obs_streaming', 'OBS is streaming'],
  ['obs_virtual_camera', 'The OBS virtual camera is on'], ['obs_replay_buffer', 'The OBS replay buffer is running'],
];
type YesNoKey = Exclude<ConditionKey, 'output_volume_at_least' | 'focused_app' | 'variable_is' | 'variable_at_least'>;
const conditionPhrases: Record<YesNoKey, readonly [string, string]> = {
  obs_streaming: ['OBS is streaming', 'OBS is not streaming'],
  obs_recording: ['OBS is recording', 'OBS is not recording'],
  obs_virtual_camera: ['the OBS virtual camera is on', 'the OBS virtual camera is off'],
  obs_replay_buffer: ['the OBS replay buffer is running', 'the OBS replay buffer is not running'],
  output_muted: ['the output is muted', 'the output is not muted'],
  timer_running: ['a timer is running', 'no timer is running'],
};
export function conditionKey(when: LeafCondition): ConditionKey {
  return Object.keys(when)[0] as ConditionKey;
}
// The single value of a yes/no, number or text test; a variable test holds a name and a value instead.
export function conditionValue(when: LeafCondition): boolean | number | string | { name: string; value: VariableValue } {
  return (when as Record<string, boolean | number | string | { name: string; value: VariableValue }>)[conditionKey(when)];
}
// Reads from OBS, so needs the connection set up in Settings; the other conditions read this Mac or the service.
export const conditionUsesObs = (when: Condition): boolean => isGroup(when) ? groupItems(when).some(conditionUsesObs) : conditionKey(when).startsWith('obs_');
const variableText = (value: VariableValue): string => typeof value === 'boolean' ? (value ? 'on' : 'off') : typeof value === 'string' ? `"${value}"` : String(value);
const leafLabel = (when: LeafCondition): string => {
  const key = conditionKey(when);
  const value = conditionValue(when);
  if (key === 'output_volume_at_least') return `the output volume is at least ${value}%`;
  if (key === 'focused_app') return `the front app is ${value}`;
  if (key === 'variable_is') { const { name, value: wanted } = value as { name: string; value: VariableValue }; return `${name || 'a variable'} is ${variableText(wanted)}`; }
  if (key === 'variable_at_least') { const { name, value: wanted } = value as { name: string; value: VariableValue }; return `${name || 'a variable'} is at least ${wanted}`; }
  return conditionPhrases[key as YesNoKey][value ? 0 : 1];
};
// "OBS is recording", "the output is not muted and OBS is streaming", "takes is at least 3 and (a timer is running or OBS is recording)".
export function conditionLabel(when: Condition, nested = false): string {
  if (!isGroup(when)) return leafLabel(when);
  const text = groupItems(when).map(item => conditionLabel(item, true)).join(groupMode(when) === 'all' ? ' and ' : ' or ');
  return nested ? `(${text})` : text;
}
// A condition of the chosen kind with its default value. A variable test starts on the first declared variable it can use.
export function defaultCondition(key: ConditionKey, variables: ReadonlyArray<Variable> = []): LeafCondition {
  switch (key) {
    case 'output_volume_at_least': return { output_volume_at_least: 50 };
    case 'focused_app': return { focused_app: 'com.apple.finder' };
    case 'variable_is': { const first = variables[0]; return { variable_is: { name: first?.name ?? '', value: first?.value ?? false } }; }
    case 'variable_at_least': return { variable_at_least: { name: variables.find(variable => typeof variable.value === 'number')?.name ?? '', value: 1 } };
    default: return { [key]: true } as LeafCondition;
  }
}

export function defaultConditional(): Action {
  return { type: 'conditional', when: defaultCondition('output_muted'), then: [{ type: 'toggle_mute' }] };
}

export function defaultMultiAction(): Action {
  return { type: 'multi_action', actions: [{ type: 'toggle_mute' }, defaultStepAction('shortcut')] };
}

const utf8 = new TextEncoder();

// Inline hints only; the Rust validator is authoritative and stricter.
export function urlIssue(url: string): string | null {
  if (!/^https?:\/\/[^/?#]/.test(url)) return 'Enter an http:// or https:// address with a host.';
  if (utf8.encode(url).length > maxUrlBytes) return `Use at most ${maxUrlBytes} bytes.`;
  if (/[^A-Za-z0-9\-._~:/?#[\]@!$&'()*+,;=%]/.test(url)) return 'Percent-encode spaces, quotes and non-ASCII characters.';
  if (/^https?:\/\/[^/?#]*@/.test(url)) return 'Remove the user name or password from the address.';
  return null;
}

// The scheme and authority of an address (scheme://host[:port]), never its path, query or fragment, which can carry
// tokens. User information is dropped. Used for the activity log and the Execute confirmation.
export function urlOrigin(url: string): string {
  const match = /^([A-Za-z][A-Za-z0-9+.-]*:)\/\/(?:[^/?#]*@)?([^/?#]*)/.exec(url);
  return match ? `${match[1].toLowerCase()}//${match[2].toLowerCase()}` : 'an unrecognized address';
}

// Mirrors is_invisible_text_character in crates/cooltek-sdk-service/src/profile.rs: format (Cf) characters such as
// zero-width spaces, bidi controls, soft hyphens and the BOM, the line/paragraph separators, and the whole
// U+2060-206F block (U+2065 is unassigned). They are invisible in an editor, so the pasted text could differ from
// what the user reviewed. Emoji ZWJ sequences (U+200D) are rejected too, as in Rust.
export const invisibleTextCharacter = /[\p{Cf}\p{Zl}\p{Zp}\u{2060}-\u{206F}]/u;

// Mirrors the obs_scene rule in Action::validate (profile.rs): the keypad label rules plus the invisible-character rule
// of text snippets, so the scene OBS receives is exactly the one reviewed.
// The same rule applies to every OBS name (scene, source, filter, scene collection, profile): validate_obs_name in profile.rs.
export function obsNameIssue(name: string, what: 'scene' | 'source' | 'filter' | 'scene collection' | 'profile' | 'input'): string | null {
  return nameIssue(name, `Enter the OBS ${what} name exactly as it appears in OBS.`);
}

// Streamlabs names follow the same rules (validate_obs_name is shared in profile.rs).
export function streamlabsNameIssue(name: string, what: 'scene' | 'scene collection' | 'source' | 'audio source'): string | null {
  return nameIssue(name, `Enter the Streamlabs ${what} name exactly as it appears in Streamlabs.`);
}

function nameIssue(name: string, empty: string): string | null {
  if (!name.trim()) return empty;
  if (utf8.encode(name).length > maxObsSceneBytes) return `Use at most ${maxObsSceneBytes} bytes.`;
  if (/[\u0000-\u001f\u007f-\u009f]/.test(name)) return 'Remove line breaks, tabs and other control characters.';
  if (invisibleTextCharacter.test(name)) return 'Remove invisible characters (zero-width spaces, direction marks, line or paragraph separators).';
  return null;
}

export function obsSceneIssue(scene: string): string | null {
  return obsNameIssue(scene, 'scene');
}

// Whether an OBS name can be referenced by a saved profile (the byte cap of obsNameIssue). The
// name choices only offer names that pass, so picking one never fails validation outright.
export function obsNameFits(name: string): boolean {
  return utf8.encode(name).length <= maxObsSceneBytes;
}

// Live cross-check against OBS's (or Streamlabs') own lists: when the editor holds the matching list
// and the exact name is not in it, say so without claiming the press must fail — the list may be
// stale, and a Refresh re-checks. Only exact matches count, like the press itself.
export function obsMissingWarning(value: string, list: ObsNameList | null, what: string, service = 'OBS'): string | null {
  if (!value.trim() || list?.status !== 'ready') return null;
  if (list.names.includes(value)) return null;
  return `${service} does not list this ${what} right now. Refresh the ${service} lists, or check the exact spelling.`;
}

export function textIssue(text: string): string | null {
  if (!text) return 'Enter the text to paste.';
  if (utf8.encode(text).length > maxTextBytes) return `Use at most ${maxTextBytes} bytes.`;
  if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/.test(text)) return 'Only newlines and tabs are allowed as control characters.';
  if (invisibleTextCharacter.test(text)) return 'Remove invisible characters (zero-width spaces, direction marks, line or paragraph separators).';
  return null;
}

// A terminal_command's command and directory: one line of 1-512 bytes each (Action::MAX_TERMINAL_COMMAND_BYTES).
export const maxTerminalCommandBytes = 512;
export const terminalAppLabels: Record<TerminalApp, string> = { terminal: english('labels.terminalApp.terminal'), iterm2: english('labels.terminalApp.iterm2') };
export type TerminalCommandAction = Extract<BindingAction, { type: 'terminal_command' }>;
export const defaultTerminalCommand = (): TerminalCommandAction => ({ type: 'terminal_command', app: 'terminal', command: 'claude' });

// Mirrors validate_terminal_line in profile.rs: one line, no control or invisible characters.
function terminalLineIssue(value: string): string | null {
  if (utf8.encode(value).length > maxTerminalCommandBytes) return `Use at most ${maxTerminalCommandBytes} bytes.`;
  if (/[\u0000-\u001f\u007f-\u009f]/.test(value)) return 'Use one line without tabs or control characters.';
  if (invisibleTextCharacter.test(value)) return 'Remove invisible characters (zero-width spaces, direction marks, line or paragraph separators).';
  return null;
}

// Mirrors the terminal_command command check in profile.rs.
export function terminalCommandIssue(command: string): string | null {
  if (!command.trim()) return 'Enter the command to run, such as claude.';
  return terminalLineIssue(command);
}

// Mirrors the terminal_command directory check in profile.rs. An empty field means no folder (the key is omitted).
export function terminalDirectoryIssue(directory: string): string | null {
  if (!directory) return null;
  if (!(directory === '~' || directory.startsWith('/') || directory.startsWith('~/'))) return 'Use a full path starting with / or ~/.';
  return terminalLineIssue(directory);
}

// Mirrors validate_timer_label in profile.rs. An empty field means no label (the key is omitted), so '' is fine here.
export function timerLabelIssue(label: string): string | null {
  if (!label) return null;
  if (!label.trim()) return 'Enter a label or leave the field empty.';
  if (label.trim() !== label) return 'Remove spaces at the start or end.';
  if (utf8.encode(label).length > maxTimerLabelBytes) return `Use at most ${maxTimerLabelBytes} bytes.`;
  if (/[\u0000-\u001f\u007f-\u009f]/.test(label)) return 'Use one line without tabs or control characters.';
  if (invisibleTextCharacter.test(label)) return 'Remove invisible characters (zero-width spaces, direction marks, line or paragraph separators).';
  return null;
}
