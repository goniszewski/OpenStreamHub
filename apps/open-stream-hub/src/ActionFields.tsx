import { AppWindow, Check, ChevronDown, FolderOpen, Plus, RotateCcw, Trash2 } from 'lucide-react';
import { rememberAppName } from './app-names';
import { useEffect, useRef, useState, useSyncExternalStore } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent, ReactNode } from 'react';
import type { BindingAction, ConditionalAction, MediaCommand, Modifier, ObsNameList, ObsMediaOp, ObsMuteOp, ObsOutputOp, ObsRecordOp, ObsReplayBufferOp, ObsSwitchOp, ProfileDirection, RunningApp, StepAction, StreamlabsVisibilityOp, SystemCommand, TerminalApp, TimerSound } from './contracts';
import { ActionPickerField, focusTrigger } from './ActionPicker';
import { defaultBindingAction, pickerWebStorage } from './action-picker-model';
import { appIcons, appListKey, appOptions, appSections, closedAppList, openAppList, queryAppList, readRecentApps, recordRecentApp, triggerKey } from './app-picker-model';
import type { AppListState } from './app-picker-model';
import { dataUrl } from './artwork-model';
import type { ObsStepAction, ObsStepType, RunProgramAction, StreamlabsAudioMode, StreamlabsStepAction, StreamlabsStepType, TerminalCommandAction } from './action-model';
import { FlowEditor } from './FlowEditor';
import { HotkeyField } from './key-capture';
import { VariableActionFields } from './VariableFields';
import { useVariables } from './variables-context';
import { actionVariableNames } from './variable-model';
import { MacroFields } from './MacroFields';
import { PluginFields } from './PluginFields';
import { catalogNames, filterTargets } from './obs-catalog';
import type { ObsCatalogView } from './obs-catalog';
import { obsConnectionLabels } from './obs-settings';
import { listRunningApps } from './focus-rules';
import { streamlabsSceneNames } from './streamlabs-catalog';
import type { StreamlabsCatalogView } from './streamlabs-catalog';
import { streamlabsConnectionLabels } from './streamlabs-settings';
import type { Invoke } from './profile-session';
import { IntegerField, SelectField } from './controls';
import { useT } from './i18n-react';
import type { Translate } from './i18n-react';
import {
  conditionUsesObs, defaultStepAction, defaultTimerSound, isFaderTemplate, isMediaKey, maxObsSceneBytes, maxRunningTimers, maxTextBytes, mediaKeyLabels, mediaLabels, multiActionSteps,
  obsMissingWarning, obsNameFits, obsNameIssue, obsOperationLabels, obsMediaOpLabels, obsMuteOpLabels, obsOutputOpLabels, obsRecordOpLabels, obsReplayBufferOpLabels, obsSceneIssue,
  obsSwitchOpLabels, profileDirectionLabels, streamlabsAudioLimits, streamlabsAudioMode, streamlabsAudioModeLabels, streamlabsNameIssue, streamlabsOperationLabels,
  streamlabsVisibilityOpLabels, switchObsOperation, switchStreamlabsOperation, systemLabels, terminalAppLabels, terminalCommandIssue, terminalDirectoryIssue, maxTerminalCommandBytes, maxRunProgramArgs, programArgIssue, programArgsIssue, programDashHint, programPathIssue, programVariableIssue, textIssue, timerLabelIssue, timerSeconds,
  timerSoundLabels, urlIssue, defaultStreamlabsAudioOp,
} from './action-model';

type Change = (action: BindingAction | null, field?: string) => void;

// One-line hint with the full story behind a "Learn more" disclosure.
function Hint({ hint, children }: { hint: string; children: ReactNode }) {
  const t = useT();
  return <>
    <p className="fine-print">{hint}</p>
    <details className="learn-more"><summary>{t('actions.learnMore')}</summary><p className="fine-print">{children}</p></details>
  </>;
}


function withConfirm(action: { type: 'obs_stream'; op: ObsOutputOp }, on: boolean): StepAction {
  return on ? { type: 'obs_stream', op: action.op, confirm: 'press_twice' } : { type: 'obs_stream', op: action.op };
}

// The press-twice note under a stream's checkbox, shared by OBS and Streamlabs.
const confirmNote = (t: Translate, on: boolean) => t(on ? 'actions.confirm.on' : 'actions.confirm.off');

function ObsSwitchSelect({ id, value, onChange }: { id: string; value: ObsSwitchOp; onChange(op: ObsSwitchOp): void }) {
  return <SelectField id={id} value={value} onChange={event => onChange(event.target.value as ObsSwitchOp)}>{(Object.keys(obsSwitchOpLabels) as ObsSwitchOp[]).map(op => <option key={op} value={op}>{obsSwitchOpLabels[op]}</option>)}</SelectField>;
}

// The one OBS connection status: shown once per editor (top-level OBS action, or a multi-action
// with OBS steps), never per step. The Set up link renders only where it can navigate. The OBS
// listings (the name fields' choices) hang off the same note: the Refresh press asks OBS again,
// and a failed listing says why in the place where the choices would be.
export function ObsConnection({ onSetUpObs, catalog }: { onSetUpObs?: () => void; catalog?: ObsCatalogView }) {
  const t = useT();
  return <div className="obs-connection" role="note">
    <p className="fine-print">{t('actions.obs.connects')} {onSetUpObs && <button type="button" className="obs-setup" onClick={onSetUpObs}>{t('actions.setUp')}</button>}</p>
    {catalog && (catalog.desktop || catalog.status !== 'idle') && <div className="obs-catalog">
      <button type="button" className="button compact" disabled={!catalog.desktop || catalog.status === 'loading'} onClick={catalog.refresh}>
        <RotateCcw size={15} />{catalog.status === 'loading' ? t('actions.listing') : t('actions.obs.refresh')}
      </button>
      {catalog.connection && <span className={`obs-chip obs-chip-${catalog.connection.state}`} title={catalog.connection.message}>{obsConnectionLabels[catalog.connection.state]}</span>}
      {catalog.status === 'failed' && <p className="fine-print obs-catalog-error" role="note">{catalog.error}</p>}
    </div>}
    <details className="learn-more"><summary>{t('actions.learnMore')}</summary><p className="fine-print">{t('actions.obs.learnMore')}</p></details>
  </div>;
}

// One OBS name field: a text input that becomes a combobox while the editor holds OBS's matching
// list — click or ArrowDown opens the choices, typing narrows them, and a name OBS does not list
// is said inline. Typing always wins: the press matches names exactly, and OBS can be asked again
// with Refresh. entry is null wherever there is no list (browser preview, OBS off or unreachable),
// which renders exactly the plain input. Hook-free and controlled (open state and the active
// option live in ObsNameField), so tests call it as a function.
const comboLimit = 50;

export function ObsCombo({ id, value, entry, open, onOpenChange, active, onActiveChange, onType, onPick, onOpenList, invalid, describedBy, service = 'OBS' }: {
  id: string; value: string; entry: ObsNameList | null; open: boolean; onOpenChange(open: boolean): void;
  active: number; onActiveChange(active: number): void; onType(value: string): void; onPick(name: string): void; onOpenList?: () => void;
  invalid?: boolean; describedBy?: string; service?: string;
}) {
  const t = useT();
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
    {open && entry && <ul className="combo-list" id={choicesId} role="listbox" aria-label={t('actions.combo.label', { service })}>
      {entry.status === 'loading' && <li className="combo-note" role="status">{t('actions.combo.loading', { service })}</li>}
      {entry.status === 'failed' && <li className="combo-note">{entry.error}</li>}
      {listed && shown.map((name, index) => (
        <li key={name} id={`${id}-choice-${index}`} role="option" aria-selected={name === value}
          className={index === at ? 'active' : undefined} onMouseDown={event => event.preventDefault()}
          onClick={() => { onPick(name); onOpenChange(false); }}>{name}</li>
      ))}
      {listed && !matches.length && <li className="combo-note" role="status">{t('actions.combo.noMatch', { service })}</li>}
      {listed && matches.length > shown.length && <li className="combo-note">{t('actions.combo.more', { count: matches.length - shown.length })}</li>}
    </ul>}
  </>;
}

