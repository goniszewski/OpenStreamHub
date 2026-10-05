// Pure helpers for the profile draft. Rust remains the authority on validity;
// these only give earlier, local feedback in the editor.

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

export type IntegerParse = { ok: true; value: number } | { ok: false; error: string };

export function parseIntegerInput(raw: string, minimum: number, maximum: number, options: { nonZero?: boolean } = {}): IntegerParse {
  const text = raw.trim();
  if (!/^[+-]?\d+$/.test(text)) return { ok: false, error: `Enter a whole number from ${minimum} to ${maximum}.` };
  const value = Number(text);
  if (value < minimum || value > maximum) return { ok: false, error: `Enter a whole number from ${minimum} to ${maximum}.` };
  if (options.nonZero && value === 0) return { ok: false, error: 'Zero has no effect. Enter a nonzero change.' };
  return { ok: true, value: value === 0 ? 0 : value };
}

export type CloseDecision = 'allow' | 'wait' | 'confirm';

// The window close guard: never lose an unsaved draft silently.
export function closeDecision(busy: boolean, unsaved: boolean): CloseDecision {
  if (!unsaved) return 'allow';
  return busy ? 'wait' : 'confirm';
}
