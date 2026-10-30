import { useState } from 'react';
import { ArrowDown, ArrowUp, ChevronDown, ChevronRight, Play, Plus, Trash2, TriangleAlert } from 'lucide-react';
import { ActionFields } from './ActionFields';
import type { Action, Binding, DialBinding, DialFunction, Profile, ReplayEvent } from './contracts';
import { useObsCatalog } from './obs-catalog';
import type { ObsCatalogStore } from './obs-catalog';
import { useStreamlabsCatalog } from './streamlabs-catalog';
import type { StreamlabsCatalogStore } from './streamlabs-catalog';
import {
  addDialFunction, deleteDialFunction, dialBindingAt, dialFunctionGestures, dialFunctionNameIssue, dialSerialIssue,
  editDialBinding, flatDialBindings, maxDialFunctionNameBytes, maxDialFunctions, moveDialFunction, moveFlatDialBindings,
  removeDialBinding, renameDialFunction,
} from './dial-function-model';
import type { Invoke, ProfileSession, SessionState } from './profile-session';
import { actionLabel, errorMessage, gestureLabels } from './ui-model';
import type { Page } from './ui-model';

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

// The row label of a function gesture: rotation reads as a turn, Long press as is.
export const dialGestureLabel = (gesture: DialGesture) => gesture === 'long_press' ? gestureLabels[gesture] : `Turn ${gestureLabels[gesture].toLowerCase()}`;

// The muted summary on a function's row.
export const dialActionCount = (item: DialFunction) => item.bindings.length ? plural(item.bindings.length, 'action') : 'No actions';

// One row per bindable gesture of a function, for the given device serial ('' is
// any serial): the action's summary, or null when it is not set, and how many
// bindings of the same gesture belong to other serials.
export function dialGestureRows(item: DialFunction, serial: string) {
  return dialFunctionGestures.map(gesture => {
    const same = item.bindings.filter(binding => binding.gesture === gesture);
    const binding = same.find(other => (other.serial_number ?? '') === serial);
    return { gesture, label: dialGestureLabel(gesture), summary: binding ? actionLabel(binding.action) : null, otherSerials: same.length - (binding ? 1 : 0) };
  });
}

// What the Move button does to each flat encoder binding, one sentence per outcome.
export function flatBindingOutcomes(flat: ReturnType<typeof flatDialBindings>, functionName: string): string[] {
  return [
    flat.moved.length ? `${plural(flat.moved.length, 'binding')} will move into ${functionName}: ${bindingNames(flat.moved)}.` : '',
    flat.conflicting.length ? `${plural(flat.conflicting.length, 'binding')} will be dropped because ${functionName} already has the same gesture: ${bindingNames(flat.conflicting)}.` : '',
    flat.dropped.length ? `${plural(flat.dropped.length, 'binding')} will be dropped because no function can hold ${flat.dropped.length === 1 ? 'it' : 'them'}: ${bindingNames(flat.dropped)}.` : '',
  ].filter(Boolean);
}

