import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import type { Scheduler } from './device-feed';
import sessionArgs from '../fixtures/session-command-args.json';
import sessionResults from '../fixtures/session-command-results.json';
import sessionEvents from '../fixtures/session-state-events.json';
import switchEvents from '../fixtures/session-switch-events.json';
import profileActionEvents from '../fixtures/session-profile-action-events.json';
import { countRecord, emptyCounters, findRun, isActiveRun, isStale, maxEntries, profileChangedError, ServiceRunner, serviceIdFor, staleNote, summarizeRecord, unknownSnapshot } from './service-runner';
import type { ServiceRun } from './service-runner';
import { stableStringify } from './draft-model';

const profile: Profile = { schema_version: 1, name: 'Everyday', bindings: [{ module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } }] };
const event = { module: 'm003k', control: 0, gesture: 'pressed' };

function setup(desktop = true) {
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const unlistened: string[] = [];
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const queue: (() => void)[] = [];
  const failures: Record<string, string> = {};
  // A command's result instead of the default, computed when it is invoked (it may emit events first).
  const results: Record<string, () => unknown> = {};
  let generation = 0;
  const invoke = (async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (failures[command]) throw new Error(failures[command]);
    if (results[command]) return results[command]();
    if (command === 'switch_session_profile') return { id: args!.profileId, profile: { ...profile, name: 'Browser' }, modified_at_ms: 1, profile_generation: generation += 1 };
    return command === 'start_service' ? serviceIdFor((args!.request as { path_hex: string }).path_hex) : undefined;
  }) as never;
  const listen = async (name: string, handler: (event: { payload: unknown }) => void) => { handlers[name] = handler; return () => { unlistened.push(name); }; };
  const schedule: Scheduler = callback => { queue.push(callback); };
  const runner = new ServiceRunner(invoke, listen, desktop, schedule, () => 1000);
  runner.profileChangeWaitMs = 50;
  const flush = () => queue.splice(0).forEach(callback => callback());
  const id = serviceIdFor('AABB');
  const record = (value: unknown) => handlers['service://record']({ payload: { service_id: id, record: value } });
  const status = (state: string, extra = {}) => handlers['service://status']({ payload: { service_id: id, state, execute: false, ...extra } });
  return { runner, handlers, unlistened, calls, flush, failures, results, id, record, status };
}
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
const options = { profile, pathHex: 'AABB', execute: false, softwareMode: false, draftUnsaved: false };

describe('summarizeRecord dial functions', () => {
  const event = { module: 'm131e', control: 0, gesture: 'right' };
  it('shows the current function and switches', () => {
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action: { type: 'volume', delta: 5 }, dial_function: { name: 'Volume', index: 0 } }, 1)?.text).toContain('[Volume]');
    expect(summarizeRecord({ type: 'event', status: 'dial_function', event, action: null, navigation: { from_function: 'Volume', to_function: 'Scroll', index: 1, count: 3 } }, 1)).toEqual({ kind: 'event', text: 'Dial function: Volume → Scroll (2/3)', at: 1 });
    expect(summarizeRecord({ type: 'action_executed', action: { type: 'toggle_mute' }, event, dial_function: { name: 'Scroll', index: 1 } }, 1)?.text).toContain(', Scroll)');
  });
});

describe('summarizeRecord display syncs', () => {
  const sync = { type: 'display_sync', schema_version: 1, page_id: 'root', depth: 0, written: [3], failed: [], not_attempted: [], missing: [], error: null, complete: true };
  it('drops clean animation frames and still shows incomplete ones', () => {
    expect(summarizeRecord({ ...sync, trigger: 'animation' }, 1)).toBeNull();
    expect(summarizeRecord({ ...sync, trigger: 'animation', missing: [4] }, 1)).toMatchObject({ kind: 'error', text: 'Key animation frame not fully written.' });
    const failed = { ...sync, trigger: 'animation', written: [], failed: [{ key_index: 3, short_write: true, attempts: 1, error: 'short write' }], complete: false };
    expect(summarizeRecord(failed, 1)).toMatchObject({ kind: 'error' });
    expect(summarizeRecord({ ...sync, trigger: 'animation', complete: false, error: 'gate closed' }, 1)?.text).toBe('Key animation frame not fully written: gate closed');
  });

  it('keeps every other display sync line unchanged', () => {
    for (const trigger of ['software_mode', 'navigation', 'dial_function']) {
      expect(summarizeRecord({ ...sync, trigger }, 1)).toEqual({ kind: 'info', text: 'display sync', at: 1 });
      expect(summarizeRecord({ ...sync, trigger, complete: false }, 1)).toEqual({ kind: 'info', text: 'display sync', at: 1 });
    }
  });
});

describe('summarizeRecord timers', () => {
  const event = { control: 2, gesture: 'pressed' };
  const action = { type: 'timer', seconds: 300, label_chars: 3, sound: 'ping' };
  it('describes start, cancel, expiry and alert failure without any label text', () => {
    expect(summarizeRecord({ type: 'timer_started', event, action, running_count: 1 }, 1)).toEqual({ kind: 'action', text: 'Started timer 5:00 (control 3 pressed).', at: 1 });
    expect(summarizeRecord({ type: 'timer_cancelled', event, action, reason: 'pressed_again' }, 1)?.text).toBe('Cancelled timer 5:00 (pressed again).');
    expect(summarizeRecord({ type: 'timer_cancelled', event: null, action, reason: 'stopped' }, 1)?.text).toBe('Cancelled timer 5:00 (run ended).');
    expect(summarizeRecord({ type: 'timer_expired', action }, 1)?.text).toBe('Countdown ended: timer 5:00.');
    expect(summarizeRecord({ type: 'timer_alert_failed', action, error: 'timer sound failed: x' }, 1)).toMatchObject({ kind: 'action_failed', text: 'Countdown alert failed (timer 5:00): timer sound failed: x' });
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action }, 1)?.text).toBe('control 3 pressed: planned timer 5:00');
    expect(summarizeRecord({ type: 'timer_alert_warning', action, warning: 'timer notification may not have been shown: x' }, 1))
      .toMatchObject({ kind: 'info', text: 'Countdown alert warning (timer 5:00): timer notification may not have been shown: x' });
  });

  it('a dry run logs a planned press as "Would run", a live one keeps the plan wording', () => {
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action }, 1, false)?.text)
      .toBe('Would run: timer 5:00 (control 3 pressed)');
    expect(summarizeRecord({ type: 'event', status: 'planned', event: { module: 'm4315', control: 0, gesture: 'pressed' }, action: { type: 'toggle_mute' } }, 2, false)?.text)
      .toBe('Would run: toggle mute (m4315 control 1 pressed)');
    // Executing runs keep the plan line next to the Executed line.
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action }, 3, true)?.text)
      .toBe('control 3 pressed: planned timer 5:00');
  });

  it('counts timer presses as executed and failed alerts on their own, never as failed actions', () => {
    const zero = { events: 0, planned: 0, executed: 0, failed: 0, timerAlertsFailed: 0 };
    const records = [
      { type: 'event', status: 'planned', event, action },
      { type: 'timer_started', event, action },
      { type: 'event', status: 'planned', event, action },
      { type: 'timer_cancelled', event, action, reason: 'pressed_again' },
      { type: 'timer_expired', action },
      { type: 'timer_alert_failed', action, error: 'x' },
      { type: 'timer_alert_warning', action, warning: 'y' },
      { type: 'timer_cancelled', event: null, action, reason: 'session_end' },
    ];
    // Failed stays equal to service_end.failed_action_count (no action_failed here).
    expect(records.reduce((counters, record) => countRecord(counters, record, true), zero)).toEqual({ events: 2, planned: 2, executed: 2, failed: 0, timerAlertsFailed: 1 });
    // A dry run executes nothing, as for other actions; a failed alert still counts.
    expect(records.reduce((counters, record) => countRecord(counters, record, false), zero)).toEqual({ events: 2, planned: 2, executed: 0, failed: 0, timerAlertsFailed: 1 });
    expect(countRecord(zero, { type: 'action_failed', action, error: 'at most 8 timers can run at once; this timer was not started' }).failed).toBe(1);
  });
});

