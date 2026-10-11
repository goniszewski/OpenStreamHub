import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import autoStart from '../fixtures/session-auto-start-events.json';
import type { LiveDevice, Profile, SessionStatePayload } from './contracts';
import { DisplaySettingsStore } from './display-settings';
import { defaultProfileNote, DisplaySettingsPanel } from './DisplaySettingsPanel';
import { DeviceFeed, serviceOwnsDevice } from './device-feed';
import { FakeStorage } from './fake-storage';
import { autoActionsNoticeText, markModuleControlNoticeShown, ModuleControlNotice, moduleControlNoticeStorageKey, moduleControlNoticeText, moduleControlNoticeVisible, shouldShowModuleControlNotice } from './ModuleControlNotice';
import { autoStartNote, RunHeaderControl, ServicePanel, useRunSession } from './ServicePanel';
import type { RunSessionOptions } from './ServicePanel';
import { autoStartLabel, deviceTakenError, isStale, reasonLabels, ServiceRunner } from './service-runner';

const [dialState, keypadState] = autoStart.events as SessionStatePayload[];
const draft: Profile = { schema_version: 1, name: 'Everyday', bindings: [] };
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

// The header Run control and the panel share the hook, as in the app.
function RunViews(props: Partial<RunSessionOptions>) {
  const run = useRunSession({ profile: draft, desktop: true, unsaved: false, platform: { platform: 'MacIntel' }, now: 2000, ...props });
  return <><RunHeaderControl run={run} /><ServicePanel run={run} /></>;
}

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
    expect(dial.execute).toBe(true);
    expect(dial.profileName).toBe('Blank (built-in)');
    expect(dial.entries[0].text).toBe('Started automatically (Live mode) when the app opened.');
    expect(isStale(dial, draft)).toBe(false);
    const keypad = t.runner.getState().runs[keypadState.session_id];
    expect(keypad.session?.autoStart).toBe('module_connected');
    expect(keypad.entries[0].text).toBe(`${autoStartLabel('module_connected', true)}.`);
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
    expect(autoStartNote('app_opened')).toBe('Started automatically (Test mode) when the app opened. Stop it to run another profile on this module.');
    expect(autoStartNote('app_opened', true)).toBe('Started automatically (Live mode) when the app opened. Actions run without asking. Stop it, or switch the header mode to Test, to end them.');
    expect(autoStartNote(undefined)).toBeNull();
    const device: LiveDevice = { path_hex: dialState.service_id!.replace('service:', ''), module: 'm131e', serial_number: dialState.serial_number };
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [device] };
    const t = setup(); t.runner.connect(); await tick();
    t.state(dialState);
    const markup = renderToStaticMarkup(<RunViews runner={t.runner} feed={feed} />);
    expect(markup).toContain(autoStartNote('app_opened', true));
    expect(markup).toContain('Actions are running on this Mac');
    expect(markup).not.toContain('Profile changed since start');
    expect(markup).toContain('aria-label="Stop"');
  });
  it('hide the automatic-start line once the session stopped, with its own reason', async () => {
    const device: LiveDevice = { path_hex: dialState.service_id!.replace('service:', ''), module: 'm131e', serial_number: dialState.serial_number };
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [device] };
    const t = setup(); t.runner.connect(); await tick();
    t.state(dialState);
    t.state({ ...dialState, phase: 'stopped', service_id: null, reason: 'module_control' });
    const markup = renderToStaticMarkup(<RunViews runner={t.runner} feed={feed} />);
    expect(markup).not.toContain(autoStartNote('app_opened'));
    expect(markup).toContain('Session stopped: OpenStreamHub no longer drives the display');
    expect(reasonLabels.module_control).toBe('OpenStreamHub no longer drives the display');
    // Stopped by a settings change the user made: an info line, not an error.
    expect(t.runner.getState().runs[dialState.session_id].entries.at(-1)).toMatchObject({ kind: 'info', text: 'Session stopped: OpenStreamHub no longer drives the display' });
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
    expect(run).toMatchObject({ profileName: 'Blank (built-in)', state: 'running', execute: true });
    expect(run.session?.autoStart).toBe('module_connected');
    expect(run.entries.map(entry => entry.text)).toEqual([`${autoStartLabel('module_connected', true)}.`]);
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
    t.handlers['service://status']({ payload: { service_id: dialState.service_id, state: 'running', execute: true } });
    t.handlers['service://record']({ payload: { service_id: dialState.service_id, record: { type: 'profile_valid' } } });
    t.state(dialState);
    const run = t.runner.getState().runs[dialState.session_id];
    expect(run.entries.map(entry => entry.text)).toEqual([`${autoStartLabel('app_opened', true)}.`, 'Profile validated.']);
    expect(run.entries.map(entry => entry.seq)).toEqual([0, 1]);
    expect(isStale(run, draft)).toBe(false);
  });
  it('name a start caused by OpenStreamHub starting to drive the display', async () => {
    const t = setup(); t.runner.connect(); await tick();
    t.state({ ...keypadState, auto_start: 'module_control_set' });
    const run = t.runner.getState().runs[keypadState.session_id];
    expect(run.session?.autoStart).toBe('module_control_set');
    expect(run.entries[0].text).toBe('Started automatically (Live mode) when OpenStreamHub started driving the display.');
    expect(autoStartNote('module_control_set')).toContain('when OpenStreamHub started driving the display. Stop it');
  });
  it('keep an automatic live feed on the same module quiet', () => {
    expect(serviceOwnsDevice.test('service service:aabb is using this device; stop it before starting a feed')).toBe(true);
    expect(serviceOwnsDevice.test('could not open the device')).toBe(false);
  });
});

