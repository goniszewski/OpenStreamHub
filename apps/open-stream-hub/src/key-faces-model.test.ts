import { describe, expect, it } from 'vitest';
import type { KeyFace, KeypadSlot, Profile, Variable } from './contracts';
import { keyFaceIssue, maximumKeyFaces } from './draft-model';
import { canAddFace, faceComparisons, faceToRow, moveFace, newFaceRow, rowToFace, setKeyFaces, withVariable } from './key-faces-model';
import type { FaceRow } from './key-faces-model';

const variables: Variable[] = [{ name: 'state', value: 'idle' }, { name: 'count', value: 3 }, { name: 'live', value: false }];
const row = (patch: Partial<FaceRow> = {}): FaceRow => ({ variable: 'state', comparison: 'is', value: 'busy', label: 'Busy', background: '#d98e04', ...patch });
type ActionSlot = Extract<KeypadSlot, { type: 'action' }>;
const tile: ActionSlot = { type: 'action', label: 'Accept', bindings: [{ gesture: 'pressed', action: { type: 'toggle_mute' } }] };
const profile = (slot: KeypadSlot = tile): Profile => ({
  schema_version: 5, name: 'p', bindings: [], variables,
  keypad: { pages: [{ id: 'root', name: 'Root', slots: [slot, ...Array<null>(14).fill(null)] }] },
});

describe('face rows', () => {
  it('round-trip a single variable test with its label and background', () => {
    const faces: KeyFace[] = [
      { when: { variable_is: { name: 'state', value: 'busy' } }, label: 'Busy', background: '#d98e04' },
      { when: { variable_at_least: { name: 'count', value: 5 } }, label: 'Many' },
      { when: { variable_is: { name: 'live', value: true } }, background: '#ff0000' },
    ];
    for (const face of faces) expect(rowToFace(faceToRow(face)!)).toEqual(face);
  });

  it('leave an empty label or background out of the face', () => {
    expect(rowToFace(row({ label: '', background: '' }))).toEqual({ when: { variable_is: { name: 'state', value: 'busy' } } });
    expect(keyFaceIssue(rowToFace(row({ label: '', background: '' })), variables)).toContain('needs a label or a background');
  });

  it('write "at least" only for a number value', () => {
    expect(rowToFace(row({ variable: 'count', comparison: 'at_least', value: 2 })).when).toEqual({ variable_at_least: { name: 'count', value: 2 } });
    expect(rowToFace(row({ comparison: 'at_least', value: 'x' })).when).toEqual({ variable_is: { name: 'state', value: 'x' } });
  });

  it('give no row for a group of tests, which the section keeps as it is', () => {
    const grouped: KeyFace = { when: { any: [{ variable_is: { name: 'live', value: true } }, { variable_at_least: { name: 'count', value: 1 } }] }, label: 'X' };
    expect(faceToRow(grouped)).toBeNull();
  });

  it('take the declared value of a newly picked variable, and fall back to "is" for one that is not a number', () => {
    expect(withVariable(row(), 'count', variables)).toMatchObject({ variable: 'count', value: 3, comparison: 'is' });
    expect(withVariable(row({ variable: 'count', comparison: 'at_least', value: 4 }), 'live', variables)).toMatchObject({ variable: 'live', value: false, comparison: 'is' });
    expect(withVariable(row({ variable: 'count', comparison: 'at_least', value: 4 }), 'count', variables).comparison).toBe('at_least');
  });

  it('offer "at least" only for a number variable', () => {
    expect(faceComparisons(row({ variable: 'count', value: 3 }), variables)).toEqual(['is', 'at_least']);
    expect(faceComparisons(row(), variables)).toEqual(['is']);
    expect(faceComparisons(row({ variable: 'live', value: true }), variables)).toEqual(['is']);
  });

  it('start a new face on the first declared variable, valid as it is', () => {
    const fresh = rowToFace(newFaceRow(variables));
    expect(fresh.when).toEqual({ variable_is: { name: 'state', value: 'idle' } });
    expect(keyFaceIssue(fresh, variables)).toBeNull();
    // With nothing declared, the face says so instead of guessing a name.
    expect(keyFaceIssue(rowToFace(newFaceRow([])), [])).toContain('not declared');
  });
});

describe('face lists', () => {
  const a: KeyFace = { when: { variable_is: { name: 'state', value: 'a' } }, label: 'A' };
  const b: KeyFace = { when: { variable_is: { name: 'state', value: 'b' } }, label: 'B' };
  const c: KeyFace = { when: { variable_is: { name: 'state', value: 'c' } }, label: 'C' };

  it('move one face by one place and ignore a move past either end', () => {
    expect(moveFace([a, b, c], 0, 1)).toEqual([b, a, c]);
    expect(moveFace([a, b, c], 2, -1)).toEqual([a, c, b]);
    expect(moveFace([a, b, c], 0, -1)).toEqual([a, b, c]);
    expect(moveFace([a, b, c], 2, 1)).toEqual([a, b, c]);
  });

  it('allow adding up to the limit', () => {
    expect(canAddFace(Array(maximumKeyFaces - 1).fill(a))).toBe(true);
    expect(canAddFace(Array(maximumKeyFaces).fill(a))).toBe(false);
  });

  it('set the faces on the tile, and drop the field when none is left', () => {
    const withFaces = setKeyFaces(profile(), ['root'], 0, tile, [a, b]);
    const slot = withFaces.keypad!.pages[0].slots[0] as ActionSlot;
    expect(slot.faces).toEqual([a, b]);
    expect(slot.label).toBe('Accept');
    const cleared = setKeyFaces(withFaces, ['root'], 0, slot, []);
    expect('faces' in (cleared.keypad!.pages[0].slots[0] as ActionSlot)).toBe(false);
  });
});
