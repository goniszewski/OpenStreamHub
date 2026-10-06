import { describe, expect, it } from 'vitest';
import switchResult from '../fixtures/focus-switch-result.json';
import focusArgs from '../fixtures/focus-command-args.json';
import focusEvents from '../fixtures/focus-changed-events.json';
import focusResults from '../fixtures/focus-command-results.json';
import switchEvents from '../fixtures/session-switch-events.json';
import type { FocusChangedPayload, FocusCurrentApp, FocusRuleInput, FocusRules, Profile, SessionStatePayload, SwitchedProfile } from './contracts';
import { bundleIdIssue, FocusRulesStore, focusChangedEvent, focusDebounceMs, maxBundleIdLength, sessionsToSwitch } from './focus-rules';
import { emptyCounters, ServiceRunner } from './service-runner';
import type { ServiceRun } from './service-runner';
import { noticesFromRuns } from './notices';

const profileA = '00000000-0000-4000-8000-00000000000a';
const profileB = '00000000-0000-4000-8000-00000000000b';
const tick = () => new Promise(resolve => setTimeout(resolve, 0));

function run(extra: Partial<ServiceRun> = {}, phase: SessionStatePayload['phase'] | null = 'running'): ServiceRun {
  return {
    serviceId: 's', state: 'running', execute: false, entries: [], counters: emptyCounters(), startedAt: 0, profileName: 'A', snapshot: '{}', draftUnsaved: false, profileId: profileA,
    ...(phase ? { session: { serviceId: 's', phase, reason: null, attempt: 0, module: 'm131e', serialNumber: 'S' } } : {}), ...extra,
  };
}

describe('focused-app rule contract fixtures', () => {
  it('command arguments match the declared input shapes', () => {
    const add: FocusRuleInput = focusArgs.save_focus_rule.rule;
    const edit: FocusRuleInput = focusArgs.save_focus_rule_edit.rule;
    expect(add.id).toBeNull();
    expect(edit.id).toBe(focusArgs.delete_focus_rule.id);
    expect(focusArgs.set_focus_rules_enabled).toEqual({ enabled: true });
    expect(Object.keys(focusArgs.switch_session_profile).sort()).toEqual(['profileId', 'sessionId']);
  });
  it('results and events carry exactly the fields the frontend reads', () => {
    const rules: FocusRules = focusResults.list_focus_rules;
    expect(Object.keys(rules).sort()).toEqual(['enabled', 'rules']);
    expect(Object.keys(rules.rules[0]).sort()).toEqual(['app_bundle_id', 'id', 'profile_id']);
    expect(focusResults.list_focus_rules_default).toEqual({ enabled: false, rules: [] });
    const current: FocusCurrentApp = focusResults.focus_current_app;
    expect(Object.keys(current).sort()).toEqual(['bundle_id', 'supported']);
    const events: FocusChangedPayload[] = focusEvents;
    expect(events.map(event => Object.keys(event).sort())).toEqual(Array(3).fill(['bundle_id', 'profile_id', 'rule_id']));
    expect(events[0].profile_id).toBe(rules.rules[0].profile_id);
    expect(events.slice(1).every(event => event.profile_id === null && event.rule_id === null)).toBe(true);
  });
  it('a switched executing session pauses with the profile_switch reason', () => {
    const states = switchEvents as SessionStatePayload[];
    expect(states.map(state => [state.phase, state.reason])).toEqual([['running', null], ['running', 'profile_switch'], ['needs_confirmation', 'profile_switch']]);
    expect(states.map(state => state.profile_generation)).toEqual([0, 1, 1]);
  });
  it('a switch returns the saved profile and the new profile generation', () => {
    const switched: SwitchedProfile = switchResult.switch_session_profile as SwitchedProfile;
    expect(Object.keys(switched).sort()).toEqual(['id', 'modified_at_ms', 'profile', 'profile_generation']);
    expect(switched.profile_generation).toBe(1);
  });
});

describe('bundle id feedback', () => {
  it('mirrors the Rust rules', () => {
    expect(bundleIdIssue(' com.apple.Safari ')).toBeNull();
    expect(bundleIdIssue('org.mozilla.firefox_beta-1')).toBeNull();
    for (const bad of ['', '  ', '.com.apple', '-x', 'com apple', 'com/apple', 'ü.app']) expect(bundleIdIssue(bad), bad).not.toBeNull();
    expect(bundleIdIssue('a'.repeat(maxBundleIdLength))).toBeNull();
    expect(bundleIdIssue('a'.repeat(maxBundleIdLength + 1))).toContain('at most');
  });
});

