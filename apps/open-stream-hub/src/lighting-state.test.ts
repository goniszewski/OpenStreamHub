import { describe, expect, it } from 'vitest';
import {
  baseGlow, lightMotion,
  appearedTargets, controlsFromRecord, kindLighting, lightingStateStorageKey, litLedColours, LightingStateStore,
  matchedModuleColour, maxLightingRecords, readLightingRecords, recordFits, restoreLighting, withDialLook, withLedLook,
} from './lighting-state';
import type { StoredLighting } from './lighting-state';
import { FakeStorage } from './fake-storage';
import type { LightingDevice } from './lighting-model';

const knobLook = { colour: '#ff0000', effect: 1, brightness: 80 };

const unit = (path: string, module: LightingDevice['module']): LightingDevice => ({ path_hex: path, module, serial_number: null });

describe('lighting state', () => {
  it('records a dial look with the values an apply validated', () => {
    const record = withDialLook(undefined, 72, '#1234fe');
    expect(record).toEqual({ kind: 'dial', backlight: 72, colour: '#1234fe' });
    expect(withDialLook(undefined, 250, 'not-a-colour')).toEqual({ kind: 'dial', backlight: 100, colour: '#ffffff' });
    expect(withDialLook({ kind: 'led', leds: [] }, 0, '#000000').kind).toBe('dial');
  });

  it('records a LED look per LED and leaves the others untouched', () => {
    const first = withLedLook(undefined, null, 3, knobLook);
    expect(first).toEqual({ kind: 'led', leds: [knobLook, knobLook, knobLook] });
    const second = withLedLook(first, 1, 3, { colour: '#00ff00', effect: 5, brightness: 40 });
    expect(second).toEqual({ kind: 'led', leds: [knobLook, { colour: '#00ff00', effect: 5, brightness: 40 }, knobLook] });
    // An out-of-range LED writes nothing new but keeps a well-formed record.
    expect(withLedLook(first, 7, 3, knobLook)).toEqual(first);
    // A dial record is replaced, not merged.
    expect(withLedLook({ kind: 'dial', backlight: 10, colour: '#ffffff' }, 0, 5, knobLook).kind).toBe('led');
  });

  it('names the record the rig shows for a module kind', () => {
    const records = {
      kn2: { kind: 'led', leds: [knobLook, null, null] } as StoredLighting,
      fd: { kind: 'led', leds: [null, null, null, null, null] } as StoredLighting,
    };
    const devices = [unit('kn1', 'm003k'), unit('kn2', 'm003k'), unit('fd', 'm005s')];
    expect(kindLighting(records, devices, 'm003k')).toEqual(records.kn2);
    expect(kindLighting(records, devices, 'm005s')).toEqual(records.fd);
    expect(kindLighting({}, devices, 'm003k')).toBeUndefined();
    expect(kindLighting(records, [], 'm005s')).toBeUndefined();
  });

  it('hands the base strip the first lit module LED colour, in inventory order', () => {
    const records = {
      kn: { kind: 'led', leds: [null, { colour: '#00ff00', effect: 0, brightness: 100 }, { colour: '#ff0000', effect: 1, brightness: 80 }] } as StoredLighting,
      fd: { kind: 'led', leds: [{ colour: '#0000ff', effect: 1, brightness: 100 }, null, null, null, null] } as StoredLighting,
      dl: { kind: 'dial', backlight: 50, colour: '#ffffff' } as StoredLighting,
    };
    const devices = [unit('dl', 'm131e'), unit('fd', 'm005s'), unit('kn', 'm003k'), unit('bs', 'm0064')];
    expect(matchedModuleColour(records, devices)).toBe('#0000ff');
    // An Off LED is not lit: the next module's lit LED wins.
    const faderOff = { ...records, fd: { kind: 'led', leds: [{ colour: '#0000ff', effect: 0, brightness: 100 }, null, null, null, null] } as StoredLighting };
    expect(matchedModuleColour(faderOff, devices)).toBe('#ff0000');
    expect(matchedModuleColour({}, devices)).toBeNull();
    expect(matchedModuleColour(records, [unit('bs', 'm0064')])).toBeNull();
  });

  it('lights only LEDs that are on', () => {
    const record: StoredLighting = {
      kind: 'led',
      leds: [
        { colour: '#ff0000', effect: 1, brightness: 80 },
        { colour: '#00ff00', effect: 0, brightness: 80 },
        { colour: '#0000ff', effect: 2, brightness: 0 },
        null,
      ],
    };
    expect(litLedColours(record, 4)).toEqual(['#ff0000', null, null, 'unknown']);
    expect(litLedColours(undefined, 3)).toEqual(['unknown', 'unknown', 'unknown']);
    expect(litLedColours({ kind: 'dial', backlight: 50, colour: '#ffffff' }, 2)).toEqual(['unknown', 'unknown']);
  });

  it('starts the controls from the stored look of the picked LED', () => {
    const record: StoredLighting = { kind: 'led', leds: [knobLook, { colour: '#00ff00', effect: 7, brightness: 30 }, null] };
    expect(controlsFromRecord(record, 'led', 1)).toEqual({ backlight: 30, colour: '#00ff00', effect: 7 });
    expect(controlsFromRecord(record, 'led', null)).toEqual({ backlight: 80, colour: '#ff0000', effect: 1 });
    expect(controlsFromRecord(record, 'led', 2)).toBeNull();
    expect(controlsFromRecord(undefined, 'led', 0)).toBeNull();
    expect(controlsFromRecord({ kind: 'dial', backlight: 40, colour: '#1234fe' }, 'dial', null))
      .toEqual({ backlight: 40, colour: '#1234fe', effect: 0 });
    expect(controlsFromRecord(record, 'dial', null)).toBeNull();
  });

  it('keeps the records across a store rebuild and drops what it cannot read', () => {
    const storage = new FakeStorage();
    const store = new LightingStateStore(storage);
    store.record(['aa'], () => ({ kind: 'dial', backlight: 60, colour: '#1234fe' }));
    store.record(['bb'], () => ({ kind: 'led', leds: [knobLook, null, null] }));
    const reread = new LightingStateStore(storage);
    expect(reread.getState().records.aa).toEqual({ kind: 'dial', backlight: 60, colour: '#1234fe' });
    expect(reread.getState().records.bb).toEqual({ kind: 'led', leds: [knobLook, null, null] });
    expect(Object.keys(reread.getState().records)).toEqual(['aa', 'bb']);
  });

  it('forgets a record and tells its listeners', () => {
    const store = new LightingStateStore(null);
    let seen: string[] = [];
    const stop = store.subscribe(() => { seen = Object.keys(store.getState().records); });
    store.record(['aa'], () => ({ kind: 'dial', backlight: 10, colour: '#ffffff' }));
    expect(seen).toEqual(['aa']);
    store.record(['aa'], () => undefined);
    expect(seen).toEqual([]);
    store.record(['aa'], () => undefined);
    stop();
    store.record(['aa'], () => ({ kind: 'dial', backlight: 10, colour: '#ffffff' }));
    expect(seen).toEqual([]);
  });

  it('reads stored records, keeping the well-formed ones only', () => {
    const storage = new FakeStorage();
    storage.setItem(lightingStateStorageKey, JSON.stringify({
      good: { kind: 'dial', backlight: 120, colour: '1234fe' },
      led: { kind: 'led', leds: [{ colour: '#00ff00', effect: 12, brightness: -5 }, 'junk', null, { colour: 'nope', effect: 1, brightness: 5 }] },
      bad: { kind: 'mixer' },
      null: null,
    }));
    const records = readLightingRecords(storage);
    expect(records.good).toEqual({ kind: 'dial', backlight: 100, colour: '#1234fe' });
    expect(records.led).toEqual({ kind: 'led', leds: [{ colour: '#00ff00', effect: 9, brightness: 0 }, null, null, null] });
    expect(records.bad).toBeUndefined();
    expect(Object.keys(records).sort()).toEqual(['bad', 'good', 'led'].filter(key => key !== 'bad'));
    storage.setItem(lightingStateStorageKey, '{broken');
    expect(readLightingRecords(storage)).toEqual({});
    storage.setItem(lightingStateStorageKey, JSON.stringify([1]));
    expect(readLightingRecords(storage)).toEqual({});
  });

  it('names the units that appeared since the last inventory', () => {
    const devices = [unit('aa', 'm003k'), unit('bb', 'm005s')];
    expect(appearedTargets(null, devices)).toEqual(devices);
    expect(appearedTargets(['aa', 'bb'], devices)).toEqual([]);
    expect(appearedTargets(['aa'], devices)).toEqual([devices[1]]);
    expect(appearedTargets(['aa', 'cc'], [])).toEqual([]);
  });

  it('sends a replugged unit its stored looks again, skipping runs and failures', async () => {
    const store = new LightingStateStore(null);
    store.record(['dl'], () => ({ kind: 'dial', backlight: 55, colour: '#1234fe' }));
    store.record(['kn'], existing => withLedLook(existing, 2, 3, knobLook));
    store.record(['ru'], () => ({ kind: 'dial', backlight: 10, colour: '#ffffff' }));
    const calls: Array<[string, unknown]> = [];
    const invoke = (async (command: string, args: unknown) => {
      calls.push([command, args]);
      if (calls.length === 2) throw new Error('transport gone');
      return 1;
    }) as never;
    const restored = await restoreLighting(store, [
      unit('dl', 'm131e'), unit('kn', 'm003k'), unit('ru', 'm131e'),
    ], invoke, device => device.path_hex === 'ru');
    expect(restored).toBe(1);
    expect(calls).toEqual([
      ['set_dial_lighting', { targets: ['dl'], backlight: 55, colour: [0x12, 0x34, 0xfe] }],
      ['set_led_lighting', { targets: ['kn'], led: 2, colour: [0xff, 0, 0], mode: 1, brightness: 80 }],
    ]);
  });

  it('sends every stored LED of a module when they differ', async () => {
    const store = new LightingStateStore(null);
    store.record(['kn'], existing => withLedLook(withLedLook(existing, 0, 3, knobLook), 1, 3, { colour: '#00ff00', effect: 5, brightness: 40 }));
    const calls: Array<[string, unknown]> = [];
    const invoke = (async (command: string, args: unknown) => { calls.push([command, args]); return 1; }) as never;
    const restored = await restoreLighting(store, [unit('kn', 'm003k')], invoke, () => false);
    expect(restored).toBe(1);
    expect(calls).toEqual([
      ['set_led_lighting', { targets: ['kn'], led: 0, colour: [0xff, 0, 0], mode: 1, brightness: 80 }],
      ['set_led_lighting', { targets: ['kn'], led: 1, colour: [0, 0xff, 0], mode: 5, brightness: 40 }],
    ]);
  });
});

