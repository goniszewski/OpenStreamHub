import { describe, expect, it } from 'vitest';
import { baseColumns, baseRows, layoutExtent, placementOf } from './base-layout';
import { createBaseOrientationStore, effectiveRotation, readBaseOrientation, turnPlacement, turnedBase, turnedKnobOrientation } from './base-orientation';

const memory = (initial?: string) => { const data = new Map<string, string>(initial ? [['open-stream-hub.base-orientation', initial]] : []); return { getItem: (key: string) => data.get(key) ?? null, setItem: (key: string, value: string) => void data.set(key, value) }; };

describe('base orientation', () => {
  it('turns every module by the base turn plus its own override', () => {
    expect(effectiveRotation('horizontal')).toBe(0);
    expect(effectiveRotation('vertical')).toBe(90);
    expect(effectiveRotation('vertical', 270)).toBe(0);
    expect(effectiveRotation('horizontal', -90)).toBe(270);
  });

  it('turns the base from 6x4 to 4x6', () => {
    expect(turnedBase('horizontal')).toEqual({ cols: baseColumns, rows: baseRows });
    expect(turnedBase('vertical')).toEqual({ cols: 4, rows: 6 });
  });

  it('turns a placement a quarter clockwise about the base and keeps it inside the turned base', () => {
    const keys = placementOf('m4315', { col: 0, row: 0 });
    expect(turnPlacement(keys, 'horizontal')).toEqual(keys);
    // Top-left 4x3 becomes top-right 3x4: the left edge (USB-C) is now the top.
    expect(turnPlacement(keys, 'vertical')).toEqual({ col: 1, row: 0, cols: 3, rows: 4 });
    // The bottom-right knob module (4,2) 2x2 goes to the bottom-left.
    expect(turnPlacement(placementOf('m003k', { col: 4, row: 2 }), 'vertical')).toEqual({ col: 0, row: 4, cols: 2, rows: 2 });
    // Overhang past the top edge becomes overhang past the right edge.
    const over = turnPlacement(placementOf('m4315', { col: 0, row: -1 }), 'vertical');
    expect(over).toEqual({ col: 2, row: 0, cols: 3, rows: 4 });
    expect(layoutExtent([over], turnedBase('vertical'))).toEqual({ col: 0, row: 0, cols: 5, rows: 6 });
  });

  it('adds the base turn to the knob module mounting', () => {
    expect(turnedKnobOrientation('single-top', 'horizontal')).toBe('single-top');
    expect(turnedKnobOrientation('single-top', 'vertical')).toBe('single-right');
    expect(turnedKnobOrientation('single-left', 'vertical')).toBe('single-top');
  });

  it('remembers the choice and ignores a bad stored value', () => {
    expect(readBaseOrientation(memory('nonsense'))).toBe('horizontal');
    const storage = memory();
    const store = createBaseOrientationStore(storage);
    let notified = 0;
    store.subscribe(() => notified++);
    store.set('vertical'); store.set('vertical');
    expect(notified).toBe(1);
    expect(readBaseOrientation(storage)).toBe('vertical');
  });
});
