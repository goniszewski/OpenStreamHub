import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import autoStart from '../fixtures/session-auto-start-events.json';
import type { LiveDevice, Profile, SessionStatePayload } from './contracts';
import { DeviceFeed, serviceOwnsDevice } from './device-feed';
import { FakeStorage } from './fake-storage';
import { markModuleControlNoticeShown, ModuleControlNotice, moduleControlNoticeStorageKey, moduleControlNoticeText, moduleControlNoticeVisible, shouldShowModuleControlNotice } from './ModuleControlNotice';
import { autoStartNote, ServicePanel } from './ServicePanel';
import { autoStartLabels, deviceTakenError, isStale, reasonLabels, ServiceRunner } from './service-runner';

const [dialState, keypadState] = autoStart.events as SessionStatePayload[];
const draft: Profile = { schema_version: 1, name: 'Everyday', bindings: [] };
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

function setup(list: unknown = autoStart.list_sessions, startService?: () => unknown) {
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const calls: string[] = [];
  const invoke = (async (command: string) => {
    calls.push(command);
    if (command === 'start_service' && startService) return startService();
    return command === 'list_sessions' ? list : undefined;
  }) as never;
  const listen = async (name: string, handler: (event: { payload: unknown }) => void) => { handlers[name] = handler; return () => {}; };
  const runner = new ServiceRunner(invoke, listen, true, callback => callback(), () => 1000);
  return { runner, handlers, calls, state: (payload: unknown) => handlers['session://state']({ payload }) };
}

describe('automatically started sessions', () => {
  it('label the run and its log, and never read as a stale copy of the draft', async () => {
    const t = setup(); t.runner.connect(); await tick();
    t.state(dialState);
    t.state(keypadState);
    const dial = t.runner.getState().runs[dialState.session_id];
    expect(dial.session?.autoStart).toBe('app_opened');
    expect(dial.execute).toBe(false);
    expect(dial.profileName).toBe('Blank (built-in)');
    expect(dial.entries[0].text).toBe('Started automatically (dry run) when the app opened.');
    expect(isStale(dial, draft)).toBe(false);
    const keypad = t.runner.getState().runs[keypadState.session_id];
    expect(keypad.session?.autoStart).toBe('module_connected');
    expect(keypad.entries[0].text).toBe(`${autoStartLabels.module_connected}.`);
    // A session the user started carries no label.
    const { auto_start: _, ...manual } = { ...dialState, session_id: 'service:ccdd', service_id: 'service:ccdd' };
    t.state(manual);
    expect(t.runner.getState().runs['service:ccdd'].session?.autoStart).toBeUndefined();
    expect(t.runner.getState().runs['service:ccdd'].entries).toEqual([]);
  });
  it('are taken in from list_sessions once the window listens, without overwriting newer events', async () => {
    const t = setup(); t.runner.connect();
    // An event that arrived first is newer than the list and wins.
    await tick();
    t.state({ ...keypadState, phase: 'reconnecting', attempt: 1 });
    await t.runner.syncSessions();
    expect(t.calls).toEqual(['list_sessions']);
    const runs = t.runner.getState().runs;
    expect(runs[dialState.session_id].session).toMatchObject({ phase: 'running', autoStart: 'app_opened' });
    expect(runs[keypadState.session_id].session).toMatchObject({ phase: 'reconnecting', autoStart: 'module_connected' });
    // Not connected, or an unusable reply: nothing changes.
    const idle = setup(); await idle.runner.syncSessions();
    expect(idle.calls).toEqual([]);
    const odd = setup({ not: 'a list' }); odd.runner.connect(); await odd.runner.syncSessions();
    expect(odd.runner.getState().runs).toEqual({});
  });
  it('are shown as started automatically in the Run panel', async () => {
    expect(autoStartNote('app_opened')).toBe('Started automatically (dry run) when the app opened. Stop it to run another profile on this module or to execute actions.');
    expect(autoStartNote(undefined)).toBeNull();
    const device: LiveDevice = { path_hex: dialState.service_id!.replace('service:', ''), module: 'm131e', serial_number: dialState.serial_number };
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [device] };
    const t = setup(); t.runner.connect(); await tick();
    t.state(dialState);
    const markup = renderToStaticMarkup(<ServicePanel runner={t.runner} feed={feed} profile={draft} desktop unsaved={false} platform={{ platform: 'MacIntel' }} now={2000} />);
    expect(markup).toContain(autoStartNote('app_opened'));
    expect(markup).toContain('Dry run: actions are logged only');
    expect(markup).not.toContain('Profile changed since start');
    expect(markup).toContain('>Stop<');
  });
  it('hide the automatic-start line once the session stopped, with its own reason', async () => {
    const device: LiveDevice = { path_hex: dialState.service_id!.replace('service:', ''), module: 'm131e', serial_number: dialState.serial_number };
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [device] };
    const t = setup(); t.runner.connect(); await tick();
    t.state(dialState);
    t.state({ ...dialState, phase: 'stopped', service_id: null, reason: 'module_control' });
    const markup = renderToStaticMarkup(<ServicePanel runner={t.runner} feed={feed} profile={draft} desktop unsaved={false} platform={{ platform: 'MacIntel' }} now={2000} />);
    expect(markup).not.toContain(autoStartNote('app_opened'));
    expect(markup).toContain('Session stopped: Module control is no longer App');
    expect(reasonLabels.module_control).toBe('Module control is no longer App');
    // Stopped by a settings change the user made: an info line, not an error.
    expect(t.runner.getState().runs[dialState.session_id].entries.at(-1)).toMatchObject({ kind: 'info', text: 'Session stopped: Module control is no longer App' });
  });
  it('never merge into a Run the backend refused because an automatic session owns the module', async () => {
    const pathHex = keypadState.service_id!.replace('service:', '');
    const options = { profile: draft, pathHex, execute: false, softwareMode: false, draftUnsaved: false };
    const refusal = `session ${keypadState.session_id} already runs this device`;
    // The window never heard of the session: the refusal is shown and the list read again.
    const t = setup(autoStart.list_sessions, () => { throw new Error(refusal); });
    t.runner.connect(); await tick();
    expect(await t.runner.start(options)).toBe(false);
    await tick(); await tick();
    expect(t.calls).toEqual(['start_service', 'list_sessions']);
    expect(t.runner.getState().error).toBe(deviceTakenError);
    const run = t.runner.getState().runs[keypadState.session_id];
    expect(run).toMatchObject({ profileName: 'Blank (built-in)', state: 'running', execute: false });
    expect(run.session?.autoStart).toBe('module_connected');
    expect(run.entries.map(entry => entry.text)).toEqual([`${autoStartLabels.module_connected}.`]);
    // Its event arrives while the refused Run waits for its reply: the user's run is replaced, not merged into.
    const late = setup([], () => { late.state(keypadState); throw new Error(refusal); });
    late.runner.connect(); await tick();
    expect(await late.runner.start(options)).toBe(false);
    const replaced = late.runner.getState().runs[keypadState.session_id];
    expect(replaced).toMatchObject({ profileName: 'Blank (built-in)', state: 'running' });
    expect(replaced.profileName).not.toBe(draft.name);
    expect(isStale(replaced, draft)).toBe(false);
    expect(late.runner.getState().error).toBe(deviceTakenError);
  });
  it('label a run that its process status created before the session event, keeping its log', async () => {
    const t = setup(); t.runner.connect(); await tick();
    t.handlers['service://status']({ payload: { service_id: dialState.service_id, state: 'running', execute: false } });
    t.handlers['service://record']({ payload: { service_id: dialState.service_id, record: { type: 'profile_valid' } } });
    t.state(dialState);
    const run = t.runner.getState().runs[dialState.session_id];
    expect(run.entries.map(entry => entry.text)).toEqual([`${autoStartLabels.app_opened}.`, 'Profile validated.']);
    expect(run.entries.map(entry => entry.seq)).toEqual([0, 1]);
    expect(isStale(run, draft)).toBe(false);
  });
  it('name a start caused by switching Module control to App', async () => {
    const t = setup(); t.runner.connect(); await tick();
    t.state({ ...keypadState, auto_start: 'module_control_set' });
    const run = t.runner.getState().runs[keypadState.session_id];
    expect(run.session?.autoStart).toBe('module_control_set');
    expect(run.entries[0].text).toBe('Started automatically (dry run) when Module control was set to App.');
    expect(autoStartNote('module_control_set')).toContain('when Module control was set to App. Stop it');
  });
  it('keep an automatic live feed on the same module quiet', () => {
    expect(serviceOwnsDevice.test('service service:aabb is using this device; stop it before starting a feed')).toBe(true);
    expect(serviceOwnsDevice.test('could not open the device')).toBe(false);
  });
});

