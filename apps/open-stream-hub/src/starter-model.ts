// Starter profiles shipped with the app, and the chooser logic around them:
// which starters fit a freshly connected module, when the first-run chooser
// is offered, and which module an inventory change should prompt for. Rust
// remains the authority on profile validity; the tests pin the structural
// rules these templates must meet.
import type { Action, Binding, BranchStep, Condition, ConditionalAction, Modifier, BindingAction, KeyFace, KeypadSlot, LiveDevice, Profile, ServiceModule, Variable, VariableValue } from './contracts';
import { currentSchema, usesKeypadPages } from './contracts';
import { diffDevices } from './device-feed';
import { keyFaceIssue, maximumKeyFaces } from './draft-model';
import { flowIssue, groupItems, isGroup, variableKind, variableNameIssue, variableValueIssue, variablesIssue } from './flow-model';
import { terminalAppLabels, terminalCommandIssue, terminalDirectoryIssue } from './action-model';
import { controlCounts, gestures } from './ui-model';

export interface StarterTemplate {
  id: string; name: string; description: string; profile: Profile;
  /** Apps this starter is meant for: offered as focused-app rules once the profile is created. */
  focusApps?: ReadonlyArray<{ bundle_id: string; name: string }>;
}

// Terminals and editors with a terminal, by bundle ID, for the terminal starter's focused-app rules.
export const terminalApps = [
  { bundle_id: 'com.apple.Terminal', name: 'Terminal' },
  { bundle_id: 'com.googlecode.iterm2', name: 'iTerm2' },
  { bundle_id: 'com.mitchellh.ghostty', name: 'Ghostty' },
  { bundle_id: 'dev.warp.Warp-Stable', name: 'Warp' },
  { bundle_id: 'net.kovidgoyal.kitty', name: 'kitty' },
  { bundle_id: 'com.github.wez.wezterm', name: 'WezTerm' },
  { bundle_id: 'org.alacritty', name: 'Alacritty' },
  { bundle_id: 'com.microsoft.VSCode', name: 'Visual Studio Code' },
] as const;

// One labelled 15-key tile with the action on press, and the faces it shows while a variable test holds.
const key = (label: string, action: Action, faces?: KeyFace[]) => ({ type: 'action', label, bindings: [{ gesture: 'pressed', action }], ...(faces ? { faces } : {}) }) as KeypadSlot;

// A root keypad page with labelled tiles in the first slots and empty ones after.
const keys = (...slots: KeypadSlot[]) => ({
  pages: [{ id: 'root', name: 'Root', slots: [...slots, ...Array.from({ length: 15 - slots.length }, () => null)] }],
});

const volume = (delta: number) => ({ type: 'volume' as const, delta });

const macBasics: StarterTemplate = {
  id: 'mac-basics',
  name: 'Mac basics',
  description: 'Volume, mute and playback on every module, ready as they are.',
  profile: {
    schema_version: currentSchema,
    name: 'Mac basics',
    bindings: [
      { module: 'm003k', control: 0, gesture: 'clockwise', action: volume(5) },
      { module: 'm003k', control: 0, gesture: 'counter_clockwise', action: volume(-5) },
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
      { module: 'm131e', control: 0, gesture: 'right', action: volume(5) },
      { module: 'm131e', control: 0, gesture: 'left', action: volume(-5) },
      { module: 'm131e', control: 0, gesture: 'pressed', action: { type: 'media', command: 'play_pause' } },
      { module: 'm005s', control: 0, gesture: 'position_changed', action: { type: 'volume_from_position' } },
      { module: 'm032le', control: 0, gesture: 'up', action: volume(5) },
      { module: 'm032le', control: 0, gesture: 'down', action: volume(-5) },
      { module: 'm032le', control: 1, gesture: 'up', action: { type: 'media', command: 'next_track' } },
      { module: 'm032le', control: 1, gesture: 'down', action: { type: 'media', command: 'previous_track' } },
    ],
    keypad: keys(
      key('Play / pause', { type: 'media', command: 'play_pause' }),
      key('Next track', { type: 'media', command: 'next_track' }),
      key('Previous track', { type: 'media', command: 'previous_track' }),
      key('Mute', { type: 'toggle_mute' }),
      key('Screenshot', { type: 'system', command: 'screenshot' }),
      key('Break timer', { type: 'timer', seconds: 300, label: 'Break' }),
    ),
  },
};

