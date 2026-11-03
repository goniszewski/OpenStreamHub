import { Plus, Trash2 } from 'lucide-react';
import type { StepAction, Variable, VariableKind, VariableValue } from './contracts';
import { IntegerField, SelectField } from './controls';
import {
  defaultVariableValue, freshVariableName, variableKind, variableKindLabels, variableLimits, variableNameIssue, variablesIssue, variableValueIssue,
} from './flow-model';
import type { BindingEntry } from './binding-model';
import { useT } from './i18n-react';
import { variablesOfKind } from './variable-model';
import { useVariables } from './variables-context';

// Opens the inspector's Variables section, brings it into view and puts the focus on Add variable.
function showVariablesPanel() {
  const section = document.querySelector<HTMLDetailsElement>('.inspector-variables');
  if (!section) return;
  section.open = true;
  section.scrollIntoView({ block: 'nearest', behavior: 'smooth' });
  document.getElementById('variable-add')?.focus({ preventScroll: true });
}

// The variable choices of an action or a test: the declared variables of the kinds it can use. A name the profile no
// longer declares stays selectable (marked) so the editor never silently swaps it; saving says it is not declared.
export function VariableSelect({ id, label, value, kinds, onChange, spaced }: {
  id: string; label: string; value: string; kinds: ReadonlyArray<VariableKind>; onChange(name: string): void; spaced?: boolean;
}) {
  const t = useT();
  const variables = variablesOfKind(useVariables(), kinds);
  const known = variables.some(variable => variable.name === value);
  return <>
    <label className={spaced ? 'spaced-label' : undefined} htmlFor={id}>{label}</label>
    <SelectField id={id} value={value} onChange={event => onChange(event.target.value)}>
      {value === '' && <option value="" disabled>{t('variables.choose')}</option>}
      {value !== '' && !known && <option value={value}>{t('variables.notDeclared', { name: value })}</option>}
      {variables.map(variable => <option key={variable.name} value={variable.name}>{variable.name}</option>)}
    </SelectField>
    {variables.length === 0 && <p className="fine-print" role="note">{t(kinds.length === 1 && kinds[0] === 'number' ? 'variables.noneNumber' : kinds.length === 1 && kinds[0] === 'bool' ? 'variables.noneBool' : 'variables.none')}{' '}
      <button type="button" className="link-button" onClick={showVariablesPanel}>{t('variables.addOne')}</button>.</p>}
  </>;
}

