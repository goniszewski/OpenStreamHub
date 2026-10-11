import type { BindingAction, Profile, StepAction } from './contracts';
import { keyLabel, mediaLabels, multiActionSteps, systemLabels, urlOrigin } from './action-model';

// What a profile can really do on this Mac once Execute runs it. Built for the Execute confirmation, which must
// disclose every capability: the whole profile is walked (flat bindings, every keypad page and tile, dial functions
// and multi_action steps), not only what the editor shows now.

export type ActionKind = BindingAction['type'];
export type RiskLevel = 'normal' | 'elevated' | 'high';
export interface TextSnippet { text: string; chars: number; pressEnter: boolean; hasNewline: boolean }
export interface ProfileCapabilities {
  // Every action, multi_action steps included (a multi_action counts once itself and once per step).
  counts: Partial<Record<ActionKind, number>>;
  // Distinct snippets (same text and Return setting listed once), in profile order.
  texts: TextSnippet[];
  textPressesEnter: boolean;
  textHasNewline: boolean;
  // Distinct scheme://host[:port] of every open_url, never a path or query.
  urlOrigins: string[];
  // Distinct bundle ids of every launch_app.
  apps: string[];
  screenshot: boolean;
  // Undo or redo is sent to the focused app.
  undoRedo: boolean;
  // A shortcut that submits a line in a terminal (see submitKey) is bound somewhere.
  submitKey: boolean;
  launchApp: boolean;
  shortcut: boolean;
  multiAction: boolean;
  // A profile key (next/previous saved profile) is bound somewhere. It switches profiles in the app only.
  switchesProfile: boolean;
  // A timer is bound somewhere: at its end the service shows a notification and plays a fixed system sound. Low risk,
  // so it never raises the level, but Execute discloses it.
  timers: boolean;
  // A play/pause, next or previous media key is bound somewhere: it reaches whichever app controls playback. Low risk.
  mediaKeys: boolean;
  // A media stop is bound somewhere: it stops Music or pauses Spotify when already running and does not start one that is
  // not running (macOS may relaunch one that quits at that very instant, without playing). Low risk.
  mediaStop: boolean;
  // Distinct OBS scene names of every obs_scene, in profile order. Each switches the OBS program scene over the
  // loopback OBS WebSocket configured in Settings.
  obsScenes: string[];
  // Distinct virtual camera / replay buffer operations (for example "toggle"), in profile order. Closed words, no user text.
  obsOutputs: string[];
  // OBS filters and source visibility changes, one plain-language entry each (names shown clipped), in profile order.
  obsItems: string[];
  // OBS scene collection and profile switches, one plain-language entry each (names shown clipped). OBS reloads, which
  // can interrupt a live output. Raises the risk to elevated.
  obsReloads: string[];
  // The steps of every distinct multi_action, in profile order.
  sequences: StepAction[][];
  risk: RiskLevel;
  // Why risk is 'high', in plain language. Empty otherwise.
  riskReasons: string[];
}

const isRecord = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);
const list = (value: unknown): unknown[] => Array.isArray(value) ? value : [];
const codePoints = (text: string) => Array.from(text).length;
const clipped = (text: string, max: number) => visibleText(Array.from(text).slice(0, max).join(''));
// \n is the only line break the service accepts; \r and the Unicode separators are rejected but still counted here.
const hasLineBreak = (text: string) => /[\n\r\u2028\u2029]/.test(text);

// Every action of the profile, top level only (multi_action steps are walked by profileCapabilities).
// Tolerates partial shapes, so a profile parsed from a run snapshot can be inspected too.
function profileActions(profile: Profile): unknown[] {
  const actions: unknown[] = [];
  for (const binding of list(profile.bindings)) if (isRecord(binding)) actions.push(binding.action);
  for (const page of list(isRecord(profile.keypad) ? profile.keypad.pages : undefined)) {
    for (const slot of list(isRecord(page) ? page.slots : undefined)) {
      if (isRecord(slot) && slot.type === 'action') for (const binding of list(slot.bindings)) if (isRecord(binding)) actions.push(binding.action);
    }
  }
  for (const dialFunction of list(profile.dial_functions)) {
    for (const binding of list(isRecord(dialFunction) ? dialFunction.bindings : undefined)) if (isRecord(binding)) actions.push(binding.action);
  }
  return actions;
}

