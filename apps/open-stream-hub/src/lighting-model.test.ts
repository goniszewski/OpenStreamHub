import { describe, expect, it, vi } from 'vitest';
import type { LiveDevice } from './contracts';
import { FakeStorage } from './fake-storage';
import {
  addColourFavourite, appliedLedStatus, appliedStatus, applyDelayMs, applyLedLighting, applyLighting, ledCount, lightingEffects, lightingKind, clampBacklight, colourChannels, createApplyQueue, defaultColour,
  favouritesStorageKey, lightingBlockReason, lightingDeviceLabel, lightingTargets, maxColourFavourites,
  normaliseHexColour, readColourFavourites, removeColourFavourite, resetLighting, resetStatus,
  writeColourFavourites,
} from './lighting-model';
import type { Invoke } from './profile-session';

const invokeRecording = (reply: number) => vi.fn(() => Promise.resolve(reply)) as unknown as Invoke;

const dial = (path: string, serial: string | null = 'MHDD01FAKE0843'): LiveDevice =>
  ({ path_hex: path, module: 'm131e', serial_number: serial });
const keypad = (): LiveDevice => ({ path_hex: 'ke', module: 'm4315', serial_number: 'MHKE01' });
const knobs = (): LiveDevice => ({ path_hex: 'kn', module: 'm003k', serial_number: null });

describe('lightingTargets', () => {
  const devices = [keypad(), dial('aa'), knobs(), dial('bb')];

  it('reaches only the picked dial per module', () => {
    expect(lightingTargets(devices, 'module', 'aa').map(device => device.path_hex)).toEqual(['aa']);
  });
  it('reaches every dial, in inventory order, for the whole kit', () => {
    expect(lightingTargets(devices, 'kit', null).map(device => device.path_hex)).toEqual(['aa', 'bb']);
  });
  it('never targets a module that cannot be lit', () => {
    expect(lightingTargets(devices, 'module', 'ke')).toEqual([]);
    expect(lightingTargets(devices, 'module', null)).toEqual([]);
  });
  it('lights a picked knob or fader module, and the whole kit means every knob and fader module', () => {
    const fader: LiveDevice = { path_hex: 'fd', module: 'm005s', serial_number: null };
    const all = [keypad(), dial('aa'), knobs(), fader];
    expect(lightingTargets(all, 'module', 'kn').map(device => device.path_hex)).toEqual(['kn']);
    expect(lightingTargets(all, 'kit', 'kn').map(device => device.path_hex)).toEqual(['kn', 'fd']);
    expect(lightingTargets(all, 'kit', 'aa').map(device => device.path_hex)).toEqual(['aa']);
  });
});

describe('lightingBlockReason', () => {
  const gate = (overrides: Partial<Parameters<typeof lightingBlockReason>[0]>) => lightingBlockReason({
    desktop: true, runOwnedTargets: false, scope: 'module', selected: dial('aa'), devices: [dial('aa')], ...overrides,
  });

  it('explains the browser preview first', () => {
    expect(gate({ desktop: false })).toBe('The browser preview cannot reach hardware.');
  });
  it('names a run that holds a target dial', () => {
    expect(gate({ runOwnedTargets: true })).toBe('A run is driving a dial. Stop the run before changing its lighting.');
  });
  it('asks for a module when nothing is connected', () => {
    expect(gate({ selected: undefined, devices: [] })).toBe('No modules are connected. Connect a module to change its lighting.');
  });
  it('keeps whole kit enabled with a dial and explains a kit without one', () => {
    expect(gate({ scope: 'kit' })).toBeNull();
    expect(gate({ scope: 'kit', selected: keypad(), devices: [keypad(), knobs()] }))
      .toBe('Whole kit lights the encoder dials, or the knob and fader modules, and none is connected.');
    expect(gate({ scope: 'kit', selected: knobs(), devices: [knobs()] })).toBeNull();
  });
  it('keeps the dial enabled and disables other modules with their name, not a hidden panel', () => {
    expect(gate({ selected: dial('aa') })).toBeNull();
    expect(gate({ selected: knobs(), devices: [knobs()] })).toBeNull();
    expect(gate({ selected: keypad(), devices: [keypad()] }))
      .toBe('15-key IPS display lighting is not available yet. Only the encoder dial, knob and fader modules are lit right now.');
  });
});

