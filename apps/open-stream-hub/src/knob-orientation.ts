import { useSyncExternalStore } from 'react';

// How the 3-knob module (M003K) is mounted, named by where its single knob sits.
// Listed in clockwise order: each step is a 90 degree clockwise turn of the module.
// One preference covers every M003K, like its bindings and live input. It stays in the webview.
export const knobOrientations = ['single-top', 'single-right', 'single-bottom', 'single-left'] as const;
export type KnobOrientation = typeof knobOrientations[number];
export const defaultKnobOrientation: KnobOrientation = 'single-top';
export const knobOrientationStorageKey = 'open-stream-hub.knob-orientation';

export const knobOrientationLabels: Record<KnobOrientation, string> = {
  'single-top': 'Single knob on top',
  'single-right': 'Single knob on the right',
  'single-bottom': 'Single knob at the bottom',
  'single-left': 'Single knob on the left',
};

export const knobRotation = (orientation: KnobOrientation) => knobOrientations.indexOf(orientation) * 90;

export function nextKnobOrientation(orientation: KnobOrientation): KnobOrientation {
  return knobOrientations[(knobOrientations.indexOf(orientation) + 1) % knobOrientations.length];
}

const isKnobOrientation = (value: unknown): value is KnobOrientation => knobOrientations.includes(value as KnobOrientation);

export function readKnobOrientation(storage: Pick<Storage, 'getItem'>): KnobOrientation {
  try {
    const value = storage.getItem(knobOrientationStorageKey);
    return isKnobOrientation(value) ? value : defaultKnobOrientation;
  } catch { return defaultKnobOrientation; }
}

export function writeKnobOrientation(storage: Pick<Storage, 'setItem'>, orientation: KnobOrientation) {
  try { storage.setItem(knobOrientationStorageKey, orientation); } catch { /* the orientation is a convenience; keep working without it */ }
}

export function createKnobOrientationStore(storage: Pick<Storage, 'getItem' | 'setItem'>) {
  let current: KnobOrientation | undefined;
  const listeners = new Set<() => void>();
  const getSnapshot = () => current ??= readKnobOrientation(storage);
  const set = (orientation: KnobOrientation) => {
    if (orientation === getSnapshot()) return;
    current = orientation;
    writeKnobOrientation(storage, orientation);
    listeners.forEach(listener => listener());
  };
  return {
    getSnapshot,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    set,
    rotate: () => set(nextKnobOrientation(getSnapshot())),
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };

export const knobOrientationStore = createKnobOrientationStore(webStorage);

export function useKnobOrientation(): KnobOrientation {
  return useSyncExternalStore(knobOrientationStore.subscribe, knobOrientationStore.getSnapshot, knobOrientationStore.getSnapshot);
}
