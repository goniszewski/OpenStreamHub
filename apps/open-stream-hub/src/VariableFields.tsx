import { Plus, Trash2 } from 'lucide-react';
import type { StepAction, Variable, VariableKind, VariableValue } from './contracts';
import { IntegerField, SelectField } from './controls';
import {
  defaultVariableValue, freshVariableName, variableKind, variableKindLabels, variableLimits, variableNameIssue, variablesIssue, variableValueIssue,
} from './flow-model';
import { variablesOfKind } from './variable-model';
import { useVariables } from './variables-context';

// The variable choices of an action or a test: the declared variables of the kinds it can use. A name the profile no
// longer declares stays selectable (marked) so the editor never silently swaps it; saving says it is not declared.
export function VariableSelect({ id, label, value, kinds, onChange, spaced }: {
  id: string; label: string; value: string; kinds: ReadonlyArray<VariableKind>; onChange(name: string): void; spaced?: boolean;
}) {
  const variables = variablesOfKind(useVariables(), kinds);
  const known = variables.some(variable => variable.name === value);
  return <>
    <label className={spaced ? 'spaced-label' : undefined} htmlFor={id}>{label}</label>
    <SelectField id={id} value={value} onChange={event => onChange(event.target.value)}>
      {value === '' && <option value="" disabled>Choose a variable</option>}
      {value !== '' && !known && <option value={value}>{value} (not declared)</option>}
      {variables.map(variable => <option key={variable.name} value={variable.name}>{variable.name}</option>)}
    </SelectField>
    {variables.length === 0 && <p className="fine-print" role="note">No {kinds.length === 1 && kinds[0] === 'number' ? 'number' : kinds.length === 1 && kinds[0] === 'bool' ? 'on / off' : ''} variable is declared yet. Add one under Variables in the inspector.</p>}
  </>;
}

// One value of the kind it already has: an on / off select, a whole number or a line of text.
export function VariableValueField({ id, label, value, onChange, spaced }: {
  id: string; label: string; value: VariableValue; onChange(value: VariableValue): void; spaced?: boolean;
}) {
  const issue = variableValueIssue(value);
  return <>
    <label className={spaced ? 'spaced-label' : undefined} htmlFor={id}>{label}</label>
    {typeof value === 'boolean'
      ? <SelectField id={id} value={value ? 'on' : 'off'} onChange={event => onChange(event.target.value === 'on')}><option value="on">On</option><option value="off">Off</option></SelectField>
      : typeof value === 'number'
      ? <IntegerField id={id} value={value} minimum={variableLimits.minimum} maximum={variableLimits.maximum} onCommit={onChange} />
      : <input id={id} spellCheck={false} value={value} aria-invalid={!!issue} aria-describedby={issue ? `${id}-error` : undefined} onChange={event => onChange(event.target.value)} />}
    {issue && typeof value === 'string' && <p id={`${id}-error`} className="field-error" role="alert">{issue}</p>}
  </>;
}

// The kind of the variable an action names, looked up in the declarations (bool when it is not declared).
const kindOfName = (variables: ReadonlyArray<Variable>, name: string): VariableKind | null => {
  const found = variables.find(variable => variable.name === name);
  return found ? variableKind(found.value) : null;
};

type VariableAction = Extract<StepAction, { type: 'set_variable' | 'toggle_variable' | 'add_to_variable' }>;

