import { describe, expect, it } from 'vitest';
import type { Action, BindingAction, BranchStep, FaderTemplate, LiveDevice, Profile } from './contracts';
import { keyFaceIssue, labelIssue } from './draft-model';
import { FakeStorage } from './fake-storage';
import { namedKeys } from './action-model';
import { bundleIdIssue } from './focus-rules';
import { actionIssue, markStartersOffered, nextModulePrompt, profileModules, shouldOfferStarters, starterIssue, starters, typingActionIssue, startersForModule, startersStorageKey } from './starter-model';
import { controlCounts, gestures, modules } from './ui-model';

const device = (path: string, module: LiveDevice['module']): LiveDevice => ({ path_hex: path, module, serial_number: null });

// Walks every action in a starter profile, including the keypad tiles and the steps inside an if / else.
function* actionsOf(profile: Profile): Generator<{ action: Action | FaderTemplate | BranchStep; where: string }> {
  function* withSteps(action: Action | FaderTemplate | BranchStep, where: string): Generator<{ action: Action | FaderTemplate | BranchStep; where: string }> {
    yield { action, where };
    if (action.type !== 'conditional') return;
    for (const [index, step] of action.then.entries()) yield* withSteps(step, `${where}/then/${index}`);
    for (const [index, step] of (action.else ?? []).entries()) yield* withSteps(step, `${where}/else/${index}`);
  }
  for (const binding of profile.bindings) yield* withSteps(binding.action, `${binding.module}/${binding.control}/${binding.gesture}`);
  for (const page of profile.keypad?.pages ?? []) for (const [index, slot] of page.slots.entries()) {
    if (slot?.type !== 'action') continue;
    for (const keypadBinding of slot.bindings) yield* withSteps(keypadBinding.action, `keypad/${page.id}/${index}`);
  }
}