// The field wrapper owning the combobox state, closing the choices on a press outside. Typing
// keeps the choices open; picking one closes them.
function ObsNameField({ id, value, entry, onCommit, onOpenList, invalid, describedBy, service }: {
  id: string; value: string; entry: ObsNameList | null; onCommit(name: string): void; onOpenList?: () => void;
  invalid?: boolean; describedBy?: string; service?: string;
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
      onType={onCommit} onPick={onCommit} onOpenList={onOpenList} invalid={invalid} describedBy={describedBy} service={service} />
  </span>;
}

// Label, combobox input, and the two inline sayings: the saved-name rules (an error) and, when the
// editor holds OBS's (or Streamlabs') list and the exact name is not in it, the live cross-check (a note).
function ObsNameRow({ id, label, value, entry, what, issue, onChange, onOpenList, spaced, service }: {
  id: string; label: string; value: string; entry: ObsNameList | null; what: string; issue: string | null;
  onChange(name: string): void; onOpenList?: () => void; spaced?: boolean; service?: string;
}) {
  const warning = obsMissingWarning(value, entry, what, service);
  const issueId = `${id}-error`;
  const warningId = `${id}-warning`;
  return <>
    <label className={spaced ? 'spaced-label' : undefined} htmlFor={id}>{label}</label>
    <ObsNameField id={id} value={value} entry={entry} onCommit={onChange} onOpenList={onOpenList} invalid={!!issue}
      describedBy={issue ? issueId : warning ? warningId : undefined} service={service} />
    {issue && <p id={issueId} className="field-error" role="alert">{issue}</p>}
    {!issue && warning && <p id={warningId} className="field-warning" role="note">{warning}</p>}
  </>;
}

// The action picker and the parameter fields of the chosen action. Top-level ids keep their
// historical names; multi-action steps prefix theirs with `step-N-`. onSetUpObs opens
// Settings, where the OBS connection is configured. catalog carries OBS's current
// listings for the OBS name fields and streamlabs Streamlabs' for its fields; without them
// they stay plain inputs. invoke reaches the macro editor's recorder and permission checks
// (desktop only).
export function ActionFields({ action, fader, selectorKey, onChange, onSetUpObs, catalog, streamlabs, invoke }: {
  action: BindingAction | null; fader: boolean; selectorKey: string; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView; streamlabs?: StreamlabsCatalogView; invoke?: Invoke;
}) {
  const t = useT();
  return <>
    <label htmlFor="binding-action">{t('actions.action')}</label>
    <ActionPickerField triggerId="binding-action" scope={fader ? 'binding-fader' : 'binding'} action={action}
      onAssign={type => onChange(type === null ? null : defaultBindingAction(type))} />
    {action?.type === 'volume_from_position' && <p className="fine-print">{t('actions.volumeFromPosition')}</p>}
    {action?.type === 'macro' && <MacroFields steps={action.steps} disabled={false} onChange={steps => onChange({ type: 'macro', steps }, 'macro-steps')} invoke={invoke} />}
    {action?.type === 'conditional' && <ConditionalFields action={action} selectorKey={selectorKey} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />}
    {action?.type === 'multi_action' && <MultiActionFields steps={action.actions} selectorKey={selectorKey} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />}
    {action?.type === 'profile' && <div className="action-fields"><label htmlFor="profile-direction">{t('actions.profile.label')}</label>
      <SelectField id="profile-direction" value={action.direction} onChange={event => onChange({ type: 'profile', direction: event.target.value as ProfileDirection })}>
        {(Object.keys(profileDirectionLabels) as ProfileDirection[]).map(direction => <option key={direction} value={direction}>{profileDirectionLabels[direction]}</option>)}
      </SelectField>
      <Hint hint={t('actions.profile.hint')}>{t('actions.profile.more')}</Hint></div>}
    {action?.type === 'timer' && <TimerFields action={action} inputKey={selectorKey} onChange={onChange} />}
    {action?.type === 'terminal_command' && <TerminalCommandFields action={action} onChange={onChange} />}
    {action?.type === 'mic_input_volume_from_position' && <p className="fine-print">{t('actions.micVolumeFromPosition')}</p>}
    {action?.type === 'obs_input_volume_from_position' && <FaderObsInputFields action={action} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} />}
    {action && !isFaderTemplate(action) && action.type !== 'macro' && action.type !== 'multi_action' && action.type !== 'conditional' && action.type !== 'profile' && action.type !== 'timer' && action.type !== 'terminal_command' &&
      <StepFields prefix="" action={action} inputKey={selectorKey} onChange={(next, field) => onChange(next, field)} onSetUpObs={onSetUpObs} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />}
  </>;
}

// The fader's OBS input volume: each position change sets the named input's volume, from 0% (silent) to 100% (unity gain, 0 dB).
function FaderObsInputFields({ action, onChange, onSetUpObs, catalog }: {
  action: Extract<BindingAction, { type: 'obs_input_volume_from_position' }>; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView;
}) {
  const t = useT();
  return <div className="action-fields">
    <ObsNameRow id="obs-input-volume-input" label={t('actions.obs.inputName')} value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what={t('actions.what.input')}
      issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ type: 'obs_input_volume_from_position', input }, 'obs-input-volume-input')} />
    <p className="fine-print">{t('actions.faderObsInput', { max: maxObsSceneBytes })}</p>
    <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />
  </div>;
}

type TimerAction = Extract<BindingAction, { type: 'timer' }>;

function TimerFields({ action, inputKey, onChange }: { action: TimerAction; inputKey: string; onChange: Change }) {
  const t = useT();
  const issue = timerLabelIssue(action.label ?? '');
  // An empty label is omitted, never written as "" (which the service rejects).
  const withLabel = (label: string): TimerAction => {
    const { label: _previous, ...rest } = action;
    return label ? { ...rest, label } : rest;
  };
  return <div className="action-fields">
    <label htmlFor="timer-seconds">{t('actions.timer.seconds')}</label>
    <IntegerField key={inputKey} id="timer-seconds" value={action.seconds} minimum={timerSeconds.minimum} maximum={timerSeconds.maximum}
      onCommit={seconds => onChange({ ...action, seconds }, 'timer-seconds')} />
    <label className="spaced-label" htmlFor="timer-label">{t('actions.timer.label')}</label>
    <input id="timer-label" spellCheck={false} value={action.label ?? ''} aria-invalid={!!issue} aria-describedby={issue ? 'timer-label-error' : undefined}
      onChange={event => onChange(withLabel(event.target.value), 'timer-label')} />
    {issue && <p id="timer-label-error" className="field-error" role="alert">{issue}</p>}
    <label className="spaced-label" htmlFor="timer-sound">{t('actions.timer.sound')}</label>
    <SelectField id="timer-sound" value={action.sound ?? defaultTimerSound} onChange={event => onChange({ ...action, sound: event.target.value as TimerSound })}>
      {(Object.keys(timerSoundLabels) as TimerSound[]).map(sound => <option key={sound} value={sound}>{timerSoundLabels[sound]}</option>)}
    </SelectField>
    <Hint hint={t('actions.timer.hint')}>{t('actions.timer.more', { max: maxRunningTimers })}</Hint>
  </div>;
}

// A terminal session: the app (a closed set), the command line and an optional folder to start in. An empty folder is
// omitted, never written as "" (which the service rejects).
function TerminalCommandFields({ action, onChange }: { action: TerminalCommandAction; onChange: Change }) {
  const t = useT();
  const commandIssue = terminalCommandIssue(action.command);
  const directoryIssue = terminalDirectoryIssue(action.directory ?? '');
  const withDirectory = (directory: string): TerminalCommandAction => {
    const { directory: _previous, ...rest } = action;
    return directory ? { ...rest, directory } : rest;
  };
  return <div className="action-fields">
    <label htmlFor="terminal-app">{t('actions.terminal.app')}</label>
    <SelectField id="terminal-app" value={action.app} onChange={event => onChange({ ...action, app: event.target.value as TerminalApp }, 'terminal-app')}>
      {(Object.keys(terminalAppLabels) as TerminalApp[]).map(app => <option key={app} value={app}>{terminalAppLabels[app]}</option>)}
    </SelectField>
    <label className="spaced-label" htmlFor="terminal-command">{t('actions.terminal.command')}</label>
    <input id="terminal-command" spellCheck={false} autoCapitalize="off" autoCorrect="off" value={action.command}
      aria-invalid={!!commandIssue} aria-describedby={commandIssue ? 'terminal-command-error' : undefined}
      onChange={event => onChange({ ...action, command: event.target.value }, 'terminal-command')} />
    {commandIssue && <p id="terminal-command-error" className="field-error" role="alert">{commandIssue}</p>}
    <label className="spaced-label" htmlFor="terminal-directory">{t('actions.terminal.directory')}</label>
    <input id="terminal-directory" spellCheck={false} autoCapitalize="off" autoCorrect="off" value={action.directory ?? ''}
      placeholder={t('actions.terminal.directoryPlaceholder')}
      aria-invalid={!!directoryIssue} aria-describedby={directoryIssue ? 'terminal-directory-error' : undefined}
      onChange={event => onChange(withDirectory(event.target.value), 'terminal-directory')} />
    {directoryIssue && <p id="terminal-directory-error" className="field-error" role="alert">{directoryIssue}</p>}
    <Hint hint={t('actions.terminal.hint')}>{t('actions.terminal.more', { max: maxTerminalCommandBytes })}</Hint>
  </div>;
}

