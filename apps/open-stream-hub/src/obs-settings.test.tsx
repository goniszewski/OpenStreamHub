import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import fixture from '../fixtures/obs-settings.json';
import type { ObsSettings, ObsSettingsView } from './contracts';
import { defaultObsSettings, isObsConnectionView, isObsSettingsView, obsConnectionLabels, obsEndpoint, obsFlags, obsHostOptions, ObsSettingsStore } from './obs-settings';
import type { ObsSettingsState } from './obs-settings';
import { obsPasswordNote, ObsSettingsPanel } from './ObsSettingsPanel';
import { summarizeRecord } from './service-runner';

const PASSWORD = 'hunter2-obs-secret';

describe('OBS settings contract', () => {
  it('matches the Rust fixture', () => {
    expect(fixture.obs_settings_default).toEqual(defaultObsSettings);
    for (const reply of [fixture.set_obs_settings_result, fixture.set_obs_password_result, fixture.clear_obs_password_result]) expect(isObsSettingsView(reply)).toBe(true);
    expect(Object.keys(fixture.set_obs_settings_args)).toEqual(['settings']);
    expect(Object.keys(fixture.set_obs_password_args)).toEqual(['password']);
    expect(fixture.clear_obs_password_args).toEqual({ password: null });
    expect(fixture.set_obs_password_result.password_set).toBe(true);
    // A reply never carries the password.
    expect(JSON.stringify([fixture.set_obs_password_result, fixture.clear_obs_password_result])).not.toContain(fixture.set_obs_password_args.password);
  });
  it('accepts only the exact loopback settings shape', () => {
    expect(isObsSettingsView({ enabled: true, host: 'localhost', port: 1, password_set: false })).toBe(true);
    for (const bad of [null, 'on', { enabled: true }, { ...defaultObsSettings, host: '192.168.1.2' }, { ...defaultObsSettings, port: 0 },
      { ...defaultObsSettings, port: 65536 }, { ...defaultObsSettings, port: 4455.5 }, { ...defaultObsSettings, password: 'x' },
      { enabled: true, host: '127.0.0.1', port: 4455 }]) {
      expect(isObsSettingsView(bad), JSON.stringify(bad)).toBe(false);
    }
    expect(obsHostOptions.map(option => option.value)).toEqual(['127.0.0.1', '::1', 'localhost']);
  });
});

describe('OBS flags', () => {
  it('mirror the Rust obs_args rules and never include the password', () => {
    expect(obsFlags(defaultObsSettings)).toEqual([]);
    expect(obsFlags({ ...defaultObsSettings, enabled: true })).toEqual(['--obs-endpoint', '127.0.0.1:4455']);
    expect(obsFlags({ enabled: true, host: '::1', port: 4456, password_set: true })).toEqual(['--obs-endpoint', '[::1]:4456', '--obs-password-stdin']);
    expect(obsEndpoint({ enabled: false, host: 'localhost', port: 80 })).toBe('localhost:80');
  });
});

