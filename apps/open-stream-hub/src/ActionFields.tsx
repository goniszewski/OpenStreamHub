import { Plus, RotateCcw, Trash2 } from 'lucide-react';
import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import type { BindingAction, MediaCommand, Modifier, ObsNameList, ObsOutputOp, ObsReplayBufferOp, ObsSwitchOp, ProfileDirection, StepAction, SystemCommand, TimerSound } from './contracts';
import { ActionPickerField } from './ActionPicker';
import { defaultBindingAction } from './action-picker-model';
import type { ObsStepAction, ObsStepType } from './action-model';
import { HotkeyField } from './key-capture';
import { MacroFields } from './MacroFields';
import { catalogNames, filterTargets } from './obs-catalog';
import type { ObsCatalogView } from './obs-catalog';
import { obsConnectionLabels } from './obs-settings';
import type { Invoke } from './profile-session';
import { IntegerField, SelectField } from './controls';
import {
  defaultStepAction, defaultTimerSound, isFaderTemplate, isMediaKey, maxObsSceneBytes, maxRunningTimers, maxTextBytes, mediaKeyLabels, mediaLabels, multiActionSteps,
  obsMissingWarning, obsNameFits, obsNameIssue, obsOperationLabels, obsOutputOpLabels, obsReplayBufferOpLabels, obsSceneIssue,
  obsSwitchOpLabels, profileDirectionLabels, switchObsOperation, systemLabels, textIssue, timerLabelIssue, timerSeconds,
  timerSoundLabels, urlIssue,
} from './action-model';

type Change = (action: BindingAction | null, field?: string) => void;

// One-line hint with the full story behind a "Learn more" disclosure.
function Hint({ hint, children }: { hint: string; children: ReactNode }) {
  return <>
    <p className="fine-print">{hint}</p>
    <details className="learn-more"><summary>Learn more</summary><p className="fine-print">{children}</p></details>
  </>;
}

const obsSwitchHint = 'Toggle follows OBS\'s current state; Turn on and Turn off do not.';
const obsSwitchMore = 'Turn on and Turn off are safer inside a multi-action.';
const obsReloadHint = 'OBS reloads its scenes and settings when this runs, which can interrupt a live stream or recording.';
const obsReloadMore = 'OBS applies the switch a moment after the press; nothing waits for it to finish, so in a multi-action put it last (a later OBS step can reach OBS while it is still reloading and fail).';

function ObsSwitchSelect({ id, value, onChange }: { id: string; value: ObsSwitchOp; onChange(op: ObsSwitchOp): void }) {
  return <SelectField id={id} value={value} onChange={event => onChange(event.target.value as ObsSwitchOp)}>{(Object.keys(obsSwitchOpLabels) as ObsSwitchOp[]).map(op => <option key={op} value={op}>{obsSwitchOpLabels[op]}</option>)}</SelectField>;
}

const obsOutputNote = 'OBS refuses Start on a running output and Stop on a stopped one.';

// The one OBS connection status: shown once per editor (top-level OBS action, or a multi-action
// with OBS steps), never per step. The Set up link renders only where it can navigate. The OBS
// listings (the name fields' choices) hang off the same note: the Refresh press asks OBS again,
// and a failed listing says why in the place where the choices would be.
export function ObsConnection({ onSetUpObs, catalog }: { onSetUpObs?: () => void; catalog?: ObsCatalogView }) {
  return <div className="obs-connection" role="note">
    <p className="fine-print">Connects to OBS 28 or later on this Mac while OBS runs with its WebSocket server on. {onSetUpObs && <button type="button" className="obs-setup" onClick={onSetUpObs}>Set up in Settings</button>}</p>
    {catalog && (catalog.desktop || catalog.status !== 'idle') && <div className="obs-catalog">
      <button type="button" className="button compact" disabled={!catalog.desktop || catalog.status === 'loading'} onClick={catalog.refresh}>
        <RotateCcw size={15} />{catalog.status === 'loading' ? 'Listing…' : 'Refresh from OBS'}
      </button>
      {catalog.connection && <span className={`obs-chip obs-chip-${catalog.connection.state}`} title={catalog.connection.message}>{obsConnectionLabels[catalog.connection.state]}</span>}
      {catalog.status === 'failed' && <p className="fine-print obs-catalog-error" role="note">{catalog.error}</p>}
    </div>}
    <details className="learn-more"><summary>Learn more</summary><p className="fine-print">The connection goes to this Mac only. Without it, the press is logged as unavailable and nothing runs. Refresh asks OBS for its current scenes, sources, filters, scene collections and profiles, so the name fields can offer choices; typing a name always works too.</p></details>
  </div>;
}

