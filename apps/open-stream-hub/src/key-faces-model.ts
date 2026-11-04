// Pure helpers for the inspector's Faces section: one row per key face, edited as a single variable test with a label
// and a background, and written back as the KeyFace the service reads. Rust remains the authority on validity
// (keyFaceIssue gives the earlier, local feedback).
import type { KeyFace, KeypadSlot, Profile, Variable, VariableValue } from './contracts';
import { maximumKeyFaces } from './draft-model';
import { variableKind } from './flow-model';
import { setKeypadSlot } from './keypad-model';

export type FaceComparison = 'is' | 'at_least';

// One face as the section edits it. An empty label or background is left out of the face (the tile's own label or
// background shows).
export interface FaceRow { variable: string; comparison: FaceComparison; value: VariableValue; label: string; background: string }

// The row of a face whose test is one variable_is or variable_at_least; null for a face the section cannot edit as a
// row (a group of tests, from a profile file), which it keeps as it is.
export function faceToRow(face: KeyFace): FaceRow | null {
  const when = face.when;
  const rest = { label: face.label ?? '', background: face.background ?? '' };
  if ('variable_is' in when) return { variable: when.variable_is.name, comparison: 'is', value: when.variable_is.value, ...rest };
  if ('variable_at_least' in when) return { variable: when.variable_at_least.name, comparison: 'at_least', value: when.variable_at_least.value, ...rest };
  return null;
}

export function rowToFace(row: FaceRow): KeyFace {
  const when: KeyFace['when'] = row.comparison === 'at_least' && typeof row.value === 'number'
    ? { variable_at_least: { name: row.variable, value: row.value } }
    : { variable_is: { name: row.variable, value: row.value } };
  return { when, ...(row.label !== '' ? { label: row.label } : {}), ...(row.background !== '' ? { background: row.background } : {}) };
}

// Picking another variable: its declared value becomes the compared value, and "at least" falls back to "is" for a
// variable that is not a number.
export function withVariable(row: FaceRow, name: string, variables: ReadonlyArray<Variable>): FaceRow {
  const declared = variables.find(variable => variable.name === name);
  const value = declared ? declared.value : row.value;
  return { ...row, variable: name, value, comparison: typeof value === 'number' ? row.comparison : 'is' };
}

// A new face: tests the first declared variable for its starting value and colours the tile red.
export function newFaceRow(variables: ReadonlyArray<Variable>): FaceRow {
  const [first] = variables;
  return { variable: first?.name ?? '', comparison: 'is', value: first ? first.value : false, label: '', background: '#c81e1e' };
}

// The comparisons a row may offer: "at least" only for a number variable.
export const faceComparisons = (row: FaceRow, variables: ReadonlyArray<Variable>): FaceComparison[] => {
  const declared = variables.find(variable => variable.name === row.variable);
  return (declared ? variableKind(declared.value) : variableKind(row.value)) === 'number' ? ['is', 'at_least'] : ['is'];
};

export const canAddFace = (faces: ReadonlyArray<KeyFace>) => faces.length < maximumKeyFaces;

// Moves the face at `from` by one place up (-1) or down (1); out of range leaves the list as it is.
export function moveFace(faces: ReadonlyArray<KeyFace>, from: number, by: -1 | 1): KeyFace[] {
  const to = from + by;
  if (from < 0 || from >= faces.length || to < 0 || to >= faces.length) return [...faces];
  const next = [...faces];
  [next[from], next[to]] = [next[to], next[from]];
  return next;
}

// The profile with the action tile's faces replaced; no faces leaves the tile without the field.
export function setKeyFaces(profile: Profile, path: readonly string[], control: number, slot: Extract<KeypadSlot, { type: 'action' }>, faces: KeyFace[]): Profile {
  const { faces: _previous, ...rest } = slot;
  return setKeypadSlot(profile, path, control, faces.length > 0 ? { ...rest, faces } : rest);
}