const streaming: StarterTemplate = {
  id: 'streaming',
  name: 'Streaming (OBS)',
  description: 'Virtual camera, replay buffer and transition on the 15-key module; volume on the knobs and encoder. Scene keys can be added in the editor once OBS is set up in Settings.',
  profile: {
    schema_version: currentSchema,
    name: 'Streaming (OBS)',
    bindings: [
      { module: 'm003k', control: 0, gesture: 'clockwise', action: volume(5) },
      { module: 'm003k', control: 0, gesture: 'counter_clockwise', action: volume(-5) },
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
      { module: 'm131e', control: 0, gesture: 'right', action: volume(5) },
      { module: 'm131e', control: 0, gesture: 'left', action: volume(-5) },
      { module: 'm131e', control: 0, gesture: 'pressed', action: { type: 'obs_virtual_camera', op: 'toggle' } },
    ],
    keypad: keys(
      key('Virtual camera', { type: 'obs_virtual_camera', op: 'toggle' }),
      key('Replay buffer', { type: 'obs_replay_buffer', op: 'toggle' }),
      key('Save replay', { type: 'obs_replay_buffer', op: 'save' }),
      key('Transition', { type: 'obs_transition' }),
      key('Mute output', { type: 'toggle_mute' }),
    ),
  },
};

const streamer: StarterTemplate = {
  id: 'streamer',
  name: 'Streamer',
  description: 'Record, pause, replay and screenshot keys, a stream start and a stream end that each need two presses, and the usual output controls. Scene, microphone and desktop-audio keys name your OBS items: add them in the editor once OBS is set up.',
  profile: {
    schema_version: currentSchema,
    name: 'Streamer',
    bindings: [
      { module: 'm003k', control: 0, gesture: 'clockwise', action: volume(5) },
      { module: 'm003k', control: 0, gesture: 'counter_clockwise', action: volume(-5) },
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } },
      { module: 'm131e', control: 0, gesture: 'right', action: volume(5) },
      { module: 'm131e', control: 0, gesture: 'left', action: volume(-5) },
      { module: 'm131e', control: 0, gesture: 'pressed', action: { type: 'obs_record', op: 'toggle_pause' } },
      { module: 'm005s', control: 0, gesture: 'position_changed', action: { type: 'volume_from_position' } },
    ],
    keypad: keys(
      key('Record', { type: 'obs_record', op: 'toggle' }),
      key('Pause rec', { type: 'obs_record', op: 'toggle_pause' }),
      key('Screenshot', { type: 'obs_screenshot' }),
      key('Replay buffer', { type: 'obs_replay_buffer', op: 'toggle' }),
      key('Save replay', { type: 'obs_replay_buffer', op: 'save' }),
      key('Go live', { type: 'obs_stream', op: 'start', confirm: 'press_twice' }),
      key('End stream', { type: 'obs_stream', op: 'stop', confirm: 'press_twice' }),
      key('Virtual camera', { type: 'obs_virtual_camera', op: 'toggle' }),
      key('Studio mode', { type: 'obs_studio_mode', op: 'toggle' }),
      key('Transition', { type: 'obs_transition' }),
      key('Mute output', { type: 'toggle_mute' }),
      key('Break timer', { type: 'timer', seconds: 300, label: 'Break' }),
    ),
  },
};

const editing: StarterTemplate = {
  id: 'editing',
  name: 'Editing and media',
  description: 'Rotate the encoder to step undo and redo, press it to play; the 15-key module gets undo, redo, playback, a screenshot key and a timer.',
  profile: {
    schema_version: currentSchema,
    name: 'Editing and media',
    bindings: [
      { module: 'm131e', control: 0, gesture: 'right', action: { type: 'system', command: 'redo' } },
      { module: 'm131e', control: 0, gesture: 'left', action: { type: 'system', command: 'undo' } },
      { module: 'm131e', control: 0, gesture: 'pressed', action: { type: 'media', command: 'play_pause' } },
      { module: 'm003k', control: 0, gesture: 'clockwise', action: volume(5) },
      { module: 'm003k', control: 0, gesture: 'counter_clockwise', action: volume(-5) },
      { module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'media', command: 'play_pause' } },
      { module: 'm005s', control: 0, gesture: 'position_changed', action: { type: 'volume_from_position' } },
    ],
    keypad: keys(
      key('Undo', { type: 'system', command: 'undo' }),
      key('Redo', { type: 'system', command: 'redo' }),
      key('Play / pause', { type: 'media', command: 'play_pause' }),
      key('Screenshot', { type: 'system', command: 'screenshot' }),
      key('Timer 10 min', { type: 'timer', seconds: 600 }),
    ),
  },
};

