import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import fixture from '../fixtures/streamlabs-settings.json';
import type { StreamlabsSettings, StreamlabsSettingsView } from './contracts';
import { defaultStreamlabsSettings, isStreamlabsConnectionView, isStreamlabsSettingsView, streamlabsConnectionLabels, streamlabsFlags, streamlabsHostOptions, StreamlabsSettingsStore } from './streamlabs-settings';
import type { StreamlabsSettingsState } from './streamlabs-settings';
import { streamlabsTokenHelp, streamlabsTokenNote, StreamlabsSettingsPanel } from './StreamlabsSettingsPanel';

const TOKEN = 's3cret-streamlabs-token';

describe('Streamlabs settings contract', () => {
  it('matches the Rust fixture', () => {
    expect(fixture.streamlabs_settings_default).toEqual(defaultStreamlabsSettings);
    for (const reply of [fixture.set_streamlabs_settings_result, fixture.set_streamlabs_token_result, fixture.clear_streamlabs_token_result]) expect(isStreamlabsSettingsView(reply)).toBe(true);
    expect(Object.keys(fixture.set_streamlabs_settings_args)).toEqual(['settings']);
    expect(Object.keys(fixture.set_streamlabs_token_args)).toEqual(['token']);
    expect(fixture.clear_streamlabs_token_args).toEqual({ token: null });
    expect(fixture.set_streamlabs_token_result.token_set).toBe(true);
    // A reply never carries the token.
    expect(JSON.stringify([fixture.set_streamlabs_token_result, fixture.clear_streamlabs_token_result])).not.toContain(fixture.set_streamlabs_token_args.token);
  });
  it('accepts only the exact loopback settings shape', () => {
    expect(isStreamlabsSettingsView({ enabled: true, host: 'localhost', port: 1, token_set: false })).toBe(true);
    for (const bad of [null, 'on', { enabled: true }, { ...defaultStreamlabsSettings, host: '192.168.1.2' }, { ...defaultStreamlabsSettings, port: 0 },
      { ...defaultStreamlabsSettings, port: 65536 }, { ...defaultStreamlabsSettings, token: 'x' }, { enabled: true, host: '127.0.0.1', port: 59650 },
      { enabled: true, host: '127.0.0.1', port: 59650, password_set: true }]) {
      expect(isStreamlabsSettingsView(bad), JSON.stringify(bad)).toBe(false);
    }
    expect(streamlabsHostOptions.map(option => option.value)).toEqual(['127.0.0.1', '::1', 'localhost']);
  });
});

describe('Streamlabs flags', () => {
  it('mirror the Rust streamlabs_args rules: only when on with a token, never the token itself', () => {
    expect(streamlabsFlags(defaultStreamlabsSettings)).toEqual([]);
    expect(streamlabsFlags({ ...defaultStreamlabsSettings, enabled: true })).toEqual([]);
    expect(streamlabsFlags({ ...defaultStreamlabsSettings, token_set: true })).toEqual([]);
    expect(streamlabsFlags({ enabled: true, host: '::1', port: 59651, token_set: true })).toEqual(['--streamlabs-endpoint', '[::1]:59651', '--streamlabs-token-stdin']);
  });
});

describe('StreamlabsSettingsStore', () => {
  it('loads and saves the endpoint and passes the token through without keeping it', async () => {
    const calls: [string, Record<string, unknown> | undefined][] = [];
    let stored: StreamlabsSettingsView = { ...defaultStreamlabsSettings };
    const invoke = async <T,>(command: string, args?: Record<string, unknown>) => {
      calls.push([command, args]);
      if (command === 'set_streamlabs_settings') stored = { ...(args!.settings as StreamlabsSettings), token_set: stored.token_set };
      if (command === 'set_streamlabs_token') stored = { ...stored, token_set: args!.token !== null };
      return stored as T;
    };
    const store = new StreamlabsSettingsStore(invoke, true);
    expect(await store.load()).toBe(true);
    expect(await store.setEnabled(true)).toBe(true);
    expect(await store.setHost('::1')).toBe(true);
    expect(await store.setPort(59651)).toBe(true);
    expect(await store.setToken(TOKEN)).toBe(true);
    expect(store.getState().settings).toEqual({ enabled: true, host: '::1', port: 59651, token_set: true });
    expect(JSON.stringify(store.getState())).not.toContain('s3cret');
    expect(await store.clearToken()).toBe(true);
    expect(await store.setToken('')).toBe(true);
    expect(calls).toEqual([
      ['streamlabs_settings', undefined],
      ['set_streamlabs_settings', { settings: { enabled: true, host: '127.0.0.1', port: 59650 } }],
      ['set_streamlabs_settings', { settings: { enabled: true, host: '::1', port: 59650 } }],
      ['set_streamlabs_settings', { settings: { enabled: true, host: '::1', port: 59651 } }],
      ['set_streamlabs_token', { token: TOKEN }],
      ['set_streamlabs_token', { token: null }],
      ['set_streamlabs_token', { token: null }],
    ]);
  });
  it('reports errors and unexpected replies without turning Streamlabs on', async () => {
    const failing = new StreamlabsSettingsStore(async () => { throw 'Streamlabs host must be 127.0.0.1, ::1 or localhost'; }, true);
    expect(await failing.setHost('localhost')).toBe(false);
    expect(failing.getState().error).toContain('Streamlabs host');
    expect(failing.getState().settings.enabled).toBe(false);
    const odd = new StreamlabsSettingsStore(async <T,>() => ({ enabled: true, host: '10.0.0.1', port: 59650, token_set: false }) as T, true);
    expect(await odd.load()).toBe(false);
    expect(odd.getState().settings.enabled).toBe(false);
    const preview = new StreamlabsSettingsStore(async () => { throw new Error('must not be called'); }, false);
    expect(await preview.setToken(TOKEN)).toBe(false);
  });
});

