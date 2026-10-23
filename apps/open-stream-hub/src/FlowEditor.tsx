import { ArrowDown, ArrowUp, GripVertical, Plus, Trash2 } from 'lucide-react';
import { Fragment, useState } from 'react';
import type { DragEvent, ReactNode } from 'react';
import type { BranchStep, Condition, ConditionalAction, LeafCondition, StepAction, VariableValue } from './contracts';
import { ActionPickerField } from './ActionPicker';
import { IntegerField, SelectField } from './controls';
import { conditionKey, conditionKeys, conditionLabel, conditionValue, conditionalSteps, defaultCondition, defaultStepAction } from './action-model';
import type { ConditionKey } from './action-model';
import { defaultBindingAction } from './action-picker-model';
import {
  canMoveCondition, canMoveStep, flowIssue, flowLimits, groupItems, groupMode, groupWith, insertCondition, insertStep, isGroup, moveCondition, moveStep, removeCondition,
  removeStep, replaceCondition, replaceStep, setGroupMode, setWhen, variableKind, variableLimits,
} from './flow-model';
import type { Branch, CombineMode, ConditionGroup, ConditionPath, ListAddress, StepAddress } from './flow-model';
import { VariableSelect, VariableValueField } from './VariableFields';
import { useVariables } from './variables-context';

// The if / else editor: a tree of conditions (AND and OR groups that nest) and, below, the Then and Otherwise lists, whose
// steps may be if / else blocks of their own. Every block can be dragged by its grip to another place in the same tree, or
// moved with the arrow buttons (the keyboard way). All edits go through flow-model.ts, which refuses what Rust would reject.

export interface FlowRenderers {
  // The parameter fields of a plain step (the same editor a multi-action step uses).
  step(args: { prefix: string; step: StepAction; inputKey: string; onChange(step: StepAction, field?: string): void }): ReactNode;
  // The application field of a "front app is" test.
  app(args: { inputId: string; bundleId: string; onChange(bundleId: string): void }): ReactNode;
}

type Via = ListAddress['via'];
type Dragging = { kind: 'condition'; owner: string; path: ConditionPath } | { kind: 'step'; address: StepAddress } | null;

const viaKey = (via: Via): string => via.map(hop => `${hop.branch}${hop.index}`).join('/');
// Ids of a nested block start with the steps that lead to it: `then-2-` for the if / else in the second Then step.
const blockBase = (via: Via): string => via.map(hop => `${hop.branch}-${hop.index + 1}-`).join('');
const pathSuffix = (path: ConditionPath): string => (path.length === 0 ? '' : `-${path.map(index => index + 1).join('-')}`);
const sameStep = (a: StepAddress, b: StepAddress): boolean =>
  a.index === b.index && a.list.branch === b.list.branch && a.list.via.length === b.list.via.length && a.list.via.every((hop, at) => hop.branch === b.list.via[at].branch && hop.index === b.list.via[at].index);

interface Context {
  root: ConditionalAction;
  selectorKey: string;
  render: FlowRenderers;
  dragging: Dragging;
  setDragging(next: Dragging): void;
  apply(next: ConditionalAction | null, field?: string): void;
}

// The thin target between two blocks: it lights up while a block that may go there is dragged over it.
function Slot({ allowed, active, onDrop }: { allowed: boolean; active: boolean; onDrop(): void }) {
  const [over, setOver] = useState(false);
  return <li className={`flow-slot${active ? ' flow-slot-active' : ''}${over ? ' flow-slot-over' : ''}`} aria-hidden="true"
    onDragOver={event => { if (!allowed) return; event.preventDefault(); event.dataTransfer.dropEffect = 'move'; setOver(true); }}
    onDragLeave={() => setOver(false)}
    onDrop={event => { event.preventDefault(); setOver(false); if (allowed) onDrop(); }} />;
}

// The whole header row starts a drag (its grip icon only shows that it can), so the label and the empty space drag too.
// Buttons and selects inside it still work as usual.
// The whole block is the drag picture.
function dragHead(onStart: () => void, onEnd: () => void) {
  return {
    draggable: true,
    onDragStart: (event: DragEvent<HTMLDivElement>) => {
      const row = event.currentTarget.closest('li');
      event.dataTransfer.effectAllowed = 'move';
      event.dataTransfer.setData('application/x-open-stream-hub-flow', 'block');
      if (row) event.dataTransfer.setDragImage(row, 12, 12);
      onStart();
    },
    onDragEnd: onEnd,
  };
}

