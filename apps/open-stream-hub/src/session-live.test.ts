import { describe, expect, it } from 'vitest';
import sessionEvents from '../fixtures/session-state-events.json';
import profileActionEvents from '../fixtures/session-profile-action-events.json';
import { DeviceFeed } from './device-feed';
import type { Scheduler } from './device-feed';
import { reduceLive, rotationStep, sessionInput } from './live-state';
import type { LiveState } from './live-state';
import { ServiceRunner, serviceIdFor } from './service-runner';

// A service `event` record as the service writes it (crates/cooltek-sdk-cli/src/service_events.rs): the event carries no module.
const eventRecord = (control: number, gesture: string, extra: Record<string, unknown> = {}) =>
  ({ type: 'event', schema_version: 1, event: { control, gesture, timestamp_micros: 14550316 }, binding: null, action: null, navigation: null, page_id: null, status: 'unbound', ...extra });
const apply = (records: unknown[], module: 'm131e' | 'm4315', state: LiveState = {}) =>
  records.map(record => sessionInput(record, module, 'S1', 'service:aabb')).reduce(reduceLive, state);

describe('sessionInput', () => {
  it('maps the hardware example to the feed payload on the session module', () => {
    const record = { type: 'event', event: { control: 0, gesture: 'pressed', timestamp_micros: 14550316 }, action: null, status: 'unbound' };
    expect(sessionInput(record, 'm4315', 'S1', 'service:aabb')).toEqual({ feed_id: 'service:aabb', module: 'm4315', serial_number: 'S1', event: { control: 0, gesture: 'pressed', timestamp_micros: 14550316 } });
  });
  it('turns the dial both ways (right is clockwise) and shows its press', () => {
    const turned = apply([eventRecord(0, 'right'), eventRecord(0, 'right'), eventRecord(0, 'left')], 'm131e');
    expect(turned.m131e?.angles[0]).toBe(rotationStep);
    expect(apply([eventRecord(0, 'left')], 'm131e').m131e?.angles[0]).toBe(-rotationStep);
    const pressed = apply([eventRecord(0, 'pressed')], 'm131e', turned);
    expect(pressed.m131e?.keysDown).toEqual([0]);
    expect(apply([eventRecord(0, 'released')], 'm131e', pressed).m131e?.keysDown).toEqual([]);
  });
  it('turns the needle by every merged turn, with no cap', () => {
    expect(apply([eventRecord(0, 'left', { coalesced_event_count: 3 })], 'm131e').m131e?.angles[0]).toBe(-3 * rotationStep);
    expect(apply([eventRecord(0, 'right', { coalesced_event_count: 30 })], 'm131e').m131e?.angles[0]).toBe(450);
    // The angle stays cumulative: a later turn continues from 450, never folded back into 0..360.
    expect(apply([eventRecord(0, 'right', { coalesced_event_count: 30 }), eventRecord(0, 'right')], 'm131e').m131e?.angles[0]).toBe(465);
    // A malformed count counts as one turn.
    for (const count of [0, -2, 1.5, '3', null]) expect(apply([eventRecord(0, 'right', { coalesced_event_count: count })], 'm131e').m131e?.angles[0]).toBe(rotationStep);
    // A press never merges; a stray count does not repeat it.
    expect(apply([eventRecord(0, 'pressed', { coalesced_event_count: 3 })], 'm131e').m131e?.keysDown).toEqual([0]);
  });
  it('advances a roller pulse by every merged step', () => {
    const rolled = apply([eventRecord(1, 'up', { coalesced_event_count: 4 })], 'm032le' as never);
    expect(rolled.m032le?.pulses[1]).toEqual({ direction: 'up', tick: 4 });
    expect(apply([eventRecord(1, 'down')], 'm032le' as never, rolled).m032le?.pulses[1]).toEqual({ direction: 'down', tick: 5 });
  });
  it('presses and releases several keypad keys, folder navigation included', () => {
    const navigation = { status: 'navigation', navigation: { kind: 'open', page_id: 'tools' } };
    const down = apply([eventRecord(0, 'pressed', navigation), eventRecord(14, 'pressed', { status: 'planned', action: { type: 'volume', delta: 7 } }), eventRecord(5, 'pressed')], 'm4315');
    expect(down.m4315?.keysDown).toEqual([0, 5, 14]);
    const up = apply([eventRecord(0, 'released', navigation), eventRecord(14, 'released')], 'm4315', down);
    expect(up.m4315?.keysDown).toEqual([5]);
    expect(up.m131e).toBeUndefined();
  });
  it('ignores other record types, malformed events, host gestures and out-of-range controls', () => {
    const ignored = [
      profileActionEvents.record.record, { type: 'action_executed', event: { control: 0, gesture: 'pressed', timestamp_micros: 1 } },
      { type: 'event' }, { type: 'event', event: null }, { type: 'event', event: { control: '0', gesture: 'pressed', timestamp_micros: 1 } },
      { type: 'event', event: { control: 0, gesture: 'pressed' } }, null, 'event', [],
      eventRecord(0, 'short_press'), eventRecord(0, 'long_press'), eventRecord(0, 'short_press', { status: 'dial_function' }), eventRecord(15, 'pressed'),
    ];
    for (const record of ignored) expect(apply([record], 'm4315')).toEqual({});
    expect(apply([eventRecord(1, 'pressed')], 'm131e')).toEqual({});
  });
});