// A shortcut that submits the current line in a terminal: Return (or Enter), and Control+M (CR), Control+J (LF) and
// Control+O (accept-line-and-down-history), which pass the service validation as plain letters.
export function submitKey(step: unknown): boolean {
  if (!isRecord(step) || step.type !== 'shortcut') return false;
  const key = String(step.key).toLowerCase();
  return key === 'return' || key === 'enter' || (['m', 'j', 'o'].includes(key) && list(step.modifiers).includes('control'));
}
const isShortcut = (step: unknown) => isRecord(step) && step.type === 'shortcut';
const isScreenshot = (step: unknown) => isRecord(step) && step.type === 'system' && step.command === 'screenshot';
// True when a step matching `first` is followed (anywhere later) by a step matching `then`.
const followedBy = (steps: unknown[], first: (step: unknown) => boolean, then: (step: unknown) => boolean) =>
  steps.some((step, index) => first(step) && steps.slice(index + 1).some(then));

// Snippets and step text are shown in full. A profile the service would accept never reaches this limit; anything
// longer (an invalid or hand-edited profile) is cut for display and rated high, so a hidden tail is never trusted.
export const maxShownChars = 4096;
export const tooLongReason = 'A snippet is too long to show in full.';

// An OBS output op is a closed word; anything else (a hand-edited profile) is shown clipped and flagged by the service.
const quoted = (name: unknown) => `“${snippetPreview(String(name ?? '')).text}”`;
const obsItemLine = (action: Record<string, unknown>) => action.type === 'obs_filter'
  ? `OBS filter ${quoted(action.filter)} on ${quoted(action.source)}: ${opWord(action.op, 'obs_filter')}`
  : `OBS source ${quoted(action.source)} in scene ${quoted(action.scene)}: ${opWord(action.op, 'obs_source_visibility')}`;
const obsInputVolumeLine = (action: Record<string, unknown>) => action.type === 'obs_set_input_volume'
  ? `OBS input ${quoted(action.input)}: set volume to ${String(action.percent)}%`
  : `OBS input ${quoted(action.input)}: volume follows the fader`;
const obsReloadLine = (action: Record<string, unknown>) =>
  `OBS ${action.type === 'obs_profile' ? 'profile' : 'scene collection'} ${quoted(action.name)}`;
const outputOps = ['toggle', 'start', 'stop'];
const obsOpSets: Record<string, string[]> = {
  obs_virtual_camera: outputOps, obs_replay_buffer: [...outputOps, 'save'],
  obs_studio_mode: ['toggle', 'on', 'off'], obs_filter: ['toggle', 'on', 'off'], obs_source_visibility: ['toggle', 'on', 'off'],
};
const opWord = (op: unknown, type: string) => typeof op === 'string' && obsOpSets[type]?.includes(op) ? op : `${clipped(String(op), 20)} (unrecognised)`;