describe('lightingDeviceLabel', () => {
  it('shows the module name with a serial tail for disambiguation', () => {
    expect(lightingDeviceLabel(dial('aa'))).toBe('Encoder IPS display · …0843');
    expect(lightingDeviceLabel(dial('aa', null))).toBe('Encoder IPS display');
    expect(lightingDeviceLabel(knobs())).toBe('3-knob module');
  });
});

describe('colours', () => {
  it('normalises the accepted hex shapes to the picker’s form', () => {
    expect(normaliseHexColour('#abc')).toBe('#aabbcc');
    expect(normaliseHexColour('ff0000')).toBe('#ff0000');
    expect(normaliseHexColour(' #6337ef ')).toBe('#6337ef');
    expect(normaliseHexColour('bright')).toBeNull();
  });
  it('splits a colour into the verified R, G, B channel order', () => {
    expect(colourChannels('#ff0000')).toEqual([255, 0, 0]);
    expect(colourChannels('#123456')).toEqual([0x12, 0x34, 0x56]);
    expect(colourChannels('nope')).toEqual([255, 255, 255]);
  });
  it('clamps the backlight like the backend bounds it', () => {
    expect(clampBacklight(0)).toBe(0);
    expect(clampBacklight(72.4)).toBe(72);
    expect(clampBacklight(150)).toBe(100);
    expect(clampBacklight(-5)).toBe(0);
    expect(clampBacklight(Number.NaN)).toBe(100);
  });
});

describe('apply and reset requests', () => {
  it('shapes the set_dial_lighting invoke', async () => {
    const invoke = invokeRecording(2);
    const written = await applyLighting(invoke, { targets: ['aa', 'bb'], backlight: 250, colour: '#abc' });
    expect(written).toBe(2);
    expect(invoke).toHaveBeenCalledWith('set_dial_lighting', { targets: ['aa', 'bb'], backlight: 100, colour: [0xaa, 0xbb, 0xcc] });
  });
  it('shapes the reset_dial_lighting invoke', async () => {
    const invoke = invokeRecording(1);
    expect(await resetLighting(invoke, ['aa'])).toBe(1);
    expect(invoke).toHaveBeenCalledWith('reset_dial_lighting', { targets: ['aa'] });
  });
  it('reports one dial and several dials differently', () => {
    expect(appliedStatus(1)).toBe('Applied to 1 dial.');
    expect(appliedStatus(2)).toBe('Applied to 2 dials.');
    expect(resetStatus(1)).toBe('The dial returned to its built-in animation.');
    expect(resetStatus(3)).toBe('3 dials returned to their built-in animation.');
  });
});

describe('colour favourites', () => {
  it('start empty and survive damaged storage', () => {
    expect(readColourFavourites(null)).toEqual([]);
    const storage = new FakeStorage();
    storage.setItem(favouritesStorageKey, '{oops');
    expect(readColourFavourites(storage)).toEqual([]);
    storage.setItem(favouritesStorageKey, JSON.stringify(['#ff0000', 7, 'zzz', '0f0']));
    expect(readColourFavourites(storage)).toEqual(['#ff0000', '#00ff00']);
  });
  it('round-trip through storage, deduplicated and newest first', () => {
    const storage = new FakeStorage();
    let colours = addColourFavourite([], '#ff0000');
    colours = addColourFavourite(colours, '00ff00');
    colours = addColourFavourite(colours, '#f00');
    writeColourFavourites(storage, colours);
    expect(colours).toEqual(['#ff0000', '#00ff00']);
    expect(readColourFavourites(storage)).toEqual(['#ff0000', '#00ff00']);
  });
  it('cap at the favourite limit and drop an unparsable colour', () => {
    const full = Array.from({ length: maxColourFavourites }, (_, index) => `#${index.toString(16).padStart(2, '0')}0000`);
    const next = addColourFavourite(full, '#ffffff');
    expect(next).toHaveLength(maxColourFavourites);
    expect(next[0]).toBe('#ffffff');
    expect(next).not.toContain(full[maxColourFavourites - 1]);
    expect(addColourFavourite(['#ff0000'], 'nope')).toEqual(['#ff0000']);
  });
  it('remove by exact colour', () => {
    expect(removeColourFavourite(['#ff0000', '#00ff00'], '#ff0000')).toEqual(['#00ff00']);
    expect(removeColourFavourite(['#ff0000'], '#00ff00')).toEqual(['#ff0000']);
  });
});