function storeWith(patch: Partial<StreamlabsSettingsState>) {
  const store = new StreamlabsSettingsStore(async <T,>() => defaultStreamlabsSettings as T, true);
  (store as unknown as { state: StreamlabsSettingsState }).state = { ...store.getState(), loaded: true, ...patch };
  return store;
}

describe('StreamlabsSettingsPanel markup', () => {
  it('is off by default, loopback only, with a write-only token field and help on where the token is', () => {
    const markup = renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({})} />);
    expect(markup).toMatch(/<input type="checkbox"(?![^>]*checked)[^>]*>Let Streamlabs actions connect to Streamlabs Desktop on this Mac/);
    expect(markup).toContain('<input id="streamlabs-token" type="password" autoComplete="off"');
    expect(markup).toMatch(/id="streamlabs-token"[^>]*value=""/);
    expect(markup).toContain('127.0.0.1:59650');
    expect(markup).toContain('loopback');
    expect(markup).toContain('Not yet tested against a real Streamlabs Desktop.');
    expect(streamlabsTokenHelp).toContain('Settings › Remote Control');
    expect(streamlabsTokenHelp).toContain('API token');
    expect(markup).toContain(streamlabsTokenNote);
    expect(streamlabsTokenNote).toContain('macOS Keychain');
    expect(markup).toContain('No token is set');
    expect(markup).toContain('Changes apply the next time a profile starts.');
  });
  it('says when a token is held and never shows it', () => {
    const markup = renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ settings: { enabled: true, host: '::1', port: 59651, token_set: true } })} />);
    expect(markup).toContain('A token is set.');
    expect(markup).toContain('[::1]:59651');
    expect(markup).toMatch(/<button type="button" class="button compact">Forget token<\/button>/);
  });
  it('is disabled until loaded and in the browser preview', () => {
    expect(renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ loaded: false })} />)).toMatch(/<input type="checkbox"[^>]*disabled=""/);
    expect(renderToStaticMarkup(<StreamlabsSettingsPanel />)).toContain('Streamlabs needs the desktop app.');
    const failed = renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ error: 'Streamlabs settings are unreadable' })} />);
    expect(failed).toContain('role="alert"><span>Streamlabs settings are unreadable</span>');
  });
});

describe('Streamlabs connection test', () => {
  const reply = { state: 'wrong_token', message: 'Streamlabs refused the API token.' };
  it('stores the backend verdict, replaces it on the next test and drops it when a setting changes', async () => {
    let answer: unknown = reply;
    const invoke = async <T,>(command: string) => (command === 'streamlabs_status' ? answer : { ...defaultStreamlabsSettings, enabled: true }) as T;
    const store = new StreamlabsSettingsStore(invoke, true);
    expect(await store.testConnection()).toBe(true);
    expect(store.getState().connection).toEqual(reply);
    answer = { state: 'connected', message: 'Connected to Streamlabs Desktop.' };
    await store.testConnection();
    expect(store.getState().connection?.state).toBe('connected');
    await store.setPort(59651);
    expect(store.getState().connection).toBeNull();
  });
  it('keeps the old verdict and shows an error when the probe fails or is unreadable', async () => {
    const failing = new StreamlabsSettingsStore(async () => { throw new Error('probe broke'); }, true);
    expect(await failing.testConnection()).toBe(false);
    expect(failing.getState().error).toContain('probe broke');
    expect(failing.getState().testing).toBe(false);
    const odd = new StreamlabsSettingsStore(async <T,>() => ({ state: 'wrong_password', message: 'x' }) as T, true);
    expect(await odd.testConnection()).toBe(false);
    expect(odd.getState().connection).toBeNull();
  });
  it('accepts exactly the states the backend names, each with its own chip label', () => {
    const labels = Object.values(streamlabsConnectionLabels);
    expect(new Set(labels).size).toBe(labels.length);
    expect(Object.keys(streamlabsConnectionLabels)).toEqual(['off', 'token_needed', 'connected', 'not_running', 'remote_control_off', 'wrong_port', 'wrong_token', 'failed']);
    expect(isStreamlabsConnectionView(reply)).toBe(true);
    for (const bad of [null, {}, { state: 'connected' }, { ...reply, extra: 1 }, { ...reply, state: 'other' }]) expect(isStreamlabsConnectionView(bad)).toBe(false);
  });
  it('shows the chip, the fix line and a test button only while Streamlabs is on', () => {
    const on = { enabled: true, host: '127.0.0.1' as const, port: 59650, token_set: true };
    expect(renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({})} />)).not.toContain('obs-chip');
    const shown = renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ settings: on, connection: { state: 'token_needed', message: 'Copy the token.' } })} />);
    expect(shown).toContain('obs-chip-token_needed">Token needed</span>');
    expect(shown).toContain('Copy the token.');
    expect(shown).toContain('Test connection</button>');
    expect(renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ settings: on })} />)).toContain('Checking…');
    const wrong = { state: 'wrong_port' as const, message: 'x' };
    expect(renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ settings: { ...on, port: 59651 }, connection: wrong })} />)).toContain('Use default port 59650');
    expect(renderToStaticMarkup(<StreamlabsSettingsPanel store={storeWith({ settings: on, connection: wrong })} />)).not.toContain('Use default port');
  });
});