describe('summarizeRecord', () => {
  it('describes the service record schema', () => {
    expect(summarizeRecord({ type: 'service_start', device: { module: 'm003k' }, execution: 'dry_run' }, 1)).toMatchObject({ kind: 'info', text: expect.stringContaining('dry run') });
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action: { type: 'volume', delta: 5 } }, 2)).toEqual({ kind: 'action', text: 'm003k control 1 pressed: planned volume +5', at: 2 });
    expect(summarizeRecord({ type: 'event', status: 'unbound', event, action: null }, 1)?.kind).toBe('event');
    expect(summarizeRecord({ type: 'event', status: 'navigation', event }, 1)?.text).toContain('folder');
    expect(summarizeRecord({ type: 'action_executed', event, action: { type: 'shortcut', key: 'c', modifiers: ['command'] } }, 1)?.text).toContain('shortcut command+c');
    expect(summarizeRecord({ type: 'action_failed', event, action: { type: 'launch_app', bundle_id: 'com.apple.finder' }, error: 'denied' }, 1)).toMatchObject({ kind: 'action_failed', text: expect.stringContaining('denied') });
    expect(summarizeRecord({ type: 'service_end', reason: 'error', error: 'queue overflow' }, 1)).toMatchObject({ kind: 'error' });
    expect(summarizeRecord({ type: 'service_end', reason: 'capture_complete', error: null }, 1)?.kind).toBe('info');
    expect(summarizeRecord({ type: 'report', status: 'decoded', error: null }, 1)).toBeNull();
    expect(summarizeRecord({ type: 'report', status: 'parse_error', error: 'short' }, 1)?.kind).toBe('error');
  });
  it('logs a macro as its shape only, never the keys it would type', () => {
    const macro = { type: 'macro', steps: [{ key: 'h', modifiers: [], delay_before_ms: 0, hold_ms: 40 }, { key: 'u', modifiers: [], delay_before_ms: 100, hold_ms: 0 }] };
    const planned = summarizeRecord({ type: 'event', status: 'planned', event, action: macro }, 1, false)!.text;
    expect(planned).toBe('Would run: macro (2 keystrokes, 0:00.1 of waits and holds) (m003k control 1 pressed)');
    expect(planned).not.toMatch(/\bh\b|\bu\b|hold_ms/);
    const live = summarizeRecord({ type: 'event', status: 'planned', event, action: macro }, 1)!.text;
    expect(live).toBe('m003k control 1 pressed: planned macro (2 keystrokes, 0:00.1 of waits and holds)');
    const executed = summarizeRecord({ type: 'action_executed', event, action: macro }, 1)!.text;
    expect(executed).toContain('macro (2 keystrokes, 0:00.1 of waits and holds)');
  });
  it('logs only the origin of an opened URL, never its path or query', () => {
    const url = 'https://Example.com:8443/reset?token=s3cr3t#frag';
    const planned = summarizeRecord({ type: 'event', status: 'planned', event, action: { type: 'open_url', url } }, 1)!.text;
    expect(planned).toBe('m003k control 1 pressed: planned open URL https://example.com:8443');
    const executed = summarizeRecord({ type: 'action_executed', event, action: { type: 'multi_action', actions: [{ type: 'open_url', url: 'https://user:pw@host.test/a?b=c' }, { type: 'toggle_mute' }] } }, 1)!.text;
    expect(executed).toContain('open URL https://host.test,');
    for (const text of [planned, executed]) expect(text).not.toMatch(/token|s3cr3t|frag|\/a\?|pw/);
  });
  it('renders redacted records (origin, chars) and old records (url, text) alike', () => {
    const at = (action: unknown) => summarizeRecord({ type: 'action_executed', event, action }, 1)!.text;
    expect(at({ type: 'open_url', origin: 'https://host.test:8443' })).toContain('open URL https://host.test:8443');
    expect(at({ type: 'text', chars: 12, press_enter: true })).toContain('paste text (12 characters)');
    expect(at({ type: 'text', text: 'hello', press_enter: false })).toContain('paste text (5 characters)');
    expect(at({ type: 'multi_action', actions: [{ type: 'open_url', origin: 'https://a.test' }, { type: 'text', chars: 3, press_enter: false }] }))
      .toContain('multi-action [open URL https://a.test, paste text (3 characters)]');
  });
  it('summarises OBS output actions by their closed operation word', () => {
    const at = (action: unknown) => summarizeRecord({ type: 'action_executed', event, action }, 1)!.text;
    expect(at({ type: 'obs_virtual_camera', op: 'toggle' })).toContain('OBS virtual camera toggle');
    expect(at({ type: 'obs_replay_buffer', op: 'save' })).toContain('OBS replay buffer save');
    expect(at({ type: 'multi_action', actions: [{ type: 'obs_replay_buffer', op: 'save' }, { type: 'toggle_mute' }] }))
      .toContain('multi-action [OBS replay buffer save, toggle mute]');
  });
  it('logs only the length of an OBS scene collection or profile name', () => {
    const at = (action: unknown) => summarizeRecord({ type: 'action_executed', event, action }, 1)!.text;
    const lines = [at({ type: 'obs_scene_collection', name: 'Secret show' }), at({ type: 'obs_profile', name_chars: 4 })];
    expect(lines[0]).toContain('OBS scene collection (11 characters)');
    expect(lines[1]).toContain('OBS profile (4 characters)');
    expect(lines[0]).not.toContain('Secret');
  });
  it('logs only the lengths of OBS source, filter and scene names, from new (_chars) and old records alike', () => {
    const at = (action: unknown) => summarizeRecord({ type: 'action_executed', event, action }, 1)!.text;
    const lines = [
      at({ type: 'obs_filter', source: 'Private cam', filter: 'Blur', op: 'off' }),
      at({ type: 'obs_filter', source_chars: 11, filter_chars: 4, op: 'off' }),
      at({ type: 'obs_source_visibility', scene: 'Secret scene', source: 'Private cam', op: 'on' }),
      at({ type: 'multi_action', actions: [{ type: 'obs_filter', source: 'Private cam', filter: 'Blur', op: 'on' }, { type: 'obs_transition' }] }),
    ];
    for (const line of lines) for (const name of ['Private cam', 'Secret scene', 'Blur']) expect(line).not.toContain(name);
    expect(lines[0]).toContain('OBS filter off (11 + 4 characters)');
    expect(lines[1]).toContain('OBS filter off (11 + 4 characters)');
    expect(lines[2]).toContain('OBS source on (12 + 11 characters)');
    expect(lines[3]).toContain('multi-action [OBS filter on (11 + 4 characters), OBS studio transition]');
    expect(at({ type: 'obs_studio_mode', op: 'toggle' })).toContain('OBS studio mode toggle');
  });
  it('logs only the length of an OBS scene name, from redacted (scene_chars) and old (scene) records alike', () => {
    const at = (action: unknown) => summarizeRecord({ type: 'action_executed', event, action }, 1)!.text;
    const long = 'Interview guest cam with a very long name';
    for (const scene of ['Live', 'BRB', long, 'Zażółć 🎥']) {
      const lines = [
        at({ type: 'obs_scene', scene }),
        at({ type: 'multi_action', actions: [{ type: 'obs_scene', scene }, { type: 'toggle_mute' }] }),
        summarizeRecord({ type: 'action_failed', event, action: { type: 'obs_scene', scene }, error: 'OBS at 127.0.0.1:4455: timed out' }, 1)!.text,
        summarizeRecord({ type: 'action_unavailable', event, action: { type: 'obs_scene', scene }, reason: 'OBS is not configured' }, 1)!.text,
      ];
      for (const line of lines) expect(line).not.toContain(scene);
      expect(lines[0]).toContain(`OBS scene (${[...scene].length} characters)`);
    }
    expect(at({ type: 'obs_scene', scene_chars: 4 })).toContain('OBS scene (4 characters)');
    expect(at({ type: 'multi_action', actions: [{ type: 'obs_scene', scene_chars: 3 }, { type: 'toggle_mute' }] }))
      .toContain('multi-action [OBS scene (3 characters), toggle mute]');
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action: { type: 'obs_scene', scene_chars: 13 } }, 1)!.text)
      .toBe('m003k control 1 pressed: planned OBS scene (13 characters)');
  });
  it('tolerates unknown shapes', () => {
    expect(summarizeRecord({ type: 'raw', line: 'hello' }, 1)).toEqual({ kind: 'raw', text: 'hello', at: 1 });
    expect(summarizeRecord('oops', 1)?.kind).toBe('raw');
    expect(summarizeRecord(null, 1)?.kind).toBe('raw');
    expect(summarizeRecord({ type: 'something_new' }, 1)).toEqual({ kind: 'info', text: 'something new', at: 1 });
    expect(summarizeRecord({ weird: true }, 1)?.kind).toBe('raw');
    expect(summarizeRecord({ type: 'event', status: 'planned', event: 3, action: 4 }, 1)?.kind).toBe('action');
  });
});