// One OBS name field: a text input that becomes a combobox while the editor holds OBS's matching
// list — click or ArrowDown opens the choices, typing narrows them, and a name OBS does not list
// is said inline. Typing always wins: the press matches names exactly, and OBS can be asked again
// with Refresh. entry is null wherever there is no list (browser preview, OBS off or unreachable),
// which renders exactly the plain input. Hook-free and controlled (open state and the active
// option live in ObsNameField), so tests call it as a function.
const comboLimit = 50;

export function ObsCombo({ id, value, entry, open, onOpenChange, active, onActiveChange, onType, onPick, onOpenList, invalid, describedBy }: {
  id: string; value: string; entry: ObsNameList | null; open: boolean; onOpenChange(open: boolean): void;
  active: number; onActiveChange(active: number): void; onType(value: string): void; onPick(name: string): void; onOpenList?: () => void;
  invalid?: boolean; describedBy?: string;
}) {
  const choicesId = `${id}-choices`;
  // Only names a profile can reference are offered; longer ones exist in OBS but cannot be saved.
  const listed = entry?.status === 'ready' ? entry.names.filter(obsNameFits) : null;
  const query = value.trim().toLowerCase();
  const matches = (listed ?? []).filter(name => name.toLowerCase().includes(query));
  const shown = matches.slice(0, comboLimit);
  const at = Math.min(Math.max(active, 0), Math.max(shown.length - 1, 0));
  const openList = () => {
    if (!entry || open) return;
    onActiveChange(Math.max(matches.findIndex(name => name === value), 0));
    onOpenChange(true);
    onOpenList?.();
  };
  const keyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (event.key === 'ArrowDown') {
      event.preventDefault();
      if (!open) openList();
      else onActiveChange(Math.min(at + 1, shown.length - 1));
    } else if (event.key === 'ArrowUp' && open) {
      event.preventDefault();
      onActiveChange(Math.max(at - 1, 0));
    } else if (event.key === 'Enter' && open && shown[at]) {
      event.preventDefault();
      onPick(shown[at]);
      onOpenChange(false);
    } else if (event.key === 'Escape' && open) {
      onOpenChange(false);
    }
  };
  return <>
    <input id={id} {...(entry ? { role: 'combobox', 'aria-expanded': open, 'aria-controls': choicesId, 'aria-autocomplete': 'list' as const } : {})}
      aria-activedescendant={open && shown[at] ? `${id}-choice-${at}` : undefined}
      aria-invalid={invalid || undefined} aria-describedby={describedBy} autoComplete="off" spellCheck={false} value={value}
      onClick={openList} onKeyDown={keyDown} onBlur={() => onOpenChange(false)}
      onChange={event => { onType(event.target.value); onActiveChange(0); }} />
    {open && entry && <ul className="combo-list" id={choicesId} role="listbox" aria-label="Names OBS currently lists">
      {entry.status === 'loading' && <li className="combo-note" role="status">Looking up OBS…</li>}
      {entry.status === 'failed' && <li className="combo-note">{entry.error}</li>}
      {listed && shown.map((name, index) => (
        <li key={name} id={`${id}-choice-${index}`} role="option" aria-selected={name === value}
          className={index === at ? 'active' : undefined} onMouseDown={event => event.preventDefault()}
          onClick={() => { onPick(name); onOpenChange(false); }}>{name}</li>
      ))}
      {listed && !matches.length && <li className="combo-note" role="status">Nothing in the OBS list matches. The name you type is kept.</li>}
      {listed && matches.length > shown.length && <li className="combo-note">{matches.length - shown.length} more — keep typing to narrow the list.</li>}
    </ul>}
  </>;
}