// A program run directly: its path, its arguments one field each (passed exactly as typed, never split or expanded) and an
// optional folder. No arguments and an empty folder are omitted, never written as [] or "" (which the service rejects
// for the folder). With variables on, {{name}} in an argument is filled in when pressed; off is omitted.
function RunProgramFields({ prefix, action, onChange }: { prefix: string; action: RunProgramAction; onChange(action: StepAction, field?: string): void }) {
  const t = useT();
  const id = (name: string) => `${prefix}${name}`;
  const args = action.args ?? [];
  // Stable row keys, so removing an argument keeps focus on the one being edited; kept as long as args.
  const nextKey = useRef(0);
  const keys = useRef<number[]>([]);
  while (keys.current.length < args.length) keys.current.push(nextKey.current++);
  keys.current.length = args.length;
  const pathIssue = programPathIssue(action.program);
  const listIssue = programArgsIssue(args);
  const directoryIssue = terminalDirectoryIssue(action.directory ?? '');
  const declaredVariables = useVariables();
  const declared = declaredVariables.map(variable => variable.name);
  const used = [...new Set(actionVariableNames(action))];
  const withVariables = (on: boolean): RunProgramAction => {
    const { variables: _previous, ...rest } = action;
    return on ? { ...rest, variables: true } : rest;
  };
  const withArgs = (next: string[]): RunProgramAction => {
    const { args: _previous, ...rest } = action;
    return next.length ? { ...rest, args: next } : rest;
  };
  const withDirectory = (directory: string): RunProgramAction => {
    const { directory: _previous, ...rest } = action;
    return directory ? { ...rest, directory } : rest;
  };
  return <div className="action-fields">
    <label htmlFor={id('program-path')}>{t('actions.program.path')}</label>
    <input id={id('program-path')} spellCheck={false} autoCapitalize="off" autoCorrect="off" value={action.program}
      placeholder={t('actions.program.pathPlaceholder')}
      aria-invalid={!!pathIssue} aria-describedby={pathIssue ? id('program-path-error') : undefined}
      onChange={event => onChange({ ...action, program: event.target.value }, 'program-path')} />
    {pathIssue && <p id={id('program-path-error')} className="field-error" role="alert">{pathIssue}</p>}
    <fieldset className="spaced-label program-args">
      <legend>{t('actions.program.args')}</legend>
      {args.map((arg, index) => {
        const argId = id(`program-arg-${index + 1}`);
        const issue = programArgIssue(arg) ?? (action.variables ? programVariableIssue(arg, declared) : null);
        const hint = !issue && action.variables ? programDashHint(arg, declaredVariables) : null;
        return <div key={keys.current[index]} className="program-arg">
          <input id={argId} spellCheck={false} autoCapitalize="off" autoCorrect="off" value={arg}
            aria-label={t('actions.program.argNumbered', { number: index + 1 })}
            aria-invalid={!!issue} aria-describedby={issue ? `${argId}-error` : hint ? `${argId}-hint` : undefined}
            onChange={event => onChange(withArgs(args.map((item, at) => at === index ? event.target.value : item)), `program-arg-${index + 1}`)} />
          <button type="button" className="button compact" aria-label={t('actions.program.removeArg', { number: index + 1 })}
            onClick={() => { keys.current.splice(index, 1); onChange(withArgs(args.filter((_, at) => at !== index))); }}><Trash2 size={16} /></button>
          {issue && <p id={`${argId}-error`} className="field-error" role="alert">{issue}</p>}
          {hint && <p id={`${argId}-hint`} className="fine-print">{hint}</p>}
        </div>;
      })}
      <button type="button" className="button compact" disabled={args.length >= maxRunProgramArgs}
        onClick={() => onChange(withArgs([...args, '']), `program-arg-${args.length + 1}`)}><Plus size={16} />{t('actions.program.addArg')}</button>
      {listIssue && <p className="field-error" role="alert">{listIssue}</p>}
      <label className="setting-toggle"><input id={id('program-variables')} type="checkbox" checked={!!action.variables}
        onChange={event => onChange(withVariables(event.target.checked), 'program-variables')} />{t('actions.program.variables')}</label>
      <p className="fine-print">{!action.variables ? t('actions.program.variablesOff')
        : `${used.length ? t('actions.program.variablesUsed', { names: used.join(', ') }) : t('actions.program.variablesNone')} ${t('actions.program.variablesOn')}`}</p>
    </fieldset>
    <label className="spaced-label" htmlFor={id('program-directory')}>{t('actions.program.directory')}</label>
    <input id={id('program-directory')} spellCheck={false} autoCapitalize="off" autoCorrect="off" value={action.directory ?? ''}
      placeholder={t('actions.terminal.directoryPlaceholder')}
      aria-invalid={!!directoryIssue} aria-describedby={directoryIssue ? id('program-directory-error') : undefined}
      onChange={event => onChange(withDirectory(event.target.value), 'program-directory')} />
    {directoryIssue && <p id={id('program-directory-error')} className="field-error" role="alert">{directoryIssue}</p>}
    <Hint hint={t('actions.program.hint')}>{t('actions.program.more', { max: maxRunProgramArgs })}</Hint>
  </div>;
}

// Numbered steps with a picker each, Remove buttons and Add step: the list a multi-action and both branches of a
// conditional edit. `idBase` keeps the ids apart (`step-2-action` for a multi-action, `then-step-2-action` in a branch).
function StepList({ steps, idBase, noun, minimum, maximum, selectorKey, update, onSetUpObs, catalog, streamlabs, invoke }: {
  steps: StepAction[]; idBase: string; noun: string; minimum: number; maximum: number; selectorKey: string; update: (next: StepAction[], field?: string) => void;
  onSetUpObs?: () => void; catalog?: ObsCatalogView; streamlabs?: StreamlabsCatalogView; invoke?: Invoke;
}) {
  const t = useT();
  return <>
    <ol className="action-steps">
      {steps.map((step, index) => {
        const prefix = `${idBase}step-${index + 1}-`;
        return <li key={`${index}:${step.type}`}>
          <label htmlFor={`${prefix}action`}>{t('actions.steps.numbered', { noun, number: index + 1 })}</label>
          <ActionPickerField triggerId={`${prefix}action`} idPrefix={prefix} scope="step" action={step} allowDrag={false}
            onAssign={type => { if (type) update(steps.map((item, at) => at === index ? defaultBindingAction(type) as StepAction : item)); }} />
          <StepFields prefix={prefix} action={step} inputKey={`${selectorKey}:${idBase}${index}:${steps.length}`}
            onChange={(next, field) => update(steps.map((item, at) => at === index ? next : item), field && `${prefix}${field}`)} onSetUpObs={onSetUpObs} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />
          <button type="button" className="button compact" disabled={steps.length <= minimum}
            aria-label={t('actions.steps.removeNamed', { noun: noun.toLowerCase(), number: index + 1 })} onClick={() => update(steps.filter((_, at) => at !== index))}><Trash2 size={16} />{t('actions.steps.remove')}</button>
        </li>;
      })}
    </ol>
    <button type="button" className="button compact" disabled={steps.length >= maximum}
      onClick={() => update([...steps, defaultStepAction('toggle_mute')])}><Plus size={16} />{t('actions.steps.add', { noun: noun.toLowerCase() })}</button>
  </>;
}

function MultiActionFields({ steps, selectorKey, onChange, onSetUpObs, catalog, streamlabs, invoke }: { steps: StepAction[]; selectorKey: string; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView; streamlabs?: StreamlabsCatalogView; invoke?: Invoke }) {
  const t = useT();
  const update = (next: StepAction[], field?: string) => onChange({ type: 'multi_action', actions: next }, field);
  return <div className="action-fields">
    <p className="fine-print">{t('actions.multi.note', { min: multiActionSteps.minimum, max: multiActionSteps.maximum })}</p>
    <StepList steps={steps} idBase="" noun={t('actions.steps.step')} minimum={multiActionSteps.minimum} maximum={multiActionSteps.maximum} selectorKey={selectorKey} update={update}
      onSetUpObs={onSetUpObs} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />
    {steps.some(step => step.type.startsWith('obs_')) && <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />}
    {steps.some(step => step.type.startsWith('streamlabs_')) && <StreamlabsConnection onSetUp={onSetUpObs} catalog={streamlabs} />}
  </div>;
}

