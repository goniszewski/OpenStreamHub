// Pure helpers for listing, searching and checking the bindings of a draft.
import type { Binding, Gesture, KeypadBinding, Profile, ServiceModule } from './contracts';
import { actionLabel, controlLabel, gestureLabels, modules } from './ui-model';

export interface BindingEntry {
  // Stable within one profile value: `flat:<index>` or `keypad:<page>:<slot>:<index>`.
  id: string;
  // Position in the profile's flat bindings, or in the key's bindings for keypad entries.
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
  title: string;
  actionText: string;
}

const moduleName = (module: ServiceModule) => modules.find(item => item.id === module)!;

// Every binding of the profile: flat bindings first, then the keypad pages in
// layout order. Keypad bindings are listed with module m4315.
export function listBindings(profile: Profile): BindingEntry[] {
  const entries: BindingEntry[] = profile.bindings.map((item, index) => ({
    id: `flat:${index}`, index, module: item.module, control: item.control, gesture: item.gesture, serial: item.serial_number ?? '',
    keypad: null, flat: item, title: `${controlLabel(item.module, item.control)} · ${gestureLabels[item.gesture]}`, actionText: actionLabel(item.action),
  }));
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
    entry.keypad?.keyLabel, entry.keypad?.pageName, entry.serial].filter(Boolean).join('\n').toLowerCase();
}

// Case-insensitive; every whitespace-separated word of the query must match
// somewhere in the module name, control, gesture, action, key label, page name
// or serial. An empty query keeps everything.
export function filterBindings(entries: BindingEntry[], query: string): BindingEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return entries;
  return entries.filter(entry => { const text = haystack(entry); return words.every(word => text.includes(word)); });
}

export type WarningKind = 'release-overlap' | 'duplicate-selector';
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
// Only m131e has short/long press gestures. The overlap rule is limited to the
// same serial scope; mixed scopes are not warned about.
export function bindingWarnings(profile: Profile): BindingWarning[] {
  const entries = listBindings(profile);
  const warnings: BindingWarning[] = [];
  const seen = new Map<string, BindingEntry>();
  for (const entry of entries) {
    const selector = `${entry.module}|${entry.keypad?.pageId ?? ''}|${entry.control}|${entry.gesture}|${entry.serial}`;
    const first = seen.get(selector);
    if (!first) { seen.set(selector, entry); continue; }
    warnings.push({
      id: `duplicate:${first.id}:${entry.id}`, kind: 'duplicate-selector', entryIds: [first.id, entry.id],
      message: `${entry.title} is bound more than once${entry.serial ? ` for serial ${entry.serial}` : ''}. The service rejects duplicate bindings, so remove one before saving.`,
    });
  }
  const releases = entries.filter(entry => !entry.keypad && entry.gesture === 'released');
  for (const release of releases) {
    for (const press of entries) {
      if (press.keypad || press.module !== release.module || press.control !== release.control || press.serial !== release.serial) continue;
      if (press.gesture !== 'short_press' && press.gesture !== 'long_press') continue;
      const name = gestureLabels[press.gesture];
      warnings.push({
        id: `overlap:${release.id}:${press.id}`, kind: 'release-overlap', entryIds: [release.id, press.id],
        message: `${controlLabel(release.module, release.control)}: Release and ${name} both fire when the button is let go${release.serial ? ` (serial ${release.serial})` : ''}. Every ${name.toLowerCase()} runs both actions, Release first.`,
      });
    }
  }
  return warnings;
}

export interface SelectedControl { module: ServiceModule; control: number; keypadPageId?: string }

// Whether an entry is on the control the inspector is showing.
export function entryOnControl(entry: BindingEntry, selected: SelectedControl): boolean {
  if (entry.module !== selected.module || entry.control !== selected.control) return false;
  return selected.module !== 'm4315' || entry.keypad?.pageId === selected.keypadPageId;
}

// Warnings that involve at least one binding on the selected control.
export function warningsForControl(warnings: BindingWarning[], entries: BindingEntry[], selected: SelectedControl): BindingWarning[] {
  const byId = new Map(entries.map(entry => [entry.id, entry]));
  return warnings.filter(warning => warning.entryIds.some(id => { const entry = byId.get(id); return entry !== undefined && entryOnControl(entry, selected); }));
}