// The field wrapper owning the combobox state, closing the choices on a press outside. Typing
// keeps the choices open; picking one closes them.
function ObsNameField({ id, value, entry, onCommit, onOpenList, invalid, describedBy }: {
  id: string; value: string; entry: ObsNameList | null; onCommit(name: string): void; onOpenList?: () => void;
  invalid?: boolean; describedBy?: string;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(0);
  const wrapper = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: PointerEvent) => { if (!wrapper.current?.contains(event.target as Node)) setOpen(false); };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [open]);
  return <span className="obs-name" ref={wrapper}>
    <ObsCombo id={id} value={value} entry={entry} open={open} onOpenChange={setOpen} active={active} onActiveChange={setActive}
      onType={onCommit} onPick={onCommit} onOpenList={onOpenList} invalid={invalid} describedBy={describedBy} />
  </span>;
}

// Label, combobox input, and the two inline sayings: the saved-name rules (an error) and, when the
// editor holds OBS's list and the exact name is not in it, the live cross-check (a note).
function ObsNameRow({ id, label, value, entry, what, issue, onChange, onOpenList, spaced }: {
  id: string; label: string; value: string; entry: ObsNameList | null; what: string; issue: string | null;
  onChange(name: string): void; onOpenList?: () => void; spaced?: boolean;
}) {
  const warning = obsMissingWarning(value, entry, what);
  const issueId = `${id}-error`;
  const warningId = `${id}-warning`;
  return <>
    <label className={spaced ? 'spaced-label' : undefined} htmlFor={id}>{label}</label>
    <ObsNameField id={id} value={value} entry={entry} onCommit={onChange} onOpenList={onOpenList} invalid={!!issue}
      describedBy={issue ? issueId : warning ? warningId : undefined} />
    {issue && <p id={issueId} className="field-error" role="alert">{issue}</p>}
    {!issue && warning && <p id={warningId} className="field-warning" role="note">{warning}</p>}
  </>;
}

// The action picker and the parameter fields of the chosen action. Top-level ids keep their
// historical names; multi-action steps prefix theirs with `step-N-`. onSetUpObs opens
// Settings, where the OBS connection is configured. catalog carries OBS's current
// listings for the OBS name fields; without it they stay plain inputs. invoke reaches
// the macro editor's recorder and permission checks (desktop only).
export function ActionFields({ action, fader, selectorKey, onChange, onSetUpObs, catalog, invoke }: {
  action: BindingAction | null; fader: boolean; selectorKey: string; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView; invoke?: Invoke;
}) {
  return <>
    <label htmlFor="binding-action">Action</label>
    <ActionPickerField triggerId="binding-action" scope={fader ? 'binding-fader' : 'binding'} action={action}
      onAssign={type => onChange(type === null ? null : defaultBindingAction(type))} />
    {action?.type === 'volume_from_position' && <p className="fine-print">Each position change plans an absolute output volume from 0% to 100%. It does not add a volume delta.</p>}
    {action?.type === 'macro' && <MacroFields steps={action.steps} disabled={false} onChange={steps => onChange({ type: 'macro', steps }, 'macro-steps')} invoke={invoke} />}
    {action?.type === 'multi_action' && <MultiActionFields steps={action.actions} selectorKey={selectorKey} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} />}
    {action?.type === 'profile' && <div className="action-fields"><label htmlFor="profile-direction">Profile</label>
      <SelectField id="profile-direction" value={action.direction} onChange={event => onChange({ type: 'profile', direction: event.target.value as ProfileDirection })}>
        {(Object.keys(profileDirectionLabels) as ProfileDirection[]).map(direction => <option key={direction} value={direction}>{profileDirectionLabels[direction]}</option>)}
      </SelectField>
      <Hint hint="Switches the run to the next or previous saved profile, in name order.">It wraps around at the ends. Only runs started from this app switch; nothing happens with a single saved profile or outside this app. When actions run on this Mac, they pause until you confirm the new profile. It cannot be a multi-action step.</Hint></div>}
    {action?.type === 'timer' && <TimerFields action={action} inputKey={selectorKey} onChange={onChange} />}
    {action?.type === 'obs_input_volume_from_position' && <FaderObsInputFields action={action} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} />}
    {action && !isFaderTemplate(action) && action.type !== 'macro' && action.type !== 'multi_action' && action.type !== 'profile' && action.type !== 'timer' &&
      <StepFields prefix="" action={action} inputKey={selectorKey} onChange={(next, field) => onChange(next, field)} onSetUpObs={onSetUpObs} catalog={catalog} />}
  </>;
}

