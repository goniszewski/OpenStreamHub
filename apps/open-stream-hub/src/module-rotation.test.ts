import { describe, expect, it, vi } from 'vitest';
import { FakeStorage } from './fake-storage';
import { createModuleRotationStore, footprintSwaps, moduleRotation, moduleRotationId, moduleRotationStorageKey, readModuleRotations } from './module-rotation';

describe('module rotation', () => {
  it('follows the base unless the module has its own', () => {
    expect(moduleRotation(90, null)).toBe(90);
    expect(moduleRotation(90, undefined)).toBe(90);
    expect(moduleRotation(90, 0)).toBe(0);
    expect(moduleRotation(0, 270)).toBe(270);
  });
  it('swaps the footprint only when the module is a quarter turn against the base', () => {
    for (const base of [0, 90, 180, 270] as const) {
      expect(footprintSwaps(base, null)).toBe(false);
      for (const own of [0, 90, 180, 270] as const) {
        expect(footprintSwaps(base, own), `${base} ${own}`).toBe((base / 90 + own / 90) % 2 === 1);
      }
    }
  });
  it('names a unit by its serial number, else its path', () => {
    expect(moduleRotationId({ serial_number: 'A1', path_hex: 'p' })).toBe('serial:A1');
    expect(moduleRotationId({ serial_number: null, path_hex: 'p' })).toBe('path:p');
  });
  it('remembers a choice, forgets it on reset and tells subscribers once per change', () => {
    const storage = new FakeStorage();
    const store = createModuleRotationStore(storage);
    const listener = vi.fn();
    store.subscribe(listener);
    store.set('a', 90);
    store.set('a', 90);
    store.set('b', 180);
    expect(store.getSnapshot()).toEqual({ a: 90, b: 180 });
    expect(readModuleRotations(storage)).toEqual({ a: 90, b: 180 });
    store.set('a', null);
    store.set('a', null);
    expect(store.getSnapshot()).toEqual({ b: 180 });
    expect(listener).toHaveBeenCalledTimes(3);
  });
  it('ignores stored values it does not understand', () => {
    const storage = new FakeStorage();
    storage.setItem(moduleRotationStorageKey, JSON.stringify({ a: 90, b: 45, c: 'x' }));
    expect(readModuleRotations(storage)).toEqual({ a: 90 });
    storage.setItem(moduleRotationStorageKey, '[1,2]');
    expect(readModuleRotations(storage)).toEqual({});
    storage.setItem(moduleRotationStorageKey, 'not json');
    expect(readModuleRotations(storage)).toEqual({});
  });
});
