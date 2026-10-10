import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import { dialDisplayArgs, dialDisplayOptions, effectiveDialDisplay, maxDialImageBytes, setDialDisplayContent, setDialImage } from './dial-display-model';

const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
const dialBound: Profile = { ...flat, bindings: [{ module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 2 } }] };
const withFunctions: Profile = { ...flat, dial_functions: [{ name: 'Volume', bindings: [] }] };

describe('dial display model', () => {
  it('offers every value with the service argument spelling', () => {
    expect(dialDisplayOptions.map(option => option.value)).toEqual(['function', 'clock', 'analog_clock', 'cpu', 'image', 'off']);
    expect(dialDisplayArgs).toEqual({ off: 'off', function: 'function', clock: 'clock', analog_clock: 'analog-clock', cpu: 'cpu', image: 'image' });
  });
  it('defaults to the function name and falls back to the analog clock for an idle dial', () => {
    expect(effectiveDialDisplay(flat)).toBe('analog_clock');
    expect(effectiveDialDisplay(dialBound)).toBe('function');
    expect(effectiveDialDisplay(withFunctions)).toBe('function');
    // Every explicit choice is kept, including off and image.
    for (const content of ['off', 'clock', 'analog_clock', 'cpu', 'image'] as const) {
      expect(effectiveDialDisplay({ ...flat, dial_display: { content } })).toBe(content);
    }
    expect(effectiveDialDisplay({ ...flat, dial_display: { content: 'function' } })).toBe('analog_clock');
  });
  it('stores the choice and clears a picture when another face is picked', () => {
    const pictured = setDialImage(flat, 'AAAA');
    expect(pictured.dial_display).toEqual({ content: 'image', image: 'AAAA' });
    // The rest of the profile is untouched and the field is carried through.
    const changed = setDialDisplayContent(pictured, 'clock');
    expect(changed.dial_display).toEqual({ content: 'clock' });
    expect(changed.name).toBe('Flat');
    expect(changed.schema_version).toBe(1);
    // The rest of the profile keeps its identity except the display field.
    expect({ ...changed, dial_display: undefined }).toEqual({ ...pictured, dial_display: undefined });
  });
  it('caps the stored picture like the Rust validator', () => {
    expect(maxDialImageBytes).toBe(64 * 1024);
  });
});