// A face shown while the text variable `agent_state` holds `value`.
const agentFace = (value: string, label: string, background: string): KeyFace => ({ when: { variable_is: { name: 'agent_state', value } }, label, background });

const shortcut = (key: string, ...modifiers: Modifier[]): Action & BranchStep => ({ type: 'shortcut', key, modifiers });
const typed = (text: string): Action => ({ type: 'text', text, press_enter: true });
// A new Terminal window running `command`: how the agent keys start a session.
const session = (command: string): Action => ({ type: 'terminal_command', app: 'terminal', command });

// While an agent waits on a permission prompt (`osh agent ask` set `agent_state` to waiting), the key answers it
// by setting `agent_decision`; otherwise it sends its usual key.
const answerOr = (decision: 'allow' | 'deny', otherwise: BranchStep): ConditionalAction => ({
  type: 'conditional',
  when: { variable_is: { name: 'agent_state', value: 'waiting' } },
  then: [{ type: 'set_variable', name: 'agent_decision', value: decision }],
  else: [otherwise],
});

// Keys for working with a coding agent (Claude Code, Codex) or any shell in a
// terminal. Everything else is sent to the app in front, so it suits the terminal
// being the focused window. It types, and its Claude and Codex keys open a new
// Terminal window running that agent, so it is the one starter that is not
// dry-run safe (see the starter tests). The Accept key follows the text variable `agent_state`, which an agent's hooks
// set through the bundled `osh` tool: amber while it works, red when it needs you, green when it is done, and its
// normal face while idle. While the agent waits on a permission prompt, Accept and the Interrupt key (then showing
// Deny) answer it through `agent_decision` (docs/agent-integration.md).
const terminalAgents: StarterTemplate = {
  id: 'terminal-agents',
  name: 'Terminal and AI agents',
  focusApps: terminalApps,
  description: 'Keys for Claude Code, Codex and any terminal: Claude and Codex keys that open a new Terminal window running that agent, then interrupt, accept, cycle the permission mode, /clear, /compact and continue, plus Ctrl+C, Ctrl+D and Ctrl+L. The knob walks the prompt history. The other keys act on the app in front, so keep the terminal focused. The Accept key shows Working, Needs you or Done while the agent_state variable says so: agent hooks can set it with the bundled osh tool. With the hook installed (osh agent setup claude), Accept and Deny answer permission prompts.',
  profile: {
    schema_version: currentSchema,
    name: 'Terminal and AI agents',
    variables: [{ name: 'agent_state', value: 'idle' }, { name: 'agent_decision', value: 'none' }],
    bindings: [
      { module: 'm131e', control: 0, gesture: 'right', action: shortcut('arrow_down') },
      { module: 'm131e', control: 0, gesture: 'left', action: shortcut('arrow_up') },
      { module: 'm131e', control: 0, gesture: 'pressed', action: shortcut('return') },
      { module: 'm003k', control: 0, gesture: 'clockwise', action: shortcut('page_down') },
      { module: 'm003k', control: 0, gesture: 'counter_clockwise', action: shortcut('page_up') },
      { module: 'm003k', control: 0, gesture: 'pressed', action: shortcut('escape') },
    ],
    keypad: keys(
      key('Accept', answerOr('allow', shortcut('return')), [
        agentFace('working', 'Working', '#d98e04'),
        agentFace('waiting', 'Needs you', '#c81e1e'),
        agentFace('done', 'Done', '#1f9d55'),
      ]),
      key('Interrupt', answerOr('deny', shortcut('escape')), [agentFace('waiting', 'Deny', '#c81e1e')]),
      key('Cycle mode', shortcut('tab', 'shift')),
      key('Continue', typed('continue')),
      key('/clear', typed('/clear')),
      key('/compact', typed('/compact')),
      key('Ctrl+C', shortcut('c', 'control')),
      key('Ctrl+D', shortcut('d', 'control')),
      key('Ctrl+L', shortcut('l', 'control')),
      key('New tab', shortcut('t', 'command')),
      key('Next tab', shortcut('tab', 'control')),
      key('Previous tab', shortcut('tab', 'control', 'shift')),
      key('Claude', session('claude')),
      key('Codex', session('codex')),
    ),
  },
};

