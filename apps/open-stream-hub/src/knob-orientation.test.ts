import { describe, expect, it } from 'vitest';
import { FakeStorage } from './fake-storage';
import { createKnobOrientationStore, knobOrientations, knobOrientationStorageKey, knobRotation, nextKnobOrientation, readKnobOrientation, writeKnobOrientation } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';
import { knobControls, moduleArtwork } from './module-controls';
import type { ImageControl } from './module-controls';

const centre = (controls: ImageControl[], control: number) => {
  const area = controls.find(entry => entry.control === control)!;
  return { x: area.left + area.width / 2, y: area.top + area.height / 2 };
};
const round = (controls: ImageControl[]) => controls.map(area => ({ ...area, left: +area.left.toFixed(6), top: +area.top.toFixed(6), width: +area.width.toFixed(6), height: +area.height.toFixed(6) }));

describe('knob control positions', () => {
  it('puts the single knob on the named side and the pair on the expected corners', () => {
    const expected: Record<KnobOrientation, Record<number, { x: number; y: number }>> = {
      // Hardware: single knob on top, 0 bottom-left, 1 bottom-right.
      'single-top': { 2: { x: 50, y: 38 }, 0: { x: 34, y: 57 }, 1: { x: 66, y: 57 } },
      // One clockwise turn: the pair moves to the left, 0 top-left, 1 bottom-left.
      'single-right': { 2: { x: 62, y: 50 }, 0: { x: 43, y: 34 }, 1: { x: 43, y: 66 } },
      // Upside down: 0 top-right, 1 top-left.
      'single-bottom': { 2: { x: 50, y: 62 }, 0: { x: 66, y: 43 }, 1: { x: 34, y: 43 } },
      // Three turns: the pair on the right, 0 bottom-right, 1 top-right.
      'single-left': { 2: { x: 38, y: 50 }, 0: { x: 57, y: 66 }, 1: { x: 57, y: 34 } },
    };
    for (const orientation of knobOrientations) {
      const controls = knobControls(orientation);
      expect(controls.map(area => area.control).sort()).toEqual([0, 1, 2]);
      for (const control of [0, 1, 2]) {
        const point = centre(controls, control);
        expect(point.x, `${orientation} ${control} x`).toBeCloseTo(expected[orientation][control].x, 6);
        expect(point.y, `${orientation} ${control} y`).toBeCloseTo(expected[orientation][control].y, 6);
      }
      for (const area of controls) {
        expect(area.round).toBe(true);
        expect(area.left).toBeGreaterThanOrEqual(0);
        expect(area.top).toBeGreaterThanOrEqual(0);
        expect(area.left + area.width).toBeLessThanOrEqual(100);
        expect(area.top + area.height).toBeLessThanOrEqual(100);
      }
    }
  });

  it('places the single knob on its side relative to the pair', () => {
    const side = (orientation: KnobOrientation) => {
      const controls = knobControls(orientation);
      const single = centre(controls, 2), a = centre(controls, 0), b = centre(controls, 1);
      return { single, pair: { x: (a.x + b.x) / 2, y: (a.y + b.y) / 2 } };
    };
    const top = side('single-top'); expect(top.single.y).toBeLessThan(top.pair.y);
    const right = side('single-right'); expect(right.single.x).toBeGreaterThan(right.pair.x);
    const bottom = side('single-bottom'); expect(bottom.single.y).toBeGreaterThan(bottom.pair.y);
    const left = side('single-left'); expect(left.single.x).toBeLessThan(left.pair.x);
  });

  it('returns to the start after four clockwise steps', () => {
    let orientation: KnobOrientation = 'single-top';
    const seen: KnobOrientation[] = [];
    for (let step = 0; step < 4; step++) { seen.push(orientation); orientation = nextKnobOrientation(orientation); }
    expect(seen).toEqual(['single-top', 'single-right', 'single-bottom', 'single-left']);
    expect(orientation).toBe('single-top');
    expect(knobOrientations.map(knobRotation)).toEqual([0, 90, 180, 270]);
    expect(round(knobControls(orientation))).toEqual(round(knobControls('single-top')));
  });

  it('keeps moduleArtwork.m003k as the verified single-top layout', () => {
    expect(moduleArtwork.m003k.controls).toEqual(knobControls('single-top'));
    expect(moduleArtwork.m003k.aspect).toBe(1);
  });
});

describe('knob orientation storage', () => {
  it('defaults to single-top', () => {
    expect(readKnobOrientation(new FakeStorage())).toBe('single-top');
    expect(createKnobOrientationStore(new FakeStorage()).getSnapshot()).toBe('single-top');
  });

  it('round-trips every orientation', () => {
    for (const orientation of knobOrientations) {
      const storage = new FakeStorage();
      writeKnobOrientation(storage, orientation);
      expect(storage.getItem(knobOrientationStorageKey)).toBe(orientation);
      expect(readKnobOrientation(storage)).toBe(orientation);
      expect(createKnobOrientationStore(storage).getSnapshot()).toBe(orientation);
    }
  });

  it('falls back to single-top for corrupt or unknown values', () => {
    for (const value of ['', 'garbage', 'SINGLE-TOP', '"single-left"', '{}', 'single-up']) {
      const storage = new FakeStorage();
      storage.setItem(knobOrientationStorageKey, value);
      expect(readKnobOrientation(storage)).toBe('single-top');
    }
  });

  it('keeps working when storage throws', () => {
    const broken = { getItem: () => { throw new Error('denied'); }, setItem: () => { throw new Error('full'); } };
    expect(readKnobOrientation(broken)).toBe('single-top');
    expect(() => writeKnobOrientation(broken, 'single-left')).not.toThrow();
    const store = createKnobOrientationStore(broken);
    expect(store.getSnapshot()).toBe('single-top');
    store.rotate();
    expect(store.getSnapshot()).toBe('single-right');
  });

  it('notifies subscribers and persists each rotation', () => {
    const storage = new FakeStorage();
    const store = createKnobOrientationStore(storage);
    let calls = 0;
    const unsubscribe = store.subscribe(() => { calls++; });
    store.rotate(); store.rotate();
    expect(store.getSnapshot()).toBe('single-bottom');
    expect(storage.getItem(knobOrientationStorageKey)).toBe('single-bottom');
    expect(calls).toBe(2);
    store.set('single-bottom');
    expect(calls).toBe(2);
    unsubscribe(); store.rotate();
    expect(calls).toBe(2);
  });

  it('does not notify or write when the value is unchanged', () => {
    const storage = new FakeStorage();
    const store = createKnobOrientationStore(storage);
    let calls = 0;
    store.subscribe(() => { calls++; });
    store.set('single-top');
    expect(calls).toBe(0);
    expect(storage.getItem(knobOrientationStorageKey)).toBeNull();
    store.set('single-left'); store.set('single-left');
    expect(calls).toBe(1);
  });
});
