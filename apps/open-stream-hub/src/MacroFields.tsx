// The recorded-macro editor: the keystroke timeline with its waits and holds,
// the recorder dialog (an event tap on this Mac), the permission rows, and the
// textual plan preview — the planned keystrokes, readable before any run. Pure
// pieces are exported separately from the stateful editor, so tests render and
// call them as functions.
import { useEffect, useRef, useState } from 'react';
import { ArrowDown, ArrowUp, Circle, Play, Square, Trash2 } from 'lucide-react';
import type { InputPermissions, MacroStep, Modifier, RecordingView } from './contracts';
import { IntegerField, ModalDialog } from './controls';
import { isMediaKey, macroDurationLabel, macroDurationMs, macroIssue, macroLimits } from './action-model';
import { KeyPickerField } from './key-capture';
import { chordLabel, foldRecording, orderedModifiers } from './key-capture-model';
import type { Invoke } from './profile-session';

// "Command+C after 120 ms (held 80 ms) then Return (held 40 ms)": the plan the
// press will follow, as text. This is the printed preview of the planned
// keystrokes; the activity log carries only the shape (the step count and
// duration), the same redaction as pasted text.
export function macroPlanText(steps: MacroStep[]): string {
  if (!steps.length) return 'Nothing recorded yet.';
  return steps.map((step, index) => {
    const chord = chordLabel(step);
    const wait = step.delay_before_ms ? ` after ${step.delay_before_ms.toLocaleString('en-US')} ms` : '';
    const hold = step.hold_ms ? ` (held ${step.hold_ms.toLocaleString('en-US')} ms)` : '';
    return `${index === 0 ? '' : 'then '}${chord}${wait}${hold}`;
  }).join(' ');
}

// Every wait longer than 1 s becomes 1 s. Long gaps usually record a pause in
// the user's own attention, not a timing the replay needs.
export function limitPauses(steps: MacroStep[], toMs = 1000): MacroStep[] {
  return steps.map(step => ({ ...step, delay_before_ms: Math.min(step.delay_before_ms, toMs) }));
}

const modifierCheckboxes: Modifier[] = ['command', 'control', 'option', 'shift'];

// One timeline row: the key (with its chord), the modifiers held with it, the
// wait before it, the hold, and its place in the order.
export function MacroStepRow({ step, index, count, disabled, onChange, onRemove, onMove }: {
  step: MacroStep; index: number; count: number; disabled: boolean;
  onChange(step: MacroStep): void; onRemove(): void; onMove(delta: 1 | -1): void;
}) {
  const prefix = `macro-step-${index + 1}`;
  return <li className="macro-step">
    <div className="macro-step-main">
      <span className="macro-step-number" aria-hidden="true">{index + 1}</span>
      <KeyPickerField triggerId={`${prefix}-key`} value={step.key} includeMedia={false} disabled={disabled}
        onPick={key => onChange({ ...step, key })} />
      <span className="macro-step-modifiers">
        {modifierCheckboxes.map(modifier => <label key={modifier}>
          <input type="checkbox" disabled={disabled} checked={step.modifiers.includes(modifier)}
            aria-label={`${modifier[0].toUpperCase() + modifier.slice(1)} with keystroke ${index + 1}`}
            onChange={event => onChange({ ...step, modifiers: event.target.checked ? [...step.modifiers, modifier] : step.modifiers.filter(item => item !== modifier) })} />
          {modifier[0].toUpperCase() + modifier.slice(1)}
        </label>)}
      </span>
    </div>
    <div className="macro-step-timing">
      <label htmlFor={`${prefix}-delay`}>Wait before (ms)</label>
      <IntegerField id={`${prefix}-delay`} value={step.delay_before_ms} minimum={macroLimits.delayMs.minimum} maximum={macroLimits.delayMs.maximum} disabled={disabled}
        onCommit={delay_before_ms => onChange({ ...step, delay_before_ms })} />
      <label htmlFor={`${prefix}-hold`}>Held (ms)</label>
      <IntegerField id={`${prefix}-hold`} value={step.hold_ms} minimum={macroLimits.holdMs.minimum} maximum={macroLimits.holdMs.maximum} disabled={disabled}
        onCommit={hold_ms => onChange({ ...step, hold_ms })} />
      <span className="macro-step-order">
        <button type="button" className="button compact" disabled={disabled || index === 0} aria-label={`Move keystroke ${index + 1} up`} onClick={() => onMove(-1)}><ArrowUp size={15} /></button>
        <button type="button" className="button compact" disabled={disabled || index === count - 1} aria-label={`Move keystroke ${index + 1} down`} onClick={() => onMove(1)}><ArrowDown size={15} /></button>
        <button type="button" className="button compact" disabled={disabled} aria-label={`Remove keystroke ${index + 1}`} onClick={onRemove}><Trash2 size={15} /></button>
      </span>
    </div>
  </li>;
}