describe('the display-control first-run notice', () => {
  it('has the agreed text and is a dismissable status, not a dialog', () => {
    expect(moduleControlNoticeText).toBe('OpenStreamHub drives the 15-key module and the encoder dial whenever it is open; their built-in animation returns when you quit. Change this in Settings, under “Let OpenStreamHub drive the display”.');
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

describe('automatic actions', () => {
  const executing = { ...keypadState, execute: true };
  it('label automatic runs by the mode, and say why one that should execute is a dry run', async () => {
    expect(autoStartLabel('app_opened', true)).toBe('Started automatically (Live mode) when the app opened');
    expect(autoStartLabel('app_opened', false)).toBe('Started automatically (Test mode) when the app opened');
    const t = setup(); t.runner.connect(); await tick();
    t.state(executing);
    t.state({ ...dialState, auto_dry_run_reason: 'this profile was imported and is not yet allowed to execute' });
    const runs = t.runner.getState().runs;
    expect(runs[keypadState.session_id].entries[0].text).toBe('Started automatically (Live mode) when the module was connected.');
    expect(runs[dialState.session_id].session?.autoDryRunReason).toBe('this profile was imported and is not yet allowed to execute');
    expect(autoStartNote('module_connected', true)).toContain('Started automatically (Live mode)');
    expect(autoStartNote('app_opened', false, 'blocked')).toContain('Actions are off for this run: blocked');
    expect(reasonLabels.run_mode).toBe('the run mode was switched');
  });
  it('keep the default profile picker in Settings, with no run-mode toggle', async () => {
    const store = new DisplaySettingsStore(async <T,>(command: string, args?: Record<string, unknown>) =>
      (command === 'default_profile' ? { profile_id: 'p-1' } : command === 'set_default_profile' ? { profile_id: args!.profileId as string | null } : { module_control: 'app', run_mode: 'test', live_offered: true }) as T, true);
    await store.load();
    await store.loadDefaultProfile();
    expect(store.getState().defaultProfile).toMatchObject({ loaded: true, profileId: 'p-1' });
    const markup = renderToStaticMarkup(<DisplaySettingsPanel store={store} profiles={[{ id: 'p-1', name: 'Everyday' }, { id: 'p-2', name: 'Streaming' }]} />);
    expect(markup).toContain('<option value="">No default</option>');
    expect(markup).toContain('<option value="p-1" selected="">Everyday</option>');
    expect(markup).toContain(defaultProfileNote);
    expect(markup).toContain('the run mode shown by the badge in the header');
    expect(await store.setDefaultProfile(null)).toBe(true);
    expect(store.getState().defaultProfile.profileId).toBeNull();
    // A default that is no longer saved is shown as skipped.
    expect(await store.setDefaultProfile('p-gone')).toBe(true);
    expect(renderToStaticMarkup(<DisplaySettingsPanel store={store} profiles={[]} />)).toContain('Missing profile (skipped)');
    // Unreadable: the error is shown, nothing is assumed.
    const broken = new DisplaySettingsStore(async () => { throw 'the last module profiles are unreadable'; }, true);
    expect(await broken.loadDefaultProfile()).toBe(false);
    expect(broken.getState().defaultProfile).toMatchObject({ loaded: false, error: 'the last module profiles are unreadable' });
  });
  it('add one sentence to the first-run notice only while the mode is Live', () => {
    expect(autoActionsNoticeText).toBe('Actions run right now; switch the badge in the header to Test to stop them.');
    const plain = renderToStaticMarkup(<ModuleControlNotice onDismiss={() => {}} />);
    expect(plain).not.toContain('Actions run right now');
    const on = renderToStaticMarkup(<ModuleControlNotice runMode="live" onDismiss={() => {}} />);
    expect(on).toContain(`${moduleControlNoticeText} ${autoActionsNoticeText}`.replace(/>/g, '&gt;'));
  });
});