describe('shipped starters', () => {
  it('are structurally valid profiles the service will accept', () => {
    expect(starters.map(item => item.name)).toEqual(['Mac basics', 'Streaming (OBS)', 'Streamer', 'Editing and media', 'Terminal and AI agents']);
    for (const item of starters) {
      expect(starterIssue(item.profile, item.id === 'terminal-agents' ? typingActionIssue : undefined), item.id).toBeNull();
      expect(labelIssue(item.profile.name, 'Profile name'), item.id).toBeNull();
      const page = item.profile.keypad!.pages[0];
      expect(labelIssue(page.name, 'Page name'), item.id).toBeNull();
      for (const slot of page.slots) {
        if (slot?.type !== 'action') continue;
        expect(labelIssue(slot.label, 'Key label'), `${item.id}: ${slot.label}`).toBeNull();
      }
    }
  });
  it('stay dry-run safe: nothing that types, launches, opens or names an OBS item', () => {
    const allowed = new Set(['volume', 'toggle_mute', 'media', 'system', 'timer', 'volume_from_position', 'obs_virtual_camera', 'obs_replay_buffer', 'obs_transition', 'obs_studio_mode', 'obs_record', 'obs_screenshot', 'obs_stream']);
    for (const item of starters.filter(starter => starter.id !== 'terminal-agents')) {
      for (const { action, where } of actionsOf(item.profile)) {
        expect(allowed.has(action.type), `${item.id} at ${where}`).toBe(true);
      }
    }
  });
  it('name valid, distinct bundle IDs for the terminal starter to switch on', () => {
    const apps = starters.find(item => item.id === 'terminal-agents')!.focusApps!;
    expect(apps.length).toBeGreaterThan(0);
    expect(new Set(apps.map(app => app.bundle_id.toLowerCase())).size).toBe(apps.length);
    for (const app of apps) expect(bundleIdIssue(app.bundle_id), app.name).toBeNull();
    expect(starters.filter(item => item.focusApps).map(item => item.id)).toEqual(['terminal-agents']);
  });
  it('only use shortcut keys the service accepts: one letter or digit, space, or a named key', () => {
    for (const item of starters) {
      for (const { action, where } of actionsOf(item.profile)) {
        if (action.type !== 'shortcut') continue;
        const accepted = /^[a-z0-9 ]$/i.test(action.key) || namedKeys.some(([name]) => name === action.key);
        expect(accepted, `${item.id} at ${where}: ${action.key}`).toBe(true);
      }
    }
  });
  it('let the terminal starter\'s Accept key follow the agent_state variable', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!;
    expect(terminal.profile.variables).toEqual([{ name: 'agent_state', value: 'idle' }, { name: 'agent_decision', value: 'none' }, { name: 'agent_waiting', value: 0 }]);
    const [accept, , ...rest] = terminal.profile.keypad!.pages[0].slots;
    expect(accept?.type === 'action' && accept.label).toBe('Accept');
    const faces = accept?.type === 'action' ? accept.faces ?? [] : [];
    expect(faces.map(face => [face.when, face.label, face.background])).toEqual([
      [{ variable_at_least: { name: 'agent_waiting', value: 2 } }, '2+ waiting', '#8f0f0f'],
      [{ variable_is: { name: 'agent_state', value: 'working' } }, 'Working', '#d98e04'],
      [{ variable_is: { name: 'agent_state', value: 'waiting' } }, 'Needs you', '#c81e1e'],
      [{ variable_is: { name: 'agent_state', value: 'done' } }, 'Done', '#1f9d55'],
    ]);
    for (const face of faces) expect(keyFaceIssue(face, terminal.profile.variables!), face.label).toBeNull();
    // Idle has no face: the key shows its normal face. Only Accept and Interrupt have faces.
    expect(rest.filter(slot => slot?.type === 'action' && slot.faces)).toEqual([]);
    expect(terminal.description).toContain('agent_state');
    expect(terminal.description).toContain('osh');
  });
  it('let Accept and Interrupt answer a waiting permission prompt and otherwise send their keys', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!;
    const [accept, deny] = terminal.profile.keypad!.pages[0].slots;
    const waiting = { variable_is: { name: 'agent_state', value: 'waiting' } };
    const action = (slot: typeof accept) => (slot?.type === 'action' ? slot.bindings[0].action : null);
    expect(action(accept)).toEqual({ type: 'conditional', when: waiting, then: [{ type: 'set_variable', name: 'agent_decision', value: 'allow' }], else: [{ type: 'shortcut', key: 'return', modifiers: [] }] });
    expect(action(deny)).toEqual({ type: 'conditional', when: waiting, then: [{ type: 'set_variable', name: 'agent_decision', value: 'deny' }], else: [{ type: 'shortcut', key: 'escape', modifiers: [] }] });
    expect(deny?.type === 'action' && deny.label).toBe('Interrupt');
    expect(deny?.type === 'action' && deny.faces).toEqual([{ when: waiting, label: 'Deny', background: '#c81e1e' }]);
    expect(terminal.description).toContain('osh agent setup claude');
    expect(terminal.description).toContain('Accept and Deny answer permission prompts');
  });
  it('check if / else and variable steps in the terminal starter only, nested steps included', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!.profile;
    const withAccept = (action: object): Profile => {
      const [accept, ...rest] = terminal.keypad!.pages[0].slots;
      const slot = { ...accept!, bindings: [{ gesture: 'pressed', action }] } as typeof accept;
      return { ...terminal, keypad: { pages: [{ ...terminal.keypad!.pages[0], slots: [slot, ...rest] }] } };
    };
    const when = { variable_is: { name: 'agent_state', value: 'waiting' } };
    const set = (name: string, value: string | number) => ({ type: 'set_variable', name, value });
    // The dry-run-safe check every other starter uses refuses both.
    expect(starterIssue(terminal)).toContain('does not belong in a starter');
    expect(starterIssue(withAccept(set('agent_decision', 'allow')))).toContain('does not belong in a starter');
    // A step that launches is refused inside a branch.
    expect(starterIssue(withAccept({ type: 'conditional', when, then: [{ type: 'launch_app', bundle_id: 'com.apple.Terminal' }] }), typingActionIssue)).toContain('launch_app does not belong');
    // Undeclared or mistyped variables, in the test or a step.
    expect(starterIssue(withAccept({ type: 'conditional', when, then: [set('missing', 'x')] }), typingActionIssue)).toContain('missing is not declared');
    expect(starterIssue(withAccept({ type: 'conditional', when, then: [set('agent_decision', 3)] }), typingActionIssue)).toContain('is text, not number');
    expect(starterIssue(withAccept({ type: 'conditional', when: { variable_is: { name: 'nope', value: 'x' } }, then: [set('agent_decision', 'allow')] }), typingActionIssue)).toContain('nope is not declared');
    // Program arguments with variables on name declared variables too (checked here with a check that lets run_program
    // through, since no starter may hold one).
    const anyAction = () => null;
    const program = (args: string[], variables = true) => ({ type: 'run_program', program: '/a', args, ...(variables ? { variables } : {}) });
    expect(starterIssue(withAccept(program(['x', '{{gone}}'])), anyAction)).toBe('Argument 2: Variable gone is not declared.');
    expect(starterIssue(withAccept(program(['{{ agent_state }}'])), anyAction)).toContain('Argument 1: Each {{ must begin');
    expect(starterIssue(withAccept(program(['{{agent_state}}'])), anyAction)).toBeNull();
    expect(starterIssue(withAccept(program(['{{gone}}'], false)), anyAction)).toBeNull();
    // The flow limits Rust enforces.
    expect(starterIssue(withAccept({ type: 'conditional', when, then: [] }), typingActionIssue)).toContain('at least one Then step');
    expect(starterIssue(withAccept({ type: 'conditional', when, then: Array(9).fill(set('agent_decision', 'allow')) }), typingActionIssue)).toContain('at most 8 steps');
    const nested = (depth: number): object => ({ type: 'conditional', when, then: [depth > 1 ? nested(depth - 1) : set('agent_decision', 'allow')] });
    expect(starterIssue(withAccept(nested(3)), typingActionIssue)).toBeNull();
    expect(starterIssue(withAccept(nested(4)), typingActionIssue)).toContain('3 levels deep');
  });
  it('refuse faces and variables the service would refuse', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!.profile;
    const withAccept = (patch: object): Profile => {
      const [accept, ...rest] = terminal.keypad!.pages[0].slots;
      return { ...terminal, keypad: { pages: [{ ...terminal.keypad!.pages[0], slots: [{ ...accept!, ...patch } as typeof accept, ...rest] }] } };
    };
    const face = { when: { variable_is: { name: 'agent_state', value: 'done' } }, label: 'Done' };
    expect(starterIssue(withAccept({ faces: Array(9).fill(face) }), typingActionIssue)).toContain('at most 8 faces');
    expect(starterIssue(withAccept({ faces: [{ ...face, when: { variable_is: { name: 'missing', value: 'x' } } }] }), typingActionIssue)).toContain('not declared');
    expect(starterIssue(withAccept({ faces: [{ ...face, background: 'amber' }] }), typingActionIssue)).toContain('hex colour');
    expect(starterIssue({ ...terminal, variables: [{ name: 'agent_state', value: 'idle' }, { name: 'agent_state', value: 'x' }] }, typingActionIssue)).toContain('declared twice');
  });
  it('let the terminal starter send keys and text, start agent sessions and answer agent hooks, and nothing else that launches, opens or reaches OBS', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!;
    for (const { action, where } of actionsOf(terminal.profile)) {
      expect(['shortcut', 'text', 'conditional', 'set_variable', 'terminal_command'], where).toContain(action.type);
      if (action.type === 'set_variable') expect(action.name, where).toBe('agent_decision');
      // Only a key's own action opens a terminal, never a step of an if / else.
      if (action.type === 'terminal_command') expect(where, where).toMatch(/^keypad\/root\/\d+$/);
    }
  });
  it('give the terminal starter Claude and Codex keys that each open a Terminal window running that agent', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!;
    const slots = terminal.profile.keypad!.pages[0].slots;
    const sessions = slots.flatMap(slot => (slot?.type === 'action' && slot.bindings[0].action.type === 'terminal_command' ? [[slot.label, slot.bindings[0].action]] : []));
    expect(sessions).toEqual([
      ['Claude', { type: 'terminal_command', app: 'terminal', command: 'claude' }],
      ['Codex', { type: 'terminal_command', app: 'terminal', command: 'codex' }],
    ]);
    // They take free slots: the existing keys keep their places, and Dictate (F19) takes the last one.
    expect(slots.slice(0, 12).map(slot => slot?.type === 'action' ? slot.label : null)).toEqual(['Accept', 'Interrupt', 'Cycle mode', 'Continue', '/clear', '/compact', 'Ctrl+C', 'Ctrl+D', 'Ctrl+L', 'New tab', 'Next tab', 'Previous tab']);
    const dictate = slots[14];
    expect(dictate?.type === 'action' && [dictate.label, dictate.bindings[0].action]).toEqual(['Dictate', { type: 'shortcut', key: 'f19', modifiers: [] }]);
    expect(slots.filter(slot => slot === null)).toHaveLength(0);
    expect(terminal.description).toContain('Claude and Codex keys');
    expect(terminal.description).toContain('F19');
  });
  it('check a terminal session in the terminal starter only, and never as a step', () => {
    const terminal = starters.find(item => item.id === 'terminal-agents')!.profile;
    const withAccept = (action: object): Profile => {
      const [accept, ...rest] = terminal.keypad!.pages[0].slots;
      const slot = { ...accept!, bindings: [{ gesture: 'pressed', action }], faces: undefined } as unknown as typeof accept;
      return { ...terminal, keypad: { pages: [{ ...terminal.keypad!.pages[0], slots: [slot, ...rest] }] } };
    };
    const session = { type: 'terminal_command', app: 'iterm2', command: 'codex', directory: '~/code' };
    expect(starterIssue(withAccept(session), typingActionIssue)).toBeNull();
    // The dry-run-safe check every other starter uses refuses it.
    expect(actionIssue(session as BindingAction)).toContain('terminal_command does not belong in a starter');
    expect(actionIssue({ type: 'run_program', program: '/usr/bin/true' })).toContain('run_program does not belong in a starter');
    expect(actionIssue({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.0.0', action: 'toggle' })).toContain('plugin does not belong in a starter');
    expect(starterIssue(withAccept({ ...session, command: 'codex\nrm -rf ~' }), typingActionIssue)).toContain('one line');
    expect(starterIssue(withAccept({ ...session, command: ' ' }), typingActionIssue)).toContain('Enter the command');
    expect(starterIssue(withAccept({ ...session, directory: 'code' }), typingActionIssue)).toContain('starting with / or ~/');
    expect(starterIssue(withAccept({ ...session, app: 'ghostty' }), typingActionIssue)).toContain('Unknown terminal app');
    const when = { variable_is: { name: 'agent_state', value: 'waiting' } };
    expect(starterIssue(withAccept({ type: 'conditional', when, then: [session] }), typingActionIssue)).toContain('cannot be an if / else step');
  });
  it('hold the stream only behind the press-twice guard', () => {
    for (const item of starters) {
      for (const { action, where } of actionsOf(item.profile)) {
        if (action.type === 'obs_stream') expect((action as { confirm?: string }).confirm, `${item.id} at ${where}`).toBe('press_twice');
      }
    }
    const unguarded: Profile = { ...starters.find(item => item.id === 'streamer')!.profile, bindings: [{ module: 'm131e', control: 0, gesture: 'pressed', action: { type: 'obs_stream', op: 'toggle' } }] };
    expect(starterIssue(unguarded)).toContain('press twice');
  });
  it('bind supported selectors without duplicates and keep the 15-key module on the keypad', () => {
    for (const item of starters) {
      const seen = new Set<string>();
      for (const binding of item.profile.bindings) {
        expect(controlCounts[binding.module], `${item.id} module`).toBeDefined();
        expect(binding.control, `${item.id} control`).toBeLessThan(controlCounts[binding.module]);
        expect(gestures[binding.module], `${item.id} gesture`).toContain(binding.gesture);
        const selector = `${binding.module}/${binding.control}/${binding.gesture}`;
        expect(seen.has(selector), `${item.id} duplicate ${selector}`).toBe(false);
        seen.add(selector);
        expect(binding.module, item.id).not.toBe('m4315');
      }
      const page = item.profile.keypad!.pages[0];
      expect(page.id).toBe('root');
      expect(page.slots).toHaveLength(15);
    }
  });
  it('cover every module together, so a connected module is always offered something', () => {
    const covered = new Set(starters.flatMap(item => profileModules(item.profile)));
    expect([...covered].sort()).toEqual(modules.map(item => item.id).sort());
  });
  it('derive their covered modules from the bindings and the keypad tiles', () => {
    expect(profileModules(starters[0].profile)).toEqual(['m003k', 'm131e', 'm005s', 'm032le', 'm4315']);
    expect(startersForModule('m032le')).toEqual([starters[0]]);
    expect(startersForModule('m4315')).toEqual(starters);
  });
});

describe('hot-plug prompt from a simulated inventory', () => {
  it('treats the first snapshot as the baseline', () => {
    expect(nextModulePrompt(null, [device('aa', 'm4315')])).toBeNull();
  });
  it('names the first added module after the baseline, and nothing on removals or swaps', () => {
    const baseline = [device('aa', 'm4315'), device('bb', 'm003k')];
    expect(nextModulePrompt(baseline, [...baseline, device('cc', 'm131e')])).toBe('m131e');
    expect(nextModulePrompt(baseline, [device('aa', 'm4315')])).toBeNull();
    expect(nextModulePrompt(baseline, [device('bb', 'm003k'), device('cc', 'm131e')])).toBe('m131e');
    // A module swap on one path is a removal plus an addition.
    expect(nextModulePrompt(baseline, [device('aa', 'm4315'), device('bb2', 'm005s')])).toBe('m005s');
    // An unchanged inventory names nobody.
    expect(nextModulePrompt(baseline, [...baseline])).toBeNull();
  });
  it('skips modules the open profile covers or that were dismissed this session', () => {
    const baseline = [device('aa', 'm4315')];
    const skip = (module: string) => module === 'm003k' || module === 'm131e';
    // The first addition is skipped, so the next one is offered.
    expect(nextModulePrompt(baseline, [...baseline, device('bb', 'm003k'), device('cc', 'm131e')], skip)).toBeNull();
    expect(nextModulePrompt(baseline, [...baseline, device('bb', 'm003k'), device('cc', 'm005s')], skip)).toBe('m005s');
    // A replug of a skipped module stays quiet.
    const after = [...baseline, device('bb', 'm003k')];
    expect(nextModulePrompt(after, [device('aa', 'm4315')], skip)).toBeNull();
  });
});

describe('first-run flag', () => {
  it('offers the chooser once on a fresh desktop store and records the offer', () => {
    const storage = new FakeStorage();
    expect(shouldOfferStarters(storage, true)).toBe(true);
    expect(shouldOfferStarters(storage, true)).toBe(true); // deciding is not offering
    markStartersOffered(storage);
    expect(storage.getItem(startersStorageKey)).toBe('offered');
    expect(shouldOfferStarters(storage, true)).toBe(false);
  });
  it('never offers outside the desktop app, without storage, or when storage is blocked', () => {
    expect(shouldOfferStarters(new FakeStorage(), false)).toBe(false);
    expect(shouldOfferStarters(null, true)).toBe(false);
    const broken = { getItem() { throw new Error('blocked'); }, setItem() { throw new Error('blocked'); } };
    expect(shouldOfferStarters(broken, true)).toBe(false);
    expect(() => markStartersOffered(broken)).not.toThrow();
    expect(() => markStartersOffered(null)).not.toThrow();
  });
});