// One value of the kind it already has: an on / off select, a whole number or a line of text.
export function VariableValueField({ id, label, value, onChange, spaced }: {
  id: string; label: string; value: VariableValue; onChange(value: VariableValue): void; spaced?: boolean;
}) {
  const t = useT();
  const issue = variableValueIssue(value);
  return <>
    <label className={spaced ? 'spaced-label' : undefined} htmlFor={id}>{label}</label>
    {typeof value === 'boolean'
      ? <SelectField id={id} value={value ? 'on' : 'off'} onChange={event => onChange(event.target.value === 'on')}><option value="on">{t('variables.on')}</option><option value="off">{t('variables.off')}</option></SelectField>
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
  const t = useT();
  const variables = useVariables();
  const id = (name: string) => `${prefix}${name}`;
  const kind = kindOfName(variables, action.name);
  switch (action.type) {
    case 'set_variable':
      return <div className="action-fields">
        <VariableSelect id={id('variable-name')} label={t('variables.variable')} value={action.name} kinds={['bool', 'number', 'text']}
          onChange={name => {
            const next = variables.find(variable => variable.name === name);
            // The value takes the variable's kind, starting from its declared value.
            onChange({ type: 'set_variable', name, value: next ? next.value : action.value }, 'variable-name');
          }} />
        {action.name !== '' && <VariableValueField id={id('variable-value')} label={t('variables.setTo')} spaced value={kind && kind !== variableKind(action.value) ? defaultVariableValue(kind) : action.value}
          onChange={value => onChange({ type: 'set_variable', name: action.name, value }, 'variable-value')} />}
        <p className="fine-print">{t('variables.setNote')}</p>
      </div>;
    case 'toggle_variable':
      return <div className="action-fields">
        <VariableSelect id={id('variable-name')} label={t('variables.boolVariable')} value={action.name} kinds={['bool']} onChange={name => onChange({ type: 'toggle_variable', name }, 'variable-name')} />
        <p className="fine-print">{t('variables.toggleNote')}</p>
      </div>;
    case 'add_to_variable':
      return <div className="action-fields">
        <VariableSelect id={id('variable-name')} label={t('variables.numberVariable')} value={action.name} kinds={['number']} onChange={name => onChange({ type: 'add_to_variable', name, amount: action.amount }, 'variable-name')} />
        <label className="spaced-label" htmlFor={id('variable-amount')}>{t('variables.amount')}</label>
        <IntegerField id={id('variable-amount')} value={action.amount} minimum={variableLimits.minimum} maximum={variableLimits.maximum} nonZero
          onCommit={amount => onChange({ type: 'add_to_variable', name: action.name, amount }, 'variable-amount')} />
        <p className="fine-print">{t('variables.limitNote', { min: variableLimits.minimum.toLocaleString('en-US'), max: variableLimits.maximum.toLocaleString('en-US') })}</p>
      </div>;
  }
}

// The profile's variables: name, kind and starting value. Values are kept across key presses while the service runs and
// start again from here when it starts. Each module's service has its own.
export function VariablesPanel({ variables, uses, users, onShow, disabled, onChange, onRename }: {
  variables: ReadonlyArray<Variable>; uses: ReadonlyMap<string, number>; users?: (name: string) => BindingEntry[]; onShow?: (entry: BindingEntry) => void; disabled: boolean;
  onChange(next: Variable[], field: string): void; onRename(from: string, to: string): void;
}) {
  const t = useT();
  const issue = variablesIssue(variables);
  const update = (index: number, next: Variable, field: string) => onChange(variables.map((variable, at) => (at === index ? next : variable)), field);
  return <section className="variables-panel" aria-labelledby="variables-heading">
    <h3 id="variables-heading">{t('variables.heading')}</h3>
    <p className="fine-print">{t('variables.intro')}</p>
    <ul className="variables-list">
      {variables.map((variable, index) => {
        const kind = variableKind(variable.value);
        const nameIssue = variableNameIssue(variable.name);
        const used = uses.get(variable.name) ?? 0;
        return <li key={index}>
          <label htmlFor={`variable-${index}-name`}>{t('variables.name')}</label>
          <input id={`variable-${index}-name`} spellCheck={false} disabled={disabled} value={variable.name} aria-invalid={!!nameIssue} aria-describedby={nameIssue ? `variable-${index}-name-error` : undefined}
            onChange={event => (used > 0 && !nameIssue ? onRename(variable.name, event.target.value) : update(index, { ...variable, name: event.target.value }, `variable-${index}-name`))} />
          {nameIssue && <p id={`variable-${index}-name-error`} className="field-error" role="alert">{nameIssue}</p>}
          <label className="spaced-label" htmlFor={`variable-${index}-kind`}>{t('variables.kind')}</label>
          <SelectField id={`variable-${index}-kind`} value={kind} disabled={disabled || used > 0}
            onChange={event => update(index, { ...variable, value: defaultVariableValue(event.target.value as VariableKind) }, `variable-${index}-kind`)}>
            {(Object.keys(variableKindLabels) as VariableKind[]).map(item => <option key={item} value={item}>{variableKindLabels[item]}</option>)}
          </SelectField>
          <VariableValueField id={`variable-${index}-value`} label={t('variables.startsAs')} spaced value={variable.value} onChange={value => update(index, { ...variable, value }, `variable-${index}-value`)} />
          <p className="fine-print">{used === 0 ? t('variables.unused') : `${users && onShow ? t('variables.usedBy', { count: users(variable.name).length }) : t('variables.used', { count: used })} ${t('variables.usedTail')}`}</p>
          {used > 0 && users && onShow && <ul className="variable-users" aria-label={t('variables.usersList')}>
            {users(variable.name).map(entry => <li key={entry.id}>
              <button type="button" className="link-button" onClick={() => onShow(entry)}>{entry.title}</button>
              <span className="fine-print"> {entry.actionText}</span>
            </li>)}
          </ul>}
          <button type="button" className="button compact" disabled={disabled || used > 0} aria-label={t('variables.removeNamed', { name: variable.name || index + 1 })}
            onClick={() => onChange(variables.filter((_, at) => at !== index), 'variable-remove')}><Trash2 size={16} />{t('variables.remove')}</button>
        </li>;
      })}
    </ul>
    {issue && <p className="field-error" role="alert">{issue}</p>}
    <button type="button" id="variable-add" className="button compact" disabled={disabled || variables.length >= variableLimits.count}
      onClick={() => onChange([...variables, { name: freshVariableName(variables), value: false }], 'variable-add')}><Plus size={16} />{t('variables.add')}</button>
  </section>;
}