export function profileCapabilities(profile: Profile): ProfileCapabilities {
  const counts: Partial<Record<ActionKind, number>> = {};
  const texts = new Map<string, TextSnippet>();
  const origins = new Set<string>();
  const apps = new Set<string>();
  const obsScenes = new Set<string>();
  const obsOutputs = new Set<string>();
  const obsItems = new Set<string>();
  const obsReloads = new Set<string>();
  const sequences = new Map<string, StepAction[]>();
  const reasons = new Set<string>();
  let screenshot = false;
  let undoRedo = false;
  let submits = false;
  let mediaKeys = false;
  let mediaStop = false;

  const visit = (action: unknown, depth: number) => {
    if (!isRecord(action)) return;
    if (typeof action.type !== 'string') { reasons.add('An action without a type cannot be checked.'); return; }
    const kind = action.type as ActionKind;
    counts[kind] = (counts[kind] ?? 0) + 1;
    switch (kind) {
      // profile only asks the session manager for another saved profile; nothing runs on the host.
      // timer shows a notification and plays a fixed system sound; its label is plain notification text.
      case 'volume': case 'set_volume': case 'toggle_mute': case 'volume_from_position': case 'profile': case 'timer': break;
      // A media key or a stop of an already running player: disclosed, but low risk.
      case 'media': if (action.command === 'stop') mediaStop = true; else mediaKeys = true; break;
      case 'shortcut': if (submitKey(action)) submits = true; break;
      case 'text': {
        const text = typeof action.text === 'string' ? action.text : '';
        const snippet = { text, chars: codePoints(text), pressEnter: action.press_enter === true, hasNewline: hasLineBreak(text) };
        texts.set(JSON.stringify([text, snippet.pressEnter]), snippet);
        if (snippet.pressEnter) reasons.add('A text snippet presses Return after pasting.');
        if (snippet.hasNewline) reasons.add('A text snippet contains line breaks.');
        if (snippet.chars > maxShownChars) reasons.add(tooLongReason);
        break;
      }
      case 'open_url':
        origins.add(urlOrigin(String(action.url)));
        if (codePoints(String(action.url)) > maxShownChars) reasons.add(tooLongReason);
        break;
      case 'launch_app': apps.add(String(action.bundle_id)); break;
      // A host effect (a request to OBS on this Mac) that only changes the visible scene: shown, but low risk.
      case 'obs_scene':
        obsScenes.add(String(action.scene));
        if (codePoints(String(action.scene)) > maxShownChars) reasons.add(tooLongReason);
        break;
      // Local OBS outputs: neither publishes to an audience by itself, so they are shown but low risk.
      case 'obs_virtual_camera': obsOutputs.add(`virtual camera ${opWord(action.op, 'obs_virtual_camera')}`); break;
      case 'obs_replay_buffer': obsOutputs.add(`replay buffer ${opWord(action.op, 'obs_replay_buffer')}`); break;
      case 'obs_studio_mode': obsOutputs.add(`studio mode ${opWord(action.op, 'obs_studio_mode')}`); break;
      case 'obs_transition': obsOutputs.add('studio transition'); break;
      // Names are user text: shown clipped, and a very long one is a reason to look closer.
      case 'obs_filter':
      case 'obs_source_visibility':
        obsItems.add(obsItemLine(action));
        for (const name of [action.scene, action.source, action.filter]) if (codePoints(String(name ?? '')) > maxShownChars) reasons.add(tooLongReason);
        break;
      // A fader (or a fixed level) driving an OBS input's volume: a local OBS effect, shown by input name.
      case 'obs_input_volume_from_position':
      case 'obs_set_input_volume':
        obsItems.add(obsInputVolumeLine(action));
        if (codePoints(String(action.input ?? '')) > maxShownChars) reasons.add(tooLongReason);
        break;
      case 'obs_scene_collection':
      case 'obs_profile':
        obsReloads.add(obsReloadLine(action));
        if (codePoints(String(action.name ?? '')) > maxShownChars) reasons.add(tooLongReason);
        break;
      case 'system': if (action.command === 'screenshot') screenshot = true; else undoRedo = true; break;
      case 'multi_action': {
        const steps = list(action.actions);
        // The service rejects nested sequences; they are still walked so nothing hides inside one.
        if (depth < 4) for (const step of steps) visit(step, depth + 1);
        sequences.set(JSON.stringify(steps), steps as StepAction[]);
        const has = (type: string) => steps.some(step => isRecord(step) && step.type === type);
        if ((has('launch_app') || has('open_url')) && (has('text') || has('shortcut'))) reasons.add('A multi-step sequence opens an app or web page and then types or sends shortcuts into it.');
        if (steps.some(isScreenshot) && has('open_url')) reasons.add('A multi-step sequence captures the screen and opens a web page.');
        // Fails closed: any key after pasted text can submit it (Return, Control+M, Control+J, Control+O, app bindings).
        if (followedBy(steps, step => isRecord(step) && step.type === 'text', isShortcut)) reasons.add('A multi-step sequence pastes text and then sends keys after it.');
        if (followedBy(steps, isScreenshot, isShortcut)) reasons.add('A multi-step sequence captures the screen and then sends keys, which can paste it into another app.');
        break;
      }
      default: {
        // Adding an Action variant without handling it here fails the build.
        const unchecked: never = kind;
        reasons.add(`An action of an unrecognised type (${clipped(String(unchecked), 40)}) cannot be checked.`);
      }
    }
  };
  for (const action of profileActions(profile)) visit(action, 0);

  const snippets = [...texts.values()];
  const elevated = snippets.length > 0 || origins.size > 0 || screenshot || obsReloads.size > 0 || (counts.multi_action ?? 0) > 0;
  return {
    counts, texts: snippets,
    textPressesEnter: snippets.some(snippet => snippet.pressEnter), textHasNewline: snippets.some(snippet => snippet.hasNewline),
    urlOrigins: [...origins], apps: [...apps], screenshot, undoRedo, submitKey: submits,
    launchApp: (counts.launch_app ?? 0) > 0, shortcut: (counts.shortcut ?? 0) > 0, multiAction: (counts.multi_action ?? 0) > 0,
    switchesProfile: (counts.profile ?? 0) > 0,
    timers: (counts.timer ?? 0) > 0,
    mediaKeys, mediaStop,
    obsScenes: [...obsScenes],
    obsOutputs: [...obsOutputs],
    obsItems: [...obsItems],
    obsReloads: [...obsReloads],
    sequences: [...sequences.values()],
    risk: reasons.size > 0 ? 'high' : elevated ? 'elevated' : 'normal',
    riskReasons: [...reasons],
  };
}

