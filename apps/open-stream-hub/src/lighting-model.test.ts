import { describe, expect, it, vi } from 'vitest';
import type { LiveDevice } from './contracts';
import { FakeStorage } from './fake-storage';
import {
  addColourFavourite, appliedStatus, applyLighting, clampBacklight, colourChannels, defaultColour,
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
    expect(lightingTargets(devices, 'module', 'kn')).toEqual([]);
    expect(lightingTargets(devices, 'module', null)).toEqual([]);
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
      .toBe('Whole kit reaches the encoder dial’s lighting only right now, and no dial is connected.');
  });
  it('keeps the dial enabled and disables other modules with their name, not a hidden panel', () => {
    expect(gate({ selected: dial('aa') })).toBeNull();
    expect(gate({ selected: knobs(), devices: [knobs()] }))
      .toBe('3-knob module lighting is not available yet. The encoder dial is the only module OpenStreamHub lights right now.');
    expect(gate({ selected: keypad(), devices: [keypad()] }))
      .toBe('15-key IPS display lighting is not available yet. The encoder dial is the only module OpenStreamHub lights right now.');
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
