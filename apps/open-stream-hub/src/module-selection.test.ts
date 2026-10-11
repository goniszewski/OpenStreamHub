import { describe, expect, it, vi } from 'vitest';
import { FakeStorage } from './fake-storage';
import type { LightingDevice } from './lighting-model';
import { createModuleSelection, resolveSelected, selectionStorageKey } from './module-selection';

const device = (path_hex: string, module: LightingDevice['module']): LightingDevice => ({ path_hex, module, serial_number: null });
const kit = [device('bs', 'm0064'), device('kn', 'm003k'), device('dl', 'm131e')];

describe('createModuleSelection', () => {
  it('starts empty, remembers a pick across a restart and notifies once per change', () => {
    const storage = new FakeStorage();
    const selection = createModuleSelection(storage);
    expect(selection.getState()).toEqual({ path: null, marked: [] });
    const listener = vi.fn();
    selection.subscribe(listener);
    selection.select('kn');
    selection.select('kn');
    expect(listener).toHaveBeenCalledTimes(1);
    expect(storage.getItem(selectionStorageKey)).toBe('kn');
    expect(createModuleSelection(storage).getState().path).toBe('kn');
  });
  it('works without storage or with storage that refuses', () => {
    expect(createModuleSelection(null).getState().path).toBeNull();
    const refusing = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    const selection = createModuleSelection(refusing);
    selection.select('bs');
    expect(selection.getState().path).toBe('bs');
  });
  it('flags marked modules without touching the pick and ignores an unchanged list', () => {
    const selection = createModuleSelection(null);
    const listener = vi.fn();
    selection.subscribe(listener);
    selection.mark(['a', 'b']);
    selection.mark(['a', 'b']);
    expect(listener).toHaveBeenCalledTimes(1);
    selection.mark([]);
    expect(selection.getState()).toEqual({ path: null, marked: [] });
  });
});

describe('resolveSelected', () => {
  it('keeps a connected pick, else falls back to the first dial, then the first module', () => {
    expect(resolveSelected(kit, 'kn')?.path_hex).toBe('kn');
    expect(resolveSelected(kit, 'gone')?.path_hex).toBe('dl');
    expect(resolveSelected(kit, null)?.path_hex).toBe('dl');
    expect(resolveSelected(kit.slice(0, 2), 'gone')?.path_hex).toBe('bs');
    expect(resolveSelected([], 'kn')).toBeUndefined();
  });
});
