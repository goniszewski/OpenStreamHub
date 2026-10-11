// Starter profiles shipped with the app, and the chooser logic around them:
// which starters fit a freshly connected module, when the first-run chooser
// is offered, and which module an inventory change should prompt for. Rust
// remains the authority on profile validity; the tests pin the structural
// rules these templates must meet.
import type { Action, Binding, BindingAction, KeypadPage, KeypadSlot, LiveDevice, Profile, ServiceModule } from './contracts';
import { diffDevices } from './device-feed';
import { controlCounts, gestures } from './ui-model';

export interface StarterTemplate { id: string; name: string; description: string; profile: Profile }

// One labelled 15-key tile with the action on press.
const key = (label: string, action: Action) => ({ type: 'action', label, bindings: [{ gesture: 'pressed', action }] }) as KeypadSlot;

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
    schema_version: 2,
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
    schema_version: 2,
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

const editing: StarterTemplate = {
  id: 'editing',
  name: 'Editing and media',
  description: 'Rotate the encoder to step undo and redo, press it to play; the 15-key module gets undo, redo, playback, a screenshot key and a timer.',
  profile: {
    schema_version: 2,
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

export const starters: StarterTemplate[] = [macBasics, streaming, editing];

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
  if (profile.schema_version !== 2) return 'Starters are schema-2 profiles.';
  if (!profile.name.trim()) return 'Profile name cannot be empty.';
  const flat = new Set<string>();
  for (const binding of profile.bindings) {
    const issue = bindingIssue(binding, check);
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
    for (const keypadBinding of slot.bindings) {
      if (keypadBinding.gesture !== 'pressed') return 'Starter tiles bind press only.';
      const issue = check(keypadBinding.action);
      if (issue) return issue;
    }
  }
  return null;
}

function bindingIssue(binding: Binding, check: ActionCheck): string | null {
  if (!(binding.module in controlCounts)) return `Unknown module ${binding.module}.`;
  if (binding.module === 'm4315') return 'The 15-key module is bound through the keypad pages, not flat bindings.';
  if (binding.control < 0 || binding.control >= controlCounts[binding.module]) return `Control ${binding.control} is out of range for ${binding.module}.`;
  if (!gestures[binding.module].includes(binding.gesture)) return `Gesture ${binding.gesture} is unsupported for ${binding.module}.`;
  return check(binding.action);
}

// Starters stay dry-run safe: nothing that types into apps, launches or opens
// anything, and no OBS action that names a scene or source of a set-up OBS.
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
      return null;
    default: return `${action.type} does not belong in a starter profile.`;
  }
}