// The fader's OBS input volume: each position change sets the named input's volume, from 0% (silent) to 100% (unity gain, 0 dB).
function FaderObsInputFields({ action, onChange, onSetUpObs, catalog }: {
  action: Extract<BindingAction, { type: 'obs_input_volume_from_position' }>; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView;
}) {
  return <div className="action-fields">
    <ObsNameRow id="obs-input-volume-input" label="OBS input name" value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what="input"
      issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ type: 'obs_input_volume_from_position', input }, 'obs-input-volume-input')} />
    <p className="fine-print">Each position change sets this OBS input's volume from 0% (silent) to 100% (unity gain, 0 dB), matched by exact name (up to {maxObsSceneBytes} bytes). It never boosts above 0 dB.</p>
    <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />
  </div>;
}

type TimerAction = Extract<BindingAction, { type: 'timer' }>;

function TimerFields({ action, inputKey, onChange }: { action: TimerAction; inputKey: string; onChange: Change }) {
  const issue = timerLabelIssue(action.label ?? '');
  // An empty label is omitted, never written as "" (which the service rejects).
  const withLabel = (label: string): TimerAction => {
    const { label: _previous, ...rest } = action;
    return label ? { ...rest, label } : rest;
  };
  return <div className="action-fields">
    <label htmlFor="timer-seconds">Countdown (seconds)</label>
    <IntegerField key={inputKey} id="timer-seconds" value={action.seconds} minimum={timerSeconds.minimum} maximum={timerSeconds.maximum}
      onCommit={seconds => onChange({ ...action, seconds }, 'timer-seconds')} />
    <label className="spaced-label" htmlFor="timer-label">Notification text (optional)</label>
    <input id="timer-label" spellCheck={false} value={action.label ?? ''} aria-invalid={!!issue} aria-describedby={issue ? 'timer-label-error' : undefined}
      onChange={event => onChange(withLabel(event.target.value), 'timer-label')} />
    {issue && <p id="timer-label-error" className="field-error" role="alert">{issue}</p>}
    <label className="spaced-label" htmlFor="timer-sound">End sound</label>
    <SelectField id="timer-sound" value={action.sound ?? defaultTimerSound} onChange={event => onChange({ ...action, sound: event.target.value as TimerSound })}>
      {(Object.keys(timerSoundLabels) as TimerSound[]).map(sound => <option key={sound} value={sound}>{timerSoundLabels[sound]}</option>)}
    </SelectField>
    <Hint hint="When the countdown ends, macOS shows a notification and plays the sound.">1 second to 24 hours. The countdown runs in the app; pressing the same control again cancels it. The countdown is not shown on the key, and stopping the run (or the Mac going to sleep) cancels every timer. At most {maxRunningTimers} run at once. It cannot be a multi-action step.</Hint>
  </div>;
}