describe('ServiceRunner', () => {
  it('starts, tracks status transitions, counts and stops', async () => {
    const t = setup(); t.runner.connect(); await tick();
    expect(await t.runner.start(options)).toBe(true);
    expect(t.calls[0]).toEqual({ command: 'start_service', args: { request: { profile, path_hex: 'AABB', execute: false, software_mode: false } } });
    expect(t.runner.getState().runs[t.id].state).toBe('starting');
    expect(t.runner.isActivePath('aabb')).toBe(true);
    t.status('running');
    expect(t.runner.getState().runs[t.id].state).toBe('running');
    t.record({ type: 'event', status: 'planned', event, action: { type: 'toggle_mute' } });
    t.record({ type: 'event', status: 'unbound', event, action: null });
    t.record({ type: 'action_executed', event, action: { type: 'toggle_mute' } });
    t.record({ type: 'action_failed', event, action: { type: 'toggle_mute' }, error: 'x' });
    expect(t.runner.getState().runs[t.id].counters.events).toBe(0);
    t.flush();
    const run = t.runner.getState().runs[t.id];
    expect(run.counters).toEqual({ events: 2, planned: 1, executed: 1, failed: 1, timerAlertsFailed: 0 });
    expect(run.entries).toHaveLength(4);
    await t.runner.stop(t.id);
    expect(t.calls.at(-1)).toEqual({ command: 'stop_service', args: { serviceId: t.id } });
    expect(t.runner.getState().runs[t.id].state).toBe('stopping');
    t.status('exited', { exit_code: 0 });
    expect(t.runner.getState().runs[t.id].state).toBe('exited');
    expect(t.runner.isActivePath('AABB')).toBe(false);
  });
  it('shows a start rejection as a failed run', async () => {
    const t = setup(); t.failures.start_service = 'Another reader owns this device.';
    expect(await t.runner.start(options)).toBe(false);
    expect(t.runner.getState().runs[t.id]).toMatchObject({ state: 'failed', error: 'Another reader owns this device.' });
  });
  it('keeps a status that arrived before start returned', async () => {
    const t = setup(); t.runner.connect(); await tick();
    const started = t.runner.start(options);
    t.status('running');
    await started;
    expect(t.runner.getState().runs[t.id].state).toBe('running');
  });
  it('records failure with stderr tail and exit code', async () => {
    const t = setup(); t.runner.connect(); await tick();
    await t.runner.start(options);
    t.status('failed', { exit_code: 2, error: 'device not found', stderr_tail: 'boom' });
    expect(t.runner.getState().runs[t.id]).toMatchObject({ state: 'failed', exitCode: 2, error: 'device not found', stderrTail: 'boom' });
  });
  it('refuses a second start while active', async () => {
    const t = setup(); await t.runner.start(options);
    expect(await t.runner.start(options)).toBe(false);
    expect(t.calls.filter(call => call.command === 'start_service')).toHaveLength(1);
    expect(t.runner.getState().error).toContain('already running');
  });
  it('caps the activity buffer', async () => {
    const t = setup(); t.runner.connect(); await tick(); await t.runner.start(options);
    for (let index = 0; index < maxEntries + 50; index += 1) t.record({ type: 'raw', line: `line ${index}` });
    t.flush();
    const entries = t.runner.getState().runs[t.id].entries;
    expect(entries).toHaveLength(maxEntries);
    expect(entries.at(-1)!.text).toBe(`line ${maxEntries + 49}`);
    // Each entry keeps a sequence number that survives trimming (a stable list key).
    expect(entries[0].seq).toBe(50);
    expect(entries.at(-1)!.seq).toBe(maxEntries + 49);
    expect(entries.every((entry, index) => entry.seq === 50 + index)).toBe(true);
  });
  it('never executes without a confirmation that resolves true', async () => {
    const t = setup();
    expect(await t.runner.start({ ...options, execute: true })).toBe(false);
    expect(await t.runner.start({ ...options, execute: true }, async () => false)).toBe(false);
    expect(t.calls).toHaveLength(0);
    expect(await t.runner.start({ ...options, execute: true }, async () => true)).toBe(true);
    expect(t.calls[0].args).toMatchObject({ request: { execute: true } });
  });
  it('does nothing in the browser preview', async () => {
    const t = setup(false); t.runner.connect();
    expect(await t.runner.start(options)).toBe(false);
    expect(Object.keys(t.handlers)).toHaveLength(0);
    expect(t.calls).toHaveLength(0);
  });
  it('unlistens on dispose and ignores later records', async () => {
    const t = setup(); t.runner.connect(); await tick(); await t.runner.start(options);
    t.runner.dispose();
    expect(t.unlistened.sort()).toEqual(['service://record', 'service://status', 'session://state']);
    t.record({ type: 'raw', line: 'late' }); t.flush();
    expect(t.runner.getState().runs[t.id].entries).toHaveLength(0);
  });
  it('flags a stale snapshot only while active, ignoring key order', async () => {
    const t = setup(); await t.runner.start(options);
    const run = t.runner.getState().runs[t.id];
    expect(isStale(run, { bindings: profile.bindings, name: 'Everyday', schema_version: 1 })).toBe(false);
    expect(isStale(run, { ...profile, name: 'Changed' })).toBe(true);
    expect(isStale({ ...run, state: 'exited' }, { ...profile, name: 'Changed' })).toBe(false);
    expect(isStale(undefined, profile)).toBe(false);
  });
});