describe('the Module control first-run notice', () => {
  it('has the agreed text and is a dismissable status, not a dialog', () => {
    expect(moduleControlNoticeText).toBe('OpenStreamHub drives the 15-key module and the encoder dial whenever it is open; their built-in animation returns when you quit. Change this in Settings > Module control.');
    const markup = renderToStaticMarkup(<ModuleControlNotice onDismiss={() => {}} />);
    expect(markup).toContain(moduleControlNoticeText.replace('>', '&gt;'));
    expect(markup).toContain('role="status"');
    expect(markup).not.toContain('dialog');
    expect(markup).toContain('aria-label="Dismiss module control notice"');
    let dismissed = 0;
    const element = ModuleControlNotice({ onDismiss: () => { dismissed += 1; } });
    const button = (element.props.children as { type: unknown; props: { onClick(): void } }[]).find(child => child.type === 'button')!;
    button.props.onClick();
    expect(dismissed).toBe(1);
  });
  it('is shown once per user, recorded as soon as it is shown', () => {
    const storage = new FakeStorage();
    expect(shouldShowModuleControlNotice(storage, true)).toBe(true);
    expect(shouldShowModuleControlNotice(storage, true)).toBe(true); // asking stores nothing
    markModuleControlNoticeShown(storage);
    expect(storage.getItem(moduleControlNoticeStorageKey)).toBe('shown');
    expect(shouldShowModuleControlNotice(storage, true)).toBe(false);
  });
  it('is visible, and so recorded, only while the guide does not cover it and the setting is App', () => {
    const shown = { pending: true, guideOpen: false, loaded: true, moduleControl: 'app' as const };
    expect(moduleControlNoticeVisible(shown)).toBe(true);
    expect(moduleControlNoticeVisible({ ...shown, guideOpen: true })).toBe(false);
    expect(moduleControlNoticeVisible({ ...shown, loaded: false })).toBe(false);
    expect(moduleControlNoticeVisible({ ...shown, moduleControl: 'built_in_animation' })).toBe(false);
    expect(moduleControlNoticeVisible({ ...shown, pending: false })).toBe(false);
  });
  it('never shows in the browser preview or when storage cannot be read', () => {
    expect(shouldShowModuleControlNotice(new FakeStorage(), false)).toBe(false);
    expect(shouldShowModuleControlNotice(null, true)).toBe(false);
    const broken = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
    expect(shouldShowModuleControlNotice(broken, true)).toBe(false);
    expect(() => markModuleControlNoticeShown(broken)).not.toThrow();
  });
});
