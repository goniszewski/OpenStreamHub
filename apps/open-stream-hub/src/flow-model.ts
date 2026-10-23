import type { BranchStep, Condition, ConditionalAction, VariableKind, VariableValue } from './contracts';

// The shape of an if / else flow and the moves that keep it valid. Mirrors Condition::validate and Action::validate in
// crates/cooltek-sdk-service/src/profile.rs (the Rust validator is authoritative and runs on save); every move here is
// refused when it would leave the flow outside these limits, so a drag can never produce a profile Rust would reject.

export const flowLimits = {
  combinedMinimum: 2, combinedMaximum: 8, conditionDepth: 3, conditionLeaves: 16,
  branchMaximum: 8, conditionalDepth: 3, steps: 32,
} as const;

export type Branch = 'then' | 'else';

const clone = <T,>(value: T): T => structuredClone(value);

// ---------------------------------------------------------------------------------------------------------------------
// Conditions

export type ConditionGroup = { all: Condition[] } | { any: Condition[] };
export type CombineMode = 'all' | 'any';

export const isGroup = (condition: Condition): condition is ConditionGroup => 'all' in condition || 'any' in condition;
export const groupMode = (group: ConditionGroup): CombineMode => ('all' in group ? 'all' : 'any');
export const groupItems = (group: ConditionGroup): Condition[] => ('all' in group ? group.all : group.any);
const makeGroup = (mode: CombineMode, items: Condition[]): ConditionGroup => (mode === 'all' ? { all: items } : { any: items });

// Index path from the root down: [] is the root itself, [1, 0] the first member of the second member.
export type ConditionPath = readonly number[];

export function conditionDepth(condition: Condition): number {
  return isGroup(condition) ? 1 + Math.max(0, ...groupItems(condition).map(conditionDepth)) : 0;
}

export function conditionLeaves(condition: Condition): number {
  return isGroup(condition) ? groupItems(condition).reduce((total, item) => total + conditionLeaves(item), 0) : 1;
}

export function conditionAt(root: Condition, path: ConditionPath): Condition | undefined {
  let node: Condition = root;
  for (const index of path) {
    if (!isGroup(node)) return undefined;
    const next: Condition | undefined = groupItems(node)[index];
    if (!next) return undefined;
    node = next;
  }
  return node;
}

// A group left with one member becomes that member, so the profile never holds a one-member group; an empty group goes.
function normalizeCondition(condition: Condition): Condition | null {
  if (!isGroup(condition)) return condition;
  const items = groupItems(condition).map(normalizeCondition).filter((item): item is Condition => item !== null);
  if (items.length === 0) return null;
  if (items.length === 1) return items[0];
  return makeGroup(groupMode(condition), items);
}

// Whether a condition tree is inside the limits; the first problem is said in a sentence.
export function conditionIssue(condition: Condition): string | null {
  if (conditionDepth(condition) > flowLimits.conditionDepth) return `Groups nest at most ${flowLimits.conditionDepth} levels deep.`;
  if (conditionLeaves(condition) > flowLimits.conditionLeaves) return `A condition holds at most ${flowLimits.conditionLeaves} tests.`;
  const walk = (node: Condition): string | null => {
    if (!isGroup(node)) return null;
    const items = groupItems(node);
    if (items.length < flowLimits.combinedMinimum || items.length > flowLimits.combinedMaximum) {
      return `A group combines ${flowLimits.combinedMinimum} to ${flowLimits.combinedMaximum} conditions.`;
    }
    for (const item of items) {
      const issue = walk(item);
      if (issue) return issue;
    }
    return null;
  };
  return walk(condition);
}

// The root as a group to edit: a lone test is a group of one until a second joins it.
const rootGroup = (root: Condition): ConditionGroup => (isGroup(root) ? clone(root) : { all: [clone(root)] });

function groupAtPath(root: ConditionGroup, path: ConditionPath): ConditionGroup | undefined {
  const node = conditionAt(root, path);
  return node && isGroup(node) ? node : undefined;
}

function finish(group: ConditionGroup): Condition | null {
  const result = normalizeCondition(group);
  return result && conditionIssue(result) === null ? result : null;
}

