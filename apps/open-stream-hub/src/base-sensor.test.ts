import { describe, expect, it, vi } from 'vitest';
import { FakeStorage } from './fake-storage';
import { createBaseSensorStore, createSensorTracker, orientationFromSensor, parseSensorReading, sensorPosition, sensorStorageKey, unavailableAfter } from './base-sensor';
import type { BaseOrientation } from './base-orientation';

describe('base sensor', () => {
  it('reads state 1 as lying down, as measured, and 0 as standing', () => {
    expect(sensorPosition(1)).toBe('landscape');
    expect(sensorPosition(0)).toBe('standing');
    expect(parseSensorReading(1)).toBe(1);
    expect(parseSensorReading(0)).toBe(0);
    expect(parseSensorReading(2)).toBeNull();
    expect(parseSensorReading('1')).toBeNull();
    expect(parseSensorReading(null)).toBeNull();
  });
  it('keeps the user\'s half turn within the sensor\'s axis', () => {
    expect(orientationFromSensor('landscape', 180)).toBe(180);
    expect(orientationFromSensor('landscape', 0)).toBe(0);
    expect(orientationFromSensor('landscape', 90)).toBe(0);
    expect(orientationFromSensor('landscape', 270)).toBe(0);
    expect(orientationFromSensor('standing', 270)).toBe(270);
    expect(orientationFromSensor('standing', 90)).toBe(90);
    expect(orientationFromSensor('standing', 0)).toBe(90);
    expect(orientationFromSensor('standing', 180)).toBe(90);
  });
  const tracker = (follow: boolean, start: BaseOrientation = 0) => {
    let current = start;
    const applied: BaseOrientation[] = [];
    const reports: (string | null)[] = [];
    const t = createSensorTracker({ follow: () => follow, current: () => current, apply: o => { current = o; applied.push(o); }, report: p => reports.push(p) });
    return { t, applied, reports, turn: (o: BaseOrientation) => { current = o; } };
  };
  it('turns the layout when the sensor first reads and when it moves, and not on every repeat', () => {
    const { t, applied, reports } = tracker(true, 0);
    t.observe(1); t.observe(1); t.observe(1);
    expect(applied).toEqual([0]);
    t.observe(0); t.observe(0);
    expect(applied).toEqual([0, 90]);
    expect(reports).toEqual(['landscape', 'standing']);
  });
  it('leaves a turn the user picked by hand alone until the base really moves', () => {
    const { t, applied, turn } = tracker(true, 0);
    t.observe(1);
    turn(180);
    t.observe(1);
    t.observe(1);
    expect(applied).toEqual([0]);
    t.observe(0);
    expect(applied).toEqual([0, 90]);
  });
  it('shows the sensor but turns nothing when following is off', () => {
    const { t, applied, reports } = tracker(false, 0);
    t.observe(0);
    expect(applied).toEqual([]);
    expect(reports).toEqual(['standing']);
  });
  it('calls the sensor unavailable only after a few failed reads in a row', () => {
    const { t, reports } = tracker(true);
    t.observe(1);
    for (let count = 1; count < unavailableAfter; count++) t.observe(null);
    expect(reports).toEqual(['landscape']);
    t.observe(null);
    expect(reports).toEqual(['landscape', null]);
    t.observe(1);
    expect(reports).toEqual(['landscape', null, 'landscape']);
  });
  it('remembers whether to follow the sensor', () => {
    const storage = new FakeStorage();
    const store = createBaseSensorStore(storage);
    expect(store.getSnapshot()).toEqual({ follow: true, position: null });
    const listener = vi.fn();
    store.subscribe(listener);
    store.setFollow(false);
    store.setFollow(false);
    store.setPosition('standing');
    expect(listener).toHaveBeenCalledTimes(2);
    expect(storage.getItem(sensorStorageKey)).toBe('off');
    expect(createBaseSensorStore(storage).getSnapshot().follow).toBe(false);
  });
});