// The two permission rows of the recorder dialog: one per macOS permission,
// each with what it does and the deep link into System Settings. Checking is
// explicit (Check again); nothing is ever requested from here.
export function PermissionRows({ permissions, error, onCheck, onOpen, busy }: {
  permissions: InputPermissions | null; error: string | null;
  onCheck(): void; onOpen(target: 'post_events' | 'listen_events'): void; busy: boolean;
}) {
  const row = (id: string, name: string, what: string, target: 'post_events' | 'listen_events', allowed: boolean | null) => <div className="permission-row" data-permission={id}>
    <span className="permission-name">{name}</span>
    <span className="fine-print">{what}</span>
    <span className={`permission-state${allowed === false ? ' denied' : ''}`} role="status">
      {allowed === null ? 'Not checked yet' : allowed ? 'Allowed' : 'Not allowed'}
    </span>
    {allowed === false && <button type="button" className="button compact" disabled={busy} onClick={() => onOpen(target)}>Open System Settings</button>}
  </div>;
  return <div className="permission-rows">
    {row('post_events', 'Send keystrokes', 'Accessibility permission, for replaying the recorded keys on this Mac.', 'post_events', permissions ? permissions.post_events : null)}
    {row('listen_events', 'Record keys', 'Input Monitoring permission, for hearing your keystrokes while recording.', 'listen_events', permissions ? permissions.listen_events : null)}
    <div className="permission-actions">
      <button type="button" className="button compact" disabled={busy} onClick={onCheck}>Check again</button>
      {error && <p className="field-error" role="alert">{error}</p>}
    </div>
  </div>;
}

// What the recorder dialog shows: the permission check, the live recording, or
// the folded result waiting to be used.
export type RecorderPhase =
  | { phase: 'closed' }
  | { phase: 'permissions' }
  | { phase: 'recording' }
  | { phase: 'review'; steps: MacroStep[]; dropped: number; trimmed: boolean };

const pollIntervalMs = 250;