// Replaces the test or group at `path` ([] replaces the whole condition).
export function replaceCondition(root: Condition, path: ConditionPath, node: Condition): Condition | null {
  if (path.length === 0) return conditionIssue(node) === null ? clone(node) : null;
  const copy = rootGroup(root);
  const parent = groupAtPath(copy, path.slice(0, -1));
  if (!parent) return null;
  groupItems(parent)[path[path.length - 1]] = clone(node);
  return finish(copy);
}

// Adds `node` as member `index` of the group at `groupPath` (the root when the path is empty).
export function insertCondition(root: Condition, groupPath: ConditionPath, index: number, node: Condition): Condition | null {
  const copy = rootGroup(root);
  const target = groupAtPath(copy, groupPath);
  if (!target) return null;
  const items = groupItems(target);
  items.splice(Math.min(Math.max(index, 0), items.length), 0, clone(node));
  if (items.length > flowLimits.combinedMaximum) return null;
  return finish(copy);
}

export function removeCondition(root: Condition, path: ConditionPath): Condition | null {
  if (path.length === 0) return null;
  const copy = rootGroup(root);
  const parent = groupAtPath(copy, path.slice(0, -1));
  if (!parent) return null;
  groupItems(parent).splice(path[path.length - 1], 1);
  return finish(copy);
}

// Turns the group at `path` into AND or OR.
export function setGroupMode(root: Condition, path: ConditionPath, mode: CombineMode): Condition | null {
  const node = conditionAt(root, path);
  if (!node || !isGroup(node)) return null;
  return replaceCondition(root, path, makeGroup(mode, clone(groupItems(node))));
}

// Wraps the test or group at `path` and a new test in a group of the other kind ("add a group here").
export function groupWith(root: Condition, path: ConditionPath, mode: CombineMode, extra: Condition): Condition | null {
  const node = conditionAt(root, path);
  if (!node) return null;
  return replaceCondition(root, path, makeGroup(mode, [clone(node), clone(extra)]));
}

// Moves the test or group at `from` to be member `toIndex` of the group at `toGroup`. The target is found before the
// source leaves its group, and a group left with one member folds into it, so the paths describe the tree as the user saw it.
export function moveCondition(root: Condition, from: ConditionPath, toGroup: ConditionPath, toIndex: number): Condition | null {
  if (from.length === 0) return null;
  const copy = rootGroup(root);
  const source = conditionAt(copy, from);
  const target = groupAtPath(copy, toGroup);
  const parent = groupAtPath(copy, from.slice(0, -1));
  if (!source || !target || !parent) return null;
  // Never into itself or one of its own members.
  if (source === target || (isGroup(source) && containsNode(source, target))) return null;
  const parentItems = groupItems(parent);
  const sourceIndex = from[from.length - 1];
  parentItems.splice(sourceIndex, 1);
  const items = groupItems(target);
  // Dropping later in the group it left: the removal shifted the slots down by one.
  const at = target === parent && toIndex > sourceIndex ? toIndex - 1 : toIndex;
  items.splice(Math.min(Math.max(at, 0), items.length), 0, source);
  if (items.length > flowLimits.combinedMaximum) return null;
  return finish(copy);
}

function containsNode(group: ConditionGroup, wanted: Condition): boolean {
  return groupItems(group).some(item => item === wanted || (isGroup(item) && containsNode(item, wanted)));
}

export const canMoveCondition = (root: Condition, from: ConditionPath, toGroup: ConditionPath, toIndex: number): boolean =>
  moveCondition(root, from, toGroup, toIndex) !== null;

// ---------------------------------------------------------------------------------------------------------------------
// Steps

// The list `branch` of the conditional reached from the root through `via`: each hop names a branch and the index of a
// nested conditional in it. Empty `via` is the root conditional itself.
export interface ListAddress { via: ReadonlyArray<{ branch: Branch; index: number }>; branch: Branch }
export interface StepAddress { list: ListAddress; index: number }

export const sameList = (a: ListAddress, b: ListAddress): boolean =>
  a.branch === b.branch && a.via.length === b.via.length && a.via.every((hop, at) => hop.branch === b.via[at].branch && hop.index === b.via[at].index);

