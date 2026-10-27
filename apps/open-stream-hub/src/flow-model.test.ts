import { describe, expect, it } from 'vitest';
import type { BranchStep, Condition, ConditionalAction } from './contracts';
import {
  canMoveCondition, canMoveStep, conditionAt, conditionDepth, conditionIssue, conditionLeaves, flowDepth, flowIssue, flowSteps, freshVariableName,
  getList, groupWith, insertCondition, insertStep, moveCondition, moveStep, removeCondition, removeStep, replaceCondition, replaceStep, setGroupMode, setList,
  variableKind, variableNameIssue, variablesIssue, variableValueIssue,
} from './flow-model';

const muted: Condition = { output_muted: true };
const timer: Condition = { timer_running: true };
const recording: Condition = { obs_recording: true };
const app: Condition = { focused_app: 'com.apple.finder' };
const mute: BranchStep = { type: 'toggle_mute' };
const flow = (then: BranchStep[], otherwise?: BranchStep[], when: Condition = muted): ConditionalAction =>
  ({ type: 'conditional', when, then, ...(otherwise ? { else: otherwise } : {}) });

describe('condition trees', () => {
  it('measures depth and tests', () => {
    expect(conditionDepth(muted)).toBe(0);
    expect(conditionDepth({ all: [muted, timer] })).toBe(1);
    expect(conditionDepth({ all: [muted, { any: [timer, { all: [app, recording] }] }] })).toBe(3);
    expect(conditionLeaves({ all: [muted, { any: [timer, { all: [app, recording] }] }] })).toBe(4);
  });

  it('finds a member by its index path', () => {
    const tree: Condition = { all: [muted, { any: [timer, app] }] };
    expect(conditionAt(tree, [])).toBe(tree);
    expect(conditionAt(tree, [1, 1])).toEqual(app);
    expect(conditionAt(tree, [0, 0])).toBeUndefined();
    expect(conditionAt(tree, [5])).toBeUndefined();
  });

  it('a lone test grows into a group when a second joins it, and shrinks back when one leaves', () => {
    const two = insertCondition(muted, [], 1, timer)!;
    expect(two).toEqual({ all: [muted, timer] });
    expect(removeCondition(two, [1])).toEqual(muted);
    expect(removeCondition(muted, [])).toBeNull();
    expect(removeCondition(muted, [0])).toBeNull();
  });

  it('removing from a nested group folds a one-member group into its parent', () => {
    const tree: Condition = { all: [muted, { any: [timer, app] }] };
    expect(removeCondition(tree, [1, 0])).toEqual({ all: [muted, app] });
  });

  it('groups a test with a new one, and switches a group between AND and OR', () => {
    const tree: Condition = { all: [muted, timer] };
    expect(groupWith(tree, [1], 'any', app)).toEqual({ all: [muted, { any: [timer, app] }] });
    expect(groupWith(muted, [], 'any', timer)).toEqual({ any: [muted, timer] });
    expect(setGroupMode(tree, [], 'any')).toEqual({ any: [muted, timer] });
    expect(setGroupMode(muted, [], 'any')).toBeNull();
    expect(replaceCondition(tree, [0], { output_volume_at_least: 10 })).toEqual({ all: [{ output_volume_at_least: 10 }, timer] });
    expect(replaceCondition(muted, [], timer)).toEqual(timer);
  });

  it('drags a test into another group, within a group, and out to the top', () => {
    const tree: Condition = { all: [muted, { any: [timer, app] }, recording] };
    expect(moveCondition(tree, [0], [1], 1)).toEqual({ all: [{ any: [timer, muted, app] }, recording] });
    // Within the same group, the drop slot is read before the test leaves: slot 3 is the end.
    expect(moveCondition(tree, [0], [], 3)).toEqual({ all: [{ any: [timer, app] }, recording, muted] });
    expect(moveCondition(tree, [2], [], 0)).toEqual({ all: [recording, muted, { any: [timer, app] }] });
    // Out of the nested group; the group left with one member folds away.
    expect(moveCondition(tree, [1, 0], [], 0)).toEqual({ all: [timer, muted, app, recording] });
  });

  it('never drags a group into itself or its own members', () => {
    const tree: Condition = { all: [muted, { any: [timer, { all: [app, recording] }] }] };
    expect(moveCondition(tree, [1], [1], 0)).toBeNull();
    expect(moveCondition(tree, [1], [1, 1], 0)).toBeNull();
    expect(moveCondition(tree, [], [], 0)).toBeNull();
  });

  it('refuses a drag past the group size, depth or test limits', () => {
    const eight: Condition = { all: Array.from({ length: 8 }, () => muted) };
    const wide: Condition = { any: [eight, timer] };
    expect(moveCondition(wide, [1], [0], 0)).toBeNull();
    const deep: Condition = { all: [muted, { any: [timer, { all: [app, recording] }] }] };
    const withExtra = insertCondition(deep, [], 2, { any: [muted, timer] })!;
    // Moving the extra group into the deepest group would make four levels.
    expect(moveCondition(withExtra, [2], [1, 1], 0)).toBeNull();
    expect(conditionIssue(deep)).toBeNull();
    const four: Condition = { all: [muted, { any: [timer, { all: [app, { any: [muted, timer] }] }] }] };
    expect(conditionIssue(four)).toMatch(/3 levels/);
    const groupOf = (count: number): Condition => ({ any: Array.from({ length: count }, () => muted) });
    expect(conditionIssue({ all: [groupOf(6), groupOf(6), groupOf(6)] })).toMatch(/16 tests/);
    expect(canMoveCondition(deep, [0], [1], 0)).toBe(true);
  });
});