// Parses a run snapshot (stableStringify of the profile) back into a profile, or null when the run has none.
export function snapshotProfile(snapshot: string): Profile | null {
  if (!snapshot.startsWith('{')) return null;
  try {
    const parsed: unknown = JSON.parse(snapshot);
    return isRecord(parsed) && Array.isArray(parsed.bindings) ? parsed as unknown as Profile : null;
  } catch {
    return null;
  }
}

const hex = (code: number) => `\\u{${code.toString(16).toUpperCase()}}`;
const escapeAll = (text: string) => Array.from(text, character => hex(character.codePointAt(0)!)).join('');
// Characters that render as nothing or as a plain space but are not one. Variation selectors stay (emoji use them).
const hiddenCharacter = /[\p{Cc}\p{Cf}\p{Zl}\p{Zp}\p{Default_Ignorable_Code_Point}\u{2060}-\u{206F}]|(?! )\p{Zs}/u;
const variationSelector = /[\u{FE00}-\u{FE0F}\u{e0100}-\u{e01ef}]/u;
// More than 3 combining marks in a row (a "Zalgo" stack) can draw over neighbouring text, so such runs are escaped.
const markRun = /(\p{M}{4,})/u;

// The text with every invisible or control character made visible: ⏎ for a newline, ⇥ for a tab and \u{…} escapes
// for everything else that would not show (other control characters, format and bidi characters, zero-width and
// unusual spaces, line/paragraph separators) or could obscure other text (runs of more than 3 combining marks).
export function visibleText(text: string): string {
  // split with a capture group puts the mark runs at the odd indices.
  return text.split(markRun).map((part, index) => index % 2 ? escapeAll(part) : Array.from(part, character =>
    character === '\n' ? '⏎' : character === '\t' ? '⇥'
      : hiddenCharacter.test(character) && !variationSelector.test(character) ? hex(character.codePointAt(0)!) : character).join('')).join('');
}

// visibleText of the whole snippet, up to maxShownChars characters (code points); `more` counts any cut tail, which
// profileCapabilities rates high (tooLongReason).
export function snippetPreview(text: string, max = maxShownChars): { text: string; more: number } {
  const characters = Array.from(text);
  return { text: visibleText(characters.slice(0, max).join('')), more: Math.max(0, characters.length - max) };
}

const modifierLabel = (modifier: string) => modifier[0].toUpperCase() + modifier.slice(1);

// One multi_action step: its kind and key fields, with text made visible.
export function stepLabel(step: StepAction): string {
  switch (step.type) {
    case 'volume': return `Change output volume by ${step.delta > 0 ? '+' : ''}${step.delta}`;
    case 'set_volume': return `Set output volume to ${step.percent}%`;
    case 'toggle_mute': return 'Toggle output mute';
    case 'shortcut': return `Press ${[...list(step.modifiers).map(String).map(modifierLabel), keyLabel(String(step.key))].join(' + ')}`;
    case 'launch_app': return `Open application ${String(step.bundle_id)}`;
    case 'open_url': {
      const preview = snippetPreview(String(step.url));
      return `Open web page ${preview.text}${preview.more ? `… (${preview.more} more characters not shown)` : ''}`;
    }
    case 'media': return step.command === 'stop' ? 'Stop Music or Spotify if running' : `Press the ${step.command === 'play_pause' ? 'Play/pause' : mediaLabels[step.command] ?? String(step.command)} media key`;
    case 'system': return step.command === 'screenshot' ? 'Capture the whole screen to the clipboard' : systemLabels[step.command] ?? String(step.command);
    case 'obs_scene': {
      const preview = snippetPreview(String(step.scene));
      return `Switch OBS to the scene “${preview.text}”${preview.more ? `… (${preview.more} more characters not shown)` : ''}`;
    }
    case 'obs_studio_mode': return `OBS studio mode: ${opWord(step.op, 'obs_studio_mode')}`;
    case 'obs_transition': return 'OBS studio transition';
    case 'obs_filter': case 'obs_source_visibility': return obsItemLine(step);
    case 'obs_set_input_volume': return obsInputVolumeLine(step);
    case 'obs_scene_collection': case 'obs_profile': return `Switch ${obsReloadLine(step)} (OBS reloads)`;
    case 'obs_virtual_camera': return `OBS virtual camera: ${opWord(step.op, 'obs_virtual_camera')}`;
    case 'obs_replay_buffer': return `OBS replay buffer: ${opWord(step.op, 'obs_replay_buffer')}`;
    case 'text': {
      const preview = snippetPreview(typeof step.text === 'string' ? step.text : '');
      return `Paste “${preview.text}”${preview.more ? `… (${preview.more} more characters not shown)` : ''}${step.press_enter ? ', then press Return' : ''}`;
    }
    default: return `Unrecognised step ${clipped(String((step as { type?: unknown }).type), 40)} (cannot be checked)`;
  }
}

