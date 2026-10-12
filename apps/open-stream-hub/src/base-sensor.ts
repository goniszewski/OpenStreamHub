import { useEffect, useSyncExternalStore } from 'react';
import { baseOrientationStore } from './base-orientation';
import type { BaseOrientation } from './base-orientation';
import type { Invoke } from './profile-session';

// EXPERIMENTAL, off by default. The base firmware has an accelerometer routine that sets one state byte, 0 or 1 (feature
// report 0x35). On a real base the byte read 1 lying flat in landscape and stayed 1 for 50 seconds of moving the base
// between lying flat, turned a quarter on the desk, and standing in both turns, so it did not follow those positions
// and what it means is not established. The code below assumes 1 is lying down and 0 standing, picks the axis only and
// leaves the half turn to the user; it is kept so the byte can be tried on other bases and firmware.
export const landscapeState = 1;
export type SensorPosition = 'landscape' | 'standing';

export const sensorPosition = (state: 0 | 1): SensorPosition => state === landscapeState ? 'landscape' : 'standing';

export function parseSensorReading(value: unknown): 0 | 1 | null {
  return value === 0 || value === 1 ? value : null;
}

// The base turn that goes with a sensor position: the current turn when it already lies on that axis, else the first
// turn on it.
export function orientationFromSensor(position: SensorPosition, current: BaseOrientation): BaseOrientation {
  const options: readonly BaseOrientation[] = position === 'landscape' ? [0, 180] : [90, 270];
  return options.includes(current) ? current : options[0];
}

// What the app does with a stream of readings: follow the sensor when it moves, and only then, so a turn the user picks
// by hand stays until the base is really moved. `null` is a failed read; a few in a row mean the sensor is not available.
export const unavailableAfter = 3;

export interface SensorTracker { observe(reading: 0 | 1 | null): void }

export function createSensorTracker(options: { follow(): boolean; current(): BaseOrientation; apply(orientation: BaseOrientation): void; report(position: SensorPosition | null): void }): SensorTracker {
  let last: 0 | 1 | null = null;
  let failures = 0;
  return {
    observe(reading) {
      if (reading === null) {
        failures += 1;
        if (failures >= unavailableAfter) { last = null; options.report(null); }
        return;
      }
      failures = 0;
      if (reading === last) return;
      last = reading;
      const position = sensorPosition(reading);
      options.report(position);
      if (options.follow()) options.apply(orientationFromSensor(position, options.current()));
    },
  };
}

export const sensorStorageKey = 'open-stream-hub.base-sensor-follow';

export interface SensorState { follow: boolean; position: SensorPosition | null }

export function createBaseSensorStore(storage: Pick<Storage, 'getItem' | 'setItem'>) {
  let current: SensorState | undefined;
  const listeners = new Set<() => void>();
  const read = (): SensorState => {
    let follow = false;
    try { follow = storage.getItem(sensorStorageKey) === 'on'; } catch { /* off by default */ }
    return { follow, position: null };
  };
  const getSnapshot = () => current ??= read();
  const change = (next: Partial<SensorState>) => {
    const before = getSnapshot();
    const merged = { ...before, ...next };
    if (merged.follow === before.follow && merged.position === before.position) return;
    current = merged;
    if (merged.follow !== before.follow) { try { storage.setItem(sensorStorageKey, merged.follow ? 'on' : 'off'); } catch { /* a convenience */ } }
    listeners.forEach(listener => listener());
  };
  return {
    getSnapshot,
    subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    setFollow: (follow: boolean) => change({ follow }),
    setPosition: (position: SensorPosition | null) => change({ position }),
  };
}

const webStorage = { getItem: (key: string) => globalThis.localStorage?.getItem(key) ?? null, setItem: (key: string, value: string) => globalThis.localStorage?.setItem(key, value) };
export const baseSensorStore = createBaseSensorStore(webStorage);
export const useBaseSensor = (): SensorState => useSyncExternalStore(baseSensorStore.subscribe, baseSensorStore.getSnapshot, baseSensorStore.getSnapshot);

export const sensorPollMs = 2000;

// Reads the base's sensor while the app runs and turns the layout with it. Does nothing without a base or outside the desktop app.
export function useBaseSensorPolling(invoke: Invoke | undefined, basePath: string | null) {
  useEffect(() => {
    if (!invoke || !basePath) { baseSensorStore.setPosition(null); return; }
    const tracker = createSensorTracker({
      follow: () => baseSensorStore.getSnapshot().follow,
      current: () => baseOrientationStore.getSnapshot(),
      apply: orientation => baseOrientationStore.set(orientation),
      report: position => baseSensorStore.setPosition(position),
    });
    let live = true;
    const poll = () => {
      invoke('read_base_orientation', { target: basePath }).then(
        reply => { if (live) tracker.observe(parseSensorReading(reply)); },
        () => { if (live) tracker.observe(null); },
      );
    };
    poll();
    const timer = setInterval(poll, sensorPollMs);
    return () => { live = false; clearInterval(timer); };
  }, [invoke, basePath]);
}