describe('sessionsToSwitch', () => {
  it('picks live sessions that are not on the matched profile', () => {
    const runs = {
      live: run(), other: run({ profileId: profileB }), unknown: run({ profileId: null }),
      stopped: run({}, 'stopped'), unmanaged: run({}, null), paused: run({ execute: true }, 'needs_confirmation'),
    };
    expect(sessionsToSwitch(runs, profileB).sort()).toEqual(['live', 'paused', 'unknown']);
    expect(sessionsToSwitch(runs, profileA).sort()).toEqual(['other', 'unknown']);
    expect(sessionsToSwitch({}, profileA)).toEqual([]);
  });
});

function setup(desktop = true, results: Record<string, unknown> = {}) {
  const calls: { command: string; args?: Record<string, unknown> }[] = [];
  const failures: Record<string, string> = {};
  const handlers: Record<string, (event: { payload: unknown }) => void> = {};
  let rules: FocusRules = { enabled: false, rules: [] };
  const invoke = (async (command: string, args?: Record<string, unknown>) => {
    calls.push({ command, args });
    if (failures[command]) throw new Error(failures[command]);
    if (command in results) return results[command];
    if (command === 'set_focus_rules_enabled') rules = { ...rules, enabled: args!.enabled as boolean };
    if (command === 'save_focus_rule') { const rule = args!.rule as FocusRuleInput; rules = { ...rules, rules: [...rules.rules, { id: 'r1', app_bundle_id: rule.app_bundle_id, profile_id: rule.profile_id }] }; }
    if (command === 'delete_focus_rule') rules = { ...rules, rules: rules.rules.filter(rule => rule.id !== args!.id) };
    return rules;
  }) as never;
  const listen = async (name: string, handler: (event: { payload: unknown }) => void) => { handlers[name] = handler; return () => { delete handlers[name]; }; };
  const switched: [string, string, string][] = [];
  const runs: Record<string, ServiceRun> = { 'service:aa': run() };
  const runner = { getState: () => ({ runs }), switchProfile: async (id: string, profile: string, why: string) => { switched.push([id, profile, why]); return true; } };
  // Manual timers: elapse() fires the pending debounce, if any.
  const timers = new Map<number, () => void>();
  let nextTimer = 1;
  const store = new FocusRulesStore(invoke, listen, desktop, runner, {
    set: (callback, ms) => { expect(ms).toBe(focusDebounceMs); timers.set(nextTimer, callback); return nextTimer++; },
    clear: handle => { timers.delete(handle as number); },
  });
  const elapse = () => { const due = [...timers.values()]; timers.clear(); due.forEach(callback => callback()); };
  return { store, calls, failures, handlers, switched, runs, timers, elapse };
}

