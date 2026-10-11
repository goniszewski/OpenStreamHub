// The key catalog behind the shortcut editor and the macro steps, and the pure
// chord logic of the "Press keys…" capture field. Everything here is pure and
// hook-free; the components in key-capture.tsx own the state.
import type { MacroStep, MediaKey, Modifier } from './contracts';
import { isMediaKey, keyLabel, macroDurationMs, macroLimits, namedKeys } from './action-model';

export interface Chord { key: string; modifiers: Modifier[] }

// A chord is at most 5 keys: the four modifiers plus the key itself.
export const maxChordModifiers = 4;

const modifierOrder: Modifier[] = ['command', 'control', 'option', 'shift'];
const modifierLabels: Record<Modifier, string> = { command: 'Command', control: 'Control', option: 'Option', shift: 'Shift' };

// The chord's modifiers, in the display order the editor's checkboxes use.
export function orderedModifiers(modifiers: Modifier[]): Modifier[] {
  return modifierOrder.filter(modifier => modifiers.includes(modifier));
}

// "Command+Shift+P". The key label is the key's own name ("Keypad Enter").
export function chordLabel(chord: Chord): string {
  return [...orderedModifiers(chord.modifiers).map(modifier => modifierLabels[modifier]), keyLabel(chord.key)].join('+');
}

// What the capture field shows while modifiers are held and no key yet: "Command+Option+…".
export function heldLabel(held: Modifier[]): string {
  return `${held.map(modifier => modifierLabels[modifier]).join('+')}+…`;
}

// The browser event shape the capture reads. event.code tells a keypad digit
// from the top-row digit, which event.key cannot.
export interface CaptureEvent { key: string; code: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }

// One result of reading a captured key.
export type CaptureResult =
  // Only modifiers (or nothing) are held so far; capture continues.
  | { pending: true; held: Modifier[] }
  // A key OpenStreamHub cannot send; capture continues with the old chord.
  | { unsupported: string }
  // A complete chord.
  | { chord: Chord };