function setup() {
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  const queue: (() => void)[] = [];
  const listen = async (name: string, handler: (event: { payload: unknown }) => void) => { handlers[name] = handler; return () => {}; };
  const schedule: Scheduler = callback => { queue.push(callback); };
  const feedCalls: string[] = [];
  const feed = new DeviceFeed((async (command: string) => { feedCalls.push(command); return command === 'list_live_devices' ? [] : 'feed'; }) as never, listen, true, schedule);
  // A start_service reply instead of the path's service id, when set.
  const replies: { start?: () => Promise<string> } = {};
  const runner = new ServiceRunner((async (command: string, args?: Record<string, unknown>) => command === 'start_service' ? (replies.start ? replies.start() : serviceIdFor((args!.request as { path_hex: string }).path_hex)) : undefined) as never, listen, true, schedule, () => 1000);
  feed.setBlocker(runner.isActivePath);
  runner.setLiveSink(feed);
  feed.connect(); runner.connect();
  const flush = () => queue.splice(0).forEach(callback => callback());
  const session = sessionEvents[0];
  const id = session.session_id;
  const record = (value: unknown, serviceId = id) => { handlers['service://record']({ payload: { service_id: serviceId, record: value } }); flush(); };
  const state = (patch: Record<string, unknown> = {}) => { handlers['session://state']({ payload: { ...session, ...patch } }); flush(); };
  const status = (value: string, serviceId = serviceIdFor('aabb')) => { handlers['service://status']({ payload: { service_id: serviceId, state: value, execute: false } }); flush(); };
  const feedInput = (module: string, control: number, gesture: string) => { handlers['device://input']({ payload: { feed_id: 'feed', module, serial_number: null, event: { control, gesture, timestamp_micros: 1 } } }); flush(); };
  const live = () => feed.getState().live;
  return { feed, runner, feedCalls, record, state, status, feedInput, live, id, flush, replies };
}
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