// Whether a flow reads from or acts on OBS anywhere: a test, a step, or inside a nested if / else.
const flowUsesObs = (action: ConditionalAction): boolean =>
  conditionUsesObs(action.when) || action.then.concat(action.else ?? []).some(step => (step.type === 'conditional' ? flowUsesObs(step) : step.type.startsWith('obs_')));
// Whether a flow acts on Streamlabs in any step (no condition reads Streamlabs).
const flowUsesStreamlabs = (action: ConditionalAction): boolean =>
  action.then.concat(action.else ?? []).some(step => (step.type === 'conditional' ? flowUsesStreamlabs(step) : step.type.startsWith('streamlabs_')));

// If / else: tests read when the key is pressed (AND / OR groups that nest), a Then list and an Otherwise list whose steps
// may be if / else blocks themselves. The tree, its drag-and-drop and its limits live in FlowEditor.
function ConditionalFields({ action, selectorKey, onChange, onSetUpObs, catalog, streamlabs, invoke }: { action: ConditionalAction; selectorKey: string; onChange: Change; onSetUpObs?: () => void; catalog?: ObsCatalogView; streamlabs?: StreamlabsCatalogView; invoke?: Invoke }) {
  const t = useT();
  return <FlowEditor action={action} selectorKey={selectorKey} onChange={(next, field) => onChange(next, field)}
    render={{
      step: ({ prefix, step, inputKey, onChange: change }) => <StepFields prefix={prefix} action={step} inputKey={inputKey} onChange={change} onSetUpObs={onSetUpObs} catalog={catalog} streamlabs={streamlabs} invoke={invoke} />,
      app: ({ inputId, bundleId, onChange: change }) => <LaunchAppFields inputId={inputId} bundleId={bundleId} invoke={invoke} onChange={change} label={t('actions.flow.frontApp')} inline hint={t('actions.flow.frontAppHint')} />,
    }}
    obsNote={flowUsesObs(action) || flowUsesStreamlabs(action) ? <>
      {flowUsesObs(action) && <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />}
      {flowUsesStreamlabs(action) && <StreamlabsConnection onSetUp={onSetUpObs} catalog={streamlabs} />}
    </> : undefined} />;
}

// An app field, name first, like the macOS app lists: a trigger with the app's icon and name opens a searchable list of
// the apps running now and the recently chosen ones, each with its icon, then "Other app…" (the system's picker for any
// other installed app); typing the bundle ID is the fallback. The stored value is always the bundle ID. A stored ID no
// running app has is shown by its installed name (app_name), or as the raw ID with a note when no installed app has it.
// Without a desktop backend (`state` null) it stays the plain bundle ID field. `inline` hides the picker's label where
// the sentence around it already says what it is (the front-app test). Hook-free (LaunchAppFields owns the state, the
// list logic is in app-picker-model), so tests call it. Icons arrive lazily; a placeholder glyph stands in meanwhile.
export interface AppPickerState {
  apps: RunningApp[]; installedName: string | null | undefined; typing: boolean; choosing: boolean; error: string | null;
  recent?: RunningApp[]; icons?: ReadonlyMap<string, string | null>; list?: AppListState;
}

const noIcons: ReadonlyMap<string, string | null> = new Map();
// The bundle ID shown as the example of the format (not text to translate).
const bundleIdExample = 'com.apple.finder';

// Icon bytes only ever reach this <img>: nothing is uploaded.
function AppIcon({ png }: { png: string | null | undefined }) {
  return png ? <img className="app-icon" src={dataUrl('image/png', png)} alt="" width={20} height={20} />
    : <span className="app-icon app-icon-placeholder" aria-hidden="true"><AppWindow size={14} /></span>;
}

export function AppPicker({ inputId, bundleId, label, inline, hint, state, onChange, onPick, onChoose, onTyping, onList }: {
  inputId: string; bundleId: string; label: string; inline?: boolean; hint: string; state: AppPickerState | null;
  onChange(bundleId: string): void; onPick?: (app: RunningApp) => void; onChoose?: () => void; onTyping?: (typing: boolean) => void; onList?: (next: AppListState) => void;
}) {
  const t = useT();
  const trimmed = bundleId.trim();
  const noteId = `${inputId}-note`;
  if (!state) return <div className="action-fields">
    <label htmlFor={inputId}>{t('actions.app.bundleIdField')}</label>
    <input id={inputId} spellCheck={false} value={bundleId} onChange={event => onChange(event.target.value)} />
    <p className="fine-print">{hint}</p>
  </div>;
  const running = state.apps.find(app => app.bundle_id.toLowerCase() === trimmed.toLowerCase());
  const name = running?.name ?? state.installedName;
  const note = trimmed !== '' && (state.typing || !running) && name !== undefined;
  const list = state.list ?? closedAppList;
  const icons = state.icons ?? noIcons;
  const current = trimmed !== '' && !running ? { bundle_id: trimmed, name: name ?? trimmed } : null;
  const sectionsFor = (query: string) => appSections(state.apps, state.recent ?? [], current, query);
  const sections = list.open ? sectionsFor(list.query) : [];
  const options = appOptions(sections);
  const ids = { label: `${inputId}-label`, search: `${inputId}-search`, list: `${inputId}-list`, other: `${inputId}-other` };
  const optionId = (index: number) => index < options.length ? `${inputId}-option-${index}` : ids.other;
  const open = (query: string) => onList?.(openAppList(appOptions(sectionsFor(query)), trimmed, query));
  const close = (refocus: boolean) => { onList?.(closedAppList); if (refocus) focusTrigger(inputId); };
  const choose = (index: number) => {
    close(true);
    const app = options[index];
    if (app) (onPick ?? (picked => onChange(picked.bundle_id)))(app); else onChoose?.();
  };
  const searchKey = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    const result = appListKey(list, event.key, options.length + 1);
    if (!result) return;
    if (result.handled) event.preventDefault();
    if (result.choose !== undefined) { choose(result.choose); return; }
    onList?.(result.next);
    if (result.close === 'refocus') focusTrigger(inputId);
  };
  const option = (app: RunningApp, index: number) => {
    const selected = app.bundle_id.toLowerCase() === trimmed.toLowerCase();
    return <div key={app.bundle_id} id={optionId(index)} role="option" aria-selected={selected} title={app.bundle_id}
      className={`app-option${index === list.active ? ' active' : ''}`} onMouseDown={event => event.preventDefault()}
      onMouseMove={() => { if (index !== list.active) onList?.({ ...list, active: index }); }} onClick={() => choose(index)}>
      <AppIcon png={icons.get(app.bundle_id)} /><span className="app-option-name">{app.name}</span>{selected && <Check size={15} aria-hidden="true" />}
    </div>;
  };
  return <div className="action-fields app-field">
    <label id={ids.label} className={inline && !state.typing ? 'sr-only' : undefined} htmlFor={inputId}>{state.typing ? t('actions.app.bundleId') : label}</label>
    {state.typing
      ? <div className="focus-bundle-row">
        <input id={inputId} spellCheck={false} autoComplete="off" autoFocus placeholder={bundleIdExample} value={bundleId} aria-describedby={note ? noteId : undefined} onChange={event => onChange(event.target.value)} />
        <button type="button" className="button" disabled={state.choosing} onClick={onChoose}><FolderOpen size={16} aria-hidden="true" />{state.choosing ? t('actions.app.choosing') : t('actions.app.chooseApp')}</button>
      </div>
      : <div className="app-picker">
        <button type="button" id={inputId} className="action-picker-trigger app-picker-trigger" aria-haspopup="listbox" aria-expanded={list.open}
          aria-controls={list.open ? ids.list : undefined} aria-labelledby={`${ids.label} ${inputId}`} aria-describedby={note ? noteId : undefined}
          aria-busy={state.choosing || undefined} title={trimmed || undefined} onClick={() => { if (list.open) close(true); else if (!state.choosing) open(''); }}
          onKeyDown={event => {
            // Stays enabled while the system picker is open, so focus is still here when it closes.
            const hit = list.open || state.choosing ? null : triggerKey(event.key, event.metaKey || event.ctrlKey || event.altKey);
            if (hit) { event.preventDefault(); open(hit.query); }
          }}>
          <AppIcon png={trimmed ? icons.get(running?.bundle_id ?? trimmed) : undefined} />
          <span className="action-picker-trigger-label">{state.choosing ? t('actions.app.choosing') : trimmed ? name ?? trimmed : t('actions.app.chooseAnApp')}</span>
          <ChevronDown size={17} aria-hidden="true" />
        </button>
        {list.open && <div className="action-picker-panel app-picker-panel">
          <label className="sr-only" htmlFor={ids.search}>{t('actions.app.search')}</label>
          <input id={ids.search} type="search" role="combobox" aria-expanded="true" aria-controls={ids.list} aria-autocomplete="list"
            aria-activedescendant={list.active >= 0 && list.active <= options.length ? optionId(list.active) : undefined}
            placeholder={t('actions.app.search')} autoComplete="off" spellCheck={false} autoFocus value={list.query}
            onChange={event => onList?.(queryAppList(event.target.value))} onKeyDown={searchKey} />
          {options.length === 0 && <p className="picker-empty" role="status">{list.query.trim() ? t('actions.app.noMatch', { query: list.query.trim() }) : t('actions.app.noneRunning')}</p>}
          <div id={ids.list} role="listbox" aria-label={t('actions.app.list')} className="app-picker-list">
            {sections.map(section => <div key={section.key} role="group" aria-labelledby={`${inputId}-group-${section.key}`}>
              <div id={`${inputId}-group-${section.key}`} role="presentation" className="picker-group-label">{section.label}</div>
              {section.apps.map(app => option(app, options.indexOf(app)))}
            </div>)}
            <div id={ids.other} role="option" aria-selected={false} className={`app-option app-option-other${list.active === options.length ? ' active' : ''}`}
              onMouseDown={event => event.preventDefault()} onMouseMove={() => { if (list.active !== options.length) onList?.({ ...list, active: options.length }); }}
              onClick={() => choose(options.length)}>
              <span className="app-icon app-icon-placeholder" aria-hidden="true"><FolderOpen size={14} /></span><span className="app-option-name">{t('actions.app.other')}</span>
            </div>
          </div>
        </div>}
      </div>}
    {note && <p id={noteId} className="fine-print" role="status">
      {name === null ? t('actions.app.notInstalled', { id: trimmed }) : state.typing ? <>{t('actions.app.named')} <strong>{name}</strong></> : t('actions.app.bundleNamed', { id: trimmed })}</p>}
    {state.error && <p className="field-error" role="alert">{state.error}</p>}
    <p className="fine-print">{hint} <button type="button" className="link-button" onClick={() => onTyping?.(!state.typing)}>{state.typing ? t('actions.app.pickByName') : t('actions.app.enterBundleId')}</button></p>
  </div>;
}

