// Pure helpers for listing, searching and checking the bindings of a draft.
import type { Binding, DialBinding, Gesture, KeypadBinding, Profile, ServiceModule } from './contracts';
import { dialFunctionGestures, dialFunctionNameIssue, maxDialFunctions } from './dial-function-model';
import { actionLabel, controlLabel, gestureLabels, gestures, modules } from './ui-model';

export interface BindingEntry {
  // Stable within one profile value: `flat:<index>`, `dial:<function>:<index>` or `keypad:<page>:<slot>:<index>`.
  id: string;
  // Position in the profile's flat bindings, in the key's bindings for keypad
  // entries, or in the function's bindings for `dial:` entries.
  index: number;
  module: ServiceModule;
  control: number;
  gesture: Gesture;
  serial: string;
  // The page and key label of a keypad binding; null for flat bindings.
  keypad: { pageId: string; pageName: string; keyLabel: string } | null;
  // Set only for flat bindings (the Edit/Remove paths differ for keypad bindings).
  flat?: Binding;
  keypadBinding?: KeypadBinding;
  // The dial function of an M131E binding listed under `dial_functions`.
  dialFunction?: { index: number; name: string };
  dialBinding?: DialBinding;
  title: string;
  actionText: string;
}

const moduleName = (module: ServiceModule) => modules.find(item => item.id === module)!;

// Every binding of the profile: flat bindings first, then dial function
// bindings in function order, then the keypad pages in layout order. Keypad
// bindings are listed with module m4315, dial function bindings with m131e.
export function listBindings(profile: Profile): BindingEntry[] {
  const entries: BindingEntry[] = profile.bindings.map((item, index) => ({
    id: `flat:${index}`, index, module: item.module, control: item.control, gesture: item.gesture, serial: item.serial_number ?? '',
    keypad: null, flat: item, title: `${controlLabel(item.module, item.control)} · ${gestureLabels[item.gesture]}`, actionText: actionLabel(item.action),
  }));
  (profile.dial_functions ?? []).forEach((dialFunction, functionIndex) => dialFunction.bindings.forEach((item, index) => entries.push({
    id: `dial:${functionIndex}:${index}`, index, module: 'm131e', control: 0, gesture: item.gesture, serial: item.serial_number ?? '',
    keypad: null, dialFunction: { index: functionIndex, name: dialFunction.name }, dialBinding: item,
    title: `Encoder · ${dialFunction.name} · ${gestureLabels[item.gesture]}`, actionText: actionLabel(item.action),
  })));
  for (const page of profile.keypad?.pages ?? []) {
    page.slots.forEach((slot, control) => {
      if (slot?.type !== 'action') return;
      slot.bindings.forEach((item, index) => entries.push({
        id: `keypad:${page.id}:${control}:${index}`, index, module: 'm4315', control, gesture: item.gesture, serial: item.serial_number ?? '',
        keypad: { pageId: page.id, pageName: page.name, keyLabel: slot.label }, keypadBinding: item,
        title: `${page.name} · Key ${control + 1} · ${gestureLabels[item.gesture]}`, actionText: actionLabel(item.action),
      }));
    });
  }
  return entries;
}

function haystack(entry: BindingEntry) {
  const module = moduleName(entry.module);
  return [module.name, module.shortName, controlLabel(entry.module, entry.control), gestureLabels[entry.gesture], entry.actionText,
    entry.keypad?.keyLabel, entry.keypad?.pageName, entry.dialFunction?.name, entry.serial].filter(Boolean).join('\n').toLowerCase();
}

// Case-insensitive; every whitespace-separated word of the query must match
// somewhere in the module name, control, gesture, action, key label, page name
// or serial. An empty query keeps everything.
export function filterBindings(entries: BindingEntry[], query: string): BindingEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return entries;
  return entries.filter(entry => { const text = haystack(entry); return words.every(word => text.includes(word)); });
}

export type WarningKind = 'release-overlap' | 'duplicate-selector' | 'dial-flat-conflict' | 'dial-function-empty' | 'dial-function-name' | 'dial-function-count';
export interface BindingWarning {
  id: string;
  kind: WarningKind;
  message: string;
  // Entry ids the warning is about; the first is where "Show" goes.
  entryIds: string[];
}