export const starters: StarterTemplate[] = [macBasics, streaming, streamer, editing, terminalAgents];

// The modules a profile sets up: every module with a flat binding, plus the
// 15-key module when its keypad page holds any tile. Used both for the
// shipped starters and for the open profile, which decides whether a freshly
// connected module is offered a starter at all.
export function profileModules(profile: Profile): ServiceModule[] {
  const found = new Set<ServiceModule>(profile.bindings.map(binding => binding.module));
  if (profile.keypad?.pages.some(page => page.slots.some(slot => slot !== null))) found.add('m4315');
  return [...found];
}

// The starters offered for a freshly connected module: only ones that set it
// up, so a fader prompt never offers a starter without fader bindings.
export function startersForModule(module: ServiceModule): StarterTemplate[] {
  return starters.filter(item => profileModules(item.profile).includes(module));
}

// The module to offer starters for after an inventory change, or null. The
// first snapshot is the baseline: modules connected at launch are not asked
// about, and neither are removals or swaps without an addition. `skip`
// leaves out modules that should not be asked — one the open profile already
// covers, or one dismissed for this session.
export function nextModulePrompt(previous: LiveDevice[] | null, next: LiveDevice[], skip: (module: ServiceModule) => boolean = () => false): ServiceModule | null {
  if (previous === null) return null;
  const { added } = diffDevices(previous, next);
  return added.find(device => !skip(device.module))?.module ?? null;
}

// The first-run chooser opens once, when storage is fresh and empty. Like the
// other local preferences, an unreadable store never nags: the chooser stays
// closed and remains reachable from the profile manager.
export const startersStorageKey = 'open-stream-hub.starters-offered';

export function shouldOfferStarters(storage: Pick<Storage, 'getItem'> | null, desktop: boolean): boolean {
  if (!desktop || !storage) return false;
  try { return storage.getItem(startersStorageKey) === null; } catch { return false; }
}

export function markStartersOffered(storage: Pick<Storage, 'setItem'> | null) {
  try { storage?.setItem(startersStorageKey, 'offered'); } catch { /* offered again next launch if storage is blocked */ }
}

// Local structural lint for the shipped starters: the same shapes the editor
// checks locally, so a broken template fails here instead of in a save. The
// service re-validates everything on save.
export type ActionCheck = (action: BindingAction) => string | null;

// `check` decides which actions the profile may hold: the starters' dry-run-safe set by default, a pack's wider set
// for an app preset pack. It is applied to flat bindings and keypad tiles alike.
export function starterIssue(profile: Profile, check: ActionCheck = actionIssue): string | null {
  if (!usesKeypadPages(profile) || profile.schema_version > currentSchema) return 'Starters are keypad-page profiles (schema 2 or 3).';
  if (!profile.name.trim()) return 'Profile name cannot be empty.';
  const variables = profile.variables ?? [];
  const variableIssue = variablesIssue(variables);
  if (variableIssue) return variableIssue;
  const flat = new Set<string>();
  for (const binding of profile.bindings) {
    const issue = bindingIssue(binding, check, variables);
    if (issue) return issue;
    flat.add(`${binding.module}/${binding.control}/${binding.gesture}`);
  }
  const pages = profile.keypad?.pages ?? [];
  if (pages.length !== 1) return 'A starter has exactly one keypad page.';
  const [page] = pages;
  if (page.id !== 'root') return 'The keypad page is the root page.';
  if (page.slots.length !== 15) return 'A keypad page has exactly 15 slots.';
  for (const slot of page.slots) {
    if (slot === null) continue;
    if (slot.type !== 'action') return 'Starters use action tiles only.';
    if (slot.bindings.length !== 1) return 'A starter tile holds one binding.';
    const faces = slot.faces ?? [];
    if (faces.length > maximumKeyFaces) return `A key holds at most ${maximumKeyFaces} faces.`;
    for (const face of faces) {
      const issue = keyFaceIssue(face, variables);
      if (issue) return issue;
    }
    for (const keypadBinding of slot.bindings) {
      if (keypadBinding.gesture !== 'pressed') return 'Starter tiles bind press only.';
      const issue = check(keypadBinding.action) ?? variableUseIssue(keypadBinding.action, variables);
      if (issue) return issue;
    }
  }
  return null;
}