describe('pruning the stored records', () => {
  const dial: StoredLighting = { kind: 'dial', backlight: 50, colour: '#ffffff' };
  const knobs: StoredLighting = { kind: 'led', leds: [knobLook, knobLook, knobLook] };
  const faders: StoredLighting = { kind: 'led', leds: [knobLook, knobLook, knobLook, knobLook, knobLook] };
  const base: StoredLighting = { kind: 'led', leds: [knobLook] };

  it('knows which module a look can still be applied to', () => {
    expect(recordFits(dial, 'm131e')).toBe(true);
    expect(recordFits(dial, 'm003k')).toBe(false);
    expect(recordFits(knobs, 'm003k')).toBe(true);
    expect(recordFits(knobs, 'm005s')).toBe(false);
    expect(recordFits(faders, 'm005s')).toBe(true);
    expect(recordFits(faders, 'm003k')).toBe(false);
    expect(recordFits(base, 'm0064')).toBe(true);
    expect(recordFits(base, 'm003k')).toBe(false);
    expect(recordFits(knobs, 'm131e')).toBe(false);
    expect(recordFits(knobs, 'm4315')).toBe(false);
  });

  it('forgets a record whose path now answers as another kind, keeps same-kind units and units that are not connected', () => {
    const storage = new FakeStorage();
    const store = new LightingStateStore(storage);
    store.record(['swapped'], () => dial);
    store.record(['resized'], () => knobs);
    store.record(['same'], () => faders);
    store.record(['away'], () => dial);
    let notified = 0;
    store.subscribe(() => { notified += 1; });
    const forgotten = store.forgetUnappliable([unit('swapped', 'm003k'), unit('resized', 'm005s'), unit('same', 'm005s'), unit('other', 'm131e')]);
    expect(forgotten.sort()).toEqual(['resized', 'swapped']);
    expect(Object.keys(store.getState().records)).toEqual(['same', 'away']);
    expect(notified).toBe(1);
    expect(Object.keys(readLightingRecords(storage))).toEqual(['same', 'away']);
    // Nothing to forget: nothing written, nobody told.
    expect(store.forgetUnappliable([unit('same', 'm005s')])).toEqual([]);
    expect(notified).toBe(1);
  });

  it('keeps the newest records past the cap, dropping the one written longest ago', () => {
    const storage = new FakeStorage();
    const store = new LightingStateStore(storage);
    for (let index = 0; index < maxLightingRecords; index += 1) store.record([`unit${index}`], () => dial);
    expect(Object.keys(store.getState().records)).toHaveLength(maxLightingRecords);
    // A rewrite makes the oldest record the newest, so the next one over the cap drops the second-oldest.
    store.record(['unit0'], () => ({ kind: 'dial', backlight: 10, colour: '#ffffff' }));
    store.record(['extra'], () => dial);
    const kept = Object.keys(store.getState().records);
    expect(kept).toHaveLength(maxLightingRecords);
    expect(kept).not.toContain('unit1');
    expect(kept.at(-2)).toBe('unit0');
    expect(kept.at(-1)).toBe('extra');
    // The order survives a restart.
    expect(Object.keys(new LightingStateStore(storage).getState().records)).toEqual(kept);
  });

  it('trims an over-long stored list on read, newest kept', () => {
    const storage = new FakeStorage();
    const stored = Object.fromEntries(Array.from({ length: maxLightingRecords + 5 }, (_, index) => [`unit${index}`, dial]));
    storage.setItem(lightingStateStorageKey, JSON.stringify(stored));
    const records = Object.keys(readLightingRecords(storage));
    expect(records).toHaveLength(maxLightingRecords);
    expect(records[0]).toBe('unit5');
    expect(records.at(-1)).toBe(`unit${maxLightingRecords + 4}`);
  });

  it('restores a same-kind replug exactly as before', async () => {
    const store = new LightingStateStore(null);
    store.record(['k'], () => knobs);
    store.forgetUnappliable([unit('k', 'm003k')]);
    const calls: string[] = [];
    const restored = await restoreLighting(store, [unit('k', 'm003k')], (async (command: string) => { calls.push(command); return 1; }) as never, () => false);
    expect(restored).toBe(1);
    expect(calls).toEqual(['set_led_lighting', 'set_led_lighting', 'set_led_lighting']);
  });
});