function Grip({ label }: { label: string }) {
  return <span className="flow-grip" title={label} aria-label={label} role="img"><GripVertical size={16} aria-hidden="true" /></span>;
}

export function FlowEditor({ action, selectorKey, onChange, render, obsNote }: {
  action: ConditionalAction; selectorKey: string; onChange(next: ConditionalAction, field?: string): void; render: FlowRenderers; obsNote?: ReactNode;
}) {
  const [dragging, setDragging] = useState<Dragging>(null);
  const context: Context = { root: action, selectorKey, render, dragging, setDragging, apply: (next, field) => { if (next) onChange(next, field); } };
  const issue = flowIssue(action);
  return <div className={`action-fields flow-editor${dragging ? ' flow-dragging' : ''}`}>
    <FlowBlock context={context} via={[]} depth={1} />
    {issue && <p className="field-error" role="alert">{issue}</p>}
    <p className="fine-print">Read when the key is pressed: if {conditionLabel(action.when)}. Every test is read{usesObs(action.when) ? ', and OBS must be running and set up' : ''}; if one cannot be read, the press fails and neither list runs. Drag a block by its grip, or use the arrows, to reorder it or move it into another group or list.</p>
    <p className="fine-print">Each list runs up to {conditionalSteps.maximum} steps in order, 280 ms apart; the first failed step stops the rest of that list. If / else blocks nest {flowLimits.conditionalDepth} levels deep, with {flowLimits.steps} steps in all, and groups of tests nest {flowLimits.conditionDepth} levels deep with {flowLimits.conditionLeaves} tests in all.</p>
    {obsNote}
  </div>;
}

const usesObs = (when: Condition): boolean => (isGroup(when) ? groupItems(when).some(usesObs) : conditionKey(when).startsWith('obs_'));

// One if / else: its tests, its Then list and its Otherwise list. A nested block is drawn inside the step that holds it.
function FlowBlock({ context, via, depth }: { context: Context; via: Via; depth: number }) {
  const owner = viaKey(via);
  const base = blockBase(via);
  const block = blockAt(context.root, via);
  if (!block) return null;
  return <div className={`flow-block${depth > 1 ? ' flow-nested' : ''}`} data-depth={depth}>
    <ConditionTree context={context} via={via} owner={owner} base={base} when={block.when} />
    <FlowList context={context} list={{ via, branch: 'then' }} depth={depth} title="Then" base={base} />
    <FlowList context={context} list={{ via, branch: 'else' }} depth={depth} title="Otherwise (optional)" base={base} />
  </div>;
}

function blockAt(root: ConditionalAction, via: Via): ConditionalAction | undefined {
  let node: ConditionalAction = root;
  for (const hop of via) {
    const next: BranchStep | undefined = (hop.branch === 'then' ? node.then : node.else ?? [])[hop.index];
    if (!next || next.type !== 'conditional') return undefined;
    node = next;
  }
  return node;
}

// ---------------------------------------------------------------------------------------------------------------------
// Conditions

