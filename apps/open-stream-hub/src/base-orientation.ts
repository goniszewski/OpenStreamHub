import { useSyncExternalStore } from 'react';
import { baseColumns, baseRows } from './base-layout';
import type { Placement } from './base-layout';
import { knobOrientations } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';

// How the MasterHUB base is used. The layout is drawn as the base lies in landscape (USB-C edges at the top and
// left); `vertical` stands it up with a quarter turn clockwise, so the USB-C edges are at the top and right.
// Every module follows the base: its artwork, controls, key grid and dial face turn with it. The base does not
// report its orientation, so it is chosen here. One preference covers the whole base and stays in the webview.
export const baseOrientations = ['horizontal', 'vertical'] as const;
export type BaseOrientation = typeof baseOrientations[number];
export const defaultBaseOrientation: BaseOrientation = 'horizontal';
export const baseOrientationStorageKey = 'open-stream-hub.base-orientation';
export const baseOrientationLabels: Record<BaseOrientation, string> = { horizontal: 'Horizontal', vertical: 'Vertical' };

// Quarter turns clockwise that the base orientation applies to everything drawn on it.
export const baseTurns = (orientation: BaseOrientation) => orientation === 'vertical' ? 1 : 0;

// A module's effective rotation in degrees: the base's turn plus the module's own (an override; 0 follows the base).
export const effectiveRotation = (orientation: BaseOrientation, override = 0) => (((baseTurns(orientation) * 90 + override) % 360) + 360) % 360;

// The base's own size once turned: 6 columns by 4 rows lying down, 4 by 6 standing.
export const turnedBase = (orientation: BaseOrientation) => baseTurns(orientation) ? { cols: baseRows, rows: baseColumns } : { cols: baseColumns, rows: baseRows };

// Where a module's cells land once the base is turned clockwise: (col, row) of a W×H footprint becomes
// (baseRows - row - H, col) with the footprint swapped.
export function turnPlacement(placement: Placement, orientation: BaseOrientation): Placement {
  if (!baseTurns(orientation)) return placement;
  return { col: baseRows - placement.row - placement.rows, row: placement.col, cols: placement.rows, rows: placement.cols };
}

// The 3-knob module keeps its own mounting preference; the base turn comes on top of it.
export function turnedKnobOrientation(knob: KnobOrientation, orientation: BaseOrientation): KnobOrientation {
  return knobOrientations[(knobOrientations.indexOf(knob) + baseTurns(orientation)) % knobOrientations.length];
}

const isBaseOrientation = (value: unknown): value is BaseOrientation => baseOrientations.includes(value as BaseOrientation);

export function readBaseOrientation(storage: Pick<Storage, 'getItem'>): BaseOrientation {
  try {
    const value = storage.getItem(baseOrientationStorageKey);
    return isBaseOrientation(value) ? value : defaultBaseOrientation;
  } catch { return defaultBaseOrientation; }
}

export function createBaseOrientationStore(storage: Pick<Storage, 'getItem' | 'setItem'>) {
  let current: BaseOrientation | undefined;
  const listeners = new Set<() => void>();
  const getSnapshot = () => current ??= readBaseOrientation(storage);
  return {
    getSnapshot,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    set(orientation: BaseOrientation) {
      if (orientation === getSnapshot()) return;
      current = orientation;
      try { storage.setItem(baseOrientationStorageKey, orientation); } catch { /* a convenience; keep working without it */ }
      listeners.forEach(listener => listener());
    },
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };
export const baseOrientationStore = createBaseOrientationStore(webStorage);
export const useBaseOrientation = (): BaseOrientation => useSyncExternalStore(baseOrientationStore.subscribe, baseOrientationStore.getSnapshot, baseOrientationStore.getSnapshot);