function MultiActionFields({ steps, selectorKey, onChange, onSetUpObs, catalog }: { steps: StepAction[]; selectorKey: string; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView }) {
  const update = (next: StepAction[], field?: string) => onChange({ type: 'multi_action', actions: next }, field);
  return <div className="action-fields">
    <p className="fine-print">Runs {multiActionSteps.minimum}–{multiActionSteps.maximum} steps in order, 280 ms apart. The first failed step stops the rest. Steps cannot be multi-actions.</p>
    <ol className="action-steps">
      {steps.map((step, index) => {
        const prefix = `step-${index + 1}-`;
        return <li key={`${index}:${step.type}`}>
          <label htmlFor={`${prefix}action`}>Step {index + 1}</label>
          <ActionPickerField triggerId={`${prefix}action`} idPrefix={prefix} scope="step" action={step} allowDrag={false}
            onAssign={type => { if (type) update(steps.map((item, at) => at === index ? defaultBindingAction(type) as StepAction : item)); }} />
          <StepFields prefix={prefix} action={step} inputKey={`${selectorKey}:${index}:${steps.length}`}
            onChange={(next, field) => update(steps.map((item, at) => at === index ? next : item), field && `${prefix}${field}`)} onSetUpObs={onSetUpObs} catalog={catalog} />
          <button type="button" className="button compact" disabled={steps.length <= multiActionSteps.minimum}
            aria-label={`Remove step ${index + 1}`} onClick={() => update(steps.filter((_, at) => at !== index))}><Trash2 size={16} />Remove</button>
        </li>;
      })}
    </ol>
    <button type="button" className="button compact" disabled={steps.length >= multiActionSteps.maximum}
      onClick={() => update([...steps, defaultStepAction('toggle_mute')])}><Plus size={16} />Add step</button>
    {steps.some(step => step.type.startsWith('obs_')) && <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />}
  </div>;
}

