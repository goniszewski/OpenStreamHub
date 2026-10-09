import { useState } from 'react';
import { ArrowDown, ArrowUp, Play, Plus, Trash2 } from 'lucide-react';
import { ActionFields } from './ActionFields';
import type { Action, Binding, DialBinding, DialFunction, Profile, ReplayEvent } from './contracts';
import { SelectField } from './controls';
import {
  addDialFunction, deleteDialFunction, dialBindingAt, dialFunctionGestures, dialFunctionNameIssue, dialSerialIssue,
  editDialBinding, flatDialBindings, maxDialFunctionNameBytes, maxDialFunctions, moveDialFunction, moveFlatDialBindings,
  removeDialBinding, renameDialFunction,
} from './dial-function-model';
import type { ProfileSession, SessionState } from './profile-session';
import { actionLabel, errorMessage, gestureLabels } from './ui-model';

export const dialFunctionHeadingId = 'dial-functions-heading';
export const dialFunctionNameId = (index: number) => `dial-function-${index}-name`;

// Moves keyboard focus to a function's name field, for example from a binding list.
export function focusDialFunction(index: number) {
  document.getElementById(dialFunctionNameId(index))?.focus();
}

// One sentence describing a preview result for the given function.
export function dialPreviewText(event: ReplayEvent, functionName: string): string {
  const navigation = event.dial_function_switch;
  if (navigation) return `Short press switches from ${navigation.from_function} to ${navigation.to_function} (${navigation.index + 1} of ${navigation.count}). No action runs.`;
  const gesture = gestureLabels[event.gesture];
  const name = event.dial_function ?? functionName;
  return event.action ? `${name} · ${gesture}: would ${actionLabel(event.action).toLowerCase()}.` : `${name} · ${gesture}: no action is bound in this function.`;
}

// Text for any single-gesture preview: dial function results name the function
// (a short press shows the planned switch); other results keep the plain wording.
export function bindingPreviewText(event: ReplayEvent): string {
  if (event.dial_function !== undefined || event.dial_function_switch) return dialPreviewText(event, event.dial_function ?? '');
  return `${gestureLabels[event.gesture]}${event.value ? ` ${event.value.raw}/${event.value.maximum}` : ''}: ${event.action ? `Would ${actionLabel(event.action).toLowerCase()}.` : 'No action is bound to this event.'}`;
}

type DialGesture = DialBinding['gesture'];

// The edited function after moving function `index` by `offset`: the selection
// follows the function, whether it was the moved one or the neighbour it swapped with.
export function selectionAfterMove(selected: number, index: number, offset: -1 | 1): number {
  if (selected === index) return index + offset;
  if (selected === index + offset) return index;
  return selected;
}

const bindingNames = (bindings: readonly Binding[]) =>
  bindings.map(item => `${gestureLabels[item.gesture]}${item.serial_number ? ` (serial ${item.serial_number})` : ''}`).join(', ');