describe('step trees', () => {
  const inner = flow([mute], [mute]);
  const root = flow([mute, inner], [mute]);
  const here = { via: [], branch: 'then' as const };
  const there = { via: [], branch: 'else' as const };
  const inInner = { via: [{ branch: 'then' as const, index: 1 }], branch: 'else' as const };

  it('measures the flow', () => {
    expect(flowDepth(root)).toBe(2);
    expect(flowSteps(root)).toBe(5);
    expect(getList(root, inInner)).toEqual([mute]);
    expect(getList(root, { via: [{ branch: 'then', index: 0 }], branch: 'then' })).toEqual([]);
  });

  it('inserts, replaces and removes steps in any list, keeping Then non-empty', () => {
    expect(insertStep(root, there, 1, { type: 'media', command: 'play_pause' })!.else).toHaveLength(2);
    expect(insertStep(flow([mute]), there, 0, mute)).toEqual({ ...flow([mute]), else: [mute] });
    expect(removeStep(root, { list: there, index: 0 })).toEqual(flow([mute, inner]));
    expect(removeStep(flow([mute]), { list: here, index: 0 })).toBeNull();
    expect(replaceStep(root, { list: here, index: 0 }, { type: 'media', command: 'stop' })!.then[0]).toEqual({ type: 'media', command: 'stop' });
    expect(setList(root, inInner, [mute, mute])).not.toBeNull();
    expect(setList(root, here, [])).toBeNull();
  });

  it('drags a step between lists and into a nested if / else', () => {
    const volume: BranchStep = { type: 'set_volume', percent: 10 };
    const tree = flow([mute, volume, inner], [mute]);
    const moved = moveStep(tree, { list: here, index: 1 }, there, 0)!;
    expect(moved.then).toEqual([mute, inner]);
    expect(moved.else).toEqual([volume, mute]);
    const into = moveStep(tree, { list: here, index: 1 }, { via: [{ branch: 'then', index: 2 }], branch: 'then' }, 1)!;
    expect(into.then).toHaveLength(2);
    expect((into.then[1] as ConditionalAction).then).toEqual([mute, volume]);
    // Within the same list: slot 3 is the end.
    expect(moveStep(tree, { list: here, index: 0 }, here, 3)!.then).toEqual([volume, inner, mute]);
    expect(moveStep(tree, { list: here, index: 2 }, here, 0)!.then).toEqual([inner, mute, volume]);
  });

  it('emptying the else list drops the key, and an else is created on demand', () => {
    const tree = flow([mute, mute], [mute]);
    expect(moveStep(tree, { list: there, index: 0 }, here, 0)).toEqual(flow([mute, mute, mute]));
    const gained = moveStep(flow([mute, mute]), { list: here, index: 0 }, there, 0)!;
    expect(gained.else).toEqual([mute]);
  });

  it('never drags an if / else into itself, and refuses what Rust would reject', () => {
    const nested = flow([mute, inner]);
    expect(moveStep(nested, { list: here, index: 1 }, { via: [{ branch: 'then', index: 1 }], branch: 'then' }, 0)).toBeNull();
    // The only Then step cannot leave.
    expect(moveStep(flow([mute]), { list: here, index: 0 }, there, 0)).toBeNull();
    // A list holds eight.
    const full = flow(Array.from({ length: 8 }, () => mute), [mute]);
    expect(moveStep(full, { list: there, index: 0 }, here, 0)).toBeNull();
    // Three levels of if / else, no more.
    const three = flow([flow([flow([mute])])]);
    expect(flowIssue(three)).toBeNull();
    const four = flow([three, mute]);
    expect(flowIssue(four)).toMatch(/3 levels/);
    expect(canMoveStep(flow([mute, three]), { list: here, index: 0 }, { via: [{ branch: 'then', index: 1 }, { branch: 'then', index: 0 }, { branch: 'then', index: 0 }], branch: 'then' }, 0)).toBe(false);
  });

  it('counts a nested if / else as a step and as its own steps against 32', () => {
    const block = () => flow(Array.from({ length: 8 }, () => mute), Array.from({ length: 8 }, () => mute));
    expect(flowIssue(flow([block(), block()]))).toMatch(/32 steps/);
    expect(flowSteps(flow([block()]))).toBe(17);
  });
});

describe('variables', () => {
  it('names are lowercase words starting with a letter', () => {
    expect(variableNameIssue('takes')).toBeNull();
    expect(variableNameIssue('take_2')).toBeNull();
    for (const bad of ['', 'Takes', '2takes', 'two words', 'a'.repeat(33), 'é']) expect(variableNameIssue(bad)).not.toBeNull();
  });

  it('values follow the bool, number and text rules', () => {
    expect(variableKind(true)).toBe('bool');
    expect(variableKind(3)).toBe('number');
    expect(variableKind('intro')).toBe('text');
    expect(variableValueIssue(1_000_000)).toBeNull();
    expect(variableValueIssue(1_000_001)).not.toBeNull();
    expect(variableValueIssue(1.5)).not.toBeNull();
    expect(variableValueIssue('a'.repeat(65))).not.toBeNull();
    expect(variableValueIssue('line\nbreak')).not.toBeNull();
  });

  it('declarations are unique and at most sixteen', () => {
    expect(variablesIssue([{ name: 'a', value: 1 }, { name: 'a', value: 2 }])).toMatch(/twice/);
    expect(variablesIssue(Array.from({ length: 17 }, (_, index) => ({ name: `v${index}`, value: true })))).toMatch(/at most 16/);
    expect(variablesIssue([{ name: 'ok', value: false }])).toBeNull();
    expect(freshVariableName([{ name: 'variable_1' }, { name: 'variable_3' }])).toBe('variable_2');
  });
});
