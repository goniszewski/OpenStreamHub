import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createModuleSelection } from './module-selection';
import type { LightingDevice } from './lighting-model';
import { LightingPage } from './LightingPage';
import { favouritesStorageKey } from './lighting-model';
import { lightingStateStorageKey, LightingStateStore } from './lighting-state';
import { FakeStorage } from './fake-storage';
import type { ServiceRunner } from './service-runner';

const dial = (path: string, serial: string | null = 'MHDD01FAKE0843'): LightingDevice =>
  ({ path_hex: path, module: 'm131e', serial_number: serial });
const knobs = (): LightingDevice => ({ path_hex: 'kn', module: 'm003k', serial_number: null });
const runOwned = { isActivePath: () => true, runsHolding: () => ['run'] } as unknown as ServiceRunner;

// Hooks (feed subscription, control state) need the SSR renderer, so these
// render through JSX instead of calling the component as a function.
const render = (props: Partial<Parameters<typeof LightingPage>[0]> = {}) =>
  renderToStaticMarkup(<LightingPage desktop devices={[]} selection={createModuleSelection(null)} {...props} />);

describe('LightingPage', () => {
  it('renders usable controls for a connected dial', () => {
    const markup = render({ devices: [dial('aa'), knobs()] });
    expect(markup).not.toContain('<fieldset class="lighting-controls" disabled');
    expect(markup).toContain('aria-label="Lighting scope"');
    expect(markup).toContain('This module');
    expect(markup).toContain('Whole kit');
    expect(markup).toContain('id="lighting-backlight"');
    expect(markup).toContain('value="100"');
    expect(markup).toContain('id="lighting-colour"');
    expect(markup).toContain('value="#ffffff"');
    expect(markup).toContain('Save colour');
    expect(markup).not.toContain('aria-label="Use colour');
    expect(markup).not.toContain('Resetting…');
  });

  it('shows the preview reason and locks the controls in the browser', () => {
    const markup = render({ desktop: false });
    expect(markup).toContain('The browser preview cannot reach hardware.');
    expect(markup).toContain('<fieldset class="lighting-controls" disabled="">');
    expect(markup).toContain('<button type="button" class="button" disabled="">');
  });

  it('explains an empty kit instead of hiding the controls', () => {
    const markup = render({ devices: [] });
    expect(markup).toContain('No modules are connected. Connect a module to change its lighting.');
    expect(markup).toContain('<fieldset class="lighting-controls" disabled="">');
    // The picker and the scope stay reachable so the user can pick another module.
    expect(markup).toContain('aria-label="Lighting scope"');
  });

  it('shows unsupported modules as disabled controls with the module named', () => {
    const keypad: LightingDevice = { path_hex: 'ke', module: 'm4315', serial_number: null };
    const markup = render({ devices: [keypad] });
    expect(markup).toContain('15-key IPS display lighting is not available yet. Only the encoder dial, knob and fader modules are lit right now.');
    expect(markup).toContain('<fieldset class="lighting-controls" disabled="">');
  });

  it('gives a knob module effect, LED and brightness controls and no reset', () => {
    const markup = render({ devices: [knobs()] });
    expect(markup).not.toContain('<fieldset class="lighting-controls" disabled');
    expect(markup).toContain('id="lighting-effect"');
    expect(markup).toContain('<option value="4">Strobe, fast</option>');
    expect(markup).toContain('id="lighting-led"');
    expect(markup).toContain('LED 3');
    expect(markup).not.toContain('LED 4');
    expect(markup).toContain('>Brightness</label>');
    expect(markup).not.toContain('lucide-rotate-ccw');
  });

  it('lists five LEDs for a fader module and keeps the dial controls free of effects', () => {
    const faders: LightingDevice = { path_hex: 'fd', module: 'm005s', serial_number: null };
    const fadersMarkup = render({ devices: [faders] });
    expect(fadersMarkup).toContain('LED 5');
    expect(fadersMarkup).not.toContain('LED 6');
    const dialMarkup = render({ devices: [dial('aa')] });
    expect(dialMarkup).not.toContain('id="lighting-effect"');
    expect(dialMarkup).toContain('>Screen backlight</label>');
    expect(dialMarkup).toContain('lucide-rotate-ccw');
  });

  it('lets a dial that a run drives take its lighting, and locks only Reset', () => {
    const markup = render({ devices: [dial('aa')], runner: runOwned });
    expect(markup).not.toContain('Lighting is locked while');
    expect(markup).toContain('<fieldset class="lighting-controls">');
    expect(markup).toContain('Pause it to reset.');
    expect(markup).toContain('<fieldset class="lighting-colour-fieldset" disabled="">');
    expect(markup).toContain('would wipe the picture the running dial shows');
    expect(markup).toContain('>Pause</button>');
    expect(markup).toMatch(/<button type="button" class="button" disabled=""[^>]*><svg[^>]*lucide-rotate-ccw/);
  });

  it('says a dial takes its lighting while active', () => {
    const markup = render({ devices: [dial('aa')] });
    expect(markup).toContain('A dial that is running its display takes the backlight but not the colour');
    expect(markup).not.toContain('is left out');
  });

  it('renders saved favourites from storage with use and remove labels', () => {
    const storage = new FakeStorage();
    storage.setItem(favouritesStorageKey, JSON.stringify(['#ff0000', '#00ff00']));
    const markup = render({ storage, devices: [dial('aa')] });
    expect(markup).toContain('aria-label="Use colour #ff0000"');
    expect(markup).toContain('aria-label="Use colour #00ff00"');
    expect(markup).toContain('aria-label="Remove colour #ff0000"');
    expect(markup).toContain('style="background:#ff0000"');
  });

  it('gives an LED module a turn-off instead of a reset, and the dial keeps its reset', () => {
    const knobsMarkup = render({ devices: [knobs()] });
    expect(knobsMarkup).toContain('>Turn off<');
    expect(knobsMarkup).toContain('lucide-lightbulb-off');
    expect(knobsMarkup).not.toContain('lucide-rotate-ccw');
    expect(knobsMarkup).toContain('Turn off switches the picked LED, or every LED, back to Off.');
    const dialMarkup = render({ devices: [dial('aa')] });
    expect(dialMarkup).toContain('>Reset<');
    expect(dialMarkup).toContain('lucide-rotate-ccw');
  });

  it('lights the base from the page like a one-LED module', () => {
    const base: LightingDevice = { path_hex: 'bs', module: 'm0064', serial_number: null };
    const markup = render({ devices: [base] });
    expect(markup).not.toContain('<fieldset class="lighting-controls" disabled');
    expect(markup).toContain('id="lighting-effect"');
    expect(markup).not.toContain('id="lighting-led"');
    expect(markup).toContain('>Turn off<');
  });

  it('offers the base strip a match-modules colour from the lit modules', () => {
    const base: LightingDevice = { path_hex: 'bs', module: 'm0064', serial_number: null };
    const storage = new FakeStorage();
    storage.setItem(lightingStateStorageKey, JSON.stringify({
      kn: { kind: 'led', leds: [{ colour: '#ff0000', effect: 1, brightness: 80 }, null, null] },
    }));
    const lighting = new LightingStateStore(storage);
    const markup = render({ devices: [base, knobs()], lighting });
    expect(markup).toContain('Match modules');
    // The lit module is there, so the shortcut is enabled and names the colour.
    expect(markup).toContain('<button type="button" class="button compact" title="Copies #ff0000 from the first lit knob or fader LED">');
    const bare = render({ devices: [base] });
    expect(bare).toContain('<button type="button" class="button compact" disabled="" title="No lit knob or fader LED to match yet">');
    // Only the base gets the shortcut; other modules get the mention in the notes only.
    expect(render({ devices: [knobs()] })).not.toContain('>Match modules<');
  });

  it('starts the controls from the stored look of the picked unit', () => {
    const storage = new FakeStorage();
    storage.setItem(lightingStateStorageKey, JSON.stringify({
      kn: { kind: 'led', leds: [{ colour: '#ff0000', effect: 2, brightness: 60 }, { colour: '#00ff00', effect: 7, brightness: 30 }, null] },
      aa: { kind: 'dial', backlight: 40, colour: '#00ffcc' },
    }));
    const lighting = new LightingStateStore(storage);
    const knobsMarkup = render({ devices: [knobs()], lighting });
    expect(knobsMarkup).toContain('value="#ff0000"');
    expect(knobsMarkup).toContain('value="60"');
    expect(knobsMarkup).toContain('<option value="2" selected="">Breathing</option>');
    const dialMarkup = render({ devices: [dial('aa'), knobs()], lighting });
    expect(dialMarkup).toContain('value="40"');
    expect(dialMarkup).toContain('value="#00ffcc"');
  });
});