describe('session input in the live preview', () => {
  it('drives only the module the session runs on', async () => {
    const t = setup(); await tick();
    t.state();
    expect(t.feed.getState().sessionModules).toEqual(['m131e']);
    t.record(eventRecord(0, 'right'));
    t.record(eventRecord(0, 'pressed'));
    expect(t.live().m131e).toMatchObject({ angles: { 0: rotationStep }, keysDown: [0] });
    expect(t.live().m4315).toBeUndefined();
    // Records that are not input never touch the preview.
    t.record(profileActionEvents.record.record);
    t.record({ type: 'service_start', device: { module: 'm4315' } });
    expect(Object.keys(t.live())).toEqual(['m131e']);
  });
  it('ignores records of a process the session moved on from, and of a run that is not reading', async () => {
    const t = setup(); await tick();
    t.state({ session_id: 'service:aa', service_id: 'service:aa' });
    t.record(eventRecord(0, 'pressed'), 'service:aa');
    t.record(eventRecord(0, 'released'), 'service:aa');
    // The session reconnects on another path: its old process still routes to the run, but its late input is dropped.
    t.state({ session_id: 'service:aa', service_id: 'service:cc' });
    t.record(eventRecord(0, 'pressed'), 'service:aa');
    expect(t.live().m131e?.keysDown ?? []).toEqual([]);
    t.record(eventRecord(0, 'pressed'), 'service:cc');
    expect(t.live().m131e?.keysDown).toEqual([0]);
    t.record(eventRecord(0, 'released'), 'service:cc');
    t.state({ session_id: 'service:aa', service_id: 'service:cc', phase: 'suspended', reason: 'sleep' });
    t.record(eventRecord(0, 'pressed'), 'service:cc');
    expect(t.live().m131e).toBeUndefined();
  });
  it('starts over when the session starts, pauses, stops or fails, so no key stays pressed', async () => {
    const t = setup(); await tick();
    t.state();
    t.record(eventRecord(0, 'pressed'));
    expect(t.live().m131e?.keysDown).toEqual([0]);
    t.state({ phase: 'suspended', reason: 'sleep' });
    expect(t.live().m131e).toBeUndefined();
    expect(t.feed.getState().sessionModules).toEqual([]);
    t.state();
    t.record(eventRecord(0, 'pressed'));
    t.state({ phase: 'stopped', reason: 'user' });
    expect(t.live().m131e).toBeUndefined();
    // A late record after the stop is dropped.
    t.record(eventRecord(0, 'pressed'));
    expect(t.live().m131e).toBeUndefined();
  });
  it('clears an unmanaged run on Stop and on failure', async () => {
    const t = setup(); await tick();
    await t.runner.start({ profile: { schema_version: 2, name: 'Keys', bindings: [] }, pathHex: 'aabb', execute: false, softwareMode: true, draftUnsaved: false, device: { path_hex: 'aabb', module: 'm4315', serial_number: 'K1' } });
    const id = serviceIdFor('aabb');
    expect(t.feed.getState().sessionModules).toEqual(['m4315']);
    const record = (value: unknown) => t.record(value, id);
    record(eventRecord(3, 'pressed'));
    expect(t.live().m4315?.keysDown).toEqual([3]);
    void t.runner.stop(id);
    expect(t.live().m4315).toBeUndefined();
    expect(t.feed.getState().sessionModules).toEqual([]);
    // A fresh run that ends with an error.
    t.status('exited');
    await t.runner.start({ profile: { schema_version: 2, name: 'Keys', bindings: [] }, pathHex: 'aabb', execute: false, softwareMode: true, draftUnsaved: false, device: { path_hex: 'aabb', module: 'm4315', serial_number: 'K1' } });
    record(eventRecord(7, 'pressed'));
    expect(t.live().m4315?.keysDown).toEqual([7]);
    t.status('failed', id);
    expect(t.live().m4315).toBeUndefined();
  });
  it('keeps a held key when the run moves to the session id start_service returns', async () => {
    const t = setup(); await tick();
    let reply!: (id: string) => void;
    t.replies.start = () => new Promise(resolve => { reply = resolve; });
    const started = t.runner.start({ profile: { schema_version: 2, name: 'Keys', bindings: [] }, pathHex: 'aabb', execute: false, softwareMode: true, draftUnsaved: false, device: { path_hex: 'aabb', module: 'm4315', serial_number: 'K1' } });
    await tick();
    t.record(eventRecord(3, 'pressed'), serviceIdFor('aabb'));
    expect(t.live().m4315?.keysDown).toEqual([3]);
    reply('session:0b8f2c1e-uuid');
    await started;
    expect(Object.keys(t.runner.getState().runs)).toEqual(['session:0b8f2c1e-uuid']);
    expect(t.live().m4315?.keysDown).toEqual([3]);
  });
  it('stopping one of two runs on the same module type keeps the other run\'s keys', async () => {
    const t = setup(); await tick();
    const start = (path: string, serial: string) => t.runner.start({ profile: { schema_version: 2, name: 'Keys', bindings: [] }, pathHex: path, execute: false, softwareMode: true, draftUnsaved: false, device: { path_hex: path, module: 'm4315', serial_number: serial } });
    await start('aa', 'A1'); await start('bb', 'B1');
    t.record(eventRecord(5, 'pressed'), serviceIdFor('bb'));
    expect(t.live().m4315?.keysDown).toEqual([5]);
    void t.runner.stop(serviceIdFor('aa'));
    t.status('exited', serviceIdFor('aa'));
    expect(t.live().m4315?.keysDown).toEqual([5]);
    expect(t.feed.getState().sessionModules).toEqual(['m4315']);
    void t.runner.stop(serviceIdFor('bb'));
    expect(t.live().m4315).toBeUndefined();
  });
  it('clears a held key when the session process exits, before any session event', async () => {
    const t = setup(); await tick();
    t.state();
    t.record(eventRecord(0, 'pressed'));
    expect(t.live().m131e?.keysDown).toEqual([0]);
    t.status('exited', t.id);
    expect(t.runner.getState().runs[t.id].session?.phase).toBe('running');
    expect(t.live().m131e).toBeUndefined();
    t.record(eventRecord(0, 'pressed'));
    expect(t.live().m131e).toBeUndefined();
  });
  it('keeps session input when a feed on another module resets, and feed input on other modules keeps working', async () => {
    const t = setup(); await tick();
    t.state();
    t.record(eventRecord(0, 'right'));
    t.feedInput('m003k', 1, 'clockwise');
    expect(t.live().m003k?.angles[1]).toBe(rotationStep);
    await t.feed.startDemo('knobs');
    expect(t.live().m131e?.angles[0]).toBe(rotationStep);
    expect(t.live().m003k).toBeUndefined();
  });
  it('a running session still blocks the feed for its own path', async () => {
    const t = setup(); await tick();
    await t.runner.start({ profile: { schema_version: 1, name: 'Dial', bindings: [] }, pathHex: 'aabb', execute: false, softwareMode: true, draftUnsaved: false, device: { path_hex: 'aabb', module: 'm131e', serial_number: 'S1' } });
    expect(t.runner.isActivePath('aabb')).toBe(true);
    await t.feed.startLive('aabb');
    expect(t.feedCalls).not.toContain('start_feed');
    expect(t.feed.getState().error).toContain('Stop the running profile');
  });
});
