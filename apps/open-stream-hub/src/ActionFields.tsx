import { Plus, Trash2 } from 'lucide-react';
import type { BindingAction, MediaCommand, Modifier, ProfileDirection, StepAction, SystemCommand, TimerSound } from './contracts';
import { IntegerField, SelectField } from './controls';
import {
  defaultMultiAction, defaultProfileAction, defaultStepAction, defaultTimerAction, defaultTimerSound, maxObsSceneBytes,
  maxRunningTimers, maxTextBytes, mediaLabels, multiActionSteps, namedKeys, obsSceneIssue, profileDirectionLabels,
  stepActionTypes, systemLabels, textIssue, timerLabelIssue, timerSeconds, timerSoundLabels, urlIssue,
} from './action-model';
import type { StepActionType } from './action-model';

type Change = (action: BindingAction | null, field?: string) => void;

// The action select and its parameter fields. Top-level ids keep their
// historical names; multi-action steps prefix theirs with `step-N-`.
export function ActionFields({ action, fader, selectorKey, onChange }: {
  action: BindingAction | null; fader: boolean; selectorKey: string; onChange: Change;
}) {
  return <>
    <label htmlFor="binding-action">Action</label>
    <SelectField id="binding-action" value={action?.type ?? 'unbound'} onChange={event => {
      const type = event.target.value;
      onChange(type === 'unbound' ? null : type === 'volume_from_position' ? { type } : type === 'multi_action' ? defaultMultiAction()
        : type === 'profile' ? defaultProfileAction() : type === 'timer' ? defaultTimerAction() : defaultStepAction(type as StepActionType));
    }}>
      <option value="unbound">Unbound</option>
      {fader ? <option value="volume_from_position">Output volume follows position</option> : <>
        {stepActionTypes.map(([type, label]) => <option key={type} value={type}>{label}</option>)}
        <option value="multi_action">Multi-action</option>
        <option value="profile">Switch profile</option>
        <option value="timer">Timer</option>
      </>}
    </SelectField>
    {action?.type === 'volume_from_position' && <p className="fine-print">Each position change plans an absolute output volume from 0% to 100%. It does not add a volume delta.</p>}
    {action?.type === 'multi_action' && <MultiActionFields steps={action.actions} selectorKey={selectorKey} onChange={onChange} />}
    {action?.type === 'profile' && <div className="action-fields"><label htmlFor="profile-direction">Profile</label>
      <SelectField id="profile-direction" value={action.direction} onChange={event => onChange({ type: 'profile', direction: event.target.value as ProfileDirection })}>
        {(Object.keys(profileDirectionLabels) as ProfileDirection[]).map(direction => <option key={direction} value={direction}>{profileDirectionLabels[direction]}</option>)}
      </SelectField>
      <p className="fine-print">Switches the run to the next or previous saved profile, in name order, wrapping around. Only runs started from this app switch; it does nothing with a single saved profile or in the standalone service. When actions run on this Mac, they pause until you confirm the new profile. It cannot be a multi-action step.</p></div>}
    {action?.type === 'timer' && <TimerFields action={action} inputKey={selectorKey} onChange={onChange} />}
    {action && action.type !== 'volume_from_position' && action.type !== 'multi_action' && action.type !== 'profile' && action.type !== 'timer' &&
      <StepFields prefix="" action={action} inputKey={selectorKey} onChange={(next, field) => onChange(next, field)} />}
  </>;
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
    <p className="fine-print">1 second to 24 hours. The running service counts down; pressing the same control again cancels it. When it ends, macOS shows a notification and plays the sound. The countdown is not shown on the key, and stopping the run (or the Mac going to sleep) cancels every timer. At most {maxRunningTimers} run at once. It cannot be a multi-action step.</p>
  </div>;
}

function MultiActionFields({ steps, selectorKey, onChange }: { steps: StepAction[]; selectorKey: string; onChange: Change }) {
  const update = (next: StepAction[], field?: string) => onChange({ type: 'multi_action', actions: next }, field);
  return <div className="action-fields">
    <p className="fine-print">Runs {multiActionSteps.minimum}–{multiActionSteps.maximum} steps in order, 280 ms apart. The first failed step stops the rest. Steps cannot be multi-actions.</p>
    <ol className="action-steps">
      {steps.map((step, index) => {
        const prefix = `step-${index + 1}-`;
        return <li key={`${index}:${step.type}`}>
          <label htmlFor={`${prefix}action`}>Step {index + 1}</label>
          <SelectField id={`${prefix}action`} value={step.type} onChange={event => update(steps.map((item, at) => at === index ? defaultStepAction(event.target.value as StepActionType) : item))}>
            {stepActionTypes.map(([type, label]) => <option key={type} value={type}>{label}</option>)}
          </SelectField>
          <StepFields prefix={prefix} action={step} inputKey={`${selectorKey}:${index}:${steps.length}`}
            onChange={(next, field) => update(steps.map((item, at) => at === index ? next : item), field && `${prefix}${field}`)} />
          <button type="button" className="button compact" disabled={steps.length <= multiActionSteps.minimum}
            aria-label={`Remove step ${index + 1}`} onClick={() => update(steps.filter((_, at) => at !== index))}><Trash2 size={16} />Remove</button>
        </li>;
      })}
    </ol>
    <button type="button" className="button compact" disabled={steps.length >= multiActionSteps.maximum}
      onClick={() => update([...steps, defaultStepAction('toggle_mute')])}><Plus size={16} />Add step</button>
  </div>;
}