describe('FocusRulesStore', () => {
  it('loads, saves, enables and deletes through the Rust commands', async () => {
    const { store, calls } = setup();
    expect(store.getState().enabled).toBe(false);
    await store.load();
    expect(store.getState()).toMatchObject({ loaded: true, enabled: false, rules: [] });
    expect(await store.save({ id: null, app_bundle_id: '  com.apple.Safari ', profile_id: profileB })).toBe(true);
    expect(calls.at(-1)).toEqual({ command: 'save_focus_rule', args: { rule: { id: null, app_bundle_id: 'com.apple.Safari', profile_id: profileB } } });
    await store.setEnabled(true);
    expect(store.getState()).toMatchObject({ enabled: true, rules: [{ id: 'r1', app_bundle_id: 'com.apple.Safari' }] });
    await store.remove('r1');
    expect(store.getState().rules).toEqual([]);
    expect(calls.map(call => call.command)).toEqual(['list_focus_rules', 'save_focus_rule', 'set_focus_rules_enabled', 'delete_focus_rule']);
  });

  it('keeps the rules and reports errors', async () => {
    const { store, failures } = setup();
    await store.load();
    failures.save_focus_rule = 'profile no longer exists in storage';
    expect(await store.save({ id: null, app_bundle_id: 'com.apple.Safari', profile_id: profileA })).toBe(false);
    expect(store.getState()).toMatchObject({ busy: false, error: 'profile no longer exists in storage', rules: [] });
    store.dismissError();
    expect(store.getState().error).toBeNull();
  });

  it('switches live sessions only when a rule matched', async () => {
    const { store, handlers, switched, elapse } = setup();
    const stop = store.connect();
    await tick();
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Terminal', rule_id: null, profile_id: null } });
    elapse();
    expect(switched).toEqual([]);
    expect(store.getState().focusedApp).toBe('com.apple.Terminal');
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Safari', rule_id: 'r1', profile_id: profileB } });
    expect(switched).toEqual([]);
    elapse();
    expect(switched).toEqual([['service:aa', profileB, 'focused com.apple.Safari']]);
    // Already on the matched profile: nothing to do.
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Mail', rule_id: 'r2', profile_id: profileA } });
    elapse();
    expect(switched).toHaveLength(1);
    handlers[focusChangedEvent]({ payload: null });
    stop();
    expect(handlers[focusChangedEvent]).toBeUndefined();
  });

  it('debounces focus changes and acts only on the latest one', async () => {
    const { store, handlers, switched, timers, elapse } = setup();
    const stop = store.connect();
    await tick();
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Safari', rule_id: 'r1', profile_id: profileB } });
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Terminal', rule_id: 'r2', profile_id: '00000000-0000-4000-8000-00000000000c' } });
    expect(timers.size).toBe(1);
    expect(store.getState().focusedApp).toBe('com.apple.Terminal');
    elapse();
    expect(switched).toEqual([['service:aa', '00000000-0000-4000-8000-00000000000c', 'focused com.apple.Terminal']]);
    // A match followed by an app without a rule switches nothing.
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Safari', rule_id: 'r1', profile_id: profileB } });
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Notes', rule_id: null, profile_id: null } });
    expect(timers.size).toBe(0);
    // Disconnecting cancels a pending switch.
    handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Safari', rule_id: 'r1', profile_id: profileB } });
    stop();
    expect(timers.size).toBe(0);
    expect(switched).toHaveLength(1);
  });

  it('a pending switch does not fire after the rules were disabled, deleted or edited', async () => {
    const { store, handlers, switched, timers, elapse } = setup();
    store.connect();
    await tick();
    await store.load();
    await store.save({ id: null, app_bundle_id: 'com.apple.Safari', profile_id: profileB });
    await store.setEnabled(true);
    const safari = () => handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Safari', rule_id: 'r1', profile_id: profileB } });
    safari();
    void store.setEnabled(false);
    expect(timers.size).toBe(0);
    await tick();
    await store.setEnabled(true);
    safari();
    void store.save({ id: 'r1', app_bundle_id: 'com.apple.Safari', profile_id: profileA });
    expect(timers.size).toBe(0);
    await tick();
    safari();
    void store.remove('r1');
    expect(timers.size).toBe(0);
    elapse();
    expect(switched).toEqual([]);
    // A match that arrived while a delete was in flight is checked against the rules when it fires.
    await store.save({ id: null, app_bundle_id: 'com.apple.Safari', profile_id: profileB });
    const removing = store.remove('r1');
    safari();
    expect(timers.size).toBe(1);
    await removing;
    elapse();
    expect(switched).toEqual([]);
    // Still enabled and still matching: it switches.
    await store.save({ id: null, app_bundle_id: 'com.apple.Safari', profile_id: profileB });
    safari();
    elapse();
    expect(switched).toEqual([['service:aa', profileB, 'focused com.apple.Safari']]);
  });

  it('a switch never fires while a rule edit or disable slower than the debounce is in flight', async () => {
    const { store, handlers, switched, timers, elapse } = setup();
    store.connect();
    await tick();
    await store.load();
    await store.save({ id: null, app_bundle_id: 'com.apple.Safari', profile_id: profileB });
    await store.setEnabled(true);
    const safari = () => handlers[focusChangedEvent]({ payload: { bundle_id: 'com.apple.Safari', rule_id: 'r1', profile_id: profileB } });
    // The edit remaps r1; a focus event matched against the old rules lands while its reply is outstanding.
    const editing = store.save({ id: 'r1', app_bundle_id: 'com.apple.Safari', profile_id: profileA });
    safari();
    elapse();
    expect(switched).toEqual([]);
    safari();
    expect(timers.size).toBe(1);
    await editing;
    // The reply cancels what was scheduled against the old rules.
    expect(timers.size).toBe(0);
    const disabling = store.setEnabled(false);
    safari();
    elapse();
    await disabling;
    expect(switched).toEqual([]);
  });

  it('use current app explains when no app is known or the platform has no watcher', async () => {
    const known = setup(true, { focus_current_app: { supported: true, bundle_id: 'com.apple.Safari' } });
    expect(await known.store.currentApp()).toBe('com.apple.Safari');
    const none = setup(true, { focus_current_app: { supported: true, bundle_id: null } });
    expect(await none.store.currentApp()).toBeNull();
    expect(none.store.getState().error).toContain('No other app');
    const unsupported = setup(true, { focus_current_app: { supported: false, bundle_id: null } });
    expect(await unsupported.store.currentApp()).toBeNull();
    expect(unsupported.store.getState().error).toContain('macOS');
  });

  it('does nothing in the browser preview', async () => {
    const { store, calls, handlers } = setup(false);
    store.connect();
    expect(await store.load()).toBe(false);
    expect(await store.currentApp()).toBeNull();
    expect(calls).toEqual([]);
    expect(handlers).toEqual({});
  });
});