// Semantics checked against docs/masterhub-service.md (Gestures section):
// - "Dial short/long gestures are classified on release after an observed press:
//   less than 600 ms is short; at least 600 ms is long. A release emits `released`
//   before the classified gesture, so bindings for both can execute two actions."
// - "Duplicate complete selectors are rejected." (module, control, gesture, serial)
// - "An exact serial binding takes precedence over a wildcard for the same
//   module, control, and gesture, independent of array order." That is the
//   documented precedence rule, not a surprise, so a serial binding next to an
//   any-serial binding of the same gesture is deliberately NOT warned about.
// - docs/masterhub-keypad-folders.md: keypad slots keep pressed/released
//   bindings side by side (a press and its release are separate moments), so
//   same-slot pressed + released is not warned about either.
// Only m131e has short/long press gestures. The overlap rule applies whenever the
// serial scopes overlap: the same serial, or either binding is any-serial (an
// any-serial Release still fires on a device whose Short press is serial-bound).
export function bindingWarnings(profile: Profile): BindingWarning[] {
  const entries = listBindings(profile);
  const warnings: BindingWarning[] = [];
  const seen = new Map<string, BindingEntry>();
  for (const entry of entries) {
    const selector = `${entry.module}|${entry.keypad?.pageId ?? ''}|${entry.dialFunction?.index ?? ''}|${entry.control}|${entry.gesture}|${entry.serial}`;
    const first = seen.get(selector);
    if (!first) { seen.set(selector, entry); continue; }
    warnings.push({
      id: `duplicate:${first.id}:${entry.id}`, kind: 'duplicate-selector', entryIds: [first.id, entry.id],
      message: `${entry.title} is bound more than once${entry.serial ? ` for serial ${entry.serial}` : ''}. The service rejects duplicate bindings, so remove one before saving.`,
    });
  }
  const releases = entries.filter(entry => !entry.keypad && !entry.dialFunction && entry.gesture === 'released');
  for (const release of releases) {
    for (const press of entries) {
      if (press.keypad || press.dialFunction || press.module !== release.module || press.control !== release.control) continue;
      if (press.serial !== release.serial && press.serial !== '' && release.serial !== '') continue;
      if (press.gesture !== 'short_press' && press.gesture !== 'long_press') continue;
      const name = gestureLabels[press.gesture];
      const serial = release.serial || press.serial;
      warnings.push({
        id: `overlap:${release.id}:${press.id}`, kind: 'release-overlap', entryIds: [release.id, press.id],
        message: `${controlLabel(release.module, release.control)}: Release and ${name} both fire when the button is let go${serial ? ` (serial ${serial})` : ''}. Every ${name.toLowerCase()} runs both actions, Release first.`,
      });
    }
  }
  warnings.push(...dialFunctionWarnings(profile, entries));
  return warnings;
}

// Mirrors the service's dial function rules (docs/masterhub-dial-functions.md#validation).
function dialFunctionWarnings(profile: Profile, entries: BindingEntry[]): BindingWarning[] {
  const functions = profile.dial_functions;
  if (!functions) return [];
  const warnings: BindingWarning[] = [];
  const flat = entries.filter(entry => entry.flat && entry.module === 'm131e');
  if (flat.length) warnings.push({
    id: 'dial-flat-conflict', kind: 'dial-flat-conflict', entryIds: flat.map(entry => entry.id),
    message: `${flat.length === 1 ? "An encoder binding isn't" : `${flat.length} encoder bindings aren't`} in a dial function, so the profile can't be saved. Move ${flat.length === 1 ? 'it' : 'them'} into a function in the encoder's Dial functions editor, or remove ${flat.length === 1 ? 'it' : 'them'}.`,
  });
  if (functions.length === 0 || functions.length > maxDialFunctions) warnings.push({
    id: 'dial-function-count', kind: 'dial-function-count', entryIds: [],
    message: 'A profile with dial functions needs 1 to 16 of them.',
  });
  functions.forEach((item, index) => {
    // Compared with earlier functions only, so a duplicate is reported once, at its second occurrence.
    const issue = dialFunctionNameIssue(item.name, functions.slice(0, index + 1), index);
    if (issue) warnings.push({
      id: `dial-function-name:${index}`, kind: 'dial-function-name', entryIds: entries.filter(entry => entry.dialFunction?.index === index).map(entry => entry.id).slice(0, 1),
      message: `Dial function ${index + 1}${item.name.trim() ? ` (“${item.name}”)` : ''}: ${issue}`,
    });
    if (!item.bindings.length) warnings.push({
      id: `dial-function-empty:${index}`, kind: 'dial-function-empty', entryIds: [],
      message: `Dial function “${item.name}” has no bindings, so the encoder does nothing while it is selected.`,
    });
  });
  return warnings;
}

// Gestures the flat binding editor may offer. While dial functions exist every
// encoder binding belongs to a function, and a short press switches functions,
// so the flat editor offers no encoder gesture.
export function editorGestures(profile: Profile, module: ServiceModule): Gesture[] {
  return module === 'm131e' && profile.dial_functions ? [] : gestures[module];
}

// Gestures a dial function binding may use: never Short press, Press or Release.
export const dialEditorGestures: ReadonlyArray<DialBinding['gesture']> = dialFunctionGestures;

export interface SelectedControl { module: ServiceModule; control: number; keypadPageId?: string }

// Whether an entry is on the control the inspector is showing.
export function entryOnControl(entry: BindingEntry, selected: SelectedControl): boolean {
  if (entry.module !== selected.module || entry.control !== selected.control) return false;
  return selected.module !== 'm4315' || entry.keypad?.pageId === selected.keypadPageId;
}

// Warnings that involve at least one flat or keypad binding on the selected
// control. Dial function bindings are edited in the dial functions editor, so warnings
// about them stay in the full binding list rather than the Inspector.
export function warningsForControl(warnings: BindingWarning[], entries: BindingEntry[], selected: SelectedControl): BindingWarning[] {
  const byId = new Map(entries.map(entry => [entry.id, entry]));
  return warnings.filter(warning => warning.entryIds.some(id => { const entry = byId.get(id); return entry !== undefined && !entry.dialFunction && entryOnControl(entry, selected); }));
}