function StepFields({ prefix, action, inputKey, onChange, onSetUpObs, catalog }: {
  prefix: string; action: StepAction; inputKey: string; onChange(action: StepAction, field?: string): void; onSetUpObs?: () => void; catalog?: ObsCatalogView;
}) {
  const id = (name: string) => `${prefix}${name}`;
  switch (action.type) {
    case 'toggle_mute': return null;
    case 'set_volume': return <div className="action-fields"><label htmlFor={id('volume-percent')}>Output volume (%)</label><IntegerField key={inputKey} id={id('volume-percent')} value={action.percent} minimum={0} maximum={100} onCommit={percent => onChange({ type: 'set_volume', percent }, 'volume-percent')} /><p className="fine-print">Plans an absolute output volume from 0% to 100%.</p></div>;
    case 'volume': return <div className="action-fields"><label htmlFor={id('volume-delta')}>Volume change (points)</label><IntegerField key={inputKey} id={id('volume-delta')} value={action.delta} minimum={-100} maximum={100} nonZero onCommit={delta => onChange({ type: 'volume', delta }, 'volume-delta')} /><p className="fine-print">Signed percentage points, −100 to +100, excluding zero.</p></div>;
    case 'shortcut': {
      const media = isMediaKey(action.key) ? action.key : null;
      return <div className="action-fields">
        <label htmlFor={id('shortcut-key')}>Key combination</label>
        <HotkeyField id={id('shortcut-key')} value={action.key} modifiers={action.modifiers}
          onChange={({ key, modifiers }) => onChange({ ...action, key, modifiers }, 'shortcut-key')} />
        <fieldset className="modifiers"><legend>Modifiers</legend>{(['command', 'control', 'option', 'shift'] as Modifier[]).map(modifier => <label key={modifier}><input type="checkbox" checked={action.modifiers.includes(modifier)} disabled={!!media} onChange={event => onChange({ ...action, modifiers: event.target.checked ? [...action.modifiers, modifier] : action.modifiers.filter(item => item !== modifier) }, 'shortcut-key')} />{modifier[0].toUpperCase() + modifier.slice(1)}</label>)}</fieldset>
        <p className="fine-print">{media
          ? `A media key (${mediaKeyLabels[media]}) reaches whichever app controls playback and cannot combine with modifiers. Needs macOS Accessibility permission.`
          : 'Focus the field and press the keys, or choose every key from the list without pressing it: letters, digits, named keys, F1–F20, the keypad and the media keys. A chord holds at most 5 keys (4 modifiers plus the key). Media keys need macOS Accessibility permission.'}</p>
      </div>;
    }
    case 'launch_app': return <div className="action-fields"><label htmlFor={id('app-bundle-id')}>Application bundle ID</label><input id={id('app-bundle-id')} spellCheck={false} value={action.bundle_id} onChange={event => onChange({ type: 'launch_app', bundle_id: event.target.value }, 'bundle-id')} /><p className="fine-print">For example, com.apple.finder. The app checks the identifier; tests do not open it.</p></div>;
    case 'open_url': {
      const issue = urlIssue(action.url);
      return <div className="action-fields"><label htmlFor={id('open-url')}>Web address</label><input id={id('open-url')} type="url" spellCheck={false} value={action.url} aria-invalid={!!issue} aria-describedby={issue ? id('open-url-error') : undefined} onChange={event => onChange({ type: 'open_url', url: event.target.value }, 'open-url')} />{issue && <p id={id('open-url-error')} className="field-error" role="alert">{issue}</p>}<p className="fine-print">http and https only, opened in the default browser. Replay does not open it.</p></div>;
    }
    case 'text': {
      const issue = textIssue(action.text);
      return <div className="action-fields"><label htmlFor={id('text-snippet')}>Text to paste</label><textarea id={id('text-snippet')} rows={3} value={action.text} aria-invalid={!!issue} aria-describedby={issue ? id('text-snippet-error') : undefined} onChange={event => onChange({ ...action, text: event.target.value }, 'text-snippet')} />{issue && <p id={id('text-snippet-error')} className="field-error" role="alert">{issue}</p>}
        <label className="setting-toggle"><input type="checkbox" checked={!!action.press_enter} onChange={event => onChange({ ...action, press_enter: event.target.checked })} />Press Return after pasting</label>
        <p className="fine-print">Up to {maxTextBytes} bytes. Pasting replaces the clipboard; plain text already on it is put back afterwards, other content is not. Needs macOS Accessibility permission.</p></div>;
    }
    case 'media': return <div className="action-fields"><label htmlFor={id('media-command')}>Media command</label><SelectField id={id('media-command')} value={action.command} onChange={event => onChange({ type: 'media', command: event.target.value as MediaCommand })}>{(Object.keys(mediaLabels) as MediaCommand[]).map(command => <option key={command} value={command}>{mediaLabels[command]}</option>)}</SelectField><p className="fine-print">{action.command === 'stop'
      ? 'macOS has no stop media key, so this stops Apple Music or pauses Spotify, only when one is already running. It does not start a player that is not running; if one quits at the very instant of the press, macOS may relaunch it, without playing. Needs macOS Automation permission for the player.'
      : 'Sends the media key to whichever app controls playback, like the key on a keyboard. Needs macOS Accessibility permission.'}</p></div>;
    case 'obs_scene': case 'obs_virtual_camera': case 'obs_replay_buffer': case 'obs_studio_mode': case 'obs_transition':
    case 'obs_filter': case 'obs_source_visibility': case 'obs_scene_collection': case 'obs_profile': case 'obs_set_input_volume':
      return <ObsFields action={action} prefix={prefix} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} />;
    case 'system': return <div className="action-fields"><label htmlFor={id('system-command')}>System action</label><SelectField id={id('system-command')} value={action.command} onChange={event => onChange({ type: 'system', command: event.target.value as SystemCommand })}>{(Object.keys(systemLabels) as SystemCommand[]).map(command => <option key={command} value={command}>{systemLabels[command]}</option>)}</SelectField><p className="fine-print">{action.command === 'screenshot' ? 'Captures the whole screen to the clipboard. Needs macOS Screen Recording permission.' : `Sends ${action.command === 'undo' ? 'Command-Z' : 'Shift-Command-Z'} to the focused app.`}</p></div>;
  }
}