describe('defaultColour', () => {
  it('is a valid white the picker and the backend accept', () => {
    expect(normaliseHexColour(defaultColour)).toBe('#ffffff');
  });
});

describe('createApplyQueue', () => {
  it('sends the last value when a change lands during a slow write', async () => {
    const sent: string[] = [];
    let value = 'first';
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const queue = createApplyQueue(async () => { sent.push(value); await gate; }, applyDelayMs);
    const first = queue.fire();
    // The slider moves while the first write is still running.
    value = 'second';
    await queue.fire();
    expect(sent).toEqual(['first']);
    release();
    await first;
    queue.dispose();
    expect(sent).toEqual(['first', 'second']);
  });

  it('coalesces several mid-flight changes into one trailing apply', async () => {
    const sent: string[] = [];
    let value = 'a';
    let release!: () => void;
    const gate = new Promise<void>(resolve => { release = resolve; });
    const queue = createApplyQueue(async () => { sent.push(value); await gate; }, applyDelayMs);
    const first = queue.fire();
    value = 'b';
    await queue.fire();
    value = 'c';
    await queue.fire();
    release();
    await first;
    queue.dispose();
    expect(sent).toEqual(['a', 'c']);
  });

  it('debounces a burst into one apply', async () => {
    const sent: string[] = [];
    const queue = createApplyQueue(async () => { sent.push('ran'); }, 15);
    queue.schedule();
    queue.schedule();
    queue.schedule();
    await new Promise(resolve => setTimeout(resolve, 60));
    queue.dispose();
    expect(sent).toEqual(['ran']);
  });

  it('stops scheduling and firing after dispose', async () => {
    const sent: string[] = [];
    const queue = createApplyQueue(async () => { sent.push('ran'); }, 5);
    queue.dispose();
    queue.schedule();
    await queue.fire();
    await new Promise(resolve => setTimeout(resolve, 30));
    expect(sent).toEqual([]);
  });
});

describe('knob and fader lighting', () => {
  it('knows which modules take which lighting', () => {
    expect(lightingKind('m131e')).toBe('dial');
    expect(lightingKind('m003k')).toBe('led');
    expect(lightingKind('m005s')).toBe('led');
    expect(lightingKind('m4315')).toBeNull();
    expect([ledCount('m003k'), ledCount('m005s'), ledCount('m131e')]).toEqual([3, 5, 0]);
  });
  it('offers every effect number 0 to 9 once, Static first', () => {
    expect(lightingEffects[0]).toEqual({ value: 1, label: 'Static' });
    expect(lightingEffects.map(item => item.value).sort()).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  });
  it('shapes the apply with raw channels, clamped effect and brightness', async () => {
    const invoke = invokeRecording(2);
    await expect(applyLedLighting(invoke, { targets: ['kn', 'fd'], led: 1, colour: '#123456', effect: 12, brightness: 140 })).resolves.toBe(2);
    expect(invoke).toHaveBeenCalledWith('set_led_lighting', {
      targets: ['kn', 'fd'], led: 1, colour: [0x12, 0x34, 0x56], mode: 9, brightness: 100,
    });
    await applyLedLighting(invoke, { targets: ['kn'], led: null, colour: '#ffffff', effect: -1, brightness: 5 });
    expect(invoke).toHaveBeenLastCalledWith('set_led_lighting', { targets: ['kn'], led: null, colour: [255, 255, 255], mode: 0, brightness: 5 });
  });
  it('words the status for modules', () => {
    expect(appliedLedStatus(1)).toBe('Applied to 1 module.');
    expect(appliedLedStatus(3)).toBe('Applied to 3 modules.');
  });
});
