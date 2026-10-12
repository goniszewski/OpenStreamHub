import { useSyncExternalStore } from 'react';
import { baseTurns } from './base-orientation';
import type { BaseOrientation } from './base-orientation';

// The base's pogo-pin pads, read off a photo of the bare base standing up (4 columns by 6 rows), in pixels of that
// 550 × 815 photo. A pad is a 2 × 2 block of pins a module's contacts touch; a stud is a round locating bump; the bus
// is the longer data connector. Positions are read by eye from the photo, so they are a guide for where modules
// seat, not a measurement.
const photo = { width: 550, height: 815 };
const pads: ReadonlyArray<readonly [number, number]> = [
  [75, 25], [208, 25], [343, 25], [478, 25], [250, 68], [387, 68], [521, 68], [521, 205], [208, 245], [478, 245],
  [73, 295], [208, 295], [343, 295], [478, 295], [250, 338], [387, 338], [521, 338], [478, 382], [521, 473],
  [208, 517], [478, 517], [250, 608], [521, 608], [478, 652], [521, 740], [208, 785], [478, 785],
];
const studs: ReadonlyArray<readonly [number, number]> = [
  [73, 70], [345, 70], [208, 205], [480, 205], [73, 338], [345, 338], [208, 473], [480, 473], [73, 608], [345, 608], [208, 748], [480, 748],
];
const bus = { x0: 108, y0: 528, x1: 138, y1: 600 };

export interface BasePins { show: boolean; turned: boolean; mirrored: boolean }
export const defaultBasePins: BasePins = { show: true, turned: false, mirrored: false };
export const basePinsStorageKey = 'open-stream-hub.base-pins';

type Point = readonly [number, number];
const quarterTurn = ([u, v]: Point): Point => [1 - v, u];

// A photo point as a fraction of the layout box. The photo stands up and the layout lies down (6 × 4) at 0°, so the
// photo is turned a quarter first; the base's own turn comes on top. A correction turns the pins 180° or mirrors them
// in the photo's own frame, for a base whose pins do not match how it is really seated.
function place(x: number, y: number, base: BaseOrientation, pins: BasePins): Point {
  let point: Point = [x / photo.width, y / photo.height];
  if (pins.mirrored) point = [1 - point[0], point[1]];
  const turns = (1 + (pins.turned ? 2 : 0) + baseTurns(base)) % 4;
  for (let turn = 0; turn < turns; turn++) point = quarterTurn(point);
  return point;
}

export interface PinLayout {
  pads: Point[];
  studs: Point[];
  bus: { x0: number; y0: number; x1: number; y1: number };
}

// Where everything lies as fractions of the layout box (0 to 1 each way) for this base turn and correction.
export function pinLayout(base: BaseOrientation, pins: BasePins): PinLayout {
  const at = ([x, y]: Point) => place(x, y, base, pins);
  const corners = [at([bus.x0, bus.y0]), at([bus.x1, bus.y1])];
  return {
    pads: pads.map(at),
    studs: studs.map(at),
    bus: {
      x0: Math.min(corners[0][0], corners[1][0]), x1: Math.max(corners[0][0], corners[1][0]),
      y0: Math.min(corners[0][1], corners[1][1]), y1: Math.max(corners[0][1], corners[1][1]),
    },
  };
}

const isBoolean = (value: unknown): value is boolean => typeof value === 'boolean';
export function readBasePins(storage: Pick<Storage, 'getItem'>): BasePins {
  try {
    const parsed: unknown = JSON.parse(storage.getItem(basePinsStorageKey) ?? '{}');
    if (typeof parsed !== 'object' || parsed === null) return defaultBasePins;
    const { show, turned, mirrored } = parsed as Record<string, unknown>;
    return { show: isBoolean(show) ? show : defaultBasePins.show, turned: isBoolean(turned) ? turned : false, mirrored: isBoolean(mirrored) ? mirrored : false };
  } catch { return defaultBasePins; }
}

export function createBasePinsStore(storage: Pick<Storage, 'getItem' | 'setItem'>) {
  let current: BasePins | undefined;
  const listeners = new Set<() => void>();
  const getSnapshot = () => current ??= readBasePins(storage);
  return {
    getSnapshot,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    set(change: Partial<BasePins>) {
      const next = { ...getSnapshot(), ...change };
      const before = getSnapshot();
      if (next.show === before.show && next.turned === before.turned && next.mirrored === before.mirrored) return;
      current = next;
      try { storage.setItem(basePinsStorageKey, JSON.stringify(next)); } catch { /* a convenience; keep working without it */ }
      listeners.forEach(listener => listener());
    },
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };
export const basePinsStore = createBasePinsStore(webStorage);
export const useBasePins = (): BasePins => useSyncExternalStore(basePinsStore.subscribe, basePinsStore.getSnapshot, basePinsStore.getSnapshot);