describe('ObsSettingsStore', () => {
  it('loads and saves the endpoint and passes the password through without keeping it', async () => {
    const calls: [string, Record<string, unknown> | undefined][] = [];
    let stored: ObsSettingsView = { ...defaultObsSettings };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_obs_settings') stored = { ...(args!.settings as ObsSettings), password_set: stored.password_set };
      if (command === 'set_obs_password') stored = { ...stored, password_set: args!.password !== null };
      return stored as T;
    };
    const store = new ObsSettingsStore(invoke, true);
    expect(store.getState().settings.enabled).toBe(false);
    expect(await store.load()).toBe(true);
    expect(await store.setEnabled(true)).toBe(true);
    expect(await store.setHost('::1')).toBe(true);
    expect(await store.setPort(4456)).toBe(true);
    expect(await store.setPassword(PASSWORD)).toBe(true);
    expect(store.getState().settings).toEqual({ enabled: true, host: '::1', port: 4456, password_set: true });
    expect(JSON.stringify(store.getState())).not.toContain('hunter2');
    expect(await store.clearPassword()).toBe(true);
    expect(await store.setPassword('')).toBe(true);
    expect(calls).toEqual([
      ['obs_settings', undefined],
      ['set_obs_settings', { settings: { enabled: true, host: '127.0.0.1', port: 4455 } }],
      ['set_obs_settings', { settings: { enabled: true, host: '::1', port: 4455 } }],
      ['set_obs_settings', { settings: { enabled: true, host: '::1', port: 4456 } }],
      ['set_obs_password', { password: PASSWORD }],
      ['set_obs_password', { password: null }],
      ['set_obs_password', { password: null }],
    ]);
    expect(store.getState().settings.password_set).toBe(false);
  });
  it('reports errors and unexpected replies without turning OBS on', async () => {
    const failing = new ObsSettingsStore(async () => { throw 'OBS host must be 127.0.0.1, ::1 or localhost'; }, true);
    expect(await failing.setHost('localhost')).toBe(false);
    expect(failing.getState().error).toContain('OBS host');
    expect(failing.getState().settings.enabled).toBe(false);
    const odd = new ObsSettingsStore(async <T,>() => ({ enabled: true, host: '10.0.0.1', port: 4455, password_set: false }) as T, true);
    expect(await odd.load()).toBe(false);
    expect(odd.getState().settings.enabled).toBe(false);
    const preview = new ObsSettingsStore(async () => { throw new Error('must not be called'); }, false);
    expect(await preview.setPassword(PASSWORD)).toBe(false);
  });
});

function storeWith(patch: Partial<ObsSettingsState>) {
  const store = new ObsSettingsStore(async <T,>() => defaultObsSettings as T, true);
  (store as unknown as { state: ObsSettingsState }).state = { ...store.getState(), loaded: true, ...patch };
  return store;
}

describe('ObsSettingsPanel markup', () => {
  it('is off by default, loopback only, with a masked password field that is never prefilled', () => {
    const markup = renderToStaticMarkup(<ObsSettingsPanel store={storeWith({})} />);
    expect(markup).toMatch(/<input type="checkbox"(?![^>]*checked)[^>]*>Let OBS actions connect to OBS on this Mac/);
    expect(markup).toContain('<input id="obs-password" type="password" autoComplete="off"');
    expect(markup).toMatch(/id="obs-password"[^>]*value=""/);
    expect(markup).toContain('>localhost (this Mac)</option>');
    expect(markup).toContain('127.0.0.1:4455');
    expect(markup).toContain(obsPasswordNote);
    expect(obsPasswordNote).toContain('enter it again after you restart the app');
    expect(markup).toContain('No password is set');
    expect(markup).toContain('Changes apply the next time a profile starts.');
  });
  it('says when a password is held and never shows it', () => {
    const markup = renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ settings: { enabled: true, host: '::1', port: 4456, password_set: true } })} />);
    expect(markup).toContain('A password is set.');
    expect(markup).toContain('[::1]:4456');
    expect(markup).toMatch(/<button type="button" class="button compact">Forget password<\/button>/);
  });
  it('is disabled until loaded and in the browser preview', () => {
    expect(renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ loaded: false })} />)).toMatch(/<input type="checkbox"[^>]*disabled=""/);
    expect(renderToStaticMarkup(<ObsSettingsPanel />)).toContain('OBS needs the desktop app.');
    const failed = renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ error: 'OBS settings are unreadable' })} />);
    expect(failed).toContain('role="alert"><span>OBS settings are unreadable</span>');
  });
});

