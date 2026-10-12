import { describe, expect, it, vi } from 'vitest';
import { FakeStorage } from './fake-storage';
import { basePinsStorageKey, createBasePinsStore, defaultBasePins, pinLayout, readBasePins } from './base-pins';

const inBox = ([u, v]: readonly [number, number]) => u >= 0 && u <= 1 && v >= 0 && v <= 1;

describe('pogo pin layout', () => {
  it('keeps every pad inside the base for every base turn and correction', () => {
    for (const base of [0, 90, 180, 270] as const) {
      for (const turned of [false, true]) {
        for (const mirrored of [false, true]) {
          const layout = pinLayout(base, { show: true, turned, mirrored });
          expect(layout.pads.length).toBeGreaterThan(20);
          expect([...layout.pads, ...layout.studs, ...layout.outline, ...layout.plate].every(inBox), `${base} ${turned} ${mirrored}`).toBe(true);
        }
      }
    }
  });
  it('lies the photo down at 0° and turns it with the base', () => {
    // The photo's top-left pad is near (0.14, 0.03): a quarter turn clockwise puts it near the top right.
    const [u, v] = pinLayout(0, defaultBasePins).pads[0];
    expect(u).toBeGreaterThan(0.9);
    expect(v).toBeLessThan(0.2);
    // At 90° the base stands up and the layout turns with it: the pad is now near the bottom right.
    const [standing] = pinLayout(90, defaultBasePins).pads;
    expect(standing[0]).toBeGreaterThan(0.8);
    expect(standing[1]).toBeGreaterThan(0.9);
  });
  it('turns the pins half way round with the correction and mirrors them with the other', () => {
    const plain = pinLayout(0, defaultBasePins).pads[0];
    const turned = pinLayout(0, { ...defaultBasePins, turned: true }).pads[0];
    expect([1 - turned[0], 1 - turned[1]].map(value => +value.toFixed(6))).toEqual(plain.map(value => +value.toFixed(6)));
    const mirrored = pinLayout(0, { ...defaultBasePins, mirrored: true }).pads[0];
    expect(mirrored[0]).toBeCloseTo(plain[0], 6);
    expect(mirrored[1]).toBeCloseTo(1 - plain[1], 6);
  });
});

describe('pogo pin settings', () => {
  it('remembers the choices and tells subscribers once per change', () => {
    const storage = new FakeStorage();
    const store = createBasePinsStore(storage);
    const listener = vi.fn();
    store.subscribe(listener);
    store.set({ turned: true });
    store.set({ turned: true });
    store.set({ show: false, mirrored: true });
    expect(store.getSnapshot()).toEqual({ show: false, turned: true, mirrored: true });
    expect(readBasePins(storage)).toEqual({ show: false, turned: true, mirrored: true });
    expect(listener).toHaveBeenCalledTimes(2);
  });
  it('falls back to showing the pins as photographed when storage is empty or unreadable', () => {
    const storage = new FakeStorage();
    expect(readBasePins(storage)).toEqual(defaultBasePins);
    storage.setItem(basePinsStorageKey, 'nope');
    expect(readBasePins(storage)).toEqual(defaultBasePins);
    storage.setItem(basePinsStorageKey, '{"show":"yes","turned":1}');
    expect(readBasePins(storage)).toEqual(defaultBasePins);
  });
});
