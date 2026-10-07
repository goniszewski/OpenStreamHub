import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import fixture from '../fixtures/display-settings.json';
import type { DialContent, DisplaySettings, Profile } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { defaultDisplaySettings, dialContentOptions, displayFlags, DisplaySettingsStore, isDisplaySettings } from './display-settings';
import type { DisplaySettingsState } from './display-settings';
import { appliesNote, DisplaySettingsPanel, displayWarning } from './DisplaySettingsPanel';
import { emptyCounters } from './service-runner';
import type { ServiceRun, ServiceStoreState } from './service-runner';

const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
const root = emptyKeypadPage();
root.slots[1] = { type: 'folder', page_id: 'sub' };
const nested: Profile = { schema_version: 2, name: 'Nested', bindings: [], keypad: { pages: [root, emptyKeypadPage('sub', 'Sub')] } };
const on: DisplaySettings = { enabled: true, dial_content: 'clock' };

describe('display settings contract', () => {
  it('matches the Rust fixture', () => {
    expect(fixture.display_settings_default).toEqual(defaultDisplaySettings);
    expect(isDisplaySettings(fixture.set_display_settings_result)).toBe(true);
    expect(Object.keys(fixture.set_display_settings_args)).toEqual(['settings']);
    expect(dialContentOptions.map(option => option.value).sort()).toEqual([...fixture.dial_contents].sort());
  });
  it('accepts only the exact settings shape', () => {
    expect(isDisplaySettings({ enabled: false, dial_content: 'function' })).toBe(true);
    expect(isDisplaySettings({ enabled: true, dial_content: 'cpu' })).toBe(true);
    for (const bad of [null, 'on', { enabled: true }, { enabled: 'yes', dial_content: 'clock' }, { enabled: true, dial_content: 'analog-clock' }, { enabled: true, dial_content: 'CPU' }, { enabled: true, dial_content: 'gpu' }, { enabled: true, dial_content: 'clock', extra: 1 }]) {
      expect(isDisplaySettings(bad)).toBe(false);
    }
  });
});

describe('display flags', () => {
  it('mirror the Rust display_args rules', () => {
    // Off, or without software mode: no flags at all.
    expect(displayFlags(defaultDisplaySettings, 'm4315', true, nested)).toEqual([]);
    expect(displayFlags(on, 'm4315', false, nested)).toEqual([]);
    expect(displayFlags(on, 'm131e', false, flat)).toEqual([]);
    expect(displayFlags(on, 'm4315', true, nested)).toEqual(['--display-sync']);
    expect(displayFlags(on, 'm4315', true, flat)).toEqual([]);
    expect(displayFlags(on, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'clock']);
    expect(displayFlags({ enabled: true, dial_content: 'analog_clock' }, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'analog-clock']);
    expect(displayFlags({ enabled: true, dial_content: 'cpu' }, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'cpu']);
    expect(displayFlags({ enabled: true, dial_content: 'cpu' }, 'm4315', true, flat)).toEqual([]);
    expect(displayFlags({ enabled: true, dial_content: 'off' }, 'm131e', true, flat)).toEqual([]);
    for (const module of ['m003k', 'm005s', 'm032le'] as const) expect(displayFlags(on, module, true, nested)).toEqual([]);
  });
});

describe('DisplaySettingsStore', () => {
  it('loads, saves and keeps the value off until the backend answers', async () => {
    const calls: [string, Record<string, unknown> | undefined][] = [];
    let stored: DisplaySettings = { enabled: false, dial_content: 'function' };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_display_settings') stored = args!.settings as DisplaySettings;
      return stored as T;
    };
    const store = new DisplaySettingsStore(invoke, true);
    expect(store.getState().settings.enabled).toBe(false);
    expect(await store.load()).toBe(true);
    expect(await store.setEnabled(true)).toBe(true);
    expect(await store.setDialContent('analog_clock' as DialContent)).toBe(true);
    expect(store.getState().settings).toEqual({ enabled: true, dial_content: 'analog_clock' });
    expect(calls).toEqual([
      ['display_settings', undefined],
      ['set_display_settings', { settings: { enabled: true, dial_content: 'function' } }],
      ['set_display_settings', { settings: { enabled: true, dial_content: 'analog_clock' } }],
    ]);
  });
  it('reports errors and unexpected replies without turning the setting on', async () => {
    const failing = new DisplaySettingsStore(async () => { throw 'display settings in /x are unreadable'; }, true);
    expect(await failing.load()).toBe(false);
    expect(failing.getState().error).toContain('unreadable');
    expect(failing.getState().settings.enabled).toBe(false);
    const odd = new DisplaySettingsStore(async <T,>() => ({ enabled: true }) as T, true);
    expect(await odd.load()).toBe(false);
    expect(odd.getState().settings.enabled).toBe(false);
    // The browser preview never calls the backend.
    const preview = new DisplaySettingsStore(async () => { throw new Error('must not be called'); }, false);
    expect(await preview.setEnabled(true)).toBe(false);
  });
});

function storeWith(patch: Partial<DisplaySettingsState>) {
  const store = new DisplaySettingsStore(async <T,>() => defaultDisplaySettings as T, true);
  (store as unknown as { state: DisplaySettingsState }).state = { ...store.getState(), loaded: true, ...patch };
  return store;
}

function runnerWith(count: number) {
  const run = { serviceId: 's', state: 'running', execute: true, entries: [], counters: emptyCounters(), startedAt: 0, profileName: 'A', snapshot: '{}', draftUnsaved: false } as ServiceRun;
  const runs = Object.fromEntries(Array.from({ length: count }, (_, index) => [`s${index}`, run]));
  const state: ServiceStoreState = { desktop: true, runs, error: null };
  return { subscribe: () => () => {}, getState: () => state };
}

describe('DisplaySettingsPanel markup', () => {
  it('is off by default, warns that it never ran on hardware and offers the dial choice', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({})} />);
    expect(markup).toMatch(/<input type="checkbox"(?![^>]*checked)[^>]*>Experimental: send images to module displays/);
    expect(markup).toContain(displayWarning);
    expect(displayWarning).toContain('never been tested on real hardware');
    expect(markup).toContain('<label for="display-dial-content">Dial screen shows</label>');
    expect(markup).toContain('>Digital clock (HH:MM)</option>');
    expect(markup).toContain('>Analog clock</option>');
    expect(markup).toContain('<option value="cpu">CPU usage (%, macOS only)</option>');
    expect(markup).toContain('Changes apply the next time you start a profile.');
  });
  it('says running profiles keep their setting and are never restarted', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: on })} runner={runnerWith(2)} />);
    expect(markup).toMatch(/<input type="checkbox"[^>]*checked=""/);
    expect(markup).toContain('The 2 running profiles keep the setting each started with');
    expect(appliesNote(1)).toContain('The running profile keeps');
    expect(appliesNote(1)).toContain('nothing is restarted for you');
    expect(appliesNote(0)).toBe('Changes apply the next time you start a profile.');
  });
  it('is disabled until loaded and in the browser preview', () => {
    const loading = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ loaded: false })} />);
    expect(loading).toMatch(/<input type="checkbox"[^>]*disabled=""/);
    const preview = renderToStaticMarkup(<DisplaySettingsPanel />);
    expect(preview).toContain('Module displays need the desktop app.');
    const failed = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ error: 'display settings are unreadable' })} />);
    expect(failed).toContain('role="alert"><span>display settings are unreadable</span>');
  });
});