const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`;

export function DialFunctionsPanel({ profile, session, state, disabled, initialSelected = 0 }: {
  profile: Profile; session: ProfileSession; state: SessionState; disabled: boolean; initialSelected?: number;
}) {
  const functions = profile.dial_functions ?? [];
  const [chosen, setChosen] = useState(initialSelected);
  const [gesture, setGesture] = useState<DialGesture>('right');
  const [serial, setSerial] = useState('');
  const [previewed, setPreviewed] = useState<number | null>(null);
  // Undo, redo or a deletion can remove the chosen function.
  const selected = functions.length ? Math.min(Math.max(chosen, 0), functions.length - 1) : -1;
  const current: DialFunction | undefined = functions[selected];
  const flat = flatDialBindings(profile, selected >= 0 ? selected : undefined);
  const flatCount = flat.moved.length + flat.conflicting.length + flat.dropped.length;
  const currentName = current ? current.name.trim() || `function ${selected + 1}` : '';

  const run = (change: (latest: Profile) => Profile, key?: string) => {
    try { session.update(change, key); } catch (reason) { session.setError(errorMessage(reason)); }
  };
  const choose = (index: number) => { setChosen(index); setPreviewed(null); session.clearPreview(); };
  const serialIssue = dialSerialIssue(serial);
  const binding = current ? dialBindingAt(profile, selected, gesture, serial) : undefined;
  const preview = async (previewGesture: DialGesture | 'short_press') => {
    setPreviewed(selected);
    await session.previewBinding({ module: 'm131e', control: 0, gesture: previewGesture, serial, faderPosition: 0, dialFunction: selected });
  };
  const previewRunning = state.preview.status === 'running' || state.keypadPreview.status === 'running';

  return <section className="dial-functions" aria-labelledby={dialFunctionHeadingId}>
    <h3 id={dialFunctionHeadingId}>Dial functions <span className="small-label">{functions.length ? `${functions.length} of ${maxDialFunctions}` : 'Off'}</span></h3>
    <p className="fine-print">A short press on the encoder switches to the next function and never runs an action. Rotation and long press use the current function. A run always starts at the first function and returns to it when it restarts. The encoder display shows the current function while OpenStreamHub drives the display (the default) and the dial screen is set to show the function name.</p>
    {!functions.length ? <>
      <p className="empty-state">This profile has no dial functions. Encoder bindings work as before, including Short press.</p>
      <button type="button" className="button" disabled={disabled} onClick={() => { run(latest => addDialFunction(latest)); choose(0); }}><Plus size={17} />Add dial functions</button>
      {flatCount > 0 && <p className="fine-print">After adding functions, the {flatCount} existing encoder binding{flatCount === 1 ? '' : 's'} must move into a function or be removed before the profile can be saved.</p>}
    </> : <>
      {flatCount > 0 && current && <div className="binding-warnings" role="region" aria-label="Encoder bindings outside dial functions">
        <p>{flatCount} encoder binding{flatCount === 1 ? ' is' : 's are'} outside the dial functions and must move into one before the profile can be saved.</p>
        {flat.conflicting.length > 0 && <p>{plural(flat.conflicting.length, 'binding')} will be dropped because {currentName} already has the same gesture: {bindingNames(flat.conflicting)}.</p>}
        {flat.dropped.length > 0 && <p>{plural(flat.dropped.length, 'binding')} will be dropped because no function can hold {flat.dropped.length === 1 ? 'it' : 'them'}: {bindingNames(flat.dropped)}.</p>}
        <button type="button" className="button compact" disabled={disabled} onClick={() => run(latest => moveFlatDialBindings(latest, selected))}>{flat.moved.length ? `Move ${plural(flat.moved.length, 'encoder binding')} into ${currentName}` : 'Remove the encoder bindings outside functions'}</button>
        <p className="fine-print">Undo restores everything this removes.</p>
      </div>}
      <ol className="dial-function-list">{functions.map((item, index) => {
        const issue = dialFunctionNameIssue(item.name, functions, index);
        const label = item.name.trim() || `function ${index + 1}`;
        return <li key={index} className={index === selected ? 'selected' : ''}>
          <label htmlFor={dialFunctionNameId(index)}>Function {index + 1} name</label>
          <input id={dialFunctionNameId(index)} disabled={disabled} value={item.name} spellCheck={false} aria-invalid={!!issue} aria-describedby={issue ? `${dialFunctionNameId(index)}-error` : undefined}
            onFocus={() => { if (index !== selected) choose(index); }}
            onChange={event => run(latest => renameDialFunction(latest, index, event.target.value), `dial-function-name:${index}`)} />
          {issue && <p id={`${dialFunctionNameId(index)}-error`} className="field-error" role="alert">{issue}</p>}
          <small>{item.bindings.length ? `${item.bindings.length} binding${item.bindings.length === 1 ? '' : 's'}` : 'No bindings: the encoder does nothing in this function'}</small>
          <div className="dial-function-actions">
            <button type="button" className="button compact" aria-pressed={index === selected} onClick={() => choose(index)}>Edit bindings</button>
            <button type="button" className="icon-button" aria-label={`Move ${label} up`} title="Move up" disabled={disabled || index === 0} onClick={() => { run(latest => moveDialFunction(latest, index, -1)); setChosen(selectionAfterMove(selected, index, -1)); }}><ArrowUp size={16} /></button>
            <button type="button" className="icon-button" aria-label={`Move ${label} down`} title="Move down" disabled={disabled || index === functions.length - 1} onClick={() => { run(latest => moveDialFunction(latest, index, 1)); setChosen(selectionAfterMove(selected, index, 1)); }}><ArrowDown size={16} /></button>
            <button type="button" className="icon-button" aria-label={`Delete ${label}`} title={functions.length === 1 ? 'Delete the last function and turn dial functions off' : 'Delete function'} disabled={disabled} onClick={() => { run(latest => deleteDialFunction(latest, index)); if (index < selected) setChosen(selected - 1); }}><Trash2 size={16} /></button>
          </div>
        </li>;
      })}</ol>
      <button type="button" className="button compact" disabled={disabled || functions.length >= maxDialFunctions} onClick={() => { run(latest => addDialFunction(latest)); choose(functions.length); }}><Plus size={16} />Add function</button>
      {functions.length >= maxDialFunctions && <p className="fine-print">A profile can have at most {maxDialFunctions} dial functions.</p>}
      <p className="fine-print">Names must be unique, at most {maxDialFunctionNameBytes} bytes, without spaces at the start or end, and without control or invisible formatting characters. Deleting the last function turns dial functions off.</p>

      {current && <div className="dial-function-editor" role="group" aria-label={`Bindings of ${current.name.trim() || `function ${selected + 1}`}`}>
        <h4>{current.name.trim() || `Function ${selected + 1}`} bindings</h4>
        {current.bindings.length > 0 && <ul className="binding-list">{current.bindings.map((item, index) => <li key={`${item.gesture}:${item.serial_number ?? ''}`} className="binding-row"><div>
          <strong>{gestureLabels[item.gesture]}</strong><span>{actionLabel(item.action)}</span><small>{item.serial_number ? `Serial: ${item.serial_number}` : 'Any device serial'}</small></div>
          <button type="button" className="button compact" onClick={() => { setGesture(item.gesture); setSerial(item.serial_number ?? ''); }}>Edit</button>
          <button type="button" className="icon-button" aria-label={`Remove ${current.name} ${gestureLabels[item.gesture]} binding${item.serial_number ? ` for serial ${item.serial_number}` : ''}`} disabled={disabled} onClick={() => run(latest => removeDialBinding(latest, selected, index))}><Trash2 size={16} /></button>
        </li>)}</ul>}
        <label htmlFor="dial-binding-gesture">Gesture</label>
        <SelectField id="dial-binding-gesture" value={gesture} onChange={event => setGesture(event.target.value as DialGesture)}>
          {dialFunctionGestures.map(item => <option key={item} value={item}>{gestureLabels[item]}</option>)}
        </SelectField>
        <label className="spaced-label" htmlFor="dial-binding-serial">Device serial (optional)</label>
        <input id="dial-binding-serial" value={serial} spellCheck={false} placeholder="Any device serial" aria-invalid={!!serialIssue} aria-describedby={serialIssue ? 'dial-binding-serial-error' : undefined} onChange={event => setSerial(event.target.value)} />
        {serialIssue && <p id="dial-binding-serial-error" className="field-error" role="alert">{serialIssue}</p>}
        <fieldset disabled={disabled || !!serialIssue}>
          <ActionFields action={binding?.action ?? null} fader={false} selectorKey={`dial:${selected}:${gesture}:${serial}`} onChange={(action, field) => {
            if (action?.type === 'volume_from_position') { session.setError('Dial functions cannot follow a fader position.'); return; }
            run(latest => editDialBinding(latest, selected, gesture, serial, action as Action | null), field && `dial:${field}:${selected}:${gesture}:${serial}`);
          }} />
        </fieldset>
        <div className="dial-preview" role="group" aria-label={`Preview ${current.name.trim() || `function ${selected + 1}`}`}>
          <p className="small-label">Previewing function: {current.name.trim() || `Function ${selected + 1}`}</p>
          <div className="event-buttons">
            {dialFunctionGestures.map(item => <button key={item} type="button" className="button compact" disabled={disabled || previewRunning || !!serialIssue} onClick={() => { void preview(item); }}><Play size={15} />{gestureLabels[item]}</button>)}
            <button type="button" className="button compact" disabled={disabled || previewRunning || !!serialIssue} onClick={() => { void preview('short_press'); }}><Play size={15} />Short press</button>
          </div>
          {previewed === selected && state.preview.status !== 'idle' && <p role={state.preview.status === 'failed' ? 'alert' : 'status'} className="fine-print">
            {state.preview.status === 'running' ? 'Resolving…' : state.preview.status === 'failed' ? state.preview.error : dialPreviewText(state.preview.event , current.name)}
          </p>}
        </div>
      </div>}
    </>}
  </section>;
}