// The one OBS editor for all nine obs_* actions: an Operation select that moves the action between
// operations (keeping the names both share) and the chosen operation's fields. The name fields are
// comboboxes when catalog holds OBS's matching list (scenes, sources, the filters of the named
// source, the items of the named scene, scene collections, profiles) and plain inputs otherwise;
// typing always works, and a name OBS does not list is said inline. The connection status shows
// once per editor — here for a top-level action, and once for a multi-action with OBS steps (in
// MultiActionFields), never per step. Ids keep their historical names per operation.
export function ObsFields({ action, prefix, onChange, onSetUpObs, catalog }: {
  action: ObsStepAction; prefix: string; onChange(action: StepAction, field?: string): void; onSetUpObs?: () => void; catalog?: ObsCatalogView;
}) {
  const id = (name: string) => `${prefix}${name}`;
  const scenes = catalogNames(catalog, catalog?.names?.scenes);
  const collections = catalogNames(catalog, catalog?.names?.scene_collections);
  const profiles = catalogNames(catalog, catalog?.names?.profiles);
  let fields: ReactNode;
  switch (action.type) {
    case 'obs_virtual_camera':
      fields = <><label htmlFor={id('obs-virtual-camera-op')}>Action</label>
        <SelectField id={id('obs-virtual-camera-op')} value={action.op} onChange={event => onChange({ type: 'obs_virtual_camera', op: event.target.value as ObsOutputOp }, 'obs-virtual-camera-op')}>
          {(Object.keys(obsOutputOpLabels) as ObsOutputOp[]).map(op => <option key={op} value={op}>{obsOutputOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={obsSwitchHint}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_replay_buffer':
      fields = <><label htmlFor={id('obs-replay-buffer-op')}>Action</label>
        <SelectField id={id('obs-replay-buffer-op')} value={action.op} onChange={event => onChange({ type: 'obs_replay_buffer', op: event.target.value as ObsReplayBufferOp }, 'obs-replay-buffer-op')}>
          {(Object.keys(obsReplayBufferOpLabels) as ObsReplayBufferOp[]).map(op => <option key={op} value={op}>{obsReplayBufferOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={`Saving needs the buffer to be running in OBS. ${obsSwitchHint}`}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_studio_mode':
      fields = <><label htmlFor={id('obs-studio-mode-op')}>Action</label>
        <ObsSwitchSelect id={id('obs-studio-mode-op')} value={action.op} onChange={op => onChange({ type: 'obs_studio_mode', op }, 'obs-studio-mode-op')} />
        <Hint hint={obsSwitchHint}>{obsSwitchMore}</Hint></>;
      break;
    case 'obs_transition':
      fields = <p className="fine-print">Triggers the studio-mode transition from the preview to the program scene. OBS refuses it while studio mode is off, and the press is logged as failed.</p>;
      break;
    case 'obs_filter': {
      // A filter can live on an input or on a scene, like in OBS's own filters dialog.
      const targets = filterTargets(catalog);
      const filters = catalog?.list('filters', action.source) ?? null;
      fields = <>
        <ObsNameRow id={id('obs-filter-source')} label="Source name" value={action.source} entry={targets} what="source"
          issue={obsNameIssue(action.source, 'source')} onChange={source => onChange({ ...action, source }, 'obs-filter-source')} />
        <ObsNameRow id={id('obs-filter-name')} label="Filter name" value={action.filter} entry={filters} what="filter" spaced
          issue={obsNameIssue(action.filter, 'filter')} onChange={filter => onChange({ ...action, filter }, 'obs-filter-name')}
          onOpenList={() => catalog?.requestNames('filters', action.source)} />
        <label className="spaced-label" htmlFor={id('obs-filter-op')}>Action</label><ObsSwitchSelect id={id('obs-filter-op')} value={action.op} onChange={op => onChange({ ...action, op }, 'obs-filter-op')} />
        <Hint hint={`Both names are matched exactly (up to ${maxObsSceneBytes} bytes each). ${obsSwitchHint}`}>{obsSwitchMore}</Hint></>;
      break;
    }
    case 'obs_source_visibility': {
      // The source choices are the items of the named scene, which is exactly what a visibility
      // there can target — fetched when the field's choices are opened.
      const inScene = catalog?.list('sceneSources', action.scene) ?? null;
      fields = <>
        <ObsNameRow id={id('obs-visibility-scene')} label="Scene name" value={action.scene} entry={scenes} what="scene"
          issue={obsNameIssue(action.scene, 'scene')} onChange={scene => onChange({ ...action, scene }, 'obs-visibility-scene')} />
        <ObsNameRow id={id('obs-visibility-source')} label="Source name" value={action.source} entry={inScene} what="source" spaced
          issue={obsNameIssue(action.source, 'source')} onChange={source => onChange({ ...action, source }, 'obs-visibility-source')}
          onOpenList={() => catalog?.requestNames('sceneSources', action.scene)} />
        <label className="spaced-label" htmlFor={id('obs-visibility-op')}>Action</label><ObsSwitchSelect id={id('obs-visibility-op')} value={action.op} onChange={op => onChange({ ...action, op }, 'obs-visibility-op')} />
        <Hint hint={`Shows or hides the source in that scene (a group name works as the scene). Both names are matched exactly (up to ${maxObsSceneBytes} bytes each). ${obsSwitchHint}`}>{obsSwitchMore}</Hint></>;
      break;
    }
    case 'obs_scene_collection': case 'obs_profile': {
      const collection = action.type === 'obs_scene_collection';
      const issue = obsNameIssue(action.name, collection ? 'scene collection' : 'profile');
      fields = <>
        <ObsNameRow id={id('obs-reload-name')} label={collection ? 'OBS scene collection name' : 'OBS profile name'} value={action.name}
          entry={collection ? collections : profiles} what={collection ? 'scene collection' : 'profile'} issue={issue}
          onChange={name => onChange({ ...action, name }, 'obs-reload-name')} />
        <Hint hint={`Switches OBS to this ${collection ? 'scene collection' : 'profile'}, matched exactly (up to ${maxObsSceneBytes} bytes). ${obsReloadHint}`}>{obsReloadMore}</Hint></>;
      break;
    }
    case 'obs_set_input_volume': {
      fields = <>
        <ObsNameRow id={id('obs-input-volume-input')} label="OBS input name" value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what="input"
          issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ ...action, input }, 'obs-input-volume-input')} />
        <label className="spaced-label" htmlFor={id('obs-input-volume-percent')}>Volume (%)</label>
        <IntegerField id={id('obs-input-volume-percent')} value={action.percent} minimum={0} maximum={100} onCommit={percent => onChange({ ...action, percent }, 'obs-input-volume-percent')} />
        <p className="fine-print">Sets the OBS input's volume to this level: 0% is silent, 100% is unity gain (0 dB). To follow a fader instead, bind a fader.</p></>;
      break;
    }
    case 'obs_scene': {
      fields = <>
        <ObsNameRow id={id('obs-scene')} label="OBS scene name" value={action.scene} entry={scenes} what="scene"
          issue={obsSceneIssue(action.scene)} onChange={scene => onChange({ type: 'obs_scene', scene }, 'obs-scene')} />
        <p className="fine-print">Makes this scene the current OBS program scene, matched exactly (up to {maxObsSceneBytes} bytes).</p></>;
      break;
    }
  }
  return <div className="action-fields">
    <label htmlFor={id('obs-operation')}>Operation</label>
    <SelectField id={id('obs-operation')} value={action.type} onChange={event => onChange(switchObsOperation(action, event.target.value as ObsStepType), 'obs-operation')}>
      {(Object.keys(obsOperationLabels) as ObsStepType[]).map(type => <option key={type} value={type}>{obsOperationLabels[type]}</option>)}
    </SelectField>
    {fields}
    {prefix === '' && <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />}
  </div>;
}
