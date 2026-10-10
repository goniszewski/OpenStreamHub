import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { LiveDevice } from './contracts';
import type { DeviceFeed } from './device-feed';
import { LightingPage } from './LightingPage';
import { favouritesStorageKey } from './lighting-model';
import { FakeStorage } from './fake-storage';
import type { ServiceRunner } from './service-runner';

const dial = (path: string, serial: string | null = 'MHDD01FAKE0843'): LiveDevice =>
  ({ path_hex: path, module: 'm131e', serial_number: serial });
const knobs = (): LiveDevice => ({ path_hex: 'kn', module: 'm003k', serial_number: null });
const idleLike = { desktop: true, devices: [] as LiveDevice[], inventoryLoaded: true, autoDetect: true, feeds: {}, live: {}, sessionModules: [], error: null };
// The component reads only subscribe and getState; a frozen snapshot stands in for the store.
const feedWith = (devices: LiveDevice[]) => ({ subscribe: () => () => {}, getState: () => ({ ...idleLike, devices }) } as unknown as DeviceFeed);
const runOwned = { isActivePath: () => true } as unknown as ServiceRunner;

// Hooks (feed subscription, control state) need the SSR renderer, so these
// render through JSX instead of calling the component as a function.
const render = (props: Partial<Parameters<typeof LightingPage>[0]> = {}) =>
  renderToStaticMarkup(<LightingPage desktop {...props} />);

describe('LightingPage', () => {
  it('renders usable controls for a connected dial', () => {
    const markup = render({ feed: feedWith([dial('aa'), knobs()]) });
    expect(markup).not.toContain('<fieldset class="lighting-controls" disabled');
    expect(markup).toContain('aria-label="Lighting scope"');
    expect(markup).toContain('This module');
    expect(markup).toContain('Whole kit');
    expect(markup).toContain('Encoder IPS display · …0843');
    expect(markup).toContain('3-knob module');
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
    const markup = render({ feed: feedWith([]) });
    expect(markup).toContain('No modules are connected. Connect a module to change its lighting.');
    expect(markup).toContain('<fieldset class="lighting-controls" disabled="">');
    // The picker and the scope stay reachable so the user can pick another module.
    expect(markup).toContain('aria-label="Lighting scope"');
  });

  it('shows unsupported modules as disabled controls with the module named', () => {
    const markup = render({ feed: feedWith([knobs()]) });
    expect(markup).toContain('3-knob module lighting is not available yet. The encoder dial is the only module OpenStreamHub lights right now.');
    expect(markup).toContain('<fieldset class="lighting-controls" disabled="">');
  });

  it('names a run that is driving a target dial', () => {
    const markup = render({ feed: feedWith([dial('aa')]), runner: runOwned });
    expect(markup).toContain('A run is driving a dial. Stop the run before changing its lighting.');
    expect(markup).toContain('<fieldset class="lighting-controls" disabled="">');
  });

  it('does not claim a run-owned dial is skipped — the page locks instead', () => {
    const markup = render({ feed: feedWith([dial('aa')]) });
    expect(markup).toContain('While a run is driving any dial, Whole kit is unavailable. Stop the run to change lighting.');
    expect(markup).not.toContain('is left out');
  });

  it('renders saved favourites from storage with use and remove labels', () => {
    const storage = new FakeStorage();
    storage.setItem(favouritesStorageKey, JSON.stringify(['#ff0000', '#00ff00']));
    const markup = render({ storage, feed: feedWith([dial('aa')]) });
    expect(markup).toContain('aria-label="Use colour #ff0000"');
    expect(markup).toContain('aria-label="Use colour #00ff00"');
    expect(markup).toContain('aria-label="Remove colour #ff0000"');
    expect(markup).toContain('style="background:#ff0000"');
  });
});