describe('session manager', () => {
  const id2 = serviceIdFor('CCDD');
  const session = (t: ReturnType<typeof setup>, phase: string, extra: Record<string, unknown> = {}) => t.handlers['session://state']({ payload: {
    session_id: t.id, service_id: t.id, phase, reason: null, attempt: 0, execute: false, module: 'm131e', serial_number: 'SER12345', ...extra } });
  const started = async (execute = false) => {
    const t = setup(); t.runner.connect(); await tick();
    await t.runner.start({ ...options, execute }, async () => true);
    t.status('running', { execute }); session(t, 'running', { execute });
    return t;
  };
  const run = (t: ReturnType<typeof setup>) => t.runner.getState().runs[t.id];
  const texts = (t: ReturnType<typeof setup>) => run(t).entries.map(entry => entry.text);

  it('listens to session events', async () => {
    const t = setup(); t.runner.connect(); await tick();
    expect(Object.keys(t.handlers).sort()).toEqual(['service://record', 'service://status', 'session://state']);
  });
  it('logs every phase and reason without duplicating unchanged events', async () => {
    const t = await started(true);
    session(t, 'suspended', { service_id: null, reason: 'sleep', execute: true });
    session(t, 'suspended', { service_id: null, reason: 'sleep', execute: true });
    session(t, 'needs_confirmation', { service_id: null, reason: 'sleep', execute: true });
    session(t, 'waiting_for_device', { service_id: null, reason: 'device_removed', execute: true });
    session(t, 'reconnecting', { service_id: null, reason: 'device_removed', attempt: 2, execute: true, next_attempt_in_ms: 4000 });
    session(t, 'running', { service_id: id2, execute: true });
    session(t, 'stopped', { service_id: null, reason: 'service_exited', detail: 'crashed', execute: true });
    expect(texts(t)).toEqual(['Mac went to sleep — actions paused', 'Paused: confirm to resume actions', 'Waiting for m131e to reconnect',
      'Reconnecting (attempt 2/5)', 'Reconnected', 'Paused: the background helper exited (crashed)']);
    expect(run(t).entries.at(-1)!.kind).toBe('error');
  });
  it('a refused start ends the run once, without reconnect attempts or the service sentence', async () => {
    const t = await started();
    const raw = 'masterhub-service: nested keypad profiles run live only with --display-sync (which requires --software-mode); in OpenStreamHub, set Module control to App in Settings';
    const friendly = 'This profile\'s 15-key pages run live only when OpenStreamHub controls the module. Module control in Settings is set to Built-in animation: set it to App, then start the profile again.';
    t.status('exited', { exit_code: 1, stderr_tail: `${raw}\n` });
    session(t, 'stopped', { service_id: null, reason: 'start_refused', detail: friendly });
    const entries = texts(t);
    expect(entries.filter(text => text.startsWith('Paused'))).toEqual([`Paused: the profile could not start (${friendly})`]);
    expect(entries.some(text => text.startsWith('Reconnecting'))).toBe(false);
    expect(entries.join('\n')).not.toContain('--display-sync');
    expect(run(t).entries.at(-1)!.kind).toBe('error');
    expect(run(t).state).toBe('exited');
    expect(run(t).session).toMatchObject({ phase: 'stopped', reason: 'start_refused', attempt: 0, detail: friendly });
  });
  it('logs a user stop as info and unplug as stopped reason', async () => {
    const t = await started();
    session(t, 'stopped', { service_id: null, reason: 'user' });
    expect(run(t).entries.at(-1)).toMatchObject({ kind: 'info', text: 'Paused: stopped by you' });
    expect(run(t).state).toBe('exited');
  });
  it('turns next_attempt_in_ms into a deadline', async () => {
    const t = await started();
    session(t, 'reconnecting', { service_id: null, attempt: 1, next_attempt_in_ms: 3000 });
    expect(run(t).session).toMatchObject({ phase: 'reconnecting', attempt: 1, nextAttemptAt: 4000 });
    session(t, 'running', { service_id: id2 });
    expect(run(t).session!.nextAttemptAt).toBeUndefined();
  });
  it('lands records and status of a reconnected process in the same session run', async () => {
    const t = await started();
    t.record({ type: 'event', status: 'planned', event, action: { type: 'toggle_mute' } }); t.flush();
    session(t, 'reconnecting', { service_id: null, attempt: 1 });
    session(t, 'running', { service_id: id2 });
    t.handlers['service://status']({ payload: { service_id: id2, state: 'running', execute: false } });
    t.handlers['service://record']({ payload: { service_id: id2, record: { type: 'event', status: 'planned', event, action: { type: 'toggle_mute' } } } });
    t.flush();
    const runs = t.runner.getState().runs;
    expect(Object.keys(runs)).toEqual([t.id]);
    expect(run(t).counters).toMatchObject({ events: 2, planned: 2 });
    expect(run(t).session!.serviceId).toBe(id2);
    // late exit of the superseded process does not end the session
    t.status('exited', { exit_code: 1 });
    expect(run(t).state).toBe('running');
    // The session owns only its current path, not the one its name came from.
    expect(t.runner.isActivePath('AABB')).toBe(false);
    expect(t.runner.isActivePath('CCDD')).toBe(true);
    // exit of the current process is applied
    t.handlers['service://status']({ payload: { service_id: id2, state: 'exited', execute: false } });
    expect(run(t).state).toBe('exited');
  });
  it('keeps blocking auto-live until the session is stopped', async () => {
    const t = await started();
    const device = { path_hex: 'ccdd', module: 'm131e' as const, serial_number: 'SER12345' };
    const other = { path_hex: 'eeff', module: 'm003k' as const, serial_number: null };
    for (const phase of ['suspended', 'waiting_for_device', 'reconnecting', 'needs_confirmation']) {
      session(t, phase, { service_id: null });
      expect(t.runner.isActivePath('ccdd', device)).toBe(true);
      expect(t.runner.isActivePath('eeff', other)).toBe(false);
      expect(t.runner.isActivePath('aabb', { ...device, path_hex: 'aabb' })).toBe(true);
      expect(t.runner.isActivePath('aabb')).toBe(false);
    }
    session(t, 'stopped', { service_id: null, reason: 'user' });
    expect(t.runner.isActivePath('ccdd', device)).toBe(false);
    expect(t.runner.isActivePath('aabb')).toBe(false);
  });
  it('keeps the ring buffer and counters across reconnects', async () => {
    const t = await started();
    for (let index = 0; index < maxEntries + 20; index += 1) t.record({ type: 'raw', line: `l${index}` });
    t.flush();
    session(t, 'suspended', { service_id: null, reason: 'sleep' });
    expect(run(t).entries).toHaveLength(maxEntries);
    expect(run(t).entries.at(-1)!.text).toBe('Mac went to sleep — actions paused');
  });
  it('stops through stop_session when no process exists and stop_service otherwise', async () => {
    const t = await started();
    await t.runner.stop(t.id);
    expect(t.calls.at(-1)).toEqual({ command: 'stop_service', args: { serviceId: t.id } });
    session(t, 'running', { service_id: id2 });
    await t.runner.stop(t.id);
    expect(t.calls.at(-1)).toEqual({ command: 'stop_service', args: { serviceId: id2 } });
    for (const phase of ['suspended', 'waiting_for_device', 'reconnecting', 'needs_confirmation']) {
      session(t, phase, { service_id: null });
      await t.runner.stop(t.id);
      expect(t.calls.at(-1)).toEqual({ command: 'stop_session', args: { sessionId: t.id } });
    }
  });
  it('resumes only after a confirmation that resolves true and only in needs_confirmation', async () => {
    const t = await started(true);
    const resumes = () => t.calls.filter(call => call.command === 'resume_session');
    session(t, 'suspended', { service_id: null, execute: true });
    expect(await t.runner.resumeSession(t.id, async () => true)).toBe(false);
    session(t, 'needs_confirmation', { service_id: null, execute: true });
    expect(await t.runner.resumeSession(t.id)).toBe(false);
    expect(await t.runner.resumeSession(t.id, async () => false)).toBe(false);
    expect(resumes()).toHaveLength(0);
    expect(await t.runner.resumeSession(t.id, async () => true)).toBe(true);
    expect(resumes()).toEqual([{ command: 'resume_session', args: { sessionId: t.id, profileGeneration: 0 } }]);
  });
  it('resumes a dry run without a confirmation, and an executing one never without one', async () => {
    const t = await started(false);
    session(t, 'needs_confirmation', { service_id: null, execute: false });
    expect(await t.runner.resumeSession(t.id)).toBe(true);
    expect(t.calls.at(-1)).toEqual({ command: 'resume_session', args: { sessionId: t.id, profileGeneration: 0 } });
    // The backend refusing the dry resume is an error, never a dialog.
    t.failures.resume_session = 'unknown session x';
    expect(await t.runner.resumeSession(t.id)).toBe(false);
    expect(t.runner.getState().error).toBe('unknown session x');
  });
  it('resume arguments and the stale-profile refusal match the Rust fixtures', () => {
    expect(Object.keys(sessionArgs.resume_session).sort()).toEqual(['profileGeneration', 'sessionId']);
    expect(sessionResults.resume_session_profile_changed).toBe(profileChangedError);
  });
  it('asks again for the new profile when a switch lands while the Resume dialog is open', async () => {
    const t = await started(true);
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'sleep' });
    const shown: string[] = [];
    const confirm = async () => {
      shown.push(run(t).profileName);
      if (shown.length === 1) {
        await t.runner.switchProfile(t.id, 'p-browser', 'focused com.apple.Safari');
        session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'sleep', profile_generation: 1 });
      }
      return true;
    };
    expect(await t.runner.resumeSession(t.id, confirm)).toBe(true);
    expect(shown).toEqual(['Everyday', 'Browser']);
    expect(t.calls.filter(call => call.command === 'resume_session')).toEqual([{ command: 'resume_session', args: { sessionId: t.id, profileGeneration: 1 } }]);
  });
  it('asks again when the backend refuses a stale profile generation', async () => {
    const t = await started(true);
    session(t, 'needs_confirmation', { service_id: null, execute: true });
    t.failures.resume_session = profileChangedError;
    let prompts = 0;
    expect(await t.runner.resumeSession(t.id, async () => { prompts += 1; if (prompts === 2) delete t.failures.resume_session; return true; })).toBe(true);
    expect(prompts).toBe(2);
    expect(t.calls.filter(call => call.command === 'resume_session')).toHaveLength(2);
    expect(t.runner.getState().error).toBeNull();
  });
  it('never confirms a profile the run does not name yet', async () => {
    const t = await started(true);
    // The session reports a newer generation than the profile name the panel shows.
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 1 });
    let prompted = false;
    expect(await t.runner.resumeSession(t.id, async () => { prompted = true; return true; })).toBe(false);
    expect(prompted).toBe(false);
    expect(t.calls.filter(call => call.command === 'resume_session')).toEqual([]);
    expect(t.runner.getState().error).toContain('profile of this run changed');
  });
  it('a declined re-prompt resumes nothing', async () => {
    const t = await started(true);
    session(t, 'needs_confirmation', { service_id: null, execute: true });
    t.failures.resume_session = profileChangedError;
    let prompts = 0;
    expect(await t.runner.resumeSession(t.id, async () => { prompts += 1; return prompts === 1; })).toBe(false);
    expect(t.calls.filter(call => call.command === 'resume_session')).toHaveLength(1);
  });
  it('resyncs the profile from session state after a switch reply timed out', async () => {
    const t = await started(true);
    t.failures.switch_session_profile = 'the session manager did not respond';
    expect(await t.runner.switchProfile(t.id, 'p-browser', 'focused com.apple.Safari')).toBe(false);
    expect(run(t)).toMatchObject({ profileName: 'Everyday', profileGeneration: 0 });
    // The backend did switch: its state names the new profile and generation.
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 1, profile_id: 'p-browser', profile_name: 'Browser' });
    expect(run(t)).toMatchObject({ profileName: 'Browser', profileGeneration: 1, profileId: 'p-browser', profileSwitched: true });
    // The state carries no profile body, so the profile the backend holds is loaded for the dialog.
    t.results.session_profile = () => ({ profile_id: 'p-browser', profile_generation: 1, profile: { ...profile, name: 'Browser' } });
    const shown: string[] = [];
    expect(await t.runner.resumeSession(t.id, async () => { shown.push(run(t).profileName); return true; })).toBe(true);
    expect(shown).toEqual(['Browser']);
    expect(t.calls.filter(call => call.command === 'session_profile')).toEqual([{ command: 'session_profile', args: { sessionId: t.id, profileGeneration: 1 } }]);
    expect(t.calls.filter(call => call.command === 'resume_session')).toEqual([{ command: 'resume_session', args: { sessionId: t.id, profileGeneration: 1 } }]);
  });
  it('resyncs the profile of a paused session after a webview reload', async () => {
    const t = setup(); t.runner.connect(); await tick();
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 2, profile_id: 'p-editor', profile_name: 'Editor' });
    expect(run(t)).toMatchObject({ profileName: 'Editor', profileGeneration: 2, profileId: 'p-editor', profileSwitched: true });
    // An older event that overtook the newer state never moves the run back.
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 1, profile_id: 'p-browser', profile_name: 'Browser' });
    expect(run(t)).toMatchObject({ profileName: 'Editor', profileGeneration: 2 });
    const shown: string[] = [];
    // The run holds no profile body after the reload, so the held profile is asked for first.
    t.results.session_profile = () => ({ profile_id: 'p-editor', profile_generation: 2, profile: { ...profile, name: 'Editor' } });
    expect(await t.runner.resumeSession(t.id, async () => { shown.push(run(t).profileName); return true; })).toBe(true);
    expect(shown).toEqual(['Editor']);
    expect(run(t).snapshot).toBe(stableStringify({ ...profile, name: 'Editor' }));
    expect(t.calls).toEqual([
      { command: 'session_profile', args: { sessionId: t.id, profileGeneration: 2 } },
      { command: 'resume_session', args: { sessionId: t.id, profileGeneration: 2 } },
    ]);
  });
  it('a profile key switch resyncs the run and is remembered as a key switch', async () => {
    const t = setup(); t.runner.connect(); await tick();
    const { record, states } = profileActionEvents;
    expect(summarizeRecord(record.record, 1)).toEqual({ kind: 'action', text: 'Profile key: next profile requested (control 1 long press).', at: 1 });
    for (const state of states) t.handlers['session://state']({ payload: state });
    const paused = states.at(-1)!;
    expect(paused).toMatchObject({ phase: 'needs_confirmation', reason: 'profile_action' });
    expect(t.runner.getState().runs[paused.session_id]).toMatchObject({
      profileName: 'Streaming', profileGeneration: 1, profileId: paused.profile_id, profileSwitched: true, profileSwitchedBy: 'key', snapshot: unknownSnapshot,
    });
    expect(t.runner.getState().runs[paused.session_id].entries.map(entry => entry.text)).toContain('Paused for the new profile: confirm to resume actions');
    // Before asking, the exact profile the backend holds for this generation is fetched for the dialog.
    t.results.session_profile = () => profileActionEvents.session_profile;
    let shown: ServiceRun | undefined;
    expect(await t.runner.resumeSession(paused.session_id, async () => { shown = t.runner.getState().runs[paused.session_id]; return true; })).toBe(true);
    expect(shown).toMatchObject({ profileName: 'Streaming', profileId: profileActionEvents.session_profile.profile_id, snapshot: stableStringify(profileActionEvents.session_profile.profile) });
    expect(t.calls).toEqual([
      { command: 'session_profile', args: profileActionEvents.session_profile_args },
      { command: 'resume_session', args: { sessionId: paused.session_id, profileGeneration: 1 } },
    ]);
  });
  it('opens no Resume dialog when the held profile cannot be loaded', async () => {
    const t = setup(); t.runner.connect(); await tick();
    const { states } = profileActionEvents;
    for (const state of states) t.handlers['session://state']({ payload: state });
    const paused = states.at(-1)!;
    t.failures.session_profile = 'the session manager did not respond';
    let prompted = false;
    expect(await t.runner.resumeSession(paused.session_id, async () => { prompted = true; return true; })).toBe(false);
    expect(prompted).toBe(false);
    expect(t.calls.map(call => call.command)).toEqual(['session_profile']);
    expect(t.runner.getState().error).toContain('Could not load the profile this run holds');
  });
  it('waits for the newer state after a refusal before asking again', async () => {
    const t = await started(true);
    session(t, 'needs_confirmation', { service_id: null, execute: true });
    t.runner.profileChangeWaitMs = 1000;
    let refused = false;
    t.results.resume_session = () => {
      if (refused) return null;
      refused = true;
      // The refusal's state event arrives after the command reply.
      setTimeout(() => session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 1, profile_id: 'p-browser', profile_name: 'Browser' }), 20);
      throw new Error(profileChangedError);
    };
    t.results.session_profile = () => ({ profile_id: 'p-browser', profile_generation: 1, profile: { ...profile, name: 'Browser' } });
    const shown: string[] = [];
    expect(await t.runner.resumeSession(t.id, async () => { shown.push(run(t).profileName); return true; })).toBe(true);
    expect(shown).toEqual(['Everyday', 'Browser']);
    expect(t.calls.filter(call => call.command === 'resume_session').map(call => call.args)).toEqual([
      { sessionId: t.id, profileGeneration: 0 }, { sessionId: t.id, profileGeneration: 1 },
    ]);
  });
  it('a stale held profile asks again for the newer generation', async () => {
    const t = setup(); t.runner.connect(); await tick();
    const { states } = profileActionEvents;
    for (const state of states) t.handlers['session://state']({ payload: state });
    const paused = states.at(-1)!;
    let fetched = 0;
    t.results.session_profile = () => {
      fetched += 1;
      if (fetched === 1) {
        // A newer switch landed: the backend refuses and resends its state.
        t.handlers['session://state']({ payload: { ...paused, profile_generation: 2, profile_name: 'Browser', profile_id: 'p-browser' } });
        throw new Error(profileChangedError);
      }
      return { profile_id: 'p-browser', profile_generation: 2, profile: { ...profile, name: 'Browser' } };
    };
    const prompts: string[] = [];
    expect(await t.runner.resumeSession(paused.session_id, async () => { prompts.push(t.runner.getState().runs[paused.session_id].profileName); return true; })).toBe(true);
    expect(prompts).toEqual(['Browser']);
    expect(t.calls.map(call => call.command)).toEqual(['session_profile', 'session_profile', 'resume_session']);
    expect(t.calls.at(-1)!.args).toEqual({ sessionId: paused.session_id, profileGeneration: 2 });
  });
  it('describes an unavailable profile key in a standalone-style log', () => {
    expect(summarizeRecord({ type: 'action_unavailable', event, action: { type: 'profile', direction: 'previous' }, reason: 'profile switching needs the OpenStreamHub session manager; no action was taken' }, 1))
      .toEqual({ kind: 'info', text: 'previous profile not available: profile switching needs the OpenStreamHub session manager; no action was taken', at: 1 });
    expect(summarizeRecord({ type: 'event', status: 'planned', event, action: { type: 'profile', direction: 'next' } }, 1)?.text).toBe('m003k control 1 pressed: planned next profile');
  });
  it('session state fixtures carry the profile the runner resyncs from', async () => {
    for (const event of [...sessionEvents, ...switchEvents]) expect(Object.keys(event)).toEqual(expect.arrayContaining(['profile_generation', 'profile_id', 'profile_name']));
    const t = setup(); t.runner.connect(); await tick();
    const paused = switchEvents.at(-1)!;
    t.handlers['session://state']({ payload: paused });
    expect(t.runner.getState().runs[paused.session_id]).toMatchObject({ profileName: paused.profile_name, profileGeneration: paused.profile_generation, profileId: paused.profile_id, profileSwitched: true });
  });
  it('a run started from an unsaved draft does not claim its saved profile', async () => {
    const draft = setup(); draft.runner.connect(); await tick();
    await draft.runner.start({ ...options, draftUnsaved: true, profileId: 'p-saved' });
    expect(draft.runner.getState().runs[draft.id].profileId).toBeNull();
    const saved = setup(); saved.runner.connect(); await tick();
    await saved.runner.start({ ...options, profileId: 'p-saved' });
    expect(saved.runner.getState().runs[saved.id]).toMatchObject({ profileId: 'p-saved', profileGeneration: 0 });
    // The backend learns the saved profile a run starts from, so a profile key's next/previous starts there.
    expect((saved.calls[0].args!.request as { profile_id?: string }).profile_id).toBe('p-saved');
    expect(draft.calls[0].args!.request).not.toHaveProperty('profile_id');
  });
  it('surfaces a failing resume or stop_session', async () => {
    const t = await started(); session(t, 'needs_confirmation', { service_id: null });
    t.failures.resume_session = 'nope';
    expect(await t.runner.resumeSession(t.id, async () => true)).toBe(false);
    expect(t.runner.getState().error).toBe('nope');
    t.failures.stop_session = 'cannot';
    await t.runner.stopSession(t.id);
    expect(t.runner.getState().error).toBe('cannot');
  });
  it('findRun matches a session by process, module or fallback', async () => {
    const t = await started(); session(t, 'waiting_for_device', { service_id: null });
    const runs = t.runner.getState().runs;
    expect(findRun(runs, undefined)).toBe(runs[t.id]);
    expect(findRun(runs, { path_hex: 'ccdd', module: 'm131e', serial_number: 'SER12345' })).toBe(runs[t.id]);
    expect(findRun(runs, { path_hex: 'ccdd', module: 'm003k', serial_number: null })).toBeUndefined();
  });
  it('a replug frees the old path for another device whose session:<uuid> events attach to its run', async () => {
    const t = await started();
    session(t, 'waiting_for_device', { service_id: null, reason: 'device_removed', attempt: 1 });
    session(t, 'running', { service_id: id2 });
    const other = { path_hex: 'aabb', module: 'm131e' as const, serial_number: 'SER-2' };
    expect(t.runner.isActivePath('aabb', other)).toBe(false);
    expect(await t.runner.start(options)).toBe(true);
    expect(t.calls.filter(call => call.command === 'start_service')).toHaveLength(2);
    expect(t.runner.getState().error).toBeNull();
    expect(t.runner.isActivePath('aabb', other)).toBe(true);
    // The new process's status and records arrive before its session event, under the old session's name.
    t.status('running');
    t.record({ type: 'event', status: 'planned', event, action: { type: 'toggle_mute' } }); t.flush();
    expect(run(t).session!.serviceId).toBe(id2);
    expect(run(t).state).toBe('running');
    expect(run(t).counters.events).toBe(0);
    const fresh = Object.values(t.runner.getState().runs).find(r => r !== run(t))!;
    expect(fresh).toMatchObject({ profileName: 'Everyday', state: 'running', counters: { events: 1 } });
    expect(findRun(t.runner.getState().runs, other)).toBe(fresh);
    const uuid = 'session:0b7c1d2e-3f40-4a5b-8c6d-7e8f9a0b1c2d';
    session(t, 'running', { session_id: uuid, service_id: t.id, serial_number: 'SER-2' });
    const runs = t.runner.getState().runs;
    expect(Object.keys(runs).sort()).toEqual([uuid, t.id].sort());
    expect(Object.values(runs).some(r => r.profileName === 'Unknown profile')).toBe(false);
    expect(runs[uuid]).toMatchObject({ serviceId: uuid, profileName: 'Everyday', counters: { events: 1 }, session: { serviceId: t.id, serialNumber: 'SER-2' } });
    expect(findRun(runs, other)).toBe(runs[uuid]);
    expect(findRun(runs, { path_hex: 'ccdd', module: 'm131e', serial_number: 'SER12345' })).toBe(runs[t.id]);
    // Later records and status of the new process land in its own run, never in the moved session.
    t.record({ type: 'event', status: 'unbound', event, action: null }); t.flush();
    expect(t.runner.getState().runs[uuid].counters.events).toBe(2);
    expect(run(t).counters.events).toBe(0);
    await t.runner.stop(uuid);
    expect(t.calls.at(-1)).toEqual({ command: 'stop_service', args: { serviceId: t.id } });
    t.status('exited', { exit_code: 0 });
    expect(t.runner.getState().runs[uuid].state).toBe('exited');
    expect(run(t).state).toBe('running');
  });
  describe('a start while asleep on a path a moved session is still named after', () => {
    const collision = sessionResults.start_service_asleep_collision;
    const uuid = collision.result;
    const device = { path_hex: 'aabb', module: collision.first_state.module as 'm131e', serial_number: collision.first_state.serial_number };
    // The backend reports the name of the profile the run was started with.
    const start = { ...options, profile: { ...profile, name: collision.first_state.profile_name } };
    // The older session t.id moved to CCDD, so the new run on AABB waits under `pending:` until its session names it.
    const moved = async () => {
      const t = await started();
      session(t, 'waiting_for_device', { service_id: null, reason: 'device_removed', attempt: 1 });
      session(t, 'running', { service_id: id2 });
      return t;
    };
    const asleepState = (t: ReturnType<typeof setup>) => t.handlers['session://state']({ payload: collision.first_state });
    const expectAdopted = (t: ReturnType<typeof setup>) => {
      const runs = t.runner.getState().runs;
      expect(Object.keys(runs).sort()).toEqual([uuid, t.id].sort());
      expect(runs[uuid]).toMatchObject({ serviceId: uuid, profileName: collision.first_state.profile_name, snapshot: expect.stringContaining(start.profile.name), session: { serviceId: null, phase: 'suspended', serialNumber: device.serial_number } });
      expect(Object.values(runs).some(r => r.profileName === 'Unknown profile')).toBe(false);
      // Nothing is left 'starting' on AABB: the path is free for the session to reclaim and a stop ends the session.
      expect(t.runner.isActivePath('aabb')).toBe(false);
      expect(t.runner.isActivePath('aabb', device)).toBe(true);
      expect(findRun(runs, device)).toBe(runs[uuid]);
      expect(isActiveRun(runs[t.id])).toBe(true);
    };
    it('the first state (no process yet) attaches to the run started on that module', async () => {
      const t = await moved();
      // Matched by module and serial as soon as the state arrives, before the start result.
      t.results.start_service = () => { asleepState(t); expectAdopted(t); return uuid; };
      expect(await t.runner.start({ ...start, device })).toBe(true);
      expectAdopted(t);
      await t.runner.stop(uuid);
      expect(t.calls.at(-1)).toEqual({ command: 'stop_session', args: { sessionId: uuid } });
    });
    it('without the device, the start result merges the run its state created', async () => {
      const t = await moved();
      t.results.start_service = () => { asleepState(t); return uuid; };
      expect(await t.runner.start(start)).toBe(true);
      expectAdopted(t);
      expect(t.runner.getState().runs[uuid].entries.map(entry => entry.text)).toEqual(['Mac went to sleep — actions paused']);
    });
    it('a start result that arrives before the state moves the run to its session id', async () => {
      const t = await moved();
      t.results.start_service = () => uuid;
      expect(await t.runner.start(start)).toBe(true);
      expect(Object.keys(t.runner.getState().runs).sort()).toEqual([uuid, t.id].sort());
      asleepState(t);
      expectAdopted(t);
    });
  });
  it('a run switched by a backend state no longer compares against the profile it started with', async () => {
    const t = await started(true);
    expect(isStale(run(t), { ...profile, name: 'Changed' })).toBe(true);
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 1, profile_id: 'p-browser', profile_name: 'Browser' });
    expect(run(t)).toMatchObject({ profileName: 'Browser', snapshot: unknownSnapshot });
    expect(isStale(run(t), profile)).toBe(false);
    expect(isStale(run(t), { ...profile, name: 'Browser' })).toBe(false);
    // A switch reply carries the profile body, and a later state of the same generation keeps it.
    await t.runner.switchProfile(t.id, 'p-editor', 'focused com.apple.TextEdit');
    session(t, 'needs_confirmation', { service_id: null, execute: true, reason: 'profile_switch', profile_generation: 1, profile_id: 'p-editor', profile_name: 'Browser' });
    expect(run(t).snapshot).not.toBe(unknownSnapshot);
    expect(isStale(run(t), { ...profile, name: 'Browser' })).toBe(false);
    expect(isStale(run(t), profile)).toBe(true);
  });
  it('a run created by a session event alone still shows the profile-changed notice', async () => {
    const t = setup(); t.runner.connect(); await tick();
    // A webview reload: the frontend never saw this run start, so it cannot know the profile matches the draft.
    session(t, 'running');
    expect(run(t)).toMatchObject({ profileName: 'Unknown profile', snapshot: '' });
    expect(isStale(run(t), profile)).toBe(true);
  });
  it('a session event without a process never takes over another serial-less run of the same module', async () => {
    const t = setup(); t.runner.connect(); await tick();
    const bare = (path_hex: string) => ({ path_hex, module: 'm003k' as const, serial_number: null });
    const first = serviceIdFor('aabb'), second = serviceIdFor('ccdd'), uuid = 'session:serial-less';
    expect(await t.runner.start({ ...options, profile: { ...profile, name: 'First' }, device: bare('aabb') })).toBe(true);
    // The second start's session (asleep, no process yet) reports before its start result.
    t.results.start_service = () => {
      session(t, 'suspended', { session_id: uuid, service_id: null, reason: 'sleep', module: 'm003k', serial_number: null });
      expect(t.runner.getState().runs[first]).toMatchObject({ profileName: 'First', state: 'starting' });
      expect(t.runner.getState().runs[first].session).toBeUndefined();
      return uuid;
    };
    expect(await t.runner.start({ ...options, pathHex: 'CCDD', profile: { ...profile, name: 'Second' }, device: bare('ccdd') })).toBe(true);
    const runs = t.runner.getState().runs;
    expect(Object.keys(runs).sort()).toEqual([first, uuid].sort());
    expect(runs[first]).toMatchObject({ profileName: 'First', state: 'starting' });
    expect(runs[first].session).toBeUndefined();
    expect(runs[uuid]).toMatchObject({ profileName: 'Second', session: { phase: 'suspended' } });
    expect(runs[second]).toBeUndefined();
  });
  describe('a pending run whose start returns the path\'s own service id', () => {
    const fresh = { ...options, profile: { ...profile, name: 'Fresh' } };
    // The session t.id moved to CCDD, so a new start on AABB waits under `pending:`.
    const moved = async () => {
      const t = await started();
      session(t, 'waiting_for_device', { service_id: null, reason: 'device_removed', attempt: 1 });
      session(t, 'running', { service_id: id2 });
      return t;
    };
    it('replaces the run of a session that ended meanwhile', async () => {
      const t = await moved();
      t.results.start_service = () => { session(t, 'stopped', { service_id: null, reason: 'device_removed' }); return t.id; };
      expect(await t.runner.start(fresh)).toBe(true);
      const runs = t.runner.getState().runs;
      expect(Object.keys(runs)).toEqual([t.id]);
      expect(runs[t.id]).toMatchObject({ serviceId: t.id, profileName: 'Fresh', state: 'starting', counters: { events: 0 } });
      expect(runs[t.id].session).toBeUndefined();
      session(t, 'running');
      expect(t.runner.getState().runs[t.id]).toMatchObject({ profileName: 'Fresh', state: 'running', session: { serviceId: t.id } });
    });
    it('merges into the run its new session already reported', async () => {
      const t = await moved();
      t.results.start_service = () => { session(t, 'running'); return t.id; };
      expect(await t.runner.start(fresh)).toBe(true);
      const runs = t.runner.getState().runs;
      expect(Object.keys(runs)).toEqual([t.id]);
      expect(runs[t.id]).toMatchObject({ profileName: 'Fresh', session: { serviceId: t.id, phase: 'running' } });
    });
    it('keeps its key while the session named after the path still runs elsewhere', async () => {
      const t = await moved();
      expect(await t.runner.start(fresh)).toBe(true);
      const runs = t.runner.getState().runs;
      expect(Object.keys(runs).sort()).toEqual([`pending:${t.id}`, t.id].sort());
      expect(runs[t.id]).toMatchObject({ profileName: 'Everyday', session: { serviceId: id2 } });
      expect(runs[`pending:${t.id}`]).toMatchObject({ profileName: 'Fresh', state: 'starting' });
    });
    // The old session ended in the backend before the new one took its name, so its 'stopped' always precedes the new
    // session's first event; only the start reply can overtake them.
    const oldStopped = (t: ReturnType<typeof setup>) => session(t, 'stopped', { service_id: null, reason: 'device_removed' });
    const newRunning = (t: ReturnType<typeof setup>) => session(t, 'running', { serial_number: 'SER-2' });
    const expectFresh = (t: ReturnType<typeof setup>) => {
      const runs = t.runner.getState().runs;
      expect(Object.keys(runs)).toEqual([t.id]);
      expect(runs[t.id]).toMatchObject({ serviceId: t.id, profileName: 'Fresh', state: 'running', counters: { events: 1 }, session: { serviceId: t.id, phase: 'running', serialNumber: 'SER-2' } });
      expect(runs[t.id].entries.map(entry => entry.text)).not.toContain('Paused: the module was unplugged');
      expect(t.runner.isActivePath('aabb')).toBe(true);
    };
    // `awaiting` is private; a stale entry has no other observable effect until a later start reuses the key.
    const awaitingSize = (t: ReturnType<typeof setup>) => (t.runner as unknown as { awaiting: Map<string, string> }).awaiting.size;
    // 'status' is the new process's 'starting' then 'running', which run_service reports inside the backend's start,
    // before the new session's first state event.
    for (const [name, order] of [
      ['reply, old stopped, new first event', ['reply', 'stopped', 'record', 'first']],
      ['old stopped, reply, new first event', ['stopped', 'reply', 'record', 'first']],
      ['old stopped, new first event, reply', ['stopped', 'record', 'first', 'reply']],
      ['reply, old stopped, status, new first event', ['reply', 'stopped', 'status', 'record', 'first']],
      ['old stopped, reply, status, new first event', ['stopped', 'reply', 'status', 'record', 'first']],
      ['old stopped, status, reply, new first event', ['stopped', 'status', 'reply', 'record', 'first']],
      ['old stopped, status, new first event, reply', ['stopped', 'status', 'record', 'first', 'reply']],
    ] as const) {
      it(`adopts the pending run, never the old one, when events arrive as: ${name}`, async () => {
        const t = await moved();
        let reply = () => {};
        const replied = new Promise<void>(resolve => { reply = resolve; });
        t.results.start_service = async () => { await replied; return t.id; };
        const starting = t.runner.start(fresh);
        await tick();
        // A reply after the old run's 'stopped' replaces the old run at once; before it, the old run stays until adoption.
        let stopped = false, replaced = false;
        for (const step of order) {
          if (step === 'reply') { replaced = stopped; reply(); expect(await starting).toBe(true); }
          if (step === 'stopped') { stopped = true; oldStopped(t); }
          if (step === 'status') {
            t.status('starting'); t.status('running');
            // The status lands in the new run (still `pending:` unless the reply already replaced the old run), never
            // in the old one.
            const runs = Object.values(t.runner.getState().runs);
            expect(runs.find(r => r.profileName === 'Fresh')).toMatchObject({ state: 'running' });
            const old = runs.filter(r => r.profileName === 'Everyday');
            expect(old).toHaveLength(replaced ? 0 : 1);
            if (!replaced) expect(old[0].state).toBe('exited');
          }
          // The new process's first record, before its session event.
          if (step === 'record') { t.record({ type: 'event', status: 'unbound', event, action: null }); t.flush(); }
          if (step === 'first') newRunning(t);
        }
        expectFresh(t);
        expect(awaitingSize(t)).toBe(0);
        // The old run's 'stopped', had it been late, would not end the adopted run either.
        await t.runner.stop(t.id);
        expect(t.calls.at(-1)).toEqual({ command: 'stop_service', args: { serviceId: t.id } });
      });
    }
    it('an executing session adopts the pending run, not an old run switched to another profile', async () => {
      const t = await moved();
      // A focused-app rule had switched the old session to generation 2.
      session(t, 'running', { service_id: id2, profile_generation: 2, profile_id: 'p-browser', profile_name: 'Browser' });
      expect(run(t)).toMatchObject({ profileName: 'Browser', profileGeneration: 2 });
      let reply = () => {};
      const replied = new Promise<void>(resolve => { reply = resolve; });
      t.results.start_service = async () => { await replied; return t.id; };
      const starting = t.runner.start({ ...fresh, execute: true }, async () => true);
      await tick();
      const newState = (phase: string, extra: Record<string, unknown> = {}) => session(t, phase, { serial_number: 'SER-2', execute: true, profile_generation: 0, profile_id: null, profile_name: 'Fresh', ...extra });
      session(t, 'stopped', { service_id: null, reason: 'device_removed', profile_generation: 2, profile_id: 'p-browser', profile_name: 'Browser' });
      t.status('starting', { execute: true }); t.status('running', { execute: true });
      // The old run is untouched by the new process's status.
      expect(run(t)).toMatchObject({ profileName: 'Browser', profileGeneration: 2, state: 'exited', session: { phase: 'stopped' } });
      newState('running');
      reply(); expect(await starting).toBe(true);
      // The Mac slept and woke: the executing session waits for confirmation.
      newState('suspended', { service_id: null, reason: 'sleep' });
      newState('needs_confirmation', { service_id: null, reason: 'sleep' });
      const runs = t.runner.getState().runs;
      expect(Object.keys(runs)).toEqual([t.id]);
      expect(runs[t.id]).toMatchObject({ profileName: 'Fresh', profileGeneration: 0, execute: true, session: { phase: 'needs_confirmation', profileGeneration: 0 } });
      expect(runs[t.id].profileSwitched).toBeFalsy();
      expect(runs[t.id].entries.map(entry => entry.text)).not.toContain('Paused: the module was unplugged');
      expect(t.runner.isActivePath('aabb')).toBe(false);
      // Resume asks once, for the profile shown, and names generation 0.
      const shown: string[] = [];
      expect(await t.runner.resumeSession(t.id, async () => { shown.push(run(t).profileName); return true; })).toBe(true);
      expect(shown).toEqual(['Fresh']);
      expect(t.calls.filter(call => call.command === 'resume_session')).toEqual([{ command: 'resume_session', args: { sessionId: t.id, profileGeneration: 0 } }]);
    });
    it('old non-stopped events never take the path\'s routing from the pending run', async () => {
      const t = await moved();
      expect(await t.runner.start(fresh)).toBe(true);
      // The old session keeps reporting from elsewhere while the pending run waits.
      session(t, 'reconnecting', { service_id: null, reason: 'device_removed', attempt: 2 });
      session(t, 'running', { service_id: id2 });
      t.status('starting'); t.status('running');
      t.record({ type: 'event', status: 'unbound', event, action: null }); t.flush();
      expect(t.runner.getState().runs[`pending:${t.id}`]).toMatchObject({ profileName: 'Fresh', state: 'running', counters: { events: 1 } });
      expect(run(t)).toMatchObject({ profileName: 'Everyday', counters: { events: 0 }, session: { serviceId: id2 } });
      oldStopped(t);
      newRunning(t);
      expectFresh(t);
    });
    it('a reply after the new session already adopted the pending run awaits nothing', async () => {
      const t = await moved();
      const device = { path_hex: 'aabb', module: 'm131e' as const, serial_number: 'SER-2' };
      let reply = () => {};
      const replied = new Promise<void>(resolve => { reply = resolve; });
      t.results.start_service = async () => { await replied; return t.id; };
      const starting = t.runner.start({ ...fresh, device });
      await tick();
      oldStopped(t);
      // Asleep: no process, matched by module and serial.
      session(t, 'suspended', { service_id: null, reason: 'sleep', serial_number: 'SER-2' });
      expect(Object.keys(t.runner.getState().runs)).toEqual([t.id]);
      reply(); expect(await starting).toBe(true);
      expect(awaitingSize(t)).toBe(0);
      expect(Object.keys(t.runner.getState().runs)).toEqual([t.id]);
      expect(run(t)).toMatchObject({ profileName: 'Fresh', session: { phase: 'suspended', serviceId: null } });
    });
    it('a pending run whose start failed never receives the old session\'s events', async () => {
      const t = await moved();
      t.failures.start_service = 'Another reader owns this device.';
      expect(await t.runner.start(fresh)).toBe(false);
      const pending = () => t.runner.getState().runs[`pending:${t.id}`];
      expect(pending()).toMatchObject({ profileName: 'Fresh', state: 'failed' });
      session(t, 'reconnecting', { service_id: null, reason: 'device_removed', attempt: 1 });
      t.status('running');
      session(t, 'running', { service_id: t.id });
      expect(pending()).toMatchObject({ profileName: 'Fresh', state: 'failed', counters: { events: 0 } });
      expect(pending().session).toBeUndefined();
      expect(run(t)).toMatchObject({ profileName: 'Everyday', state: 'running', session: { phase: 'running', serviceId: t.id } });
      session(t, 'stopped', { service_id: null, reason: 'user' });
      delete t.failures.start_service;
      expect(await t.runner.start(fresh)).toBe(true);
      expect(run(t)).toMatchObject({ profileName: 'Fresh', state: 'starting' });
      expect(pending()).toMatchObject({ state: 'failed' });
    });
    it('a pending run stopped before its session reports is adopted, stays stopping, then exits', async () => {
      const t = await moved();
      expect(await t.runner.start(fresh)).toBe(true);
      await t.runner.stop(`pending:${t.id}`);
      expect(t.calls.at(-1)).toEqual({ command: 'stop_service', args: { serviceId: t.id } });
      expect(t.runner.getState().runs[`pending:${t.id}`]).toMatchObject({ state: 'stopping' });
      oldStopped(t);
      newRunning(t);
      expect(Object.keys(t.runner.getState().runs)).toEqual([t.id]);
      expect(run(t)).toMatchObject({ profileName: 'Fresh', state: 'stopping', session: { phase: 'running', serviceId: t.id } });
      session(t, 'stopped', { service_id: null, reason: 'user', serial_number: 'SER-2' });
      expect(run(t)).toMatchObject({ profileName: 'Fresh', state: 'exited', session: { phase: 'stopped' } });
      expect(t.runner.isActivePath('aabb')).toBe(false);
      expect(await t.runner.start(fresh)).toBe(true);
    });
    it('a late old stopped event does not end a run adopted on its own process', async () => {
      const t = await moved();
      expect(await t.runner.start(fresh)).toBe(true);
      // Defensive: the new session's first event before the old 'stopped'.
      newRunning(t);
      expect(t.runner.getState().runs[t.id]).toMatchObject({ profileName: 'Fresh', session: { serviceId: t.id, phase: 'running' } });
      oldStopped(t);
      expect(Object.keys(t.runner.getState().runs)).toEqual([t.id]);
      expect(t.runner.getState().runs[t.id]).toMatchObject({ profileName: 'Fresh', state: 'running', session: { phase: 'running' } });
      // The adopted session's own stop still ends it.
      session(t, 'stopped', { service_id: null, reason: 'user', serial_number: 'SER-2' });
      expect(t.runner.getState().runs[t.id]).toMatchObject({ state: 'exited', session: { phase: 'stopped' } });
    });
    it('old non-stopped events after the reply stay on the old run', async () => {
      const t = await moved();
      expect(await t.runner.start(fresh)).toBe(true);
      session(t, 'waiting_for_device', { service_id: null, reason: 'device_removed', attempt: 1 });
      expect(t.runner.getState().runs[`pending:${t.id}`]).toMatchObject({ profileName: 'Fresh', state: 'starting' });
      expect(t.runner.getState().runs[t.id]).toMatchObject({ profileName: 'Everyday', session: { phase: 'waiting_for_device' } });
      oldStopped(t);
      session(t, 'suspended', { service_id: null, reason: 'sleep', serial_number: 'SER-2' });
      expect(Object.keys(t.runner.getState().runs)).toEqual([t.id]);
      expect(t.runner.getState().runs[t.id]).toMatchObject({ profileName: 'Fresh', session: { phase: 'suspended', serialNumber: 'SER-2' } });
    });
  });
  it('a new start after a stopped session begins a clean run', async () => {
    const t = await started(); session(t, 'stopped', { service_id: null, reason: 'user' });
    expect(await t.runner.start(options)).toBe(true);
    expect(run(t).session).toBeUndefined();
    expect(run(t).state).toBe('starting');
  });
});

