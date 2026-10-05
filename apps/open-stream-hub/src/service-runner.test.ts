import { describe, expect, it } from 'vitest';
import type { Profile } from './contracts';
import type { Scheduler } from './device-feed';
import { isStale, maxEntries, ServiceRunner, serviceIdFor, summarizeRecord } from './service-runner';

const profile: Profile = { schema_version: 1, name: 'Everyday', bindings: [{ module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } }] };
const event = { module: 'm003k', control: 0, gesture: 'pressed' };

function setup(desktop = true) {
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const unlistened: string[] = [];
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const queue: (() => void)[] = [];
  const failures: Record<string, string> = {};
  const invoke = (async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (failures[command]) throw new Error(failures[command]);
    return command === 'start_service' ? serviceIdFor((args!.request as { path_hex: string }).path_hex) : undefined;
  }) as never;
  const listen = async (name: string, handler: (event: { payload: unknown }) => void) => { handlers[name] = handler; return () => { unlistened.push(name); }; };
  const schedule: Scheduler = callback => { queue.push(callback); };
  const runner = new ServiceRunner(invoke, listen, desktop, schedule, () => 1000);
  const flush = () => queue.splice(0).forEach(callback => callback());
  const id = serviceIdFor('AABB');
  const record = (value: unknown) => handlers['service://record']({ payload: { service_id: id, record: value } });
  const status = (state: string, extra = {}) => handlers['service://status']({ payload: { service_id: id, state, execute: false, ...extra } });
  return { runner, handlers, unlistened, calls, flush, failures, id, record, status };
}
const tick = () => new Promise(resolve => setTimeout(resolve, 0));
const options = { profile, pathHex: 'AABB', execute: false, softwareMode: false, draftUnsaved: false };

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
    expect(run.counters).toEqual({ events: 2, planned: 1, executed: 1, failed: 1 });
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
    expect(t.unlistened.sort()).toEqual(['service://record', 'service://status']);
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