// The dialog itself: Start and Stop like the official recorder, a live
// keystroke count, and the folded result. Recording never starts without the
// Input Monitoring permission; a check error carries the deep link.
export function MacroRecorderDialog({ state, onCancel, onStart, onStop, onUse, permissions, permissionError, onCheckPermissions, onOpenSettings, busy, live, disabled }: {
  state: RecorderPhase; onCancel(): void; onStart(): void; onStop(): void; onUse(steps: MacroStep[]): void;
  permissions: InputPermissions | null; permissionError: string | null;
  onCheckPermissions(): void; onOpenSettings(target: 'post_events' | 'listen_events'): void;
  busy: boolean; live: RecordingView | null; disabled: boolean;
}) {
  const open = state.phase !== 'closed';
  return <ModalDialog open={open} labelledBy="macro-recorder-title" describedBy="macro-recorder-message"
    onCancel={onCancel} className="macro-recorder">
    <h2 id="macro-recorder-title">Record a macro</h2>
    <p id="macro-recorder-message" className="fine-print">Press the keys in order, with the pauses you want between them. While it runs, the recording hears every keystroke on this Mac — type only what the macro should replay. It keeps at most 60 s or {macroLimits.steps.maximum} keystrokes.</p>
    <PermissionRows permissions={permissions} error={permissionError} onCheck={onCheckPermissions} onOpen={onOpenSettings} busy={busy} />
    {state.phase === 'permissions' && <div className="dialog-actions">
      <button type="button" className="button" onClick={onCancel}>Cancel</button>
      <button type="button" className="button primary" disabled={disabled || busy || !permissions?.listen_events || !!permissionError} onClick={onStart}>
        <Play size={16} />Start recording
      </button>
    </div>}
    {state.phase === 'recording' && <div className="macro-recording" role="status">
      <p><Circle size={15} className="spinning" /> Recording… {live ? `${live.events.length} key events` : 'starting…'}</p>
      {live?.capped && <p className="field-warning" role="note">The recording is full. Stop it and use what was captured.</p>}
      <div className="dialog-actions">
        <button type="button" className="button primary" disabled={busy} onClick={onStop}><Square size={16} />Stop</button>
      </div>
    </div>}
    {state.phase === 'review' && <div className="macro-review">
      <p role="status">The recording became {state.steps.length} keystroke{state.steps.length === 1 ? '' : 's'}. {state.trimmed ? 'Waits or holds beyond the limits were trimmed. ' : ''}{state.dropped ? `${state.dropped} key${state.dropped === 1 ? '' : 's'} cannot be replayed and ${state.dropped === 1 ? 'was' : 'were'} left out.` : ''}</p>
      <p className="macro-plan">{macroPlanText(state.steps)}</p>
      <div className="dialog-actions">
        <button type="button" className="button" disabled={busy} onClick={onCancel}>Discard</button>
        <button type="button" className="button primary" disabled={disabled || busy || !state.steps.length}
          onClick={() => onUse(state.steps)}>Use recording</button>
      </div>
    </div>}
  </ModalDialog>;
}