export function conditionalAt(root: ConditionalAction, via: ListAddress['via']): ConditionalAction | undefined {
  let node: ConditionalAction = root;
  for (const hop of via) {
    const next: BranchStep | undefined = (hop.branch === 'then' ? node.then : node.else ?? [])[hop.index];
    if (!next || next.type !== 'conditional') return undefined;
    node = next;
  }
  return node;
}

// The array behind an address, created (as `else`) when the branch is missing.
function listOf(owner: ConditionalAction, branch: Branch): BranchStep[] {
  if (branch === 'then') return owner.then;
  owner.else ??= [];
  return owner.else;
}

export const getList = (root: ConditionalAction, address: ListAddress): BranchStep[] =>
  (conditionalAt(root, address.via)?.[address.branch === 'then' ? 'then' : 'else'] ?? []);

export function flowDepth(root: ConditionalAction): number {
  return 1 + Math.max(0, ...(root.then.concat(root.else ?? [])).map(step => (step.type === 'conditional' ? flowDepth(step) : 0)));
}

// Steps in every branch, a nested conditional counting as a step and as its own steps (Action::flow_steps).
export function flowSteps(root: ConditionalAction): number {
  return root.then.concat(root.else ?? []).reduce((total, step) => total + 1 + (step.type === 'conditional' ? flowSteps(step) : 0), 0);
}

// A conditional's empty `else` is left out of the profile, never written as [].
function tidy(node: ConditionalAction): void {
  if (node.else && node.else.length === 0) delete node.else;
  for (const step of node.then.concat(node.else ?? [])) if (step.type === 'conditional') tidy(step);
}

// The first rule a flow breaks, in a sentence, or null.
export function flowIssue(root: ConditionalAction): string | null {
  const check = (node: ConditionalAction): string | null => {
    if (node.then.length === 0) return 'Every If needs at least one Then step.';
    if (node.then.length > flowLimits.branchMaximum || (node.else?.length ?? 0) > flowLimits.branchMaximum) return `A list holds at most ${flowLimits.branchMaximum} steps.`;
    const own = conditionIssue(node.when);
    if (own) return own;
    for (const step of node.then.concat(node.else ?? [])) {
      if (step.type !== 'conditional') continue;
      const inner = check(step);
      if (inner) return inner;
    }
    return null;
  };
  const own = check(root);
  if (own) return own;
  if (flowDepth(root) > flowLimits.conditionalDepth) return `If / else blocks nest at most ${flowLimits.conditionalDepth} levels deep.`;
  if (flowSteps(root) > flowLimits.steps) return `A flow holds at most ${flowLimits.steps} steps in all its lists.`;
  return null;
}

function finishFlow(copy: ConditionalAction): ConditionalAction | null {
  tidy(copy);
  return flowIssue(copy) === null ? copy : null;
}

// Replaces the whole list at `address` (used for edits inside a list, so the limits still apply).
export function setList(root: ConditionalAction, address: ListAddress, steps: BranchStep[]): ConditionalAction | null {
  const copy = clone(root);
  const owner = conditionalAt(copy, address.via);
  if (!owner) return null;
  if (address.branch === 'then') owner.then = clone(steps);
  else owner.else = clone(steps);
  return finishFlow(copy);
}

export function insertStep(root: ConditionalAction, list: ListAddress, index: number, step: BranchStep): ConditionalAction | null {
  const copy = clone(root);
  const owner = conditionalAt(copy, list.via);
  if (!owner) return null;
  const steps = listOf(owner, list.branch);
  steps.splice(Math.min(Math.max(index, 0), steps.length), 0, clone(step));
  return finishFlow(copy);
}

export function removeStep(root: ConditionalAction, address: StepAddress): ConditionalAction | null {
  const copy = clone(root);
  const owner = conditionalAt(copy, address.list.via);
  if (!owner) return null;
  listOf(owner, address.list.branch).splice(address.index, 1);
  return finishFlow(copy);
}

export function replaceStep(root: ConditionalAction, address: StepAddress, step: BranchStep): ConditionalAction | null {
  const copy = clone(root);
  const owner = conditionalAt(copy, address.list.via);
  if (!owner) return null;
  listOf(owner, address.list.branch)[address.index] = clone(step);
  return finishFlow(copy);
}