function ConditionTree({ context, via, owner, base, when }: { context: Context; via: Via; owner: string; base: string; when: Condition }) {
  const variables = useVariables();
  const { apply, root, dragging, setDragging } = context;
  const edit = (next: Condition | null, field?: string) => apply(next ? setWhen(root, via, next) : null, field);
  const dragCondition = dragging?.kind === 'condition' && dragging.owner === owner ? dragging : null;
  // What "Add test" and "Add group" would produce, or null when the limits forbid it (the button is then disabled).
  const plus = (groupPath: ConditionPath, group: ConditionGroup | null) => {
    const size = group ? groupItems(group).length : 1;
    const other: CombineMode = group && groupMode(group) === 'any' ? 'all' : 'any';
    const pair = [defaultCondition('output_muted'), defaultCondition('output_muted')];
    const fresh: Condition = other === 'all' ? { all: pair } : { any: pair };
    return { test: insertCondition(when, groupPath, size, defaultCondition('output_muted')), group: insertCondition(when, groupPath, size, fresh), other };
  };

  // Plain functions, not components: a component defined here would be a new type on every render and lose focus on each keystroke.
  const leaf = (node: LeafCondition, path: ConditionPath, parent: ConditionPath, label: string) => {
    const key = conditionKey(node);
    const value = conditionValue(node);
    const suffix = pathSuffix(path);
    const id = (name: string) => `${base}conditional-${name}${suffix}`;
    const replace = (next: LeafCondition, field: string) => edit(replaceCondition(when, path, next), id(field));
    const index = path[path.length - 1] ?? 0;
    return <li key={path.join('.')} className={`flow-condition${dragCondition && sameIndices(dragCondition.path, path) ? ' flow-dragged' : ''}`}>
      <div className="flow-row-head" {...(path.length > 0 ? dragHead(() => setDragging({ kind: 'condition', owner, path }), () => setDragging(null)) : {})}>
        {path.length > 0 && <Grip label="Drag this test to move it" />}
        <label htmlFor={id('when')}>{label}</label>
        {path.length > 0 && <ConditionButtons when={when} path={path} index={index} parent={parent} edit={edit} what="test" />}
      </div>
      <SelectField id={id('when')} value={key} onChange={event => replace(defaultCondition(event.target.value as ConditionKey, variables), 'when')}>
        {conditionKeys.map(([kind, text]) => <option key={kind} value={kind}>{text}</option>)}
      </SelectField>
      {typeof value === 'object'
        ? <VariableTest idBase={id('variable')} node={node as VariableLeaf} onChange={next => replace(next, 'variable')} />
        : typeof value === 'string'
        ? <>{context.render.app({ inputId: id('app'), bundleId: value, onChange: bundleId => replace({ focused_app: bundleId }, 'app') })}
          <p className="fine-print">Compared with the front app's bundle ID when the key is pressed, without regard to letter case. To act in every other app, swap the two lists.</p></>
        : typeof value === 'number'
        ? <><label className="spaced-label" htmlFor={id('volume')}>Volume (%)</label>
          <IntegerField key={`${context.selectorKey}:${owner}:${path.join('.')}`} id={id('volume')} value={value} minimum={0} maximum={100} onCommit={percent => replace({ output_volume_at_least: percent }, 'volume')} /></>
        : <><label className="spaced-label" htmlFor={id('value')}>The statement is</label>
          <SelectField id={id('value')} value={value ? 'true' : 'false'} onChange={event => replace({ [key]: event.target.value === 'true' } as LeafCondition, 'value')}>
            <option value="true">True</option><option value="false">False</option>
          </SelectField></>}
    </li>;
  };

  const group = (node: ConditionGroup, path: ConditionPath, isRoot: boolean) => {
    const items = groupItems(node);
    const mode = groupMode(node);
    const modeId = `${base}conditional-mode${pathSuffix(path)}`;
    const parent = path.slice(0, -1);
    const slot = (at: number) => {
      const allowed = !!dragCondition && canMoveCondition(when, dragCondition.path, path, at);
      return <Slot key={`slot-${at}`} allowed={allowed} active={!!dragCondition && allowed}
        onDrop={() => { if (dragCondition) { edit(moveCondition(when, dragCondition.path, path, at), `${base}conditional-add`); setDragging(null); } }} />;
    };
    return <div className={`flow-group${isRoot ? ' flow-group-root' : ''}`} role="group" aria-label={`${mode === 'all' ? 'All of these' : 'Any of these'}${isRoot ? '' : ' (group)'}`}>
      <div className="flow-row-head" {...(isRoot ? {} : dragHead(() => setDragging({ kind: 'condition', owner, path }), () => setDragging(null)))}>
        {!isRoot && <Grip label="Drag this group to move it" />}
        <label htmlFor={modeId}>If</label>
        <SelectField id={modeId} value={mode} onChange={event => edit(setGroupMode(when, path, event.target.value as CombineMode), modeId)}>
          <option value="all">All of these hold (AND)</option><option value="any">Any one holds (OR)</option>
        </SelectField>
        {!isRoot && <ConditionButtons when={when} path={path} index={path[path.length - 1]} parent={parent} edit={edit} what="group" />}
      </div>
      <ol className="flow-members">
        {items.map((item, at) => <Fragment key={at}>
          {slot(at)}
          {isGroup(item)
            ? <li className={`flow-condition flow-condition-group${dragCondition && sameIndices(dragCondition.path, [...path, at]) ? ' flow-dragged' : ''}`}>{group(item, [...path, at], false)}</li>
            : leaf(item, [...path, at], path, at === 0 ? 'Test' : mode === 'all' ? 'And' : 'Or')}
        </Fragment>)}
        {slot(items.length)}
      </ol>
      {(() => {
        const next = plus(path, node);
        return <div className="flow-actions">
          <button type="button" id={isRoot ? `${base}conditional-add` : undefined} className="button compact" disabled={!next.test} onClick={() => edit(next.test, `${base}conditional-add`)}><Plus size={16} />Add test</button>
          <button type="button" className="button compact" disabled={!next.group} onClick={() => edit(next.group, `${base}conditional-add-group`)}><Plus size={16} />Add {next.other === 'all' ? 'AND' : 'OR'} group</button>
        </div>;
      })()}
    </div>;
  };

  if (isGroup(when)) return group(when, [], true);
  return <div className="flow-group flow-group-root flow-single">
    <ol className="flow-members">{leaf(when, [], [], 'If')}</ol>
    {(() => {
      const next = plus([], null);
      return <div className="flow-actions">
        <button type="button" id={`${base}conditional-add`} className="button compact" disabled={!next.test} onClick={() => edit(next.test, `${base}conditional-add`)}><Plus size={16} />Add test</button>
        <button type="button" className="button compact" disabled={!next.group} onClick={() => edit(next.group, `${base}conditional-add-group`)}><Plus size={16} />Add OR group</button>
      </div>;
    })()}
  </div>;
}

