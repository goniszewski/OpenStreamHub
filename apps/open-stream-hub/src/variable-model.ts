import type { BindingAction, BranchStep, Condition, KeypadSlot, Profile, Variable, VariableKind } from './contracts';
import type { BindingEntry } from './binding-model';
import { listBindings } from './binding-model';
import { groupItems, groupMode, isGroup, variableKind } from './flow-model';
import { pluginReferences, programReferences } from './action-model';

// Which variables the actions of a profile use, and the edits that keep every use in step with the declarations.

type AnyAction = BindingAction | BranchStep;

const conditionNames = (when: Condition, into: string[]): string[] => {
  if (isGroup(when)) { groupItems(when).forEach(item => conditionNames(item, into)); return into; }
  if ('variable_is' in when) into.push(when.variable_is.name);
  else if ('variable_at_least' in when) into.push(when.variable_at_least.name);
  return into;
};

// Every variable name an action (and everything inside it) refers to, once per use.
export function actionVariableNames(action: AnyAction | null | undefined, into: string[] = []): string[] {
  if (!action) return into;
  switch (action.type) {
    case 'set_variable': case 'toggle_variable': case 'add_to_variable': into.push(action.name); break;
    case 'multi_action': action.actions.forEach(step => actionVariableNames(step, into)); break;
    case 'conditional':
      conditionNames(action.when, into);
      action.then.concat(action.else ?? []).forEach(step => actionVariableNames(step, into));
      break;
    // With variables on, each {{name}} in an argument is a use; malformed text (refused by the service) names none.
    case 'run_program': into.push(...programReferences(action).names); break;
    // Each {{name}} in a text parameter the action fills in is a use, the same way.
    case 'plugin': into.push(...pluginReferences(action).names); break;
    default: break;
  }
  return into;
}

// Every action of a profile: flat bindings, keypad tiles and dial functions.
export function profileBindingActions(profile: Profile): BindingAction[] {
  const actions: BindingAction[] = profile.bindings.map(binding => binding.action);
  for (const page of profile.keypad?.pages ?? []) {
    for (const slot of page.slots) if (slot?.type === 'action') actions.push(...slot.bindings.map(binding => binding.action));
  }
  for (const dialFunction of profile.dial_functions ?? []) actions.push(...dialFunction.bindings.map(binding => binding.action));
  return actions;
}

// How many times each declared name is used by an action or a test.
export function variableUseCounts(profile: Profile): Map<string, number> {
  const counts = new Map<string, number>();
  for (const action of profileBindingActions(profile)) {
    for (const name of actionVariableNames(action)) counts.set(name, (counts.get(name) ?? 0) + 1);
  }
  // Key faces test variables too.
  for (const page of profile.keypad?.pages ?? []) {
    for (const slot of page.slots) {
      if (slot?.type !== 'action') continue;
      for (const face of slot.faces ?? []) for (const name of conditionNames(face.when, [])) counts.set(name, (counts.get(name) ?? 0) + 1);
    }
  }
  return counts;
}

// The bindings whose action (or a test inside it) uses a variable, so the editor can show and open each one.
export function variableUsers(profile: Profile, name: string): BindingEntry[] {
  return listBindings(profile).filter(entry => {
    const action = entry.flat?.action ?? entry.dialBinding?.action ?? entry.keypadBinding?.action;
    return actionVariableNames(action).includes(name);
  });
}

const renameCondition = (when: Condition, from: string, to: string): Condition => {
  if (isGroup(when)) {
    const items = groupItems(when).map(item => renameCondition(item, from, to));
    return groupMode(when) === 'all' ? { all: items } : { any: items };
  }
  if ('variable_is' in when && when.variable_is.name === from) return { variable_is: { ...when.variable_is, name: to } };
  if ('variable_at_least' in when && when.variable_at_least.name === from) return { variable_at_least: { ...when.variable_at_least, name: to } };
  return when;
};

function renameAction<A extends AnyAction>(action: A, from: string, to: string): A {
  const step = action as AnyAction;
  switch (step.type) {
    case 'set_variable': case 'toggle_variable': case 'add_to_variable': return (step.name === from ? { ...step, name: to } : step) as A;
    case 'multi_action': return { ...step, actions: step.actions.map(item => renameAction(item, from, to)) } as A;
    case 'conditional': {
      const next = { ...step, when: renameCondition(step.when, from, to), then: step.then.map(item => renameAction(item, from, to)) };
      return (step.else ? { ...next, else: step.else.map(item => renameAction(item, from, to)) } : next) as A;
    }
    // Names are whole tokens between {{ and }}, so replacing the reference renames exactly that variable. With variables
    // off the references are renamed too, so turning variables back on still names declared variables. Changed
    // arguments are a new program for the Execute confirmation either way.
    case 'run_program':
      return (step.args ? { ...step, args: step.args.map(arg => arg.split(`{{${from}}}`).join(`{{${to}}}`)) } : step) as A;
    // Every text parameter, so a parameter that does not take variables keeps reading the same once it does.
    case 'plugin':
      return (step.params ? { ...step, params: Object.fromEntries(Object.entries(step.params).map(([name, value]) =>
        [name, typeof value === 'string' ? value.split(`{{${from}}}`).join(`{{${to}}}`) : value])) } : step) as A;
    default: return action;
  }
}

// Renames a declaration and every use of it, so a rename never breaks a binding.
export function renameVariable(profile: Profile, from: string, to: string): Profile {
  const rename = <T extends { action: BindingAction }>(binding: T): T => ({ ...binding, action: renameAction(binding.action, from, to) });
  // Key faces keep their tests in step too; a slot without faces is left without the field.
  const renameSlot = (slot: Extract<KeypadSlot, { type: 'action' }>) => ({
    ...slot,
    bindings: slot.bindings.map(rename),
    ...(slot.faces ? { faces: slot.faces.map(face => ({ ...face, when: renameCondition(face.when, from, to) })) } : {}),
  });
  return {
    ...profile,
    variables: (profile.variables ?? []).map(variable => (variable.name === from ? { ...variable, name: to } : variable)),
    bindings: profile.bindings.map(rename),
    ...(profile.keypad ? {
      keypad: { pages: profile.keypad.pages.map(page => ({ ...page, slots: page.slots.map(slot => (slot?.type === 'action' ? renameSlot(slot) : slot)) })) },
    } : {}),
    ...(profile.dial_functions ? { dial_functions: profile.dial_functions.map(fn => ({ ...fn, bindings: fn.bindings.map(rename) })) } : {}),
  };
}

// The declared variables a test or action can use: those of the wanted kinds.
export const variablesOfKind = (variables: ReadonlyArray<Variable>, kinds: ReadonlyArray<VariableKind>): Variable[] =>
  variables.filter(variable => kinds.includes(variableKind(variable.value)));
