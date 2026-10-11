import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import fixture from '../fixtures/display-settings.json';
import type { DisplaySettings, Profile } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { appControls, defaultDisplaySettings, displayFlags, DisplaySettingsStore, isDisplaySettings, moduleControlOptions } from './display-settings';
import { dialDisplayArgs, dialDisplayOptions } from './dial-display-model';
import type { DisplaySettingsState } from './display-settings';
import { appliesNote, DisplaySettingsPanel, moduleControlCaveat, moduleControlNotes, pressFeedbackNote, testWhileEditingNote } from './DisplaySettingsPanel';
import { emptyCounters } from './service-runner';
import type { ServiceRun, ServiceStoreState } from './service-runner';

const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
const root = emptyKeypadPage();
root.slots[1] = { type: 'folder', page_id: 'sub' };
const nested: Profile = { schema_version: 2, name: 'Nested', bindings: [], keypad: { pages: [root, emptyKeypadPage('sub', 'Sub')] } };
const app: DisplaySettings = { module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true };
const builtIn: DisplaySettings = { module_control: 'built_in_animation', run_mode: 'test', live_offered: true, press_feedback: true };
const otherModules = ['m003k', 'm005s', 'm032le'] as const;

// The dial screen choice is a profile setting; these stand in for profiles
// that picked a face in the dial editor.
const withDialDisplay = (profile: Profile, content: string, image?: string): Profile =>
  ({ ...profile, dial_display: { content, ...(image ? { image } : {}) } } as Profile);

