import { describe, expect, it } from 'vitest';
import { baseColumns, baseRows, layoutExtent, placementOf } from './base-layout';
import { baseOrientations, createBaseOrientationStore, effectiveRotation, readBaseOrientation, turnPlacement, turnedBase, turnedKnobOrientation } from './base-orientation';

const memory = (initial?: string) => { const data = new Map<string, string>(initial ? [['open-stream-hub.base-orientation', initial]] : []); return { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => void data.set(key, value) }; };

describe('base orientation', () => {
  it('turns every module by the base turn plus its own override', () => {
    expect(effectiveRotation(0)).toBe(0);
    expect(effectiveRotation(90)).toBe(90);
    expect(effectiveRotation(90, 270)).toBe(0);
    expect(effectiveRotation(180, 180)).toBe(0);
    expect(effectiveRotation(0, -90)).toBe(270);
  });

  it('turns the base from 6x4 to 4x6 and back', () => {
    expect(baseOrientations.map(turnedBase)).toEqual([{ cols: baseColumns, rows: baseRows }, { cols: 4, rows: 6 }, { cols: baseColumns, rows: baseRows }, { cols: 4, rows: 6 }]);
  });

  it('turns a placement a quarter at a time and keeps it inside the turned base', () => {
    const keys = placementOf('m4315', { col: 0, row: 0 });
    expect(turnPlacement(keys, 0)).toEqual(keys);
    // Top-left 4x3 becomes top-right 3x4: the left edge (USB-C) is now the top.
    expect(turnPlacement(keys, 90)).toEqual({ col: 1, row: 0, cols: 3, rows: 4 });
    // Half a turn puts it at the bottom right of the 6x4 base.
    expect(turnPlacement(keys, 180)).toEqual({ col: 2, row: 1, cols: 4, rows: 3 });
    expect(turnPlacement(keys, 270)).toEqual({ col: 0, row: 2, cols: 3, rows: 4 });
    // The bottom-right knob module (4,2) 2x2 goes to the bottom-left, then the top-left, then the top-right.
    const knobs = placementOf('m003k', { col: 4, row: 2 });
    expect(turnPlacement(knobs, 90)).toEqual({ col: 0, row: 4, cols: 2, rows: 2 });
    expect(turnPlacement(knobs, 180)).toEqual({ col: 0, row: 0, cols: 2, rows: 2 });
    expect(turnPlacement(knobs, 270)).toEqual({ col: 2, row: 0, cols: 2, rows: 2 });
    // Overhang past the top edge becomes overhang past the right edge, then the bottom, then the left.
    const over = placementOf('m4315', { col: 0, row: -1 });
    expect(turnPlacement(over, 90)).toEqual({ col: 2, row: 0, cols: 3, rows: 4 });
    expect(layoutExtent([turnPlacement(over, 90)], turnedBase(90))).toEqual({ col: 0, row: 0, cols: 5, rows: 6 });
    expect(turnPlacement(over, 180)).toEqual({ col: 2, row: 2, cols: 4, rows: 3 });
    expect(turnPlacement(over, 270)).toEqual({ col: -1, row: 2, cols: 3, rows: 4 });
  });

  it('adds the base turn to the knob module mounting', () => {
    expect(turnedKnobOrientation('single-top', 0)).toBe('single-top');
    expect(turnedKnobOrientation('single-top', 90)).toBe('single-right');
    expect(turnedKnobOrientation('single-top', 180)).toBe('single-bottom');
    expect(turnedKnobOrientation('single-top', 270)).toBe('single-left');
    expect(turnedKnobOrientation('single-left', 90)).toBe('single-top');
  });

  it('remembers the choice, reads the first version\'s words, and ignores a bad stored value', () => {
    expect(readBaseOrientation(memory('nonsense'))).toBe(0);
    expect(readBaseOrientation(memory('45'))).toBe(0);
    expect(readBaseOrientation(memory('vertical'))).toBe(90);
    expect(readBaseOrientation(memory('horizontal'))).toBe(0);
    const storage = memory();
    const store = createBaseOrientationStore(storage);
    let notified = 0;
    store.subscribe(() => notified++);
    store.set(270); store.set(270);
    expect(notified).toBe(1);
    expect(readBaseOrientation(storage)).toBe(270);
  });
});