function StepFields({ prefix, action, inputKey, onChange }: {
  prefix: string; action: StepAction; inputKey: string; onChange(action: StepAction, field?: string): void;
}) {
  const id = (name: string) => `${prefix}${name}`;
  switch (action.type) {
    case 'toggle_mute': return null;
    case 'set_volume': return <div className="action-fields"><label htmlFor={id('volume-percent')}>Output volume (%)</label><IntegerField key={inputKey} id={id('volume-percent')} value={action.percent} minimum={0} maximum={100} onCommit={percent => onChange({ type: 'set_volume', percent }, 'volume-percent')} /><p className="fine-print">Plans an absolute output volume from 0% to 100%.</p></div>;
    case 'volume': return <div className="action-fields"><label htmlFor={id('volume-delta')}>Volume change (points)</label><IntegerField key={inputKey} id={id('volume-delta')} value={action.delta} minimum={-100} maximum={100} nonZero onCommit={delta => onChange({ type: 'volume', delta }, 'volume-delta')} /><p className="fine-print">Signed percentage points, −100 to +100, excluding zero.</p></div>;
    case 'shortcut': {
      const named = namedKeys.some(([name]) => name === action.key);
      return <div className="action-fields">
        <label htmlFor={id('shortcut-key-kind')}>Key</label>
        <SelectField id={id('shortcut-key-kind')} value={named ? action.key : 'character'} onChange={event => onChange({ ...action, key: event.target.value === 'character' ? 'a' : event.target.value }, 'shortcut-key')}>
          <option value="character">Letter, digit or space</option>
          {namedKeys.map(([name, label]) => <option key={name} value={name}>{label}</option>)}
        </SelectField>
        {!named && <><label className="spaced-label" htmlFor={id('shortcut-key')}>Character</label><input id={id('shortcut-key')} value={action.key} maxLength={1} onChange={event => onChange({ ...action, key: event.target.value }, 'shortcut-key')} /></>}
        <p className="fine-print">One ASCII letter, digit or space, or a named key such as an arrow, Return or F1–F20. Punctuation and key sequences are not supported.</p>
        <fieldset className="modifiers"><legend>Modifiers</legend>{(['command', 'control', 'option', 'shift'] as Modifier[]).map(modifier => <label key={modifier}><input type="checkbox" checked={action.modifiers.includes(modifier)} onChange={event => onChange({ ...action, modifiers: event.target.checked ? [...action.modifiers, modifier] : action.modifiers.filter(item => item !== modifier) })} />{modifier[0].toUpperCase() + modifier.slice(1)}</label>)}</fieldset>
      </div>;
    }
    case 'launch_app': return <div className="action-fields"><label htmlFor={id('app-bundle-id')}>Application bundle ID</label><input id={id('app-bundle-id')} spellCheck={false} value={action.bundle_id} onChange={event => onChange({ type: 'launch_app', bundle_id: event.target.value }, 'bundle-id')} /><p className="fine-print">For example, com.apple.finder. The service validates the identifier; replay does not open it.</p></div>;
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
    case 'obs_scene': {
      const issue = obsSceneIssue(action.scene);
      return <div className="action-fields"><label htmlFor={id('obs-scene')}>OBS scene name</label><input id={id('obs-scene')} spellCheck={false} value={action.scene} aria-invalid={!!issue} aria-describedby={issue ? id('obs-scene-error') : undefined} onChange={event => onChange({ type: 'obs_scene', scene: event.target.value }, 'obs-scene')} />{issue && <p id={id('obs-scene-error')} className="field-error" role="alert">{issue}</p>}
        <p className="fine-print">Makes this scene the current OBS program scene, matched exactly (up to {maxObsSceneBytes} bytes). Needs OBS 28 or later with its WebSocket server on, and OBS turned on in Settings; the connection goes to this Mac only. Without it the press is logged as unavailable and nothing runs.</p></div>;
    }
    case 'system': return <div className="action-fields"><label htmlFor={id('system-command')}>System action</label><SelectField id={id('system-command')} value={action.command} onChange={event => onChange({ type: 'system', command: event.target.value as SystemCommand })}>{(Object.keys(systemLabels) as SystemCommand[]).map(command => <option key={command} value={command}>{systemLabels[command]}</option>)}</SelectField><p className="fine-print">{action.command === 'screenshot' ? 'Captures the whole screen to the clipboard. Needs macOS Screen Recording permission.' : `Sends ${action.command === 'undo' ? 'Command-Z' : 'Shift-Command-Z'} to the focused app.`}</p></div>;
  }
}