const sameIndices = (a: ConditionPath, b: ConditionPath): boolean => a.length === b.length && a.every((index, at) => index === b[at]);

// Move up, move down and Remove for a test or group: the keyboard way to do what the grip does.
function ConditionButtons({ when, path, index, parent, edit, what }: {
  when: Condition; path: ConditionPath; index: number; parent: ConditionPath; edit(next: Condition | null, field?: string): void; what: 'test' | 'group';
}) {
  const up = moveCondition(when, path, parent, index - 1);
  const down = moveCondition(when, path, parent, index + 2);
  const removed = removeCondition(when, path);
  return <span className="flow-buttons">
    <button type="button" className="button compact flow-icon" disabled={index === 0 || !up} aria-label={`Move this ${what} up`} title="Move up" onClick={() => edit(up)}><ArrowUp size={15} /></button>
    <button type="button" className="button compact flow-icon" disabled={!down} aria-label={`Move this ${what} down`} title="Move down" onClick={() => edit(down)}><ArrowDown size={15} /></button>
    <button type="button" className="button compact flow-icon" disabled={!removed} aria-label={`Remove this ${what}`} title="Remove" onClick={() => edit(removed)}><Trash2 size={15} /></button>
  </span>;
}

// "A variable is …" and "A number variable is at least …": the variable, and the value it is compared with.
type VariableLeaf = Extract<LeafCondition, { variable_is: unknown } | { variable_at_least: unknown }>;
function VariableTest({ idBase, node, onChange }: { idBase: string; node: VariableLeaf; onChange(next: LeafCondition): void }) {
  const variables = useVariables();
  if ('variable_at_least' in node) {
    const { name, value } = node.variable_at_least;
    return <>
      <VariableSelect id={`${idBase}-name`} label="Number variable" spaced value={name} kinds={['number']} onChange={next => onChange({ variable_at_least: { name: next, value } })} />
      <label className="spaced-label" htmlFor={`${idBase}-value`}>At least</label>
      <IntegerField id={`${idBase}-value`} value={value} minimum={variableLimits.minimum} maximum={variableLimits.maximum} onCommit={next => onChange({ variable_at_least: { name, value: next } })} />
    </>;
  }
  const { name, value } = node.variable_is;
  return <>
    <VariableSelect id={`${idBase}-name`} label="Variable" spaced value={name} kinds={['bool', 'number', 'text']}
      onChange={next => { const found = variables.find(variable => variable.name === next); onChange({ variable_is: { name: next, value: found ? found.value : value } }); }} />
    <VariableValueField id={`${idBase}-value`} label="Is equal to" spaced value={value as VariableValue} onChange={next => onChange({ variable_is: { name, value: next } })} />
    <p className="fine-print">A {variableKind(value)} variable is compared with a {variableKind(value)} value.</p>
  </>;
}

