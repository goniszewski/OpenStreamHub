import { useSyncExternalStore } from 'react';
import { baseOrientations, baseTurns } from './base-orientation';
import type { BaseOrientation } from './base-orientation';

// A module follows the base's rotation unless it has its own: one fixed rotation for one module (faders running left
// to right, a key grid turned a quarter). The choice belongs to the unit, named by its serial number when it has one
// and by its path otherwise, and stays in the webview like the base's rotation.
export const moduleRotationStorageKey = 'open-stream-hub.module-rotation';

export type ModuleRotations = Readonly<Record<string, BaseOrientation>>;

export const moduleRotationId = (device: { serial_number?: string | null; path_hex: string }) => device.serial_number ? `serial:${device.serial_number}` : `path:${device.path_hex}`;

// The rotation a module is drawn and read at: its own when it has one, else the base's.
export const moduleRotation = (base: BaseOrientation, own: BaseOrientation | null | undefined): BaseOrientation => own ?? base;

// Whether the module's footprint on the base is turned a quarter against the base's own turn, so its cells swap.
export const footprintSwaps = (base: BaseOrientation, own: BaseOrientation | null | undefined) => (baseTurns(moduleRotation(base, own)) + baseTurns(base)) % 2 === 1;

const isRotation = (value: unknown): value is BaseOrientation => baseOrientations.includes(value as BaseOrientation);

export function readModuleRotations(storage: Pick<Storage, 'getItem'>): ModuleRotations {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(moduleRotationStorageKey) ?? '{}');
    if (typeof parsed !== 'object' || parsed === null || Array.isArray(parsed)) return {};
    return Object.fromEntries(Object.entries(parsed).filter(([, value]) => isRotation(value))) as ModuleRotations;
  } catch { return {}; }
}

export function createModuleRotationStore(storage: Pick<Storage, 'getItem' | 'setItem'>) {
  let current: ModuleRotations | undefined;
  const listeners = new Set<() => void>();
  const getSnapshot = () => current ??= readModuleRotations(storage);
  return {
    getSnapshot,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    // `null` goes back to following the base.
    set(id: string, rotation: BaseOrientation | null) {
      const before = getSnapshot();
      if ((before[id] ?? null) === rotation) return;
      const { [id]: _removed, ...rest } = before;
      current = rotation === null ? rest : { ...rest, [id]: rotation };
      try { storage.setItem(moduleRotationStorageKey, JSON.stringify(current)); } catch { /* a convenience; keep working without it */ }
      listeners.forEach(listener => listener());
    },
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };
export const moduleRotationStore = createModuleRotationStore(webStorage);
export const useModuleRotations = (): ModuleRotations => useSyncExternalStore(moduleRotationStore.subscribe, moduleRotationStore.getSnapshot, moduleRotationStore.getSnapshot);
