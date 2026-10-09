// Pure helpers for the profile draft. Rust remains the authority on validity;
// these only give earlier, local feedback in the editor.

import type { Profile } from './contracts';

export const maximumLabelBytes = 128;

// JSON with object keys sorted, so the dirty check ignores key order.
export function stableStringify(value: unknown): string {
  return JSON.stringify(value, (_key, item: unknown) => {
    if (!item || typeof item !== 'object' || Array.isArray(item)) return item;
    return Object.fromEntries(Object.keys(item).sort().map(key => [key, (item as Record<string, unknown>)[key]]));
  });
}

const encoder = new TextEncoder();

// Truncates on a code point boundary to fit Rust's byte limit.
export function clampUtf8(text: string, maximumBytes = maximumLabelBytes): string {
  if (encoder.encode(text).length <= maximumBytes) return text;
  let result = '';
  let bytes = 0;
  for (const character of text) {
    const size = encoder.encode(character).length;
    if (bytes + size > maximumBytes) break;
    result += character;
    bytes += size;
  }
  return result;
}

// Mirrors the Rust rule for profile names, page names and key labels.
export function labelIssue(value: string, field: string): string | null {
  if (!value.trim()) return `${field} cannot be empty.`;
  if (encoder.encode(value).length > maximumLabelBytes) return `${field} must be at most ${maximumLabelBytes} bytes.`;
  if (/[\u0000-\u001f\u007f-\u009f]/.test(value)) return `${field} cannot contain control characters.`;
  return null;
}

// Why the profile cannot be saved yet, from the same rules the editor fields
// show while typing. Automatic saving skips the write for such a draft — it
// stays a local recovery copy until the field passes — instead of failing on
// the backend for text the user is still settling. Control values (modules,
// gestures, action fields) are chosen in editors, never typed freely, so the
// backend's remaining rules are not mirrored here.
export function profileIssue(profile: Profile): string | null {
  const direct = labelIssue(profile.name, 'Profile name');
  if (direct) return direct;
  for (const page of profile.keypad?.pages ?? []) {
    const pageIssue = labelIssue(page.name, 'Page name');
    if (pageIssue) return pageIssue;
    for (const slot of page.slots) {
      if (slot?.type !== 'action') continue;
      const labelIssueFound = labelIssue(slot.label, 'Key label');
      if (labelIssueFound) return labelIssueFound;
    }
  }
  for (const dialFunction of profile.dial_functions ?? []) {
    const functionIssue = labelIssue(dialFunction.name, 'Function name');
    if (functionIssue) return functionIssue;
  }
  return null;
}

export type IntegerParse = { ok: true; value: number } | { ok: false; error: string };

export function parseIntegerInput(raw: string, minimum: number, maximum: number, options: { nonZero?: boolean } = {}): IntegerParse {
  const text = raw.trim();
  if (!/^[+-]?\d+$/.test(text)) return { ok: false, error: `Enter a whole number from ${minimum} to ${maximum}.` };
  const value = Number(text);
  if (value < minimum || value > maximum) return { ok: false, error: `Enter a whole number from ${minimum} to ${maximum}.` };
  if (options.nonZero && value === 0) return { ok: false, error: 'Zero has no effect. Enter a nonzero change.' };
  return { ok: true, value: value === 0 ? 0 : value };
}