// ---------------------------------------------------------------------------------------------------------------------
// Steps

function FlowList({ context, list, depth, title, base }: { context: Context; list: ListAddress; depth: number; title: string; base: string }) {
  const { root, apply, dragging, setDragging, render, selectorKey } = context;
  const steps = (list.branch === 'then' ? blockAt(root, list.via)?.then : blockAt(root, list.via)?.else) ?? [];
  const listBase = `${base}${list.branch}-`;
  const dragStep = dragging?.kind === 'step' ? dragging : null;
  const noun = list.branch === 'then' ? 'Then step' : 'Otherwise step';
  const at = (index: number): StepAddress => ({ list, index });
  const slot = (index: number) => {
    const allowed = !!dragStep && canMoveStep(root, dragStep.address, list, index);
    return <Slot key={`slot-${index}`} allowed={allowed} active={allowed}
      onDrop={() => { if (dragStep) { apply(moveStep(root, dragStep.address, list, index), `${listBase}add`); setDragging(null); } }} />;
  };
  const addStep = insertStep(root, list, steps.length, defaultStepAction('toggle_mute'));
  return <>
    <h4 className="spaced-label">{title}</h4>
    <ol className="action-steps flow-steps" aria-label={title}>
      {steps.map((step, index) => {
        const prefix = `${listBase}step-${index + 1}-`;
        const address = at(index);
        const up = moveStep(root, address, list, index - 1);
        const down = moveStep(root, address, list, index + 2);
        const removed = removeStep(root, address);
        const nestedVia: Via = [...list.via, { branch: list.branch as Branch, index }];
        return <Fragment key={index}>
          {slot(index)}
          <li className={dragStep && sameStep(dragStep.address, address) ? 'flow-dragged' : undefined}>
            <div className="flow-row-head" {...dragHead(() => setDragging({ kind: 'step', address }), () => setDragging(null))}>
              <Grip label="Drag this step to move it" />
              <label htmlFor={`${prefix}action`}>{noun} {index + 1}</label>
              <span className="flow-buttons">
                <button type="button" className="button compact flow-icon" disabled={index === 0 || !up} aria-label={`Move ${noun.toLowerCase()} ${index + 1} up`} title="Move up" onClick={() => apply(up)}><ArrowUp size={15} /></button>
                <button type="button" className="button compact flow-icon" disabled={!down} aria-label={`Move ${noun.toLowerCase()} ${index + 1} down`} title="Move down" onClick={() => apply(down)}><ArrowDown size={15} /></button>
                <button type="button" className="button compact flow-icon" disabled={!removed} aria-label={`Remove ${noun.toLowerCase()} ${index + 1}`} title="Remove" onClick={() => apply(removed)}><Trash2 size={15} /></button>
              </span>
            </div>
            <ActionPickerField triggerId={`${prefix}action`} idPrefix={prefix} scope="branch" action={step} allowDrag={false}
              onAssign={type => { if (type) apply(replaceStep(root, address, defaultBindingAction(type) as BranchStep)); }} />
            {step.type === 'conditional'
              ? <FlowBlock context={context} via={nestedVia} depth={depth + 1} />
              : render.step({ prefix, step, inputKey: `${selectorKey}:${listBase}${index}:${steps.length}`, onChange: (next, field) => apply(replaceStep(root, address, next), field && `${prefix}${field}`) })}
          </li>
        </Fragment>;
      })}
      {slot(steps.length)}
    </ol>
    <button type="button" id={`${listBase}add`} className="button compact" disabled={!addStep} onClick={() => apply(addStep)}><Plus size={16} />Add {noun.toLowerCase()}</button>
  </>;
}