const plural = (count: number, one: string, many = `${one}s`) => `${count} ${count === 1 ? one : many}`;
const sum = (counts: Partial<Record<ActionKind, number>>, kinds: ActionKind[]) => kinds.reduce((total, kind) => total + (counts[kind] ?? 0), 0);

// The capabilities in plain language, most consequential first.
export function capabilityLines(capabilities: ProfileCapabilities): string[] {
  const { counts } = capabilities;
  const lines: string[] = [];
  if (capabilities.texts.length) lines.push(`Pastes ${plural(capabilities.texts.length, 'text snippet')} into the focused app (shown below).`);
  if (capabilities.textPressesEnter) lines.push('Presses Return after pasting. In a terminal this runs commands.');
  if (capabilities.textHasNewline) lines.push('Pastes text with line breaks. In a terminal each line can run as a command.');
  if (capabilities.urlOrigins.length) lines.push(`Opens web pages: ${capabilities.urlOrigins.join(', ')}.`);
  if (capabilities.screenshot) lines.push('Captures the whole screen to the clipboard.');
  if (capabilities.multiAction) lines.push(`Runs ${plural(capabilities.sequences.length, 'multi-step sequence')} (up to ${multiActionSteps.maximum} steps each, shown below).`);
  if (capabilities.apps.length) lines.push(`Opens applications: ${capabilities.apps.join(', ')}.`);
  if (capabilities.shortcut) lines.push(`Sends ${plural(counts.shortcut ?? 0, 'keyboard shortcut')} to the focused app.`);
  if (capabilities.submitKey && capabilities.texts.length) lines.push('Has a key that submits a line (Return, Control+M, Control+J or Control+O). Pressed after pasted text, it runs commands in a terminal.');
  if (capabilities.undoRedo) lines.push('Sends Undo or Redo to the focused app.');
  if (capabilities.obsScenes.length) lines.push(`Switches OBS scenes through the OBS WebSocket on this Mac (when OBS is turned on in Settings): ${capabilities.obsScenes.map(scene => `“${snippetPreview(scene).text}”`).join(', ')}.`);
  if (capabilities.obsOutputs.length) lines.push(`Controls OBS outputs through the OBS WebSocket on this Mac (when OBS is turned on in Settings): ${capabilities.obsOutputs.join(', ')}.`);
  if (capabilities.obsItems.length) lines.push(`Changes OBS filters or source visibility through the OBS WebSocket on this Mac (when OBS is turned on in Settings): ${capabilities.obsItems.join('; ')}.`);
  if (capabilities.obsReloads.length) lines.push(`Switches the OBS scene collection or profile through the OBS WebSocket on this Mac (when OBS is turned on in Settings); OBS reloads and this can interrupt a live stream or recording: ${capabilities.obsReloads.join('; ')}.`);
  if (sum(counts, ['volume', 'set_volume', 'toggle_mute', 'volume_from_position'])) lines.push('Changes the output volume or mute.');
  if (capabilities.mediaKeys) lines.push('Sends media keys (play/pause, next, previous) to whichever app controls playback. macOS may ask for Accessibility permission.');
  if (capabilities.mediaStop) lines.push('Stops Music or Spotify if running; it does not start a player that is not running (one that quits at that instant may be relaunched by macOS, without playing). macOS may ask for Automation permission to control them.');
  if (capabilities.timers) lines.push(`Runs countdown timers (${plural(counts.timer ?? 0, 'timer')} bound). When one ends, macOS shows a notification and plays a system sound. Pressing the same control again cancels it; stopping the run cancels them all.`);
  if (capabilities.switchesProfile) lines.push('Switches profile: a key moves this run to the next or previous saved profile. Actions then pause until you confirm the new profile.');
  if (!lines.length) lines.push('No actions are bound yet, so nothing will run until you add some.');
  return lines;
}