function bindingIssue(binding: Binding, check: ActionCheck, variables: ReadonlyArray<Variable>): string | null {
  if (!(binding.module in controlCounts)) return `Unknown module ${binding.module}.`;
  if (binding.module === 'm4315') return 'The 15-key module is bound through the keypad pages, not flat bindings.';
  if (binding.control < 0 || binding.control >= controlCounts[binding.module]) return `Control ${binding.control} is out of range for ${binding.module}.`;
  if (!gestures[binding.module].includes(binding.gesture)) return `Gesture ${binding.gesture} is unsupported for ${binding.module}.`;
  return check(binding.action) ?? variableUseIssue(binding.action, variables);
}

// Every variable an action sets or tests, nested steps included, is declared with the kind it uses
// (Action::check_variables in profile.rs).
function variableUseIssue(action: BindingAction, variables: ReadonlyArray<Variable>): string | null {
  const use = (name: string, value: VariableValue, wanted = variableKind(value)): string | null => {
    const declared = variables.find(variable => variable.name === name);
    if (!declared) return `Variable ${name} is not declared.`;
    return variableKind(declared.value) === wanted ? null : `Variable ${name} is ${variableKind(declared.value)}, not ${wanted}.`;
  };
  if (action.type === 'set_variable') return use(action.name, action.value);
  if (action.type !== 'conditional') return null;
  const leaves = (condition: Condition): Condition[] => (isGroup(condition) ? groupItems(condition).flatMap(leaves) : [condition]);
  for (const leaf of leaves(action.when)) {
    const issue = 'variable_is' in leaf ? use(leaf.variable_is.name, leaf.variable_is.value)
      : 'variable_at_least' in leaf ? use(leaf.variable_at_least.name, leaf.variable_at_least.value, 'number') : null;
    if (issue) return issue;
  }
  for (const step of action.then.concat(action.else ?? [])) {
    const issue = variableUseIssue(step, variables);
    if (issue) return issue;
  }
  return null;
}

// The terminal starter is the one that sends keys and text, starts agent sessions and answers agent hooks, so it is
// checked with this: keys, typed text, opening a terminal window with a command, setting a variable and if / else on
// top of what every starter may hold. Nothing else launches or opens anything. An if / else keeps the limits Rust
// enforces, and its steps are checked the same way; a terminal command is never one of them.
export function typingActionIssue(action: BindingAction): string | null {
  switch (action.type) {
    case 'shortcut':
    case 'text':
      return null;
    case 'terminal_command':
      if (!(action.app in terminalAppLabels)) return `Unknown terminal app ${String(action.app)}.`;
      return terminalCommandIssue(action.command) ?? terminalDirectoryIssue(action.directory ?? '');
    case 'set_variable': return variableNameIssue(action.name) ?? variableValueIssue(action.value);
    case 'conditional': {
      const flow = flowIssue(action);
      if (flow) return flow;
      for (const step of action.then.concat(action.else ?? [])) {
        const issue = (step as BindingAction).type === 'terminal_command' ? 'terminal_command cannot be an if / else step.' : typingActionIssue(step);
        if (issue) return issue;
      }
      return null;
    }
    default: return actionIssue(action);
  }
}

// Starters stay dry-run safe: nothing that types into apps, launches or opens
// anything, and no OBS action that names a scene or source of a set-up OBS. Going live is allowed only behind the
// press-twice guard.
export function actionIssue(action: BindingAction): string | null {
  switch (action.type) {
    case 'volume': return action.delta === 0 || action.delta < -100 || action.delta > 100 ? 'Volume must be nonzero and between -100 and 100.' : null;
    case 'timer':
      if (!(action.seconds >= 1 && action.seconds <= 86_400)) return 'Timer seconds must be 1 through 86400.';
      if (action.label !== undefined && action.label.length > 64) return 'Timer label too long.';
      return null;
    case 'toggle_mute':
    case 'media':
    case 'system':
    case 'volume_from_position':
    case 'obs_virtual_camera':
    case 'obs_replay_buffer':
    case 'obs_transition':
    case 'obs_studio_mode':
    case 'obs_record':
    case 'obs_screenshot':
      return null;
    // Going live or ending a broadcast cannot be undone from the controller: a starter may hold it only behind the
    // press-twice guard.
    case 'obs_stream': return action.confirm === 'press_twice' ? null : 'A starter holds the stream only behind press twice.';
    default: return `${action.type} does not belong in a starter profile.`;
  }
}
