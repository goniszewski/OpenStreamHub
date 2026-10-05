import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import type { RigItem } from './rig-model';
import { addRigModule, boundControlCount, controlAction, defaultRig, faderFromPointer, maxRigItems, moveRigModule, readRig, removeRigModule, rigStorageKey, syncDeviceRig, writeRig } from './rig-model';

const profile: Profile = { schema_version: 1, name: 'p', bindings: [
  { module: 'm003k', control: 1, gesture: 'pressed', action: { type: 'toggle_mute' } },
  { module: 'm003k', control: 1, gesture: 'clockwise', action: { type: 'volume', delta: 5 } },
  { module: 'm005s', control: 2, gesture: 'position_changed', action: { type: 'volume_from_position' } },
] };

describe('rig arrangement', () => {
  const rig: RigItem[] = [{ id: 'a', module: 'm003k' }, { id: 'b', module: 'm131e' }];
  it('adds up to the limit and removes by id', () => {
    expect(addRigModule(rig, 'm005s', 'c').map(item => item.id)).toEqual(['a', 'b', 'c']);
    let full = rig; for (let i = 0; full.length < maxRigItems; i++) full = addRigModule(full, 'm003k', `x${i}`);
    expect(addRigModule(full, 'm003k', 'over')).toBe(full);
    expect(removeRigModule(rig, 'a').map(item => item.id)).toEqual(['b']);
  });
  it('moves items and ignores moves past either end', () => {
    expect(moveRigModule(rig, 'a', 1).map(item => item.id)).toEqual(['b', 'a']);
    expect(moveRigModule(rig, 'a', -1)).toBe(rig);
    expect(moveRigModule(rig, 'b', 1)).toBe(rig);
    expect(moveRigModule(rig, 'missing', 1)).toBe(rig);
  });
});

describe('rig storage', () => {
  it('round-trips and falls back to the default for missing or invalid data', () => {
    const data = new Map<string, string>(); const storage = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
    expect(readRig(storage)).toBe(defaultRig);
    writeRig(storage, [{ id: 'a', module: 'm005s' }]);
    expect(readRig(storage)).toEqual([{ id: 'a', module: 'm005s' }]);
    data.set(rigStorageKey, '[{"id":"a","module":"nope"}]'); expect(readRig(storage)).toBe(defaultRig);
    data.set(rigStorageKey, 'not json'); expect(readRig(storage)).toBe(defaultRig);
  });
  it('survives storage that throws', () => {
    expect(readRig({ getItem: () => { throw new Error('blocked'); } })).toBe(defaultRig);
    expect(() => writeRig({ setItem: () => { throw new Error('full'); } }, [])).not.toThrow();
  });
});

describe('device rig sync', () => {
  const dev = (path_hex: string, module: 'm003k' | 'm131e' = 'm003k') => ({ path_hex, module, serial_number: null });
  const manual: RigItem[] = [{ id: 'a', module: 'm005s' }];
  it('appends connected devices and leaves manual items alone', () => {
    const next = syncDeviceRig(manual, [dev('01'), dev('02', 'm131e')]);
    expect(next).toEqual([...manual, { id: 'device:01', module: 'm003k' }, { id: 'device:02', module: 'm131e' }]);
    expect(syncDeviceRig(next, [dev('01'), dev('02', 'm131e')])).toBe(next);
  });
  it('removes vanished devices and handles a swap in one snapshot', () => {
    const rig = syncDeviceRig(manual, [dev('01'), dev('02', 'm131e')]);
    expect(syncDeviceRig(rig, [dev('02', 'm131e')]).map(item => item.id)).toEqual(['a', 'device:02']);
    expect(syncDeviceRig(rig, [dev('02', 'm131e'), dev('03')]).map(item => item.id)).toEqual(['a', 'device:02', 'device:03']);
    expect(syncDeviceRig(rig, []).map(item => item.id)).toEqual(['a']);
  });
  it('keeps a reordered position and never persists device items', () => {
    const rig = moveRigModule(syncDeviceRig(manual, [dev('01')]), 'device:01', -1);
    expect(syncDeviceRig(rig, [dev('01')])).toBe(rig);
    const data = new Map<string, string>(); const storage = { getItem: (k: string) => data.get(k) ?? null, setItem: (k: string, v: string) => void data.set(k, v) };
    writeRig(storage, rig);
    expect(readRig(storage)).toEqual(manual);
    data.set(rigStorageKey, JSON.stringify([...manual, { id: 'device:zz', module: 'm003k' }]));
    expect(readRig(storage)).toEqual(manual);
  });
});

describe('rig control display', () => {
  it('shows the first binding in gesture order and counts bound controls', () => {
    expect(controlAction(profile, 'm003k', 1)).toEqual({ type: 'volume', delta: 5 });
    expect(controlAction(profile, 'm003k', 0)).toBeUndefined();
    expect(boundControlCount(profile, 'm003k')).toBe(1);
    expect(boundControlCount(profile, 'm005s')).toBe(1);
    expect(boundControlCount(profile, 'm4315')).toBe(0);
  });
  it('maps a fader pointer position, top to bottom, onto 0..maximum', () => {
    expect(faderFromPointer(0, 200, 100)).toBe(100);
    expect(faderFromPointer(200, 200, 100)).toBe(0);
    expect(faderFromPointer(50, 200, 100)).toBe(75);
    expect(faderFromPointer(-40, 200, 100)).toBe(100);
    expect(faderFromPointer(999, 200, 100)).toBe(0);
    expect(faderFromPointer(10, 0, 100)).toBe(0);
  });
});