function LaunchAppFields({ inputId, bundleId, invoke, onChange, label, inline, hint }: {
  inputId: string; bundleId: string; invoke?: Invoke; onChange(bundleId: string): void; label?: string; inline?: boolean; hint?: string;
}) {
  const t = useT();
  const [apps, setApps] = useState<RunningApp[]>([]);
  const [found, setFound] = useState<{ id: string; name: string | null } | null>(null);
  const [typing, setTyping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [choosing, setChoosing] = useState(false);
  const [list, setList] = useState<AppListState>(closedAppList);
  useSyncExternalStore(appIcons.subscribe, appIcons.version, appIcons.version);
  const call = useRef(invoke);
  call.current = invoke;
  const trimmed = bundleId.trim();
  const recent = list.open ? readRecentApps(pickerWebStorage) : [];
  const refresh = () => { if (call.current) void listRunningApps(call.current).then(list => { list.forEach(app => rememberAppName(app.bundle_id, app.name)); setApps(list); }); };
  useEffect(refresh, []);
  useEffect(() => {
    if (!call.current || trimmed === '') return;
    let current = true;
    const timer = setTimeout(() => {
      call.current?.<string | null>('app_name', { bundleId: trimmed }).then(name => { rememberAppName(trimmed, name); if (current) setFound({ id: trimmed, name }); }, () => {});
    }, 300);
    return () => { current = false; clearTimeout(timer); };
  }, [trimmed]);
  // The trigger's icon right away; the rows' icons once the list opens, never blocking it.
  const runningId = apps.find(app => app.bundle_id.toLowerCase() === trimmed.toLowerCase())?.bundle_id;
  useEffect(() => {
    if (call.current) void appIcons.load(call.current, [runningId ?? trimmed, ...(list.open ? [...apps, ...readRecentApps(pickerWebStorage)].map(app => app.bundle_id) : [])]);
  }, [list.open, apps, trimmed, runningId]);
  // A press outside the field closes the list without moving focus; the active row stays in view.
  useEffect(() => {
    if (!list.open) return;
    const close = (event: PointerEvent) => { if (!document.getElementById(inputId)?.closest('.app-picker')?.contains(event.target as Node)) setList(closedAppList); };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [list.open, inputId]);
  useEffect(() => {
    if (!list.open) return;
    const active = document.getElementById(`${inputId}-search`)?.getAttribute('aria-activedescendant');
    if (active) document.getElementById(active)?.scrollIntoView?.({ block: 'nearest' });
  }, [list.open, list.active, inputId]);
  const pick = (app: RunningApp) => { rememberAppName(app.bundle_id, app.name); recordRecentApp(pickerWebStorage, app); setFound({ id: app.bundle_id, name: app.name }); onChange(app.bundle_id); };
  const choose = async () => {
    if (!invoke || choosing) return;
    setChoosing(true);
    setError(null);
    try {
      const app = await invoke<{ bundle_id: string; name: string } | null>('choose_app');
      if (app) pick(app);
    } catch (reason) {
      setError(typeof reason === 'string' ? reason : t('actions.app.chooseFailed'));
    } finally { setChoosing(false); }
  };
  const state: AppPickerState | null = invoke
    ? { apps, installedName: found?.id === trimmed ? found.name : undefined, typing, choosing, error, recent, icons: appIcons.icons, list } : null;
  const note = hint ?? (invoke ? t('actions.app.pickHint') : t('actions.app.typeHint', { example: bundleIdExample }));
  return <AppPicker inputId={inputId} bundleId={bundleId} label={label ?? t('actions.app.label')} inline={inline} hint={note} state={state} onChange={onChange} onPick={pick}
    onChoose={() => { void choose(); }} onTyping={next => { setList(closedAppList); setTyping(next); }}
    onList={next => { if (next.open && !list.open) refresh(); setList(next); }} />;
}

function StepFields({ prefix, action, inputKey, onChange, onSetUpObs, catalog, streamlabs, invoke }: {
  prefix: string; action: StepAction; inputKey: string; onChange(action: StepAction, field?: string): void; onSetUpObs?: () => void; catalog?: ObsCatalogView; streamlabs?: StreamlabsCatalogView; invoke?: Invoke;
}) {
  const t = useT();
  const id = (name: string) => `${prefix}${name}`;
  switch (action.type) {
    case 'toggle_mute': return null;
    case 'set_volume': return <div className="action-fields"><label htmlFor={id('volume-percent')}>{t('actions.volume.output')}</label><IntegerField key={inputKey} id={id('volume-percent')} value={action.percent} minimum={0} maximum={100} onCommit={percent => onChange({ type: 'set_volume', percent }, 'volume-percent')} /><p className="fine-print">{t('actions.volume.outputNote')}</p></div>;
    case 'volume': return <div className="action-fields"><label htmlFor={id('volume-delta')}>{t('actions.volume.change')}</label><IntegerField key={inputKey} id={id('volume-delta')} value={action.delta} minimum={-100} maximum={100} nonZero onCommit={delta => onChange({ type: 'volume', delta }, 'volume-delta')} /><p className="fine-print">{t('actions.volume.changeNote')}</p></div>;
    case 'shortcut': {
      const media = isMediaKey(action.key) ? action.key : null;
      return <div className="action-fields">
        <label htmlFor={id('shortcut-key')}>{t('actions.shortcut.key')}</label>
        <HotkeyField id={id('shortcut-key')} value={action.key} modifiers={action.modifiers}
          onChange={({ key, modifiers }) => onChange({ ...action, key, modifiers }, 'shortcut-key')} />
        <fieldset className="modifiers"><legend>{t('actions.shortcut.modifiers')}</legend>{(['command', 'control', 'option', 'shift'] as Modifier[]).map(modifier => <label key={modifier}><input type="checkbox" checked={action.modifiers.includes(modifier)} disabled={!!media} onChange={event => onChange({ ...action, modifiers: event.target.checked ? [...action.modifiers, modifier] : action.modifiers.filter(item => item !== modifier) }, 'shortcut-key')} />{t(`actions.shortcut.${modifier}`)}</label>)}</fieldset>
        <p className="fine-print">{media
          ? t('actions.shortcut.mediaNote', { key: mediaKeyLabels[media] })
          : t('actions.shortcut.note')}</p>
      </div>;
    }
    case 'run_program': return <RunProgramFields prefix={prefix} action={action} onChange={onChange} />;
    case 'plugin': return <PluginFields prefix={prefix} action={action} onChange={onChange} invoke={invoke} hint={(summary, more) => <Hint hint={summary}>{more}</Hint>} />;
    case 'launch_app': return <LaunchAppFields inputId={id('app-bundle-id')} bundleId={action.bundle_id} invoke={invoke} onChange={bundleId => onChange({ type: 'launch_app', bundle_id: bundleId }, 'bundle-id')} />;
    case 'open_url': {
      const issue = urlIssue(action.url);
      return <div className="action-fields"><label htmlFor={id('open-url')}>{t('actions.url.label')}</label><input id={id('open-url')} type="url" spellCheck={false} value={action.url} aria-invalid={!!issue} aria-describedby={issue ? id('open-url-error') : undefined} onChange={event => onChange({ type: 'open_url', url: event.target.value }, 'open-url')} />{issue && <p id={id('open-url-error')} className="field-error" role="alert">{issue}</p>}<p className="fine-print">{t('actions.url.note')}</p></div>;
    }
    case 'text': {
      const issue = textIssue(action.text);
      return <div className="action-fields"><label htmlFor={id('text-snippet')}>{t('actions.text.label')}</label><textarea id={id('text-snippet')} rows={3} value={action.text} aria-invalid={!!issue} aria-describedby={issue ? id('text-snippet-error') : undefined} onChange={event => onChange({ ...action, text: event.target.value }, 'text-snippet')} />{issue && <p id={id('text-snippet-error')} className="field-error" role="alert">{issue}</p>}
        <label className="setting-toggle"><input type="checkbox" checked={!!action.press_enter} onChange={event => onChange({ ...action, press_enter: event.target.checked })} />{t('actions.text.pressEnter')}</label>
        <p className="fine-print">{t('actions.text.note', { max: maxTextBytes })}</p></div>;
    }
    case 'media': return <div className="action-fields"><label htmlFor={id('media-command')}>{t('actions.media.label')}</label><SelectField id={id('media-command')} value={action.command} onChange={event => onChange({ type: 'media', command: event.target.value as MediaCommand })}>{(Object.keys(mediaLabels) as MediaCommand[]).map(command => <option key={command} value={command}>{mediaLabels[command]}</option>)}</SelectField><p className="fine-print">{action.command === 'stop'
      ? t('actions.media.stopNote')
      : t('actions.media.note')}</p></div>;
    case 'obs_scene': case 'obs_virtual_camera': case 'obs_replay_buffer': case 'obs_stream': case 'obs_record': case 'obs_studio_mode': case 'obs_transition': case 'obs_screenshot':
    case 'obs_filter': case 'obs_source_visibility': case 'obs_scene_collection': case 'obs_profile': case 'obs_set_input_volume':
    case 'obs_input_mute': case 'obs_adjust_input_volume': case 'obs_media_source':
      return <ObsFields action={action} prefix={prefix} onChange={onChange} onSetUpObs={onSetUpObs} catalog={catalog} />;
    case 'streamlabs_record': case 'streamlabs_stream': case 'streamlabs_studio_mode': case 'streamlabs_transition': case 'streamlabs_scene':
    case 'streamlabs_source_visibility': case 'streamlabs_audio':
      return <StreamlabsFields action={action} prefix={prefix} onChange={onChange} onSetUp={onSetUpObs} catalog={streamlabs} />;
    case 'set_variable': case 'toggle_variable': case 'add_to_variable': return <VariableActionFields prefix={prefix} action={action} onChange={onChange} />;
    case 'system': return <div className="action-fields"><label htmlFor={id('system-command')}>{t('actions.system.label')}</label><SelectField id={id('system-command')} value={action.command} onChange={event => onChange({ type: 'system', command: event.target.value as SystemCommand })}>{(Object.keys(systemLabels) as SystemCommand[]).map(command => <option key={command} value={command}>{systemLabels[command]}</option>)}</SelectField><p className="fine-print">{action.command === 'screenshot' ? t('actions.system.screenshotNote') : t('actions.system.sendsNote', { keys: t(action.command === 'undo' ? 'actions.system.undoKeys' : 'actions.system.redoKeys') })}</p></div>;
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
  const t = useT();
  const obsSwitchHint = t('actions.obs.switchHint');
  const obsSwitchMore = t('actions.obs.switchMore');
  const obsOutputNote = t('actions.obs.outputNote');
  const id = (name: string) => `${prefix}${name}`;
  const scenes = catalogNames(catalog, catalog?.names?.scenes);
  const collections = catalogNames(catalog, catalog?.names?.scene_collections);
  const profiles = catalogNames(catalog, catalog?.names?.profiles);
  let fields: ReactNode;
  switch (action.type) {
    case 'obs_virtual_camera':
      fields = <><label htmlFor={id('obs-virtual-camera-op')}>{t('actions.action')}</label>
        <SelectField id={id('obs-virtual-camera-op')} value={action.op} onChange={event => onChange({ type: 'obs_virtual_camera', op: event.target.value as ObsOutputOp }, 'obs-virtual-camera-op')}>
          {(Object.keys(obsOutputOpLabels) as ObsOutputOp[]).map(op => <option key={op} value={op}>{obsOutputOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={obsSwitchHint}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_stream':
      fields = <><label htmlFor={id('obs-stream-op')}>{t('actions.action')}</label>
        <SelectField id={id('obs-stream-op')} value={action.op} onChange={event => onChange({ ...action, op: event.target.value as ObsOutputOp }, 'obs-stream-op')}>
          {(Object.keys(obsOutputOpLabels) as ObsOutputOp[]).map(op => <option key={op} value={op}>{obsOutputOpLabels[op]}</option>)}
        </SelectField>
        <label className="setting-toggle"><input type="checkbox" checked={action.confirm === 'press_twice'} onChange={event => onChange(withConfirm(action, event.target.checked), 'obs-stream-confirm')} />{t('actions.pressTwice')}</label>
        <p className="fine-print">{t('actions.obs.streamNote')} {confirmNote(t, action.confirm === 'press_twice')}</p>
        <Hint hint={obsSwitchHint}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_record':
      fields = <><label htmlFor={id('obs-record-op')}>{t('actions.action')}</label>
        <SelectField id={id('obs-record-op')} value={action.op} onChange={event => onChange({ type: 'obs_record', op: event.target.value as ObsRecordOp }, 'obs-record-op')}>
          {(Object.keys(obsRecordOpLabels) as ObsRecordOp[]).map(op => <option key={op} value={op}>{obsRecordOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={`${t('actions.obs.recordNeeds')} ${obsSwitchHint}`}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_replay_buffer':
      fields = <><label htmlFor={id('obs-replay-buffer-op')}>{t('actions.action')}</label>
        <SelectField id={id('obs-replay-buffer-op')} value={action.op} onChange={event => onChange({ type: 'obs_replay_buffer', op: event.target.value as ObsReplayBufferOp }, 'obs-replay-buffer-op')}>
          {(Object.keys(obsReplayBufferOpLabels) as ObsReplayBufferOp[]).map(op => <option key={op} value={op}>{obsReplayBufferOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={`${t('actions.obs.replayNeeds')} ${obsSwitchHint}`}>{obsOutputNote} {obsSwitchMore}</Hint></>;
      break;
    case 'obs_studio_mode':
      fields = <><label htmlFor={id('obs-studio-mode-op')}>{t('actions.action')}</label>
        <ObsSwitchSelect id={id('obs-studio-mode-op')} value={action.op} onChange={op => onChange({ type: 'obs_studio_mode', op }, 'obs-studio-mode-op')} />
        <Hint hint={obsSwitchHint}>{obsSwitchMore}</Hint></>;
      break;
    case 'obs_screenshot':
      fields = <p className="fine-print">{t('actions.obs.screenshotNote')}</p>;
      break;
    case 'obs_transition':
      fields = <p className="fine-print">{t('actions.obs.transitionNote')}</p>;
      break;
    case 'obs_filter': {
      // A filter can live on an input or on a scene, like in OBS's own filters dialog.
      const targets = filterTargets(catalog);
      const filters = catalog?.list('filters', action.source) ?? null;
      fields = <>
        <ObsNameRow id={id('obs-filter-source')} label={t('actions.obs.sourceName')} value={action.source} entry={targets} what={t('actions.what.source')}
          issue={obsNameIssue(action.source, 'source')} onChange={source => onChange({ ...action, source }, 'obs-filter-source')} />
        <ObsNameRow id={id('obs-filter-name')} label={t('actions.obs.filterName')} value={action.filter} entry={filters} what={t('actions.what.filter')} spaced
          issue={obsNameIssue(action.filter, 'filter')} onChange={filter => onChange({ ...action, filter }, 'obs-filter-name')}
          onOpenList={() => catalog?.requestNames('filters', action.source)} />
        <label className="spaced-label" htmlFor={id('obs-filter-op')}>{t('actions.action')}</label><ObsSwitchSelect id={id('obs-filter-op')} value={action.op} onChange={op => onChange({ ...action, op }, 'obs-filter-op')} />
        <Hint hint={`${t('actions.obs.bothExact', { max: maxObsSceneBytes })} ${obsSwitchHint}`}>{obsSwitchMore}</Hint></>;
      break;
    }
    case 'obs_source_visibility': {
      // The source choices are the items of the named scene, which is exactly what a visibility
      // there can target — fetched when the field's choices are opened.
      const inScene = catalog?.list('sceneSources', action.scene) ?? null;
      fields = <>
        <ObsNameRow id={id('obs-visibility-scene')} label={t('actions.obs.sceneName')} value={action.scene} entry={scenes} what={t('actions.what.scene')}
          issue={obsNameIssue(action.scene, 'scene')} onChange={scene => onChange({ ...action, scene }, 'obs-visibility-scene')} />
        <ObsNameRow id={id('obs-visibility-source')} label={t('actions.obs.sourceName')} value={action.source} entry={inScene} what={t('actions.what.source')} spaced
          issue={obsNameIssue(action.source, 'source')} onChange={source => onChange({ ...action, source }, 'obs-visibility-source')}
          onOpenList={() => catalog?.requestNames('sceneSources', action.scene)} />
        <label className="spaced-label" htmlFor={id('obs-visibility-op')}>{t('actions.action')}</label><ObsSwitchSelect id={id('obs-visibility-op')} value={action.op} onChange={op => onChange({ ...action, op }, 'obs-visibility-op')} />
        <Hint hint={`${t('actions.obs.visibilityHint', { max: maxObsSceneBytes })} ${obsSwitchHint}`}>{obsSwitchMore}</Hint></>;
      break;
    }
    case 'obs_scene_collection': case 'obs_profile': {
      const collection = action.type === 'obs_scene_collection';
      const issue = obsNameIssue(action.name, collection ? 'scene collection' : 'profile');
      const what = t(collection ? 'actions.what.sceneCollection' : 'actions.what.profile');
      fields = <>
        <ObsNameRow id={id('obs-reload-name')} label={t(collection ? 'actions.obs.collectionName' : 'actions.obs.profileName')} value={action.name}
          entry={collection ? collections : profiles} what={what} issue={issue}
          onChange={name => onChange({ ...action, name }, 'obs-reload-name')} />
        <Hint hint={`${t('actions.obs.reloadSwitch', { what, max: maxObsSceneBytes })} ${t('actions.obs.reloadHint')}`}>{t('actions.obs.reloadMore')}</Hint></>;
      break;
    }
    case 'obs_set_input_volume': {
      fields = <>
        <ObsNameRow id={id('obs-input-volume-input')} label={t('actions.obs.inputName')} value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what={t('actions.what.input')}
          issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ ...action, input }, 'obs-input-volume-input')} />
        <label className="spaced-label" htmlFor={id('obs-input-volume-percent')}>{t('actions.obs.volume')}</label>
        <IntegerField id={id('obs-input-volume-percent')} value={action.percent} minimum={0} maximum={100} onCommit={percent => onChange({ ...action, percent }, 'obs-input-volume-percent')} />
        <p className="fine-print">{t('actions.obs.volumeNote')}</p></>;
      break;
    }
    case 'obs_input_mute':
      fields = <>
        <ObsNameRow id={id('obs-input-mute-input')} label={t('actions.obs.inputName')} value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what={t('actions.what.input')}
          issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ ...action, input }, 'obs-input-mute-input')} />
        <label className="spaced-label" htmlFor={id('obs-input-mute-op')}>{t('actions.action')}</label>
        <SelectField id={id('obs-input-mute-op')} value={action.op} onChange={event => onChange({ ...action, op: event.target.value as ObsMuteOp }, 'obs-input-mute-op')}>
          {(Object.keys(obsMuteOpLabels) as ObsMuteOp[]).map(op => <option key={op} value={op}>{obsMuteOpLabels[op]}</option>)}
        </SelectField>
        <p className="fine-print">{t('actions.obs.muteNote')}</p></>;
      break;
    case 'obs_adjust_input_volume':
      fields = <>
        <ObsNameRow id={id('obs-adjust-volume-input')} label={t('actions.obs.inputName')} value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what={t('actions.what.input')}
          issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ ...action, input }, 'obs-adjust-volume-input')} />
        <label className="spaced-label" htmlFor={id('obs-adjust-volume-step')}>{t('actions.obs.step')}</label>
        <IntegerField id={id('obs-adjust-volume-step')} value={action.step_db} minimum={-20} maximum={20} onCommit={step_db => onChange({ ...action, step_db }, 'obs-adjust-volume-step')} />
        <p className="fine-print">{t('actions.obs.stepNote')}</p></>;
      break;
    case 'obs_media_source':
      fields = <>
        <ObsNameRow id={id('obs-media-input')} label={t('actions.obs.mediaName')} value={action.input} entry={catalogNames(catalog, catalog?.names?.sources)} what={t('actions.what.input')}
          issue={obsNameIssue(action.input, 'input')} onChange={input => onChange({ ...action, input }, 'obs-media-input')} />
        <label className="spaced-label" htmlFor={id('obs-media-op')}>{t('actions.action')}</label>
        <SelectField id={id('obs-media-op')} value={action.op} onChange={event => onChange({ ...action, op: event.target.value as ObsMediaOp }, 'obs-media-op')}>
          {(Object.keys(obsMediaOpLabels) as ObsMediaOp[]).map(op => <option key={op} value={op}>{obsMediaOpLabels[op]}</option>)}
        </SelectField>
        <p className="fine-print">{t('actions.obs.mediaNote')}</p></>;
      break;
    case 'obs_scene': {
      fields = <>
        <ObsNameRow id={id('obs-scene')} label={t('actions.obs.sceneNameObs')} value={action.scene} entry={scenes} what={t('actions.what.scene')}
          issue={obsSceneIssue(action.scene)} onChange={scene => onChange({ type: 'obs_scene', scene }, 'obs-scene')} />
        <p className="fine-print">{t('actions.obs.sceneNote', { max: maxObsSceneBytes })}</p></>;
      break;
    }
  }
  return <div className="action-fields">
    <label htmlFor={id('obs-operation')}>{t('actions.operation')}</label>
    <SelectField id={id('obs-operation')} value={action.type} onChange={event => onChange(switchObsOperation(action, event.target.value as ObsStepType), 'obs-operation')}>
      {(Object.keys(obsOperationLabels) as ObsStepType[]).map(type => <option key={type} value={type}>{obsOperationLabels[type]}</option>)}
    </SelectField>
    {fields}
    {prefix === '' && <ObsConnection onSetUpObs={onSetUpObs} catalog={catalog} />}
  </div>;
}

// The one Streamlabs connection status, shown once per editor like ObsConnection, with the
// Streamlabs listings hanging off it the same way: the Refresh press asks Streamlabs again, and a
// listing that could not be loaded says so quietly, leaving the name fields plain inputs.
export function StreamlabsConnection({ onSetUp, catalog }: { onSetUp?: () => void; catalog?: StreamlabsCatalogView }) {
  const t = useT();
  return <div className="obs-connection" role="note">
    <p className="fine-print">{t('actions.streamlabs.connects')} {onSetUp && <button type="button" className="obs-setup" onClick={onSetUp}>{t('actions.setUp')}</button>}</p>
    {catalog && (catalog.desktop || catalog.status !== 'idle') && <div className="obs-catalog">
      <button type="button" className="button compact" disabled={!catalog.desktop || catalog.status === 'loading'} onClick={catalog.refresh}>
        <RotateCcw size={15} />{catalog.status === 'loading' ? t('actions.listing') : t('actions.streamlabs.refresh')}
      </button>
      {catalog.connection && <span className={`obs-chip obs-chip-${catalog.connection.state}`} title={catalog.connection.message}>{streamlabsConnectionLabels[catalog.connection.state]}</span>}
      {catalog.status === 'failed' && <p className="fine-print" role="note">{t('actions.streamlabs.loadFailed')} {catalog.error}</p>}
    </div>}
    <details className="learn-more"><summary>{t('actions.learnMore')}</summary><p className="fine-print">{t('actions.streamlabs.learnMore', { max: maxObsSceneBytes })}</p></details>
  </div>;
}

// The one Streamlabs editor for the seven streamlabs_* actions, shaped like ObsFields: an
// Operation select (keeping the scene and source names both operations share) and the chosen
// operation's fields. The name fields are comboboxes while catalog holds Streamlabs' matching list
// and plain inputs otherwise, as in ObsFields. The status shows once per editor: here for a
// top-level action, and once for a multi-action or if / else with Streamlabs steps.
export function StreamlabsFields({ action, prefix, onChange, onSetUp, catalog }: {
  action: StreamlabsStepAction; prefix: string; onChange(action: StepAction, field?: string): void; onSetUp?: () => void; catalog?: StreamlabsCatalogView;
}) {
  const t = useT();
  const streamlabsToggleMore = t('actions.streamlabs.toggleMore');
  const id = (name: string) => `${prefix}${name}`;
  let fields: ReactNode;
  switch (action.type) {
    case 'streamlabs_record':
      fields = <Hint hint={t('actions.streamlabs.recordHint')}>{streamlabsToggleMore}</Hint>;
      break;
    case 'streamlabs_stream':
      fields = <>
        <label className="setting-toggle"><input type="checkbox" checked={action.confirm === 'press_twice'}
          onChange={event => onChange(event.target.checked ? { type: 'streamlabs_stream', confirm: 'press_twice' } : { type: 'streamlabs_stream' }, 'streamlabs-stream-confirm')} />{t('actions.pressTwice')}</label>
        <p className="fine-print">{t('actions.streamlabs.streamNote')} {confirmNote(t, action.confirm === 'press_twice')}</p>
        <Hint hint={t('actions.streamlabs.streamHint')}>{streamlabsToggleMore}</Hint></>;
      break;
    case 'streamlabs_studio_mode':
      fields = <Hint hint={t('actions.streamlabs.studioHint')}>{streamlabsToggleMore}</Hint>;
      break;
    case 'streamlabs_transition':
      fields = <p className="fine-print">{t('actions.streamlabs.transitionNote')}</p>;
      break;
    case 'streamlabs_scene': {
      const collection = action.collection ?? '';
      // An empty collection is omitted (the active one), never written as "" or null.
      const withCollection = (next: string): StepAction => next ? { type: 'streamlabs_scene', scene: action.scene, collection: next } : { type: 'streamlabs_scene', scene: action.scene };
      fields = <>
        <ObsNameRow id={id('streamlabs-scene')} label={t('actions.streamlabs.sceneName')} value={action.scene} entry={streamlabsSceneNames(catalog, collection)} what={t('actions.what.scene')} service="Streamlabs"
          issue={streamlabsNameIssue(action.scene, 'scene')} onChange={scene => onChange({ ...action, scene }, 'streamlabs-scene')} />
        <ObsNameRow id={id('streamlabs-scene-collection')} label={t('actions.streamlabs.collection')} value={collection} entry={catalogNames(catalog, catalog?.names?.scene_collections)}
          what={t('actions.what.sceneCollection')} spaced service="Streamlabs"
          issue={collection ? streamlabsNameIssue(collection, 'scene collection') : null} onChange={next => onChange(withCollection(next), 'streamlabs-scene-collection')} />
        <Hint hint={t('actions.streamlabs.sceneHint', { max: maxObsSceneBytes })}>{t('actions.streamlabs.sceneMore')}</Hint></>;
      break;
    }
    case 'streamlabs_source_visibility':
      fields = <>
        <ObsNameRow id={id('streamlabs-visibility-scene')} label={t('actions.obs.sceneName')} value={action.scene} entry={catalogNames(catalog, catalog?.names?.scenes)} what={t('actions.what.scene')} service="Streamlabs"
          issue={streamlabsNameIssue(action.scene, 'scene')} onChange={scene => onChange({ ...action, scene }, 'streamlabs-visibility-scene')} />
        {/* The source choices are the items of the named scene (folders and what they hold), fetched when the choices open. */}
        <ObsNameRow id={id('streamlabs-visibility-source')} label={t('actions.obs.sourceName')} value={action.source} entry={catalog?.list('sceneSources', action.scene) ?? null} what={t('actions.what.source')} spaced service="Streamlabs"
          issue={streamlabsNameIssue(action.source, 'source')} onChange={source => onChange({ ...action, source }, 'streamlabs-visibility-source')}
          onOpenList={() => catalog?.requestNames('sceneSources', action.scene)} />
        <label className="spaced-label" htmlFor={id('streamlabs-visibility-op')}>{t('actions.action')}</label>
        <SelectField id={id('streamlabs-visibility-op')} value={action.op} onChange={event => onChange({ ...action, op: event.target.value as StreamlabsVisibilityOp }, 'streamlabs-visibility-op')}>
          {(Object.keys(streamlabsVisibilityOpLabels) as StreamlabsVisibilityOp[]).map(op => <option key={op} value={op}>{streamlabsVisibilityOpLabels[op]}</option>)}
        </SelectField>
        <Hint hint={t('actions.streamlabs.visibilityHint', { max: maxObsSceneBytes })}>{t('actions.streamlabs.visibilityMore')}</Hint></>;
      break;
    case 'streamlabs_audio': {
      const mode = streamlabsAudioMode(action.op);
      const op = action.op;
      fields = <>
        <ObsNameRow id={id('streamlabs-audio-source')} label={t('actions.streamlabs.audioName')} value={action.source} entry={catalogNames(catalog, catalog?.names?.audio_sources)} what={t('actions.what.audioSource')} service="Streamlabs"
          issue={streamlabsNameIssue(action.source, 'audio source')} onChange={source => onChange({ ...action, source }, 'streamlabs-audio-source')} />
        <label className="spaced-label" htmlFor={id('streamlabs-audio-mode')}>{t('actions.streamlabs.change')}</label>
        <SelectField id={id('streamlabs-audio-mode')} value={mode} onChange={event => onChange({ ...action, op: defaultStreamlabsAudioOp(event.target.value as StreamlabsAudioMode) }, 'streamlabs-audio-mode')}>
          {(Object.keys(streamlabsAudioModeLabels) as StreamlabsAudioMode[]).map(item => <option key={item} value={item}>{streamlabsAudioModeLabels[item]}</option>)}
        </SelectField>
        {'set' in op && <>
          <label className="spaced-label" htmlFor={id('streamlabs-audio-set')}>{t('actions.streamlabs.level')}</label>
          <IntegerField id={id('streamlabs-audio-set')} value={op.set} minimum={streamlabsAudioLimits.set.minimum} maximum={streamlabsAudioLimits.set.maximum}
            onCommit={set => onChange({ ...action, op: { set } }, 'streamlabs-audio-set')} />
          <p className="fine-print">{t('actions.streamlabs.levelNote')}</p></>}
        {'adjust' in op && <>
          <label className="spaced-label" htmlFor={id('streamlabs-audio-adjust')}>{t('actions.streamlabs.step')}</label>
          <IntegerField id={id('streamlabs-audio-adjust')} value={op.adjust} minimum={streamlabsAudioLimits.adjust.minimum} maximum={streamlabsAudioLimits.adjust.maximum} nonZero
            onCommit={adjust => onChange({ ...action, op: { adjust } }, 'streamlabs-audio-adjust')} />
          <p className="fine-print">{t('actions.streamlabs.stepNote')}</p></>}
        {'mute' in op && <>
          <label className="spaced-label" htmlFor={id('streamlabs-audio-mute')}>{t('actions.action')}</label>
          <SelectField id={id('streamlabs-audio-mute')} value={op.mute} onChange={event => onChange({ ...action, op: { mute: event.target.value as ObsMuteOp } }, 'streamlabs-audio-mute')}>
            {(Object.keys(obsMuteOpLabels) as ObsMuteOp[]).map(item => <option key={item} value={item}>{obsMuteOpLabels[item]}</option>)}
          </SelectField>
          <p className="fine-print">{t('actions.streamlabs.muteNote')}</p></>}
      </>;
      break;
    }
  }
  return <div className="action-fields">
    <label htmlFor={id('streamlabs-operation')}>{t('actions.operation')}</label>
    <SelectField id={id('streamlabs-operation')} value={action.type} onChange={event => onChange(switchStreamlabsOperation(action, event.target.value as StreamlabsStepType), 'streamlabs-operation')}>
      {(Object.keys(streamlabsOperationLabels) as StreamlabsStepType[]).map(type => <option key={type} value={type}>{streamlabsOperationLabels[type]}</option>)}
    </SelectField>
    {fields}
    {prefix === '' && <StreamlabsConnection onSetUp={onSetUp} catalog={catalog} />}
  </div>;
}