describe('light motion and the base glow', () => {
  const led = (effect: number, brightness = 100, colour = '#ff0000') => ({ colour, effect, brightness });
  const record = (...leds: ReturnType<typeof led>[]) => ({ kind: 'led' as const, leds });
  it('names how lit LEDs move, ignoring off and dark ones', () => {
    expect(lightMotion(undefined, 3)).toBeNull();
    expect(lightMotion(record(led(1), led(1), led(1)), 3)).toBeNull();
    expect(lightMotion(record(led(2), led(1), led(1)), 3)).toBe('pulse');
    expect(lightMotion(record(led(2), led(4), led(1)), 3)).toBe('blink');
    expect(lightMotion(record(led(1), led(9), led(1)), 3)).toBe('cycle');
    expect(lightMotion(record(led(9, 0), led(0), led(1)), 3)).toBeNull();
  });
  it('glows with the base strip colour and brightness only while it is lit', () => {
    expect(baseGlow(undefined)).toBeNull();
    expect(baseGlow(record(led(0)))).toBeNull();
    expect(baseGlow(record(led(1, 0)))).toBeNull();
    expect(baseGlow(record(led(1, 60, '#00ff00')))).toEqual({ colour: '#00ff00', brightness: 60, motion: null });
    expect(baseGlow(record(led(7)))?.motion).toBe('cycle');
  });
});

describe('what modules report about themselves', () => {
  it('fills in a unit with no stored look, never overriding a stored one', () => {
    const store = new LightingStateStore(null);
    store.setReported('fd', { brightness: [100, 100, 100, 100, 100], modes: [9, 9, 9, 9, 9] });
    store.setReported('kn', { brightness: [100, 100, 100], modes: [9, 9, 9] });
    store.record(['kn'], () => ({ kind: 'led', leds: [{ colour: '#00ff00', effect: 1, brightness: 50 }, null, null] }));
    expect(store.getState().records.fd).toBeUndefined();
    expect(litLedColours(store.getState().shown.fd, 5).every(colour => colour !== null && colour !== 'unknown')).toBe(true);
    expect(lightMotion(store.getState().shown.fd, 5)).toBe('cycle');
    expect(store.getState().shown.kn).toBe(store.getState().records.kn);
  });
  it('keeps every LED of a rainbow in step', () => {
    const colours = litLedColours({ kind: 'led', leds: Array.from({ length: 5 }, () => ({ colour: '#ff0000', effect: 9, brightness: 100 })) }, 5);
    expect(new Set(colours).size).toBe(1);
  });
});