// The keys that are themselves modifiers or locks: they never resolve a chord.
const modifierOnlyKeys = new Set(['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'Fn', 'FnLock', 'NumLock', 'ScrollLock', 'SymbolLock', 'ContextMenu', 'Hyper', 'Super']);

const numpadCodes: Record<string, string> = {
  Numpad0: 'keypad_0', Numpad1: 'keypad_1', Numpad2: 'keypad_2', Numpad3: 'keypad_3', Numpad4: 'keypad_4',
  Numpad5: 'keypad_5', Numpad6: 'keypad_6', Numpad7: 'keypad_7', Numpad8: 'keypad_8', Numpad9: 'keypad_9',
  NumpadDecimal: 'keypad_decimal', NumpadAdd: 'keypad_plus', NumpadSubtract: 'keypad_minus',
  NumpadMultiply: 'keypad_multiply', NumpadDivide: 'keypad_divide', NumpadClear: 'keypad_clear',
  NumpadEnter: 'keypad_enter', NumpadEqual: 'keypad_equals',
};
const namedCodes: Record<string, string> = {
  Enter: 'return', Tab: 'tab', Escape: 'escape', Backspace: 'delete', Delete: 'forward_delete',
  Home: 'home', End: 'end', PageUp: 'page_up', PageDown: 'page_down',
  ArrowLeft: 'arrow_left', ArrowRight: 'arrow_right', ArrowDown: 'arrow_down', ArrowUp: 'arrow_up',
};
const mediaKeysByCode: Record<string, MediaKey> = {
  MediaPlayPause: 'play_pause', MediaTrackNext: 'next_track', MediaTrackPrevious: 'previous_track',
};

// Reads one captured keydown into a chord result. The keypad is resolved from
// event.code (a keypad "4" and the top-row "4" share an event.key), function
// keys and named keys from event.code, and one ASCII letter, digit or space
// from event.key. Media keys are recognised too, though macOS usually eats
// them before a web page sees them — the key picker is the reliable way.
export function chordFromEvent(event: CaptureEvent): CaptureResult {
  const held = orderedModifiers([
    ...(event.metaKey ? ['command' as const] : []),
    ...(event.ctrlKey ? ['control' as const] : []),
    ...(event.altKey ? ['option' as const] : []),
    ...(event.shiftKey ? ['shift' as const] : []),
  ]);
  if (modifierOnlyKeys.has(event.key)) return { pending: true, held };
  const key = numpadCodes[event.code]
    ?? namedCodes[event.code]
    ?? (/^F([1-9]|1[0-9]|20)$/.test(event.code) ? event.code.toLowerCase() : undefined)
    ?? mediaKeysByCode[event.key]
    ?? (/^[a-z0-9 ]$/i.test(event.key) ? event.key.toLowerCase() : undefined);
  if (!key) return { unsupported: event.key };
  // Media keys are sent bare: the validator refuses modifiers on them.
  return { chord: { key, modifiers: isMediaKey(key) ? [] : held } };
}

// One entry of the key catalog.
export interface KeyEntry { key: string; label: string }
// `media` marks the hardware media keys, which only the shortcut editor offers
// (macros cannot hold them).
export interface KeyGroup { id: string; label: string; keys: KeyEntry[]; media: boolean }

// The full catalog a key picker offers: every key without pressing it.
// Letters and digits carry the literal character; everything else uses its
// named key.
export function keyCatalog(): KeyGroup[] {
  const named = new Map(namedKeys);
  const byName = (names: string[]) => names.map(key => ({ key, label: named.get(key) ?? key }));
  const mediaEntries: KeyEntry[] = (Object.keys(mediaKeysByCode) as string[])
    .map(code => mediaKeysByCode[code])
    .filter((key, index, all) => all.indexOf(key) === index)
    .map(key => ({ key, label: keyLabel(key) }));
  return [
    {
      id: 'letters', label: 'Letters, digits, space', media: false,
      keys: [...'abcdefghijklmnopqrstuvwxyz'].map(key => ({ key, label: key.toUpperCase() }))
        .concat([...('0123456789')].map(key => ({ key, label: key })))
        .concat([{ key: ' ', label: 'Space' }]),
    },
    {
      id: 'named', label: 'Keys', media: false,
      keys: byName(['return', 'tab', 'space', 'delete', 'escape', 'forward_delete', 'home', 'end', 'page_up', 'page_down', 'arrow_left', 'arrow_right', 'arrow_down', 'arrow_up']),
    },
    {
      id: 'functions', label: 'Function keys', media: false,
      keys: byName(namedKeys.filter(([name]) => /^f\d+$/.test(name)).map(([name]) => name)),
    },
    {
      id: 'keypad', label: 'Keypad', media: false,
      keys: byName(namedKeys.filter(([name]) => name.startsWith('keypad_')).map(([name]) => name)),
    },
    { id: 'media', label: 'Media keys', media: true, keys: mediaEntries },
  ];
}

// Case-insensitive; every whitespace-separated word of the query must appear
// in the label or the key name. An empty query keeps everything.
export function searchKeys(query: string, group: KeyGroup): KeyEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return group.keys;
  return group.keys.filter(entry => words.every(word => `${entry.label} ${entry.key}`.toLowerCase().includes(word)));
}

// Canonical modifier order for a recorded flag set.
export function modifiersFromFlags(flags: { shift: boolean; control: boolean; option: boolean; command: boolean }): Modifier[] {
  return orderedModifiers([
    ...(flags.command ? ['command' as const] : []),
    ...(flags.control ? ['control' as const] : []),
    ...(flags.option ? ['option' as const] : []),
    ...(flags.shift ? ['shift' as const] : []),
  ]);
}

export interface FoldedRecording { steps: MacroStep[]; dropped: number; trimmed: boolean }

// The macOS virtual key codes of the modifier and lock keys (kVK_Command …
// kVK_Function). A recording reports them as keyless events, and they are
// folded away silently: the modifiers they add are read from each key's flags.
const modifierKeyCodes = new Set([54, 55, 56, 57, 58, 59, 60, 61, 62, 63]);

// Folds a recording's raw down/up events into macro steps: each key down
// opens a step (with the modifiers held at that moment), its key up closes it
// with the hold time, and the wait before a step is the gap after the previous
// release. Auto-repeat (a second down while the key is held) and modifier
// keys fold away; keys no macro can hold are dropped and counted. Delays and
// holds beyond the limits are capped, and steps past the total duration are
// cut — `trimmed` says so either way, and the editor tells the user.
export function foldRecording(events: { key: string | null; key_code: number; down: boolean; shift: boolean; control: boolean; option: boolean; command: boolean; at_ms: number }[]): FoldedRecording {
  const steps: MacroStep[] = [];
  const open = new Map<string, { modifiers: Modifier[]; at: number }>();
  let previousEnd = 0;
  let dropped = 0;
  let trimmed = false;
  const cap = (value: number, maximum: number) => {
    if (value > maximum) {
      trimmed = true;
      return maximum;
    }
    return Math.max(0, value);
  };
  for (const event of events) {
    if (event.key === null) {
      // Modifier and lock keys fold away; any other unmapped key is dropped
      // (its down event counts it once, the up is just its release).
      if (event.down && !modifierKeyCodes.has(event.key_code)) dropped += 1;
      continue;
    }
    if (event.down) {
      if (!open.has(event.key)) open.set(event.key, { modifiers: modifiersFromFlags(event), at: event.at_ms });
      continue; // a second down while held is auto-repeat
    }
    const press = open.get(event.key);
    if (!press) continue; // an up with no down (captured mid-key)
    open.delete(event.key);
    // The wait replays the gap between the previous release and this key
    // down; the leading silence before the first key is never replayed.
    const wait = steps.length === 0 ? 0 : cap(press.at - previousEnd, macroLimits.delayMs.maximum);
    steps.push({
      key: event.key,
      modifiers: press.modifiers,
      delay_before_ms: wait,
      hold_ms: cap(event.at_ms - press.at, macroLimits.holdMs.maximum),
    });
    previousEnd = event.at_ms;
  }
  dropped += open.size; // a key still held when the recording stopped
  if (macroDurationMs(steps) > macroLimits.durationMs) {
    // Keep the first steps that fit; the editor says the tail was cut.
    const kept: MacroStep[] = [];
    let total = 0;
    for (const step of steps) {
      const cost = step.delay_before_ms + step.hold_ms;
      if (total + cost > macroLimits.durationMs) break;
      total += cost;
      kept.push(step);
    }
    return { steps: kept, dropped: dropped + steps.length - kept.length, trimmed: true };
  }
  return { steps, dropped, trimmed };
}