describe('applying a saved profile', () => {
  const saved: Profile = { ...profile, name: 'Everyday', bindings: [{ module: 'm003k', control: 1, gesture: 'pressed', action: { type: 'toggle_mute' } }] };
  const applied = (sessions: unknown): Record<string, () => unknown> => ({
    apply_saved_profile: () => ({ id: 'saved-1', profile: saved, modified_at_ms: 2, sessions }),
  });
  // A run started from the saved profile, its session live.
  const emit = (t: ReturnType<typeof setup>, phase: string, extra: Record<string, unknown> = {}) =>
    t.handlers['session://state']({ payload: { session_id: t.id, service_id: t.id, phase, reason: null, attempt: 0, execute: false, module: 'm131e', serial_number: 'SER12345', ...extra } });
  const runningSaved = async (execute = false) => {
    const t = setup(); t.runner.connect(); await tick();
    await t.runner.start({ ...options, profileId: 'saved-1', execute }, async () => true);
    t.status('running', { execute }); emit(t, 'running', { execute });
    return t;
  };
  const run = (t: ReturnType<typeof setup>) => t.runner.getState().runs[t.id];
  const savedSession = (t: ReturnType<typeof setup>, phase: string, extra: Record<string, unknown> = {}) =>
    emit(t, phase, { reason: 'profile_saved', profile_generation: 3, profile_id: 'saved-1', profile_name: 'Everyday', ...extra });

  it('resyncs the runs the save applied to and reports what happened', async () => {
    const t = await runningSaved();
    Object.assign(t.results, applied([{ session_id: t.id, profile_generation: 3, paused: false }]));
    expect(await t.runner.applySavedProfile('saved-1')).toEqual({ ok: true, switched: 1, paused: 0, refused: [] });
    expect(t.calls.at(-1)).toEqual({ command: 'apply_saved_profile', args: { profileId: 'saved-1' } });
    expect(run(t)).toMatchObject({
      profileName: 'Everyday', profileId: 'saved-1', profileGeneration: 3, profileSwitched: false, draftUnsaved: false,
      snapshot: stableStringify(saved),
    });
    expect(run(t).entries.at(-1)!.text).toContain('Saved profile applied');
    // The matching session event changes nothing further: the run holds the
    // saved body, not an unknown switch snapshot, and stays unswitched.
    savedSession(t, 'running');
    expect(run(t)).toMatchObject({ snapshot: stableStringify(saved), profileSwitched: false });
  });

  it('reports a paused executing session', async () => {
    const t = await runningSaved(true);
    Object.assign(t.results, applied([{ session_id: t.id, profile_generation: 3, paused: true }]));
    expect(await t.runner.applySavedProfile('saved-1')).toEqual({ ok: true, switched: 1, paused: 1, refused: [] });
    savedSession(t, 'needs_confirmation');
    expect(run(t).session).toMatchObject({ phase: 'needs_confirmation', reason: 'profile_saved' });
  });

  it('keeps a refused session unchanged and carries the reason', async () => {
    const t = await runningSaved(true);
    const before = run(t);
    Object.assign(t.results, applied([{ session_id: t.id, profile_generation: 0, paused: false, error: 'dry-run only' }]));
    expect(await t.runner.applySavedProfile('saved-1')).toEqual({ ok: true, switched: 0, paused: 0, refused: ['dry-run only'] });
    expect(run(t)).toEqual(before);
    expect(t.runner.getState().error).toBeNull();
  });

  it('shows a command failure in the run panel and returns it', async () => {
    const t = await runningSaved();
    t.failures['apply_saved_profile'] = 'the saved profile is gone';
    expect(await t.runner.applySavedProfile('saved-1')).toEqual({ ok: false, error: 'the saved profile is gone' });
    expect(t.runner.getState().error).toContain('the saved profile is gone');
  });

  it('falls back to the unknown snapshot when an event names a generation the runner missed', async () => {
    const t = await runningSaved();
    savedSession(t, 'running');
    expect(run(t)).toMatchObject({ snapshot: unknownSnapshot, profileId: 'saved-1', profileSwitched: false, profileName: 'Everyday' });
    // A later switch by a rule still names itself as before.
    emit(t, 'running', { reason: 'profile_switch', profile_generation: 4, profile_id: 'other', profile_name: 'Other' });
    expect(run(t)).toMatchObject({ snapshot: unknownSnapshot, profileSwitched: true, profileSwitchedBy: 'rule' });
  });

  it('names what is stale: unsaved changes for a saved run, a restart for an unsaved-draft run', () => {
    const draft: Profile = { ...profile, name: 'Everyday' };
    const appliedRun = { serviceId: 's', state: 'running' as const, execute: false, entries: [], counters: emptyCounters(), startedAt: 0, profileName: 'Everyday', snapshot: stableStringify(saved), draftUnsaved: false, profileId: 'saved-1' };
    expect(staleNote(appliedRun, draft)).toBe('Unsaved changes are not applied to the running profile yet. Save to apply them.');
    expect(staleNote(appliedRun, saved)).toBeNull();
    const draftRun = { ...appliedRun, draftUnsaved: true, snapshot: stableStringify(profile) };
    expect(staleNote(draftRun, { ...profile, name: 'Edited' })).toBe('Profile changed since start. Restart to apply.');
    expect(staleNote(undefined, draft)).toBeNull();
  });
});