describe('ServiceRunner.switchProfile', () => {
  const profile: Profile = { schema_version: 1, name: 'Browser', bindings: [] };
  function runnerWith(runs: Record<string, ServiceRun>, result: SwitchedProfile | Error) {
    const calls: { command: string; args?: Record<string, unknown> }[] = [];
    let release: () => void = () => {};
    const gate = new Promise<void>(resolve => { release = resolve; });
    const invoke = (async (command: string, args?: Record<string, unknown>) => {
      calls.push({ command, args });
      await gate;
      if (result instanceof Error) throw result;
      return result;
    }) as never;
    const runner = new ServiceRunner(invoke, async () => () => {}, true, callback => callback(), () => 5);
    (runner as unknown as { state: unknown }).state = { desktop: true, error: null, runs };
    return { runner, calls, release };
  }

  it('updates the run to the saved profile and ignores repeats while in flight', async () => {
    const { runner, calls, release } = runnerWith({ s: run() }, { id: profileB, profile, modified_at_ms: 1, profile_generation: 1 });
    const first = runner.switchProfile('s', profileB, 'focused com.apple.Safari');
    expect(await runner.switchProfile('s', profileB, 'again')).toBe(false);
    release();
    expect(await first).toBe(true);
    expect(calls).toEqual([{ command: 'switch_session_profile', args: { sessionId: 's', profileId: profileB } }]);
    const updated = runner.getState().runs.s;
    expect(updated).toMatchObject({ profileId: profileB, profileName: 'Browser', draftUnsaved: false, profileGeneration: 1, profileSwitched: true });
    expect(updated.entries.at(-1)?.text).toBe('Switched to profile "Browser" (focused com.apple.Safari)');
    expect(await runner.switchProfile('s', profileB, 'same')).toBe(false);
  });

  it('serializes switches: one request in flight, only the newest waiting', async () => {
    const profileC = '00000000-0000-4000-8000-00000000000c';
    const profileD = '00000000-0000-4000-8000-00000000000d';
    const sent: string[] = [];
    const replies: (() => void)[] = [];
    const invoke = ((_command: string, args?: Record<string, unknown>) => {
      const id = args!.profileId as string;
      sent.push(id);
      const generation = sent.length;
      return new Promise(resolve => { replies.push(() => resolve({ id, profile: { ...profile, name: id }, modified_at_ms: 1, profile_generation: generation })); });
    }) as never;
    const runner = new ServiceRunner(invoke, async () => () => {}, true, callback => callback(), () => 5);
    (runner as unknown as { state: unknown }).state = { desktop: true, error: null, runs: { s: run() } };
    const toB = runner.switchProfile('s', profileB, 'b');
    const toC = runner.switchProfile('s', profileC, 'c');
    const toD = runner.switchProfile('s', profileD, 'd');
    expect(sent).toEqual([profileB]);
    expect(await toC).toBe(false);
    replies[0]();
    expect(await toB).toBe(true);
    expect(runner.getState().runs.s).toMatchObject({ profileId: profileB, profileGeneration: 1 });
    await tick();
    expect(sent).toEqual([profileB, profileD]);
    replies[1]();
    expect(await toD).toBe(true);
    expect(runner.getState().runs.s).toMatchObject({ profileId: profileD, profileName: profileD, profileGeneration: 2, profileSwitched: true });
    // The queue is empty again: a new switch is sent at once.
    void runner.switchProfile('s', profileB, 'b');
    expect(sent).toEqual([profileB, profileD, profileB]);
  });

  it('refuses unmanaged or stopped runs and reports failures', async () => {
    const { runner, calls, release } = runnerWith({ plain: run({}, null), done: run({}, 'stopped'), s: run() }, new Error('session service:aa has stopped'));
    release();
    expect(await runner.switchProfile('plain', profileB, 'x')).toBe(false);
    expect(await runner.switchProfile('done', profileB, 'x')).toBe(false);
    expect(await runner.switchProfile('missing', profileB, 'x')).toBe(false);
    expect(calls).toEqual([]);
    expect(await runner.switchProfile('s', profileB, 'x')).toBe(false);
    expect(runner.getState().error).toBe('session service:aa has stopped');
    expect(runner.getState().runs.s.profileId).toBe(profileA);
  });

  it('a paused switched run raises a notice that never names a profile that may change', () => {
    const paused = run({ execute: true, profileName: 'Browser' }, 'needs_confirmation');
    paused.session!.reason = 'profile_switch';
    const { add } = noticesFromRuns({ s: paused }, { s: 'running' });
    expect(add).toEqual([expect.objectContaining({ kind: 'paused', action: 'review', text: 'Focused-app rule switched the profile — confirm to resume actions' })]);
    expect(add[0].text).not.toContain('Browser');
  });
});
