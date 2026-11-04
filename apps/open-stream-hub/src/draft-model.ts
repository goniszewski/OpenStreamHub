// Pure helpers for the profile draft. Rust remains the authority on validity;
// these only give earlier, local feedback in the editor.

import type { Condition, KeyFace, Profile, Variable } from './contracts';
import { conditionIssue, groupItems, isGroup, variableKind } from './flow-model';

export const maximumLabelBytes = 128;
// Most key faces on one action tile (KeyFace::MAX_PER_TILE).
export const maximumKeyFaces = 8;

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
      const faces = slot.faces ?? [];
      if (faces.length > maximumKeyFaces) return `A key holds at most ${maximumKeyFaces} faces.`;
      for (const face of faces) {
        const faceIssue = keyFaceIssue(face, profile.variables ?? []);
        if (faceIssue) return faceIssue;
      }
    }
  }
  for (const dialFunction of profile.dial_functions ?? []) {
    const functionIssue = labelIssue(dialFunction.name, 'Function name');
    if (functionIssue) return functionIssue;
  }
  return null;
}

// Whether a test reads only declared variables of the kind it compares, as `KeyFace::validate` and
// `Condition::check_variables` require.
function variableTestIssue(when: Condition, variables: ReadonlyArray<Variable>): string | null {
  if (isGroup(when)) {
    for (const item of groupItems(when)) {
      const issue = variableTestIssue(item, variables);
      if (issue) return issue;
    }
    return null;
  }
  const test = 'variable_is' in when ? when.variable_is : 'variable_at_least' in when ? when.variable_at_least : null;
  if (!test) return 'A key face can only test variables.';
  const declared = variables.find(variable => variable.name === test.name);
  if (!declared) return `Variable ${test.name} is not declared.`;
  const wanted = 'variable_at_least' in when ? 'number' : variableKind(test.value);
  if (variableKind(declared.value) !== wanted) return `Variable ${test.name} is ${variableKind(declared.value)}, not ${wanted}.`;
  return null;
}

// Mirrors KeyFace::validate in crates/cooltek-sdk-service/src/keypad.rs.
export function keyFaceIssue(face: KeyFace, variables: ReadonlyArray<Variable>): string | null {
  if (face.label === undefined && face.background === undefined) return 'A key face needs a label or a background.';
  if (face.label !== undefined) {
    const issue = labelIssue(face.label, 'Key face label');
    if (issue) return issue;
  }
  if (face.background !== undefined && !/^#?([0-9a-f]{3}|[0-9a-f]{6})$/i.test(face.background)) {
    return 'Key face background must be a hex colour such as #000000 or #fff.';
  }
  return conditionIssue(face.when) ?? variableTestIssue(face.when, variables);
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
