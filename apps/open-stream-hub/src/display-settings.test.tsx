import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import fixture from '../fixtures/display-settings.json';
import type { DialContent, DisplaySettings, Profile } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { appControls, defaultDisplaySettings, dialContentOptions, displayFlags, DisplaySettingsStore, isDisplaySettings, moduleControlOptions } from './display-settings';
import type { DisplaySettingsState } from './display-settings';
import { appliesNote, DisplaySettingsPanel, moduleControlCaveat, moduleControlNotes } from './DisplaySettingsPanel';
import { emptyCounters } from './service-runner';
import type { ServiceRun, ServiceStoreState } from './service-runner';

const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
const root = emptyKeypadPage();
root.slots[1] = { type: 'folder', page_id: 'sub' };
const nested: Profile = { schema_version: 2, name: 'Nested', bindings: [], keypad: { pages: [root, emptyKeypadPage('sub', 'Sub')] } };
const app: DisplaySettings = { module_control: 'app', dial_content: 'clock', auto_execute: false };
const builtIn: DisplaySettings = { module_control: 'built_in_animation', dial_content: 'clock', auto_execute: false };
const otherModules = ['m003k', 'm005s', 'm032le'] as const;

describe('display settings contract', () => {
  it('matches the Rust fixture', () => {
    expect(fixture.display_settings_default).toEqual(defaultDisplaySettings);
    expect(defaultDisplaySettings.module_control).toBe('app');
    expect(isDisplaySettings(fixture.set_display_settings_result)).toBe(true);
    // Off unless the user turns it on.
    expect(fixture.display_settings_default.auto_execute).toBe(false);
    expect(fixture.set_display_settings_result.auto_execute).toBe(true);
    expect(Object.keys(fixture.set_default_profile_args)).toEqual(['profileId']);
    expect(fixture.default_profile_result.profile_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(fixture.default_profile_none).toEqual({ profile_id: null });
    expect(Object.keys(fixture.set_display_settings_args)).toEqual(['settings']);
    expect(dialContentOptions.map(option => option.value).sort()).toEqual([...fixture.dial_contents].sort());
    expect(moduleControlOptions.map(option => option.value)).toEqual(fixture.module_controls);
    expect(moduleControlOptions.map(option => option.label)).toEqual(['App', 'Built-in animation']);
  });
  it('accepts only the exact settings shape', () => {
    expect(isDisplaySettings({ module_control: 'app', dial_content: 'function', auto_execute: false })).toBe(true);
    expect(isDisplaySettings({ module_control: 'built_in_animation', dial_content: 'cpu', auto_execute: true })).toBe(true);
    // Run actions automatically is required and strictly a boolean.
    expect(isDisplaySettings({ module_control: 'app', dial_content: 'function' })).toBe(false);
    expect(isDisplaySettings({ module_control: 'app', dial_content: 'function', auto_execute: 'yes' })).toBe(false);
    // The old toggle's shape is migrated by Rust and never reaches the frontend.
    for (const bad of [null, 'on', { module_control: 'app' }, { enabled: true, dial_content: 'clock' }, { module_control: 'App', dial_content: 'clock' }, { module_control: 'app', dial_content: 'analog-clock' }, { module_control: 'app', dial_content: 'gpu' }, { module_control: 'app', dial_content: 'clock', extra: 1 }]) {
      expect(isDisplaySettings(bad)).toBe(false);
    }
  });
});

describe('display flags and software mode', () => {
  it('mirror the Rust spawn_display rules', () => {
    // Built-in animation, or without software mode: no flags at all.
    expect(displayFlags(builtIn, 'm4315', true, nested)).toEqual([]);
    expect(displayFlags(builtIn, 'm131e', true, flat)).toEqual([]);
    expect(displayFlags(app, 'm4315', false, nested)).toEqual([]);
    expect(displayFlags(app, 'm131e', false, flat)).toEqual([]);
    expect(displayFlags(app, 'm4315', true, nested)).toEqual(['--display-sync']);
    // A flat 15-key profile is written as one page, so it is drawn too.
    expect(displayFlags(app, 'm4315', true, flat)).toEqual(['--display-sync']);
    expect(displayFlags(app, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'clock']);
    // The function name needs something on the dial; otherwise the dial shows the analog clock.
    const dialBound: Profile = { ...flat, bindings: [{ module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 2 } }] };
    expect(displayFlags(defaultDisplaySettings, 'm131e', true, dialBound)).toEqual(['--display-sync', '--dial-content', 'function']);
    expect(displayFlags(defaultDisplaySettings, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'analog-clock']);
    expect(displayFlags({ ...app, dial_content: 'analog_clock' }, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'analog-clock']);
    expect(displayFlags({ ...app, dial_content: 'cpu' }, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'cpu']);
    expect(displayFlags({ ...app, dial_content: 'cpu' }, 'm4315', true, flat)).toEqual(['--display-sync']);
    expect(displayFlags({ ...app, dial_content: 'off' }, 'm131e', true, flat)).toEqual([]);
  });
  it('App control covers the keypad and dial only and never gives other modules flags', () => {
    for (const module of ['m4315', 'm131e'] as const) expect(appControls(module)).toBe(true);
    for (const module of otherModules) {
      expect(appControls(module)).toBe(false);
      for (const settings of [app, builtIn, defaultDisplaySettings]) {
        for (const profile of [flat, nested]) expect(displayFlags(settings, module, true, profile)).toEqual([]);
      }
    }
  });
});