export function DialFunctionsPanel({ profile, session, state, disabled, initialSelected = 0, setPage, catalog: catalogStore, streamlabs: streamlabsStore, invoke }: {
  profile: Profile; session: ProfileSession; state: SessionState; disabled: boolean; initialSelected?: number; setPage?: (page: Page) => void; catalog?: ObsCatalogStore; streamlabs?: StreamlabsCatalogStore;
  // The desktop backend, for the app pickers of Open app and the front-app test (absent in the browser preview).
  invoke?: Invoke;
}) {
  const catalog = useObsCatalog(catalogStore);
  const streamlabs = useStreamlabsCatalog(streamlabsStore);
  const functions = profile.dial_functions ?? [];
  const [chosen, setChosen] = useState(initialSelected);
  const [gesture, setGesture] = useState<DialGesture>('right');
  const [serial, setSerial] = useState('');
  const [advanced, setAdvanced] = useState(false);
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
  const previewDisabled = disabled || previewRunning || !!serialIssue;
  const playButton = (item: DialGesture | 'short_press', label: string) =>
    <button type="button" className="icon-button" aria-label={`Preview ${label} in ${currentName}`} title="Preview" disabled={previewDisabled} onClick={() => { void preview(item); }}><Play size={15} /></button>;
  // Device serial lives under Advanced; editing a serial-specific binding opens it.
  const editSerialBinding = (item: DialBinding) => { setGesture(item.gesture); setSerial(item.serial_number ?? ''); setAdvanced(true); };

  const editor = current && <div className="dial-function-editor" role="group" aria-label={`Bindings of ${currentName}`}>
    {serial && !serialIssue && <p className="small-label">For device serial {serial}</p>}
    <ul className="dial-gesture-rows">
      {dialGestureRows(current, serial).map(row => <li key={row.gesture} className={row.gesture === gesture ? 'selected' : undefined}>
        <div className="dial-gesture-row">
          <button type="button" className="dial-gesture-select" aria-pressed={row.gesture === gesture} onClick={() => setGesture(row.gesture)}>
            <strong>{row.label}</strong><span className={row.summary ? undefined : 'muted'}>{row.summary ?? 'Not set'}{row.otherSerials > 0 ? ` · +${row.otherSerials} for other serials` : ''}</span>
          </button>
          {row.summary && <button type="button" className="button compact" aria-label={`Clear ${row.label}`} disabled={disabled || !!serialIssue} onClick={() => run(latest => editDialBinding(latest, selected, row.gesture, serial, null))}>Clear</button>}
          {playButton(row.gesture, row.label)}
        </div>
        {row.gesture === gesture && <fieldset disabled={disabled || !!serialIssue}>
          <ActionFields action={binding?.action ?? null} fader={false} selectorKey={`dial:${selected}:${gesture}:${serial}`} onSetUpObs={setPage ? () => setPage('settings') : undefined} catalog={catalog} streamlabs={streamlabs} invoke={invoke} onChange={(action, field) => {
            if (action?.type === 'volume_from_position' || action?.type === 'mic_input_volume_from_position') { session.setError('Dial functions cannot follow a fader position.'); return; }
            run(latest => editDialBinding(latest, selected, gesture, serial, action as Action | null), field && `dial:${field}:${selected}:${gesture}:${serial}`);
          }} />
        </fieldset>}
      </li>)}
      <li className="note">
        <div className="dial-gesture-row"><div><strong>Short press</strong><span className="muted">Switches function; cannot be bound</span></div>{playButton('short_press', 'Short press')}</div>
      </li>
    </ul>
    {previewed === selected && state.preview.status !== 'idle' && <p role={state.preview.status === 'failed' ? 'alert' : 'status'} className="fine-print">
      {state.preview.status === 'running' ? 'Resolving…' : state.preview.status === 'failed' ? state.preview.error : dialPreviewText(state.preview.event, current.name)}
    </p>}
    <button type="button" className="dial-advanced-toggle" aria-expanded={advanced} aria-controls="dial-function-advanced" onClick={() => setAdvanced(open => !open)}>{advanced ? <ChevronDown size={14} aria-hidden="true" /> : <ChevronRight size={14} aria-hidden="true" />}Advanced</button>
    {advanced && <div id="dial-function-advanced" className="dial-advanced">
      <label htmlFor="dial-binding-serial">Device serial (optional)</label>
      <input id="dial-binding-serial" value={serial} spellCheck={false} placeholder="Any device serial" aria-invalid={!!serialIssue} aria-describedby={serialIssue ? 'dial-binding-serial-error' : undefined} onChange={event => setSerial(event.target.value)} />
      {serialIssue && <p id="dial-binding-serial-error" className="field-error" role="alert">{serialIssue}</p>}
      {current.bindings.some(item => item.serial_number) && <ul className="binding-list">{current.bindings.map((item, index) => item.serial_number ? <li key={`${item.gesture}:${item.serial_number}`} className="binding-row"><div>
        <strong>{dialGestureLabel(item.gesture)} · serial {item.serial_number}</strong><span>{actionLabel(item.action)}</span></div>
        <button type="button" className="button compact" onClick={() => editSerialBinding(item)}>Edit</button>
        <button type="button" className="icon-button" aria-label={`Remove ${current.name} ${gestureLabels[item.gesture]} binding for serial ${item.serial_number}`} disabled={disabled} onClick={() => run(latest => removeDialBinding(latest, selected, index))}><Trash2 size={16} /></button>
      </li> : null)}</ul>}
    </div>}
  </div>;

  return <section className="dial-functions" aria-labelledby={dialFunctionHeadingId}>
    <h3 id={dialFunctionHeadingId}>Dial functions <span className="small-label">{functions.length ? `${functions.length} of ${maxDialFunctions}` : 'Off'}</span></h3>
    <p className="dial-functions-intro">Short press switches function; turning and long press run the current one.</p>
    <details className="learn-more"><summary>How functions work</summary><ul className="fine-print notes-list">
      <li>A short press on the encoder switches to the next function and never runs an action. Turning and long press use the current function.</li>
      <li>A run always starts at the first function and returns to it when it restarts.</li>
      <li>The encoder display shows the current function while OpenStreamHub drives the display (the default) and the dial screen is set to show the function name.</li>
      <li>Names must be unique, at most {maxDialFunctionNameBytes} bytes, without spaces at the start or end, and without control or invisible formatting characters.</li>
      <li>Deleting the last function turns dial functions off. Undo brings it back.</li>
    </ul></details>
    {!functions.length ? <>
      <p className="empty-state">No functions yet{flatCount > 0 ? `; the ${plural(flatCount, 'existing encoder binding')} will need to move into one` : ''}.</p>
      <button type="button" className="button" disabled={disabled} onClick={() => { run(latest => addDialFunction(latest)); choose(0); }}><Plus size={17} />Add dial functions</button>
    </> : <>
      {flatCount > 0 && current && <div className="dial-flat-banner" role="region" aria-label="Encoder bindings outside dial functions">
        <TriangleAlert size={16} aria-hidden="true" />
        <div>
          <p>{flatCount} encoder binding{flatCount === 1 ? " isn't" : "s aren't"} in a function yet and the profile can't be saved until {flatCount === 1 ? 'it is' : 'they are'}.</p>
          <button type="button" className="button compact" disabled={disabled} onClick={() => run(latest => moveFlatDialBindings(latest, selected))}>{flat.moved.length ? `Move ${plural(flat.moved.length, 'encoder binding')} into ${currentName}` : 'Remove the encoder bindings outside functions'}</button>
          <details className="learn-more"><summary>What happens to each binding</summary><ul className="fine-print notes-list">
            {flatBindingOutcomes(flat, currentName).map(line => <li key={line}>{line}</li>)}
            <li>Undo restores them.</li>
          </ul></details>
        </div>
      </div>}
      <ol className="dial-function-list">{functions.map((item, index) => {
        const issue = dialFunctionNameIssue(item.name, functions, index);
        const label = item.name.trim() || `function ${index + 1}`;
        return <li key={index} className={index === selected ? 'selected' : ''}>
          <div className="dial-function-row">
            <label className="sr-only" htmlFor={dialFunctionNameId(index)}>Function {index + 1} name</label>
            <input id={dialFunctionNameId(index)} disabled={disabled} value={item.name} spellCheck={false} aria-invalid={!!issue} aria-describedby={issue ? `${dialFunctionNameId(index)}-error` : undefined}
              onFocus={() => { if (index !== selected) choose(index); }}
              onChange={event => run(latest => renameDialFunction(latest, index, event.target.value), `dial-function-name:${index}`)} />
            <button type="button" className="dial-function-summary" aria-pressed={index === selected} aria-label={`Edit ${label}, ${dialActionCount(item)}`} onClick={() => choose(index)}>{dialActionCount(item)}</button>
            <div className="dial-function-actions">
              <button type="button" className="icon-button" aria-label={`Move ${label} up`} title="Move up" disabled={disabled || index === 0} onClick={() => { run(latest => moveDialFunction(latest, index, -1)); setChosen(selectionAfterMove(selected, index, -1)); }}><ArrowUp size={15} /></button>
              <button type="button" className="icon-button" aria-label={`Move ${label} down`} title="Move down" disabled={disabled || index === functions.length - 1} onClick={() => { run(latest => moveDialFunction(latest, index, 1)); setChosen(selectionAfterMove(selected, index, 1)); }}><ArrowDown size={15} /></button>
              <button type="button" className="icon-button" aria-label={`Delete ${label}`} title={functions.length === 1 ? 'Delete the last function and turn dial functions off' : 'Delete function'} disabled={disabled} onClick={() => { run(latest => deleteDialFunction(latest, index)); if (index < selected) setChosen(selected - 1); }}><Trash2 size={15} /></button>
            </div>
          </div>
          {issue && <p id={`${dialFunctionNameId(index)}-error`} className="field-error" role="alert">{issue}</p>}
          {index === selected && editor}
        </li>;
      })}</ol>
      <button type="button" className="button compact" disabled={disabled || functions.length >= maxDialFunctions} onClick={() => { run(latest => addDialFunction(latest)); choose(functions.length); }}><Plus size={16} />Add function</button>
      {functions.length >= maxDialFunctions && <p className="fine-print">A profile can have at most {maxDialFunctions} dial functions.</p>}
    </>}
  </section>;
}