describe('display settings contract', () => {
  it('matches the Rust fixture', () => {
    expect(fixture.display_settings_default).toEqual(defaultDisplaySettings);
    expect(defaultDisplaySettings.module_control).toBe('app');
    // A fresh install reads Live, with the one-time confirmation still owed.
    expect(defaultDisplaySettings.run_mode).toBe('live');
    expect(defaultDisplaySettings.live_offered).toBe(false);
    expect(isDisplaySettings(fixture.set_display_settings_result)).toBe(true);
    // Test unless the user switches the header badge to Live.
    expect(fixture.display_settings_default.run_mode).toBe('live');
    expect(fixture.display_settings_default.live_offered).toBe(false);
    expect(fixture.set_display_settings_result.run_mode).toBe('live');
    expect(fixture.set_display_settings_result.live_offered).toBe(true);
    // The editing hold's stored shape: the held Test names the Live to restore.
    expect(fixture.held_settings).toEqual({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, held_from: 'live' });
    // Without a hold the marker is absent, never null.
    expect(Object.keys(fixture.display_settings_default)).toHaveLength(4);
    expect(Object.keys(fixture.set_default_profile_args)).toEqual(['profileId']);
    expect(fixture.default_profile_result.profile_id).toMatch(/^[0-9a-f-]{36}$/);
    expect(fixture.default_profile_none).toEqual({ profile_id: null });
    expect(Object.keys(fixture.set_display_settings_args)).toEqual(['settings']);
    // The dial screen values match the service's --dial-content spellings.
    expect(dialDisplayOptions.map(option => option.value).sort()).toEqual([...fixture.dial_contents].sort());
    expect(Object.keys(dialDisplayArgs).sort()).toEqual([...fixture.dial_contents].sort());
    // The profile's dial display setting serializes exactly these fields.
    expect(fixture.dial_display_sample).toEqual({ content: 'image', image: 'AAAA' });
    expect(fixture.dial_display_function_json).toEqual({ content: 'function' });
    expect(moduleControlOptions.map(option => option.value)).toEqual(fixture.module_controls);
    expect(moduleControlOptions.map(option => option.label)).toEqual(['App', 'Built-in animation']);
  });
  it('accepts only the exact settings shape', () => {
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true })).toBe(true);
    expect(isDisplaySettings({ module_control: 'built_in_animation', run_mode: 'live', live_offered: false, press_feedback: true })).toBe(true);
    // All three fields are required and strict about their values.
    expect(isDisplaySettings({ module_control: 'app' })).toBe(false);
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'test' })).toBe(false);
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'yes', live_offered: true, press_feedback: true })).toBe(false);
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'Live', live_offered: true, press_feedback: true })).toBe(false);
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'test', live_offered: 'yes' })).toBe(false);
    // The editing hold's marker: "live" beside the held Test, or absent.
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, held_from: 'live' })).toBe(true);
    expect(isDisplaySettings(fixture.held_settings)).toBe(true);
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, held_from: 'test' })).toBe(false);
    expect(isDisplaySettings({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, held_from: null })).toBe(false);
    // The earlier shapes (the toggle, the file with a global dial choice) are
    // migrated by Rust and never reach the frontend.
    for (const bad of [null, 'on', { module_control: 'app' }, { enabled: true, dial_content: 'clock' }, { module_control: 'App', run_mode: 'test', live_offered: true, press_feedback: true }, { module_control: 'app', dial_content: 'clock', run_mode: 'test', live_offered: true, press_feedback: true }, { module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, extra: 1 }, { module_control: 'app', auto_execute: true }]) {
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
    expect(displayFlags(app, 'm4315', true, nested)).toEqual(['--display-sync', '--press-feedback']);
    // A flat 15-key profile is written as one page, so it is drawn too.
    expect(displayFlags(app, 'm4315', true, flat)).toEqual(['--display-sync', '--press-feedback']);
    expect(displayFlags({ ...app, press_feedback: false }, 'm4315', true, flat)).toEqual(['--display-sync']);
  });
  it('take the dial content from the profile, with the idle fallback', () => {
    // The default is the function name, which needs something on the dial;
    // otherwise the dial shows the analog clock.
    const dialBound: Profile = { ...flat, bindings: [{ module: 'm131e', control: 0, gesture: 'right', action: { type: 'volume', delta: 2 } }] };
    expect(displayFlags(app, 'm131e', true, flat)).toEqual(['--display-sync', '--dial-content', 'analog-clock']);
    expect(displayFlags(app, 'm131e', true, dialBound)).toEqual(['--display-sync', '--dial-content', 'function']);
    // An explicit choice in the profile is kept, whatever it is.
    expect(displayFlags(app, 'm131e', true, withDialDisplay(flat, 'analog_clock'))).toEqual(['--display-sync', '--dial-content', 'analog-clock']);
    expect(displayFlags(app, 'm131e', true, withDialDisplay(flat, 'clock'))).toEqual(['--display-sync', '--dial-content', 'clock']);
    expect(displayFlags(app, 'm131e', true, withDialDisplay(flat, 'cpu'))).toEqual(['--display-sync', '--dial-content', 'cpu']);
    expect(displayFlags(app, 'm131e', true, withDialDisplay(flat, 'image', 'AAAA'))).toEqual(['--display-sync', '--dial-content', 'image']);
    // A profile whose dial display is off: no display flags at all.
    expect(displayFlags(app, 'm131e', true, withDialDisplay(flat, 'off'))).toEqual([]);
    // The keypad ignores the dial setting, the dial ignores the keypad pages.
    expect(displayFlags(app, 'm4315', true, withDialDisplay(flat, 'cpu'))).toEqual(['--display-sync', '--press-feedback']);
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
    let stored: DisplaySettings = { module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_display_settings') stored = args!.settings as DisplaySettings;
      return stored as T;
    };
    const store = new DisplaySettingsStore(invoke, true);
    expect(store.getState().settings.module_control).toBe('app');
    expect(await store.load()).toBe(true);
    expect(await store.setModuleControl('built_in_animation')).toBe(true);
    expect(store.getState().settings).toEqual({ module_control: 'built_in_animation', run_mode: 'test', live_offered: true, press_feedback: true });
    expect(await store.setRunMode('live')).toBe(true);
    expect(store.getState().settings).toEqual({ module_control: 'built_in_animation', run_mode: 'live', live_offered: true, press_feedback: true });
    expect(calls).toEqual([
      ['display_settings', undefined],
      ['set_display_settings', { settings: { module_control: 'built_in_animation', run_mode: 'test', live_offered: true, press_feedback: true } }],
      ['set_display_settings', { settings: { module_control: 'built_in_animation', run_mode: 'live', live_offered: true, press_feedback: true } }],
    ]);
  });
  it('saves the press feedback switch and keeps the rest of the settings', async () => {
    let stored: DisplaySettings = { module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      if (command === 'set_display_settings') stored = args!.settings as DisplaySettings;
      return stored as T;
    };
    const store = new DisplaySettingsStore(invoke, true);
    await store.load();
    expect(await store.setPressFeedback(false)).toBe(true);
    expect(stored).toEqual({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: false });
    expect(store.getState().settings.press_feedback).toBe(false);
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
    expect(await preview.setRunMode('live')).toBe(false);
  });
  it('records the one-time confirmation whichever way it is answered', async () => {
    const calls: [string, Record<string, unknown> | undefined][] = [];
    let stored: DisplaySettings = { module_control: 'app', run_mode: 'live', live_offered: false, press_feedback: true };
    const store = new DisplaySettingsStore(async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_display_settings') stored = args!.settings as DisplaySettings;
      return stored as T;
    }, true);
    (store as unknown as { state: unknown }).state = { ...store.getState(), loaded: true, settings: stored };
    // Confirming lifts the hold and keeps Live.
    expect(await store.confirmLive()).toBe(true);
    expect(stored).toEqual({ module_control: 'app', run_mode: 'live', live_offered: true, press_feedback: true });
    // Declining saves Test, once, so the offer never nags again.
    expect(await store.declineLiveOffer()).toBe(true);
    expect(stored).toEqual({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true });
    // A confirmed Run with actions is a confirmation of the mode too.
    expect(await store.markLiveConfirmed()).toBe(true);
    expect(stored).toEqual({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true });
  });
  it('holds the mode for editing without answering the confirmation, and a switch ends the hold', async () => {
    const calls: [string, Record<string, unknown> | undefined][] = [];
    let stored: DisplaySettings = { module_control: 'app', run_mode: 'live', live_offered: true, press_feedback: true };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_display_settings') stored = args!.settings as DisplaySettings;
      return stored as T;
    };
    const store = new DisplaySettingsStore(invoke, true);
    (store as unknown as { state: unknown }).state = { ...store.getState(), loaded: true, settings: stored };
    // The first edit holds Test over the user's Live, with the restore target
    // stored beside it, and remembers what was executing (the runs the
    // restore may return to Live in silence).
    expect(store.beginEditingHold(true, ['s1', 's2'])).toBe(true);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(store.editingHeld).toBe(true);
    expect(store.editingSnapshotRuns).toEqual(new Set(['s1', 's2']));
    expect(stored).toEqual({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, held_from: 'live' });
    // Already held, or the setting off, or the mode already Test: nothing.
    expect(store.beginEditingHold(true, [])).toBe(false);
    expect(store.beginEditingHold(false, [])).toBe(false);
    // A fresh install's owed confirmation is never answered by the hold.
    const fresh = new DisplaySettingsStore(invoke, true);
    (fresh as unknown as { state: unknown }).state = { ...fresh.getState(), loaded: true, settings: { module_control: 'app', run_mode: 'live', live_offered: false, press_feedback: true } };
    fresh.beginEditingHold(true, []);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(stored).toEqual({ module_control: 'app', run_mode: 'test', live_offered: false, press_feedback: true, held_from: 'live' });
    // A hold read back from storage (the launch after a quit or crash
    // mid-edit) holds without any begin, with no snapshot: this session has
    // seen nothing execute, so its restore never asks.
    const held: DisplaySettings = { module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true, held_from: 'live' };
    const fromDisk = new DisplaySettingsStore(invoke, true);
    (fromDisk as unknown as { state: unknown }).state = { ...fromDisk.getState(), loaded: true, settings: held };
    expect(fromDisk.editingHeld).toBe(true);
    expect(fromDisk.editingSnapshotRuns).toBeNull();
    // Its first observation as editing seeds the snapshot, without a save.
    expect(fromDisk.beginEditingHold(true, ['s9'])).toBe(false);
    expect(fromDisk.editingSnapshotRuns).toEqual(new Set(['s9']));
    expect(fromDisk.getState().settings).toEqual(held);
    // Dropping the hold keeps the mode but stores the marker as absent.
    expect(fromDisk.dropEditingHold()).toBe(true);
    await new Promise(resolve => setTimeout(resolve, 0));
    expect(stored).toEqual({ module_control: 'app', run_mode: 'test', live_offered: true, press_feedback: true });
    expect(fromDisk.editingHeld).toBe(false);
    // Dropping a hold that is not on does nothing.
    expect(fromDisk.dropEditingHold()).toBe(false);
    // The user's own switch during a hold ends it: the marker goes with the
    // save, so nothing is restored underneath them later.
    expect(store.editingHeld).toBe(true);
    await store.setRunMode('live');
    expect(store.editingHeld).toBe(false);
    expect(store.editingSnapshotRuns).toBeNull();
    expect(stored).toEqual({ module_control: 'app', run_mode: 'live', live_offered: true, press_feedback: true });
  });
  it('seeds the browser preview for captures and never the desktop app', () => {
    const preview = new DisplaySettingsStore(async () => { throw new Error('must not be called'); }, false);
    preview.seedPreview({ module_control: 'app', run_mode: 'live', live_offered: true, press_feedback: true });
    expect(preview.getState().loaded).toBe(true);
    expect(preview.getState().settings.run_mode).toBe('live');
    const desktop = new DisplaySettingsStore(async <T,>() => defaultDisplaySettings as T, true);
    desktop.seedPreview({ module_control: 'app', run_mode: 'live', live_offered: true, press_feedback: true });
    expect(desktop.getState().loaded).toBe(false);
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
  it('shows the display control set to App by default, with the honest caveats', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({})} />);
    expect(markup).toContain('<h2 id="display-settings-title">Let OpenStreamHub drive the display</h2>');
    expect(markup).toContain('<option value="app" selected="">App</option>');
    expect(markup).toContain('<option value="built_in_animation">Built-in animation</option>');
    expect(markup).toContain(moduleControlNotes.app.replace(/"/g, '&quot;'));
    expect(markup).not.toContain('Experimental');
    expect(markup).not.toContain('never been tested');
    expect(moduleControlCaveat).toContain('first three keys');
    expect(moduleControlCaveat).toContain('latency has not been measured');
    expect(markup).toContain(moduleControlCaveat);
    // The dial screen choice moved into the profile's dial editor.
    expect(markup).not.toContain('display-dial-content');
    expect(moduleControlNotes.app).toContain('dial editor');
    expect(markup).toContain('Changes apply the next time a profile starts.');
    // The mode is the header badge's business; the panel has no toggle for it.
    expect(markup).not.toContain('Run actions automatically');
    expect(markup).not.toContain('setting-toggle');
  });
  it('offers the press feedback switch, on by default, only while the app drives the module', () => {
    const on = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({})} />);
    expect(on).toContain('Dim a key while it is pressed');
    expect(on).toMatch(/id="display-press-feedback"[^>]*checked=""/);
    expect(on).toContain(pressFeedbackNote);
    const off = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: { ...app, press_feedback: false } })} />);
    expect(off).not.toMatch(/id="display-press-feedback"[^>]*checked=""/);
    expect(renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: builtIn })} />)).toMatch(/id="display-press-feedback"[^>]*disabled=""/);
  });
  it('Built-in animation explains what it gives up', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: builtIn })} />);
    expect(markup).toContain('<option value="built_in_animation" selected="">Built-in animation</option>');
    expect(markup).toContain(moduleControlNotes.built_in_animation.replace(/"/g, '&quot;'));
  });
  it('says running profiles keep their setting and are never restarted', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ settings: app })} runner={runnerWith(2)} />);
    expect(markup).toContain('The 2 running profiles keep the setting each started with');
    expect(appliesNote(1)).toContain('The running profile keeps');
    expect(appliesNote(1)).toContain('nothing is restarted for you');
    expect(appliesNote(0)).toBe('Changes apply the next time a profile starts.');
  });
  it('is disabled until loaded and in the browser preview', () => {
    const loading = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ loaded: false })} />);
    expect(loading).toMatch(/<select id="display-module-control" disabled=""/);
    const preview = renderToStaticMarkup(<DisplaySettingsPanel />);
    expect(preview).toContain('This setting needs the desktop app.');
    const failed = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({ error: 'display settings are unreadable' })} />);
    expect(failed).toContain('role="alert"><span>display settings are unreadable</span>');
  });
  it('offers the editing hold on by default, with its plain description', () => {
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({})} />);
    expect(markup).toContain('>Run mode</h3>');
    expect(markup).toContain('Switch to Test mode while editing');
    expect(markup).toMatch(/<input id="run-mode-editing" type="checkbox"[^>]*checked=""/);
    expect(markup).toContain(testWhileEditingNote.replace(/"/g, '&quot;'));
    // Off is rendered as off, and the checkbox alone still works in the
    // browser preview (the rest of the panel needs the desktop app).
    const off = renderToStaticMarkup(<DisplaySettingsPanel store={storeWith({})} testWhileEditing={false} />);
    expect(off).toMatch(/<input id="run-mode-editing" type="checkbox"(?:[^>]*)\/>/);
    expect(off).not.toMatch(/run-mode-editing[^>]*checked=""/);
    const preview = renderToStaticMarkup(<DisplaySettingsPanel />);
    expect(preview).toContain('Switch to Test mode while editing');
    expect(preview).toMatch(/<input id="run-mode-editing" type="checkbox" disabled=""/);
  });
});