describe('DisplaySettingsStore', () => {
  it('loads, saves and shows App until the backend answers', async () => {
    const calls: [string, Record<string, unknown> | undefined][] = [];
    let stored: DisplaySettings = { module_control: 'app', dial_content: 'function', auto_execute: false };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_display_settings') stored = args!.settings as DisplaySettings;
      return stored as T;
    };
    const store = new DisplaySettingsStore(invoke, true);
    expect(store.getState().settings.module_control).toBe('app');
    expect(await store.load()).toBe(true);
    expect(await store.setModuleControl('built_in_animation')).toBe(true);
    expect(await store.setDialContent('analog_clock' as DialContent)).toBe(true);
    expect(store.getState().settings).toEqual({ module_control: 'built_in_animation', dial_content: 'analog_clock', auto_execute: false });
    expect(calls).toEqual([
      ['display_settings', undefined],
      ['set_display_settings', { settings: { module_control: 'built_in_animation', dial_content: 'function', auto_execute: false } }],
      ['set_display_settings', { settings: { module_control: 'built_in_animation', dial_content: 'analog_clock', auto_execute: false } }],
    ]);
  });
  it('reports errors and unexpected replies', async () => {
    const failing = new DisplaySettingsStore(async () => { throw 'display settings in /x are unreadable'; }, true);
    expect(await failing.load()).toBe(false);
    expect(failing.getState().error).toContain('unreadable');
    expect(failing.getState().loaded).toBe(false);
    const odd = new DisplaySettingsStore(async <T,>() => ({ enabled: true, dial_content: 'clock' }) as T, true);
    expect(await odd.load()).toBe(false);
    expect(odd.getState().error).toContain('not understood');
    // The browser preview never calls the backend.
    const preview = new DisplaySettingsStore(async () => { throw new Error('must not be called'); }, false);
    expect(await preview.setModuleControl('built_in_animation')).toBe(false);
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
  it('shows Module control set to App by default, with the honest caveats and the dial choice', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({})} />);
    expect(markup).toContain('<h2 id="display-settings-title">Module control</h2>');
    expect(markup).toContain('<option value="app" selected="">App</option>');
    expect(markup).toContain('<option value="built_in_animation">Built-in animation</option>');
    expect(markup).toContain(moduleControlNotes.app.replace(/"/g, '&quot;'));
    expect(markup).not.toContain('Experimental');
    expect(markup).not.toContain('never been tested');
    expect(moduleControlCaveat).toContain('first three keys');
    expect(moduleControlCaveat).toContain('latency has not been measured');
    expect(markup).toContain(moduleControlCaveat);
    expect(markup).toContain('<label for="display-dial-content">Dial screen shows</label>');
    expect(markup).toMatch(/<select id="display-dial-content">/);
    expect(markup).toContain('>Digital clock (HH:MM)</option>');
    expect(markup).toContain('>Analog clock</option>');
    expect(markup).toContain('<option value="cpu">CPU usage (%, macOS only)</option>');
    expect(markup).toContain('Changes apply the next time you start a profile.');
  });
  it('Built-in animation explains what it gives up and locks the dial choice', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: builtIn })} />);
    expect(markup).toContain('<option value="built_in_animation" selected="">Built-in animation</option>');
    expect(markup).toContain(moduleControlNotes.built_in_animation.replace(/"/g, '&quot;'));
    expect(markup).toMatch(/<select id="display-dial-content" disabled="">/);
  });
  it('says running profiles keep their setting and are never restarted', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: app })} runner={runnerWith(2)} />);
    expect(markup).toContain('The 2 running profiles keep the setting each started with');
    expect(appliesNote(1)).toContain('The running profile keeps');
    expect(appliesNote(1)).toContain('nothing is restarted for you');
    expect(appliesNote(0)).toBe('Changes apply the next time you start a profile.');
  });
  it('is disabled until loaded and in the browser preview', () => {
    const loading = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ loaded: false })} />);
    expect(loading).toMatch(/<select id="display-module-control" disabled=""/);
    const preview = renderToStaticMarkup(<DisplaySettingsPanel />);
    expect(preview).toContain('Module control needs the desktop app.');
    const failed = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ error: 'display settings are unreadable' })} />);
    expect(failed).toContain('role="alert"><span>display settings are unreadable</span>');
  });
});