// Moves a step (with everything inside a nested if / else) to slot `toIndex` of another or the same list. A step never
// goes inside itself, and a move that would empty a Then list, overfill a list or pass the depth or size limits is refused.
export function moveStep(root: ConditionalAction, from: StepAddress, to: ListAddress, toIndex: number): ConditionalAction | null {
  const copy = clone(root);
  const fromOwner = conditionalAt(copy, from.list.via);
  const toOwner = conditionalAt(copy, to.via);
  if (!fromOwner || !toOwner) return null;
  const source = listOf(fromOwner, from.list.branch)[from.index];
  if (!source) return null;
  const target = listOf(toOwner, to.branch);
  if (source.type === 'conditional' && ownsList(source, target)) return null;
  const sourceList = listOf(fromOwner, from.list.branch);
  sourceList.splice(from.index, 1);
  const at = sourceList === target && toIndex > from.index ? toIndex - 1 : toIndex;
  target.splice(Math.min(Math.max(at, 0), target.length), 0, source);
  return finishFlow(copy);
}

function ownsList(node: ConditionalAction, list: BranchStep[]): boolean {
  if (node.then === list || node.else === list) return true;
  return node.then.concat(node.else ?? []).some(step => step.type === 'conditional' && ownsList(step, list));
}

export const canMoveStep = (root: ConditionalAction, from: StepAddress, to: ListAddress, toIndex: number): boolean =>
  moveStep(root, from, to, toIndex) !== null;

// ---------------------------------------------------------------------------------------------------------------------
// Variables

export const variableLimits = { count: 16, nameBytes: 32, textBytes: 64, minimum: -1_000_000, maximum: 1_000_000 } as const;

export const variableKind = (value: VariableValue): VariableKind => (typeof value === 'boolean' ? 'bool' : typeof value === 'number' ? 'number' : 'text');

export const variableKindLabels: Record<VariableKind, string> = { bool: 'On / off', number: 'Number', text: 'Text' };

export function defaultVariableValue(kind: VariableKind): VariableValue {
  return kind === 'bool' ? false : kind === 'number' ? 0 : '';
}

// Mirrors validate_variable_name in profile.rs.
export function variableNameIssue(name: string): string | null {
  if (!/^[a-z][a-z0-9_]{0,31}$/.test(name)) return `Use 1 to ${variableLimits.nameBytes} characters: a-z, 0-9 and _, starting with a letter.`;
  return null;
}

const utf8 = new TextEncoder();

export function variableValueIssue(value: VariableValue): string | null {
  if (typeof value === 'number') {
    return Number.isInteger(value) && value >= variableLimits.minimum && value <= variableLimits.maximum
      ? null : `Use a whole number from ${variableLimits.minimum.toLocaleString('en-US')} to ${variableLimits.maximum.toLocaleString('en-US')}.`;
  }
  if (typeof value === 'string') {
    if (utf8.encode(value).length > variableLimits.textBytes) return `Use at most ${variableLimits.textBytes} bytes.`;
    if (/[\u0000-\u001f\u007f-\u009f]/.test(value)) return 'Use one line without control characters.';
  }
  return null;
}

// The first problem in a list of declarations, in a sentence, or null.
export function variablesIssue(variables: ReadonlyArray<{ name: string; value: VariableValue }>): string | null {
  if (variables.length > variableLimits.count) return `A profile declares at most ${variableLimits.count} variables.`;
  const seen = new Set<string>();
  for (const variable of variables) {
    const issue = variableNameIssue(variable.name) ?? variableValueIssue(variable.value);
    if (issue) return `${variable.name || 'A variable'}: ${issue}`;
    if (seen.has(variable.name)) return `${variable.name} is declared twice.`;
    seen.add(variable.name);
  }
  return null;
}

// A name for a new variable that no declaration uses yet.
export function freshVariableName(variables: ReadonlyArray<{ name: string }>): string {
  const taken = new Set(variables.map(variable => variable.name));
  for (let number = 1; ; number += 1) if (!taken.has(`variable_${number}`)) return `variable_${number}`;
}

// Sets the condition of the conditional reached through `via`.
export function setWhen(root: ConditionalAction, via: ListAddress['via'], when: Condition): ConditionalAction | null {
  const copy = clone(root);
  const owner = conditionalAt(copy, via);
  if (!owner) return null;
  owner.when = clone(when);
  return finishFlow(copy);
}