// The stateful editor the action fields render. `invoke` is present only in
// the desktop app; in the browser preview the recorder stays hidden (nothing
// can listen there), while the timeline stays fully editable.
export function MacroFields({ steps, disabled, onChange, invoke }: {
  steps: MacroStep[]; disabled: boolean; onChange(steps: MacroStep[]): void; invoke?: Invoke;
}) {
  const [recorder, setRecorder] = useState<RecorderPhase>({ phase: 'closed' });
  const [permissions, setPermissions] = useState<InputPermissions | null>(null);
  const [permissionError, setPermissionError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [live, setLive] = useState<RecordingView | null>(null);
  const poll = useRef<ReturnType<typeof setInterval> | null>(null);
  useEffect(() => () => {
    if (poll.current) clearInterval(poll.current);
  }, []);
  const issue = macroIssue(steps);
  const update = (next: MacroStep[]) => onChange(next);
  const updateAt = (index: number, step: MacroStep) => update(steps.map((item, at) => at === index ? step : item));
  const move = (index: number, delta: 1 | -1) => {
    const next = [...steps];
    const [item] = next.splice(index, 1);
    next.splice(index + delta, 0, item);
    update(next);
  };
  const stopPolling = () => {
    if (poll.current) {
      clearInterval(poll.current);
      poll.current = null;
    }
  };
  const checkPermissions = async () => {
    if (!invoke) return;
    setBusy(true);
    try {
      setPermissions(await invoke<InputPermissions>('input_permissions'));
      setPermissionError(null);
    } catch (error) {
      setPermissionError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const openSettings = async (target: 'post_events' | 'listen_events') => {
    if (!invoke) return;
    setBusy(true);
    try {
      await invoke('open_permission_settings', { target });
    } catch (error) {
      setPermissionError(String(error));
    } finally {
      setBusy(false);
    }
  };
  const stopRecording = async () => {
    if (!invoke) return;
    setBusy(true);
    try {
      const view = await invoke<RecordingView>('macro_record_stop');
      stopPolling();
      setLive(null);
      setRecorder({ phase: 'review', ...foldRecording(view.events) });
    } catch (error) {
      stopPolling();
      setPermissionError(String(error));
      setRecorder({ phase: 'permissions' });
    } finally {
      setBusy(false);
    }
  };
  const beginRecording = async () => {
    if (!invoke) return;
    setBusy(true);
    try {
      await invoke('macro_record_start');
      setLive(null);
      setRecorder({ phase: 'recording' });
      poll.current = setInterval(() => {
        void invoke<RecordingView>('macro_record_state').then(view => {
          setLive(view);
          if (view.capped) void stopRecording();
        }).catch(() => { /* the next tick retries; a dead run shows at Stop. */ });
      }, pollIntervalMs);
    } catch (error) {
      setPermissionError(String(error));
      setRecorder({ phase: 'permissions' });
    } finally {
      setBusy(false);
    }
  };
  const closeRecorder = () => {
    stopPolling();
    setRecorder({ phase: 'closed' });
  };
  return <div className="action-fields macro-fields">
    <p className="fine-print">Replays {steps.length === 1 ? '1 keystroke' : `${steps.length} keystrokes`} in order: {macroDurationLabel(macroDurationMs(steps))} (m:ss) of waits and holds. Limits: {macroLimits.steps.minimum}–{macroLimits.steps.maximum} keystrokes, waits up to {macroLimits.delayMs.maximum.toLocaleString('en-US')} ms, holds up to {macroLimits.holdMs.maximum.toLocaleString('en-US')} ms, {macroDurationLabel(macroLimits.durationMs)} in total.</p>
    <ol className="macro-timeline">
      {steps.map((step, index) => <MacroStepRow key={index} step={step} index={index} count={steps.length} disabled={disabled}
        onChange={next => updateAt(index, next)}
        onRemove={() => update(steps.filter((_, at) => at !== index))}
        onMove={delta => move(index, delta)} />)}
    </ol>
    <div className="macro-actions">
      <KeyPickerField triggerId="macro-add-key" value="" includeMedia={false} disabled={disabled || steps.length >= macroLimits.steps.maximum}
        triggerLabel="Add keystroke"
        onPick={key => onChange([...steps, { key, modifiers: [], delay_before_ms: 0, hold_ms: 80 }])} />
      {invoke && <button type="button" className="button compact" disabled={disabled} onClick={() => { void checkPermissions(); setRecorder({ phase: 'permissions' }); }}>Record macro</button>}
      <button type="button" className="button compact" disabled={disabled || !steps.some(step => step.delay_before_ms > 1000)}
        onClick={() => update(limitPauses(steps))}>Limit pauses to 1 s</button>
    </div>
    <p className="macro-plan" role="status">Planned: {macroPlanText(steps)}</p>
    {issue && <p className="field-error" role="alert">{issue}</p>}
    <MacroRecorderDialog state={recorder} onCancel={closeRecorder}
      onStart={() => void beginRecording()} onStop={() => void stopRecording()}
      onUse={useSteps => { update(useSteps); closeRecorder(); }}
      permissions={permissions} permissionError={permissionError} busy={busy} live={live} disabled={disabled}
      onCheckPermissions={() => void checkPermissions()} onOpenSettings={target => void openSettings(target)} />
  </div>;
}

// Whether a step's key is a media key — impossible in a validated macro, but a
// hand-edited profile is shown honestly instead of being silently accepted.
export const stepKeyNote = (step: MacroStep): string | null =>
  isMediaKey(step.key) ? 'Media keys cannot be macro keys. Use the Media control action instead.' : null;

// The ordered modifier list of a step, for compact displays.
export const stepModifiers = (step: MacroStep): Modifier[] => orderedModifiers(step.modifiers);
