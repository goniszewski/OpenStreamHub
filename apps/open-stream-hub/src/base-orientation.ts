import { useSyncExternalStore } from 'react';
import { baseColumns, baseRows } from './base-layout';
import type { Placement } from './base-layout';
import { knobOrientations } from './knob-orientation';
import type { KnobOrientation } from './knob-orientation';

// How the MasterHUB base is used, as the quarter turns clockwise it is turned from lying down with its USB-C edges at
// the top and left (the layout as drawn at 0). At 90 the USB-C edges are at the top and right, at 180 at the bottom and
// right, at 270 at the bottom and left. Every module follows the base: its artwork, controls, key grid and dial face
// turn with it. The base does not report its orientation, so it is chosen here. One preference covers the whole base
// and stays in the webview.
export const baseOrientations = [0, 90, 180, 270] as const;
export type BaseOrientation = typeof baseOrientations[number];
export const defaultBaseOrientation: BaseOrientation = 0;
export const baseOrientationStorageKey = 'open-stream-hub.base-orientation';
export const baseOrientationLabels: Record<BaseOrientation, string> = { 0: '0°', 90: '90°', 180: '180°', 270: '270°' };

// Quarter turns clockwise that the base orientation applies to everything drawn on it.
export const baseTurns = (orientation: BaseOrientation) => orientation / 90;

// A module's effective rotation in degrees: the base's turn plus the module's own (an override; 0 follows the base).
export const effectiveRotation = (orientation: BaseOrientation, override = 0) => (((orientation + override) % 360) + 360) % 360;

// Which edges carry the USB-C ports once the base is turned.
export const usbEdges: Record<BaseOrientation, string> = { 0: 'top and left', 90: 'top and right', 180: 'bottom and right', 270: 'bottom and left' };

// The base's own size once turned: 6 columns by 4 rows lying down, 4 by 6 standing.
export const turnedBase = (orientation: BaseOrientation) => baseTurns(orientation) % 2 ? { cols: baseRows, rows: baseColumns } : { cols: baseColumns, rows: baseRows };

// Where a module's cells land once the base is turned clockwise, one quarter turn at a time: a W×H footprint at
// (col, row) on a base of `rows` rows becomes a H×W footprint at (rows - row - H, col) on a base turned on its side.
export function turnPlacement(placement: Placement, orientation: BaseOrientation): Placement {
  let current = placement;
  let rows = baseRows;
  let cols = baseColumns;
  for (let turn = 0; turn < baseTurns(orientation); turn++) {
    current = { col: rows - current.row - current.rows, row: current.col, cols: current.rows, rows: current.cols };
    [cols, rows] = [rows, cols];
  }
  return current;
}

// The 3-knob module keeps its own mounting preference; the base turn comes on top of it.
export function turnedKnobOrientation(knob: KnobOrientation, orientation: BaseOrientation): KnobOrientation {
  return knobOrientations[(knobOrientations.indexOf(knob) + baseTurns(orientation)) % knobOrientations.length];
}

const isBaseOrientation = (value: unknown): value is BaseOrientation => baseOrientations.includes(value as BaseOrientation);
// Stored as degrees; the first version stored the words.
const parseStored = (value: string | null): BaseOrientation => value === 'vertical' ? 90 : isBaseOrientation(Number(value)) && value !== null && value !== '' ? Number(value) as BaseOrientation : defaultBaseOrientation;

export function readBaseOrientation(storage: Pick<Storage, 'getItem'>): BaseOrientation {
  try {
    return parseStored(storage.getItem(baseOrientationStorageKey));
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
      try { storage.setItem(baseOrientationStorageKey, String(orientation)); } catch { /* a convenience; keep working without it */ }
      listeners.forEach(listener => listener());
    },
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };
export const baseOrientationStore = createBaseOrientationStore(webStorage);
export const useBaseOrientation = (): BaseOrientation => useSyncExternalStore(baseOrientationStore.subscribe, baseOrientationStore.getSnapshot, baseOrientationStore.getSnapshot);