describe('OBS connection test', () => {
  const reply = { state: 'wrong_password', message: 'OBS refused the password.' };
  it('stores the backend verdict, replaces it on the next test and drops it when a setting changes', async () => {
    let answer: unknown = reply;
    const invoke = async <T,>(command: string) => (command === 'obs_status' ? answer : { ...defaultObsSettings, enabled: true }) as T;
    const store = new ObsSettingsStore(invoke, true);
    expect(await store.testConnection()).toBe(true);
    expect(store.getState().connection).toEqual(reply);
    answer = { state: 'connected', message: 'Connected to OBS.' };
    await store.testConnection();
    expect(store.getState().connection?.state).toBe('connected');
    await store.setPort(4456);
    expect(store.getState().connection).toBeNull();
  });
  it('keeps the old verdict and shows an error when the probe fails or is unreadable', async () => {
    const failing = new ObsSettingsStore(async () => { throw new Error('probe broke'); }, true);
    expect(await failing.testConnection()).toBe(false);
    expect(failing.getState().error).toContain('probe broke');
    expect(failing.getState().testing).toBe(false);
    const odd = new ObsSettingsStore(async <T,>() => ({ state: 'fine', message: 'x' }) as T, true);
    expect(await odd.testConnection()).toBe(false);
    expect(odd.getState().connection).toBeNull();
    expect(await new ObsSettingsStore(async () => { throw new Error('no'); }, false).testConnection()).toBe(false);
  });
  it('accepts exactly the states the backend names, each with its own chip label', () => {
    const labels = Object.values(obsConnectionLabels);
    expect(new Set(labels).size).toBe(labels.length);
    expect(isObsConnectionView(reply)).toBe(true);
    for (const bad of [null, {}, { state: 'connected' }, { ...reply, extra: 1 }, { ...reply, state: 'other' }]) expect(isObsConnectionView(bad)).toBe(false);
  });
  it('shows the chip, the fix line and a test button only while OBS is on', () => {
    const on = { enabled: true, host: '127.0.0.1' as const, port: 4455, password_set: false };
    const off = renderToStaticMarkup(<ObsSettingsPanel store={storeWith({})} />);
    expect(off).not.toContain('obs-chip');
    const shown = renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ settings: on, connection: { state: 'websocket_off', message: 'Tick Enable WebSocket server.' } })} />);
    expect(shown).toContain('obs-chip-websocket_off">WebSocket off</span>');
    expect(shown).toContain('Tick Enable WebSocket server.');
    expect(shown).toContain('Test connection</button>');
    expect(renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ settings: on })} />)).toContain('Checking…');
    // The default port is offered only when the saved one differs and a connection state is known.
    const wrong = { state: 'wrong_port' as const, message: 'x' };
    expect(shown).not.toContain('Use default port');
    expect(renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ settings: { ...on, port: 4456 }, connection: wrong })} />)).toContain('Use default port 4455');
    expect(renderToStaticMarkup(<ObsSettingsPanel store={storeWith({ settings: on, connection: wrong })} />)).not.toContain('Use default port');
  });
});

describe('OBS activity records', () => {
  it('summarize unavailable and failed scene switches with the scene length (never the name) and a bounded reason', () => {
    const event = { timestamp_micros: 1, control: 0, gesture: 'pressed' };
    const unavailable = summarizeRecord({ type: 'action_unavailable', schema_version: 1, event, action: { type: 'obs_scene', scene_chars: 4 }, reason: 'OBS is not configured for this run (turn on OBS in OpenStreamHub Settings, or pass --obs-endpoint); no action was taken' }, 0);
    expect(unavailable?.text).toBe('OBS scene (4 characters) not available: OBS is not configured for this run (turn on OBS in OpenStreamHub Settings, or pass --obs-endpoint); no action was taken');
    const failed = summarizeRecord({ type: 'action_failed', schema_version: 1, event, action: { type: 'obs_scene', scene_chars: 4 }, error: 'OBS at 127.0.0.1:4455: OBS refused the scene switch (no scene with that name)' }, 0);
    expect(failed?.text).toContain('OBS scene (4 characters) failed');
    expect(failed?.text).toContain('no scene with that name');
  });
});