// Set, flip or add: the parameter fields of the three variable actions.
export function VariableActionFields({ prefix, action, onChange }: { prefix: string; action: VariableAction; onChange(action: StepAction, field?: string): void }) {
  const variables = useVariables();
  const id = (name: string) => `${prefix}${name}`;
  const kind = kindOfName(variables, action.name);
  switch (action.type) {
    case 'set_variable':
      return <div className="action-fields">
        <VariableSelect id={id('variable-name')} label="Variable" value={action.name} kinds={['bool', 'number', 'text']}
          onChange={name => {
            const next = variables.find(variable => variable.name === name);
            // The value takes the variable's kind, starting from its declared value.
            onChange({ type: 'set_variable', name, value: next ? next.value : action.value }, 'variable-name');
          }} />
        <VariableValueField id={id('variable-value')} label="Set it to" spaced value={kind && kind !== variableKind(action.value) ? defaultVariableValue(kind) : action.value}
          onChange={value => onChange({ type: 'set_variable', name: action.name, value }, 'variable-value')} />
        <p className="fine-print">Replaces the value. It is kept until the service stops; each module's service keeps its own.</p>
      </div>;
    case 'toggle_variable':
      return <div className="action-fields">
        <VariableSelect id={id('variable-name')} label="On / off variable" value={action.name} kinds={['bool']} onChange={name => onChange({ type: 'toggle_variable', name }, 'variable-name')} />
        <p className="fine-print">Flips it from on to off or from off to on.</p>
      </div>;
    case 'add_to_variable':
      return <div className="action-fields">
        <VariableSelect id={id('variable-name')} label="Number variable" value={action.name} kinds={['number']} onChange={name => onChange({ type: 'add_to_variable', name, amount: action.amount }, 'variable-name')} />
        <label className="spaced-label" htmlFor={id('variable-amount')}>Add (negative to subtract)</label>
        <IntegerField id={id('variable-amount')} value={action.amount} minimum={variableLimits.minimum} maximum={variableLimits.maximum} nonZero
          onCommit={amount => onChange({ type: 'add_to_variable', name: action.name, amount }, 'variable-amount')} />
        <p className="fine-print">A result past {variableLimits.minimum.toLocaleString('en-US')} or {variableLimits.maximum.toLocaleString('en-US')} stops at that limit.</p>
      </div>;
  }
}

// The profile's variables: name, kind and starting value. Values are kept across key presses while the service runs and
// start again from here when it starts. Each module's service has its own.
export function VariablesPanel({ variables, uses, disabled, onChange, onRename }: {
  variables: ReadonlyArray<Variable>; uses: ReadonlyMap<string, number>; disabled: boolean;
  onChange(next: Variable[], field: string): void; onRename(from: string, to: string): void;
}) {
  const issue = variablesIssue(variables);
  const update = (index: number, next: Variable, field: string) => onChange(variables.map((variable, at) => (at === index ? next : variable)), field);
  return <section className="variables-panel" aria-labelledby="variables-heading">
    <h3 id="variables-heading">Variables</h3>
    <p className="fine-print">Remember a state between key presses: an on / off flag, a counter or a word. Actions set them and If / else blocks test them. They start from the values below when a run starts.</p>
    <ul className="variables-list">
      {variables.map((variable, index) => {
        const kind = variableKind(variable.value);
        const nameIssue = variableNameIssue(variable.name);
        const used = uses.get(variable.name) ?? 0;
        return <li key={index}>
          <label htmlFor={`variable-${index}-name`}>Name</label>
          <input id={`variable-${index}-name`} spellCheck={false} disabled={disabled} value={variable.name} aria-invalid={!!nameIssue} aria-describedby={nameIssue ? `variable-${index}-name-error` : undefined}
            onChange={event => (used > 0 && !nameIssue ? onRename(variable.name, event.target.value) : update(index, { ...variable, name: event.target.value }, `variable-${index}-name`))} />
          {nameIssue && <p id={`variable-${index}-name-error`} className="field-error" role="alert">{nameIssue}</p>}
          <label className="spaced-label" htmlFor={`variable-${index}-kind`}>Kind</label>
          <SelectField id={`variable-${index}-kind`} value={kind} disabled={disabled || used > 0}
            onChange={event => update(index, { ...variable, value: defaultVariableValue(event.target.value as VariableKind) }, `variable-${index}-kind`)}>
            {(Object.keys(variableKindLabels) as VariableKind[]).map(item => <option key={item} value={item}>{variableKindLabels[item]}</option>)}
          </SelectField>
          <VariableValueField id={`variable-${index}-value`} label="Starts as" spaced value={variable.value} onChange={value => update(index, { ...variable, value }, `variable-${index}-value`)} />
          <p className="fine-print">{used === 0 ? 'Not used yet.' : `Used ${used} ${used === 1 ? 'time' : 'times'}. Remove those first to delete it or change its kind.`}</p>
          <button type="button" className="button compact" disabled={disabled || used > 0} aria-label={`Remove variable ${variable.name || index + 1}`}
            onClick={() => onChange(variables.filter((_, at) => at !== index), 'variable-remove')}><Trash2 size={16} />Remove</button>
        </li>;
      })}
    </ul>
    {issue && <p className="field-error" role="alert">{issue}</p>}
    <button type="button" id="variable-add" className="button compact" disabled={disabled || variables.length >= variableLimits.count}
      onClick={() => onChange([...variables, { name: freshVariableName(variables), value: false }], 'variable-add')}><Plus size={16} />Add variable</button>
  </section>;
}
