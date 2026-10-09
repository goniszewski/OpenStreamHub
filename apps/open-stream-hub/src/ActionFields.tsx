import { Plus, Trash2 } from 'lucide-react';
import type { ReactNode } from 'react';
import type { BindingAction, MediaCommand, Modifier, ObsOutputOp, ObsReplayBufferOp, ObsSwitchOp, ProfileDirection, StepAction, SystemCommand, TimerSound } from './contracts';
import { ActionPickerField } from './ActionPicker';
import { defaultBindingAction } from './action-picker-model';
import type { ObsStepAction, ObsStepType, StepActionType } from './action-model';
import { IntegerField, SelectField } from './controls';
import {
  defaultStepAction, defaultTimerSound, maxObsSceneBytes, maxRunningTimers, maxTextBytes, mediaLabels, multiActionSteps,
  namedKeys, obsNameIssue, obsOperationLabels, obsOutputOpLabels, obsReplayBufferOpLabels, obsSceneIssue,
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

// The action picker and the parameter fields of the chosen action. Top-level ids keep their
// historical names; multi-action steps prefix theirs with `step-N-`. onSetUpObs opens
// Settings, where the OBS connection is configured.
export function ActionFields({ action, fader, selectorKey, onChange, onSetUpObs }: {
  action: BindingAction | null; fader: boolean; selectorKey: string; onChange: Change; onSetUpObs?: () => void;
}) {
  return <>
    <label htmlFor="binding-action">Action</label>
    <ActionPickerField triggerId="binding-action" scope={fader ? 'binding-fader' : 'binding'} action={action}
      onAssign={type => onChange(type === null ? null : defaultBindingAction(type))} />
    {action?.type === 'volume_from_position' && <p className="fine-print">Each position change plans an absolute output volume from 0% to 100%. It does not add a volume delta.</p>}
    {action?.type === 'multi_action' && <MultiActionFields steps={action.actions} selectorKey={selectorKey} onChange={onChange} onSetUpObs={onSetUpObs} />}
    {action?.type === 'profile' && <div className="action-fields"><label htmlFor="profile-direction">Profile</label>
      <SelectField id="profile-direction" value={action.direction} onChange={event => onChange({ type: 'profile', direction: event.target.value as ProfileDirection })}>
        {(Object.keys(profileDirectionLabels) as ProfileDirection[]).map(direction => <option key={direction} value={direction}>{profileDirectionLabels[direction]}</option>)}
      </SelectField>
      <Hint hint="Switches the run to the next or previous saved profile, in name order.">It wraps around at the ends. Only runs started from this app switch; nothing happens with a single saved profile or outside this app. When actions run on this Mac, they pause until you confirm the new profile. It cannot be a multi-action step.</Hint></div>}
    {action?.type === 'timer' && <TimerFields action={action} inputKey={selectorKey} onChange={onChange} />}
    {action && action.type !== 'volume_from_position' && action.type !== 'multi_action' && action.type !== 'profile' && action.type !== 'timer' &&
      <StepFields prefix="" action={action} inputKey={selectorKey} onChange={(next, field) => onChange(next, field)} onSetUpObs={onSetUpObs} />}
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
    <Hint hint="When the countdown ends, macOS shows a notification and plays the sound.">1 second to 24 hours. The countdown runs in the app; pressing the same control again cancels it. The countdown is not shown on the key, and stopping the run (or the Mac going to sleep) cancels every timer. At most {maxRunningTimers} run at once. It cannot be a multi-action step.</Hint>
  </div>;
}

function MultiActionFields({ steps, selectorKey, onChange, onSetUpObs }: { steps: StepAction[]; selectorKey: string; onChange: Change; onSetUpObs?: () => void }) {
  const update = (next: StepAction[], field?: string) => onChange({ type: 'multi_action', actions: next }, field);
  return <div className="action-fields">
    <p className="fine-print">Runs {multiActionSteps.minimum}–{multiActionSteps.maximum} steps in order, 280 ms apart. The first failed step stops the rest. Steps cannot be multi-actions.</p>
    <ol className="action-steps">
      {steps.map((step, index) => {
        const prefix = `step-${index + 1}-`;
        return <li key={`${index}:${step.type}`}>
          <label htmlFor={`${prefix}action`}>Step {index + 1}</label>
          <ActionPickerField triggerId={`${prefix}action`} idPrefix={prefix} scope="step" action={step} allowDrag={false}
            onAssign={type => { if (type) update(steps.map((item, at) => at === index ? defaultStepAction(type as StepActionType) : item)); }} />
          <StepFields prefix={prefix} action={step} inputKey={`${selectorKey}:${index}:${steps.length}`}
            onChange={(next, field) => update(steps.map((item, at) => at === index ? next : item), field && `${prefix}${field}`)} onSetUpObs={onSetUpObs} />
          <button type="button" className="button compact" disabled={steps.length <= multiActionSteps.minimum}
            aria-label={`Remove step ${index + 1}`} onClick={() => update(steps.filter((_, at) => at !== index))}><Trash2 size={16} />Remove</button>
        </li>;
      })}
    </ol>
    <button type="button" className="button compact" disabled={steps.length >= multiActionSteps.maximum}
      onClick={() => update([...steps, defaultStepAction('toggle_mute')])}><Plus size={16} />Add step</button>
  </div>;
}

function StepFields({ prefix, action, inputKey, onChange, onSetUpObs }: {
  prefix: string; action: StepAction; inputKey: string; onChange(action: StepAction, field?: string): void; onSetUpObs?: () => void;
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
    case 'obs_filter': case 'obs_source_visibility': case 'obs_scene_collection': case 'obs_profile':
      return <ObsFields action={action} prefix={prefix} onChange={onChange} onSetUpObs={onSetUpObs} />;
    case 'system': return <div className="action-fields"><label htmlFor={id('system-command')}>System action</label><SelectField id={id('system-command')} value={action.command} onChange={event => onChange({ type: 'system', command: event.target.value as SystemCommand })}>{(Object.keys(systemLabels) as SystemCommand[]).map(command => <option key={command} value={command}>{systemLabels[command]}</option>)}</SelectField><p className="fine-print">{action.command === 'screenshot' ? 'Captures the whole screen to the clipboard. Needs macOS Screen Recording permission.' : `Sends ${action.command === 'undo' ? 'Command-Z' : 'Shift-Command-Z'} to the focused app.`}</p></div>;
  }
}

// The one OBS editor for all nine obs_* actions: an Operation select that moves the action between
// operations (keeping the names both share), the chosen operation's fields, and a single connection
// status with the Set up link to Settings. Ids keep their historical names per operation.
export function ObsFields({ action, prefix, onChange, onSetUpObs }: {
  action: ObsStepAction; prefix: string; onChange(action: StepAction, field?: string): void; onSetUpObs?: () => void;
}) {
  const id = (name: string) => `${prefix}${name}`;
  let fields: ReactNode;
  switch (action.type) {
    case 'obs_virtual_camera':
      fields = <><label htmlFor={id('obs-virtual-camera-op')}>Virtual camera</label>
        <SelectField id={id('obs-virtual-camera-op')} value={action.op} onChange={event => onChange({ type: 'obs_virtual_camera', op: event.target.value as ObsOutputOp }, 'obs-virtual-camera-op')}>
          {(Object.keys(obsOutputOpLabels) as ObsOutputOp[]).map(op => <option key={op} value={op}>{obsOutputOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={obsSwitchHint}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_replay_buffer':
      fields = <><label htmlFor={id('obs-replay-buffer-op')}>Replay buffer</label>
        <SelectField id={id('obs-replay-buffer-op')} value={action.op} onChange={event => onChange({ type: 'obs_replay_buffer', op: event.target.value as ObsReplayBufferOp }, 'obs-replay-buffer-op')}>
          {(Object.keys(obsReplayBufferOpLabels) as ObsReplayBufferOp[]).map(op => <option key={op} value={op}>{obsReplayBufferOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={`Saving needs the buffer to be running in OBS. ${obsSwitchHint}`}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_studio_mode':
      fields = <><label htmlFor={id('obs-studio-mode-op')}>Studio mode</label>
        <ObsSwitchSelect id={id('obs-studio-mode-op')} value={action.op} onChange={op => onChange({ type: 'obs_studio_mode', op }, 'obs-studio-mode-op')} />
        <Hint hint={obsSwitchHint}>{obsSwitchMore}</Hint></>;
      break;
    case 'obs_transition':
      fields = <p className="fine-print">Triggers the studio-mode transition from the preview to the program scene. OBS refuses it while studio mode is off, and the press is logged as failed.</p>;
      break;
    case 'obs_filter': {
      const sourceIssue = obsNameIssue(action.source, 'source');
      const filterIssue = obsNameIssue(action.filter, 'filter');
      fields = <>
        <label htmlFor={id('obs-filter-source')}>Source name</label><input id={id('obs-filter-source')} spellCheck={false} value={action.source} aria-invalid={!!sourceIssue} aria-describedby={sourceIssue ? id('obs-filter-source-error') : undefined} onChange={event => onChange({ ...action, source: event.target.value }, 'obs-filter-source')} />{sourceIssue && <p id={id('obs-filter-source-error')} className="field-error" role="alert">{sourceIssue}</p>}
        <label className="spaced-label" htmlFor={id('obs-filter-name')}>Filter name</label><input id={id('obs-filter-name')} spellCheck={false} value={action.filter} aria-invalid={!!filterIssue} aria-describedby={filterIssue ? id('obs-filter-name-error') : undefined} onChange={event => onChange({ ...action, filter: event.target.value }, 'obs-filter-name')} />{filterIssue && <p id={id('obs-filter-name-error')} className="field-error" role="alert">{filterIssue}</p>}
        <label className="spaced-label" htmlFor={id('obs-filter-op')}>Filter</label><ObsSwitchSelect id={id('obs-filter-op')} value={action.op} onChange={op => onChange({ ...action, op }, 'obs-filter-op')} />
        <Hint hint={`Both names are matched exactly (up to ${maxObsSceneBytes} bytes each). ${obsSwitchHint}`}>{obsSwitchMore}</Hint></>;
      break;
    }
    case 'obs_source_visibility': {
      const sceneIssue = obsNameIssue(action.scene, 'scene');
      const sourceIssue = obsNameIssue(action.source, 'source');
      fields = <>
        <label htmlFor={id('obs-visibility-scene')}>Scene name</label><input id={id('obs-visibility-scene')} spellCheck={false} value={action.scene} aria-invalid={!!sceneIssue} aria-describedby={sceneIssue ? id('obs-visibility-scene-error') : undefined} onChange={event => onChange({ ...action, scene: event.target.value }, 'obs-visibility-scene')} />{sceneIssue && <p id={id('obs-visibility-scene-error')} className="field-error" role="alert">{sceneIssue}</p>}
        <label className="spaced-label" htmlFor={id('obs-visibility-source')}>Source name</label><input id={id('obs-visibility-source')} spellCheck={false} value={action.source} aria-invalid={!!sourceIssue} aria-describedby={sourceIssue ? id('obs-visibility-source-error') : undefined} onChange={event => onChange({ ...action, source: event.target.value }, 'obs-visibility-source')} />{sourceIssue && <p id={id('obs-visibility-source-error')} className="field-error" role="alert">{sourceIssue}</p>}
        <label className="spaced-label" htmlFor={id('obs-visibility-op')}>Visibility</label><ObsSwitchSelect id={id('obs-visibility-op')} value={action.op} onChange={op => onChange({ ...action, op }, 'obs-visibility-op')} />
        <Hint hint={`Shows or hides the source in that scene (a group name works as the scene). Both names are matched exactly (up to ${maxObsSceneBytes} bytes each). ${obsSwitchHint}`}>{obsSwitchMore}</Hint></>;
      break;
    }
    case 'obs_scene_collection': case 'obs_profile': {
      const collection = action.type === 'obs_scene_collection';
      const issue = obsNameIssue(action.name, collection ? 'scene collection' : 'profile');
      fields = <><label htmlFor={id('obs-reload-name')}>{collection ? 'OBS scene collection name' : 'OBS profile name'}</label><input id={id('obs-reload-name')} spellCheck={false} value={action.name} aria-invalid={!!issue} aria-describedby={issue ? id('obs-reload-name-error') : undefined} onChange={event => onChange({ ...action, name: event.target.value }, 'obs-reload-name')} />{issue && <p id={id('obs-reload-name-error')} className="field-error" role="alert">{issue}</p>}
        <Hint hint={`Switches OBS to this ${collection ? 'scene collection' : 'profile'}, matched exactly (up to ${maxObsSceneBytes} bytes). ${obsReloadHint}`}>{obsReloadMore}</Hint></>;
      break;
    }
    case 'obs_scene': {
      const issue = obsSceneIssue(action.scene);
      fields = <><label htmlFor={id('obs-scene')}>OBS scene name</label><input id={id('obs-scene')} spellCheck={false} value={action.scene} aria-invalid={!!issue} aria-describedby={issue ? id('obs-scene-error') : undefined} onChange={event => onChange({ type: 'obs_scene', scene: event.target.value }, 'obs-scene')} />{issue && <p id={id('obs-scene-error')} className="field-error" role="alert">{issue}</p>}
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
    <div className="obs-connection" role="note">
      <p className="fine-print">Connects to OBS 28 or later on this Mac while OBS runs with its WebSocket server on. <button type="button" className="obs-setup" onClick={onSetUpObs}>Set up in Settings</button></p>
      <details className="learn-more"><summary>Learn more</summary><p className="fine-print">The connection goes to this Mac only. Without it, the press is logged as unavailable and nothing runs.</p></details>
    </div>
  </div>;
}
