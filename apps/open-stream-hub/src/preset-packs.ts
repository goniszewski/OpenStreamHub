// App preset packs: ready-made profiles for one application (editing, audio, calls, music), written as JSON files in
// `presets/` and checked here before they are offered. A contributor adds a pack by adding a file; nothing here changes.
// A pack becomes a new saved profile that can run only as a dry run until the user reviews it and allows it to execute
// actions (the same flag an imported .mhz profile gets). Rust validates the profile again when it is created.
import type { BindingAction, Modifier, Profile } from './contracts';
import { isMediaKey, namedKeys } from './action-model';
import { labelIssue } from './draft-model';
import { bundleIdIssue } from './focus-rules';
import { starterIssue } from './starter-model';
import type { ServiceModule } from './contracts';
import { profileModules } from './starter-model';

// Licences a pack may be shared under, as SPDX identifiers. All of them allow the app to ship and copy the pack.
export const packLicences = ['CC0-1.0', 'MIT', 'Apache-2.0', 'CC-BY-4.0'] as const;
export type PackLicence = (typeof packLicences)[number];

export interface PresetPack {
  pack_version: 1;
  id: string;
  name: string;
  description: string;
  // The application the pack is for, and optionally its bundle ID (it also names the suggested focused-app rule).
  app: { name: string; bundle_id?: string };
  license: PackLicence;
  // Where the shortcuts and layout come from: the vendor's own documentation, or "original work".
  source: string;
  // Whether the author checked every shortcut against the application on a Mac. False is honest and fine: the chooser says so.
  verified: boolean;
  // Offer to switch to the profile when the app comes to the front.
  focus_rule?: { bundle_id: string };
  profile: Profile;
}

export interface RejectedPack { file: string; issues: string[] }

const packKeys = new Set(['pack_version', 'id', 'name', 'description', 'app', 'license', 'source', 'verified', 'focus_rule', 'profile']);
const modifierNames = new Set<Modifier>(['command', 'control', 'option', 'shift']);
const namedKeyNames = new Set(namedKeys.map(([name]) => name));
const maxIssues = 12;

// What a pack may bind. No typed text, no URLs, no launching and no multi-step sequences: a pack is keyboard shortcuts,
// media, system and volume controls and the simple OBS toggles, so reading one reveals everything it can do.
const allowedActions = new Set(['shortcut', 'media', 'system', 'volume', 'set_volume', 'toggle_mute', 'volume_from_position', 'timer', 'obs_virtual_camera', 'obs_replay_buffer', 'obs_transition']);

export function packActionIssue(action: BindingAction): string | null {
  if (!allowedActions.has(action.type)) return `A pack cannot hold a "${action.type}" action; it may hold ${[...allowedActions].join(', ')}.`;
  if (action.type === 'shortcut') {
    const { key, modifiers } = action;
    if (!(isMediaKey(key) || namedKeyNames.has(key) || /^[a-z0-9]$/.test(key))) return `"${key}" is not a key a shortcut can send (use a lowercase letter or digit, or a named key such as space, return or f13).`;
    if (new Set(modifiers).size !== modifiers.length || modifiers.some(modifier => !modifierNames.has(modifier))) return `The shortcut "${key}" lists a modifier twice or one that does not exist.`;
    if (isMediaKey(key) && modifiers.length > 0) return `The media key "${key}" cannot combine with modifiers.`;
  }
  if (action.type === 'volume' && (action.delta === 0 || action.delta < -100 || action.delta > 100)) return 'Volume must be nonzero and between -100 and 100.';
  if (action.type === 'timer' && !(action.seconds >= 1 && action.seconds <= 86_400)) return 'Timer seconds must be 1 through 86400.';
  return null;
}

const isObject = (value: unknown): value is Record<string, unknown> => typeof value === 'object' && value !== null && !Array.isArray(value);

// Every reason a parsed JSON value is not a usable pack (up to a dozen), or none. The profile's own structure is checked
// by the same lint the shipped starters use, with the wider set of actions a pack may hold.
export function packIssues(raw: unknown): string[] {
  if (!isObject(raw)) return ['A pack is a JSON object.'];
  const issues: string[] = [];
  const add = (message: string | null) => { if (message && issues.length < maxIssues) issues.push(message); };
  for (const key of Object.keys(raw)) if (!packKeys.has(key)) add(`Unknown field "${key}".`);
  if (raw.pack_version !== 1) add('pack_version must be 1.');
  if (typeof raw.id !== 'string' || !/^[a-z0-9][a-z0-9-]{1,39}$/.test(raw.id)) add('id must be 2 to 40 lowercase letters, digits or hyphens, starting with a letter or digit.');
  if (typeof raw.name !== 'string') add('name must be text.'); else add(labelIssue(raw.name, 'Pack name'));
  if (typeof raw.description !== 'string' || raw.description.trim().length < 10 || raw.description.length > 300) add('description must be 10 to 300 characters.');
  if (!isObject(raw.app) || typeof raw.app.name !== 'string') add('app must name the application: {"name": "…"}.');
  else {
    add(labelIssue(raw.app.name, 'App name'));
    for (const key of Object.keys(raw.app)) if (key !== 'name' && key !== 'bundle_id') add(`Unknown app field "${key}".`);
    if (raw.app.bundle_id !== undefined) add(typeof raw.app.bundle_id === 'string' ? bundleIdIssue(raw.app.bundle_id) : 'app.bundle_id must be text.');
  }
  if (!packLicences.includes(raw.license as PackLicence)) add(`license must be one of ${packLicences.join(', ')}.`);
  if (typeof raw.source !== 'string' || raw.source.trim().length < 10 || raw.source.length > 400) add('source must say where the shortcuts come from, in 10 to 400 characters.');
  if (typeof raw.verified !== 'boolean') add('verified must be true or false.');
  if (raw.focus_rule !== undefined) {
    if (!isObject(raw.focus_rule) || typeof raw.focus_rule.bundle_id !== 'string') add('focus_rule must be {"bundle_id": "…"}.');
    else add(bundleIdIssue(raw.focus_rule.bundle_id));
  }
  if (!isObject(raw.profile)) add('profile must be a profile object.');
  else {
    const profile = raw.profile as unknown as Profile;
    if (!Array.isArray(profile.bindings) || typeof profile.name !== 'string') add('profile needs a name and a bindings list.');
    else {
      add(starterIssue(profile, packActionIssue));
      if (profile.keypad !== undefined) for (const slot of profile.keypad.pages?.[0]?.slots ?? []) if (slot?.type === 'action') add(labelIssue(slot.label, 'Key label'));
      add(labelIssue(profile.name, 'Profile name'));
      if (profile.bindings.length + (profile.keypad?.pages[0]?.slots.filter(slot => slot !== null).length ?? 0) < 3) add('A pack binds at least three controls.');
    }
  }
  return issues;
}

export function parsePack(text: string): { pack: PresetPack | null; issues: string[] } {
  let raw: unknown;
  try { raw = JSON.parse(text); } catch (reason) { return { pack: null, issues: [`This is not valid JSON: ${reason instanceof Error ? reason.message : 'unreadable'}.`] }; }
  const issues = packIssues(raw);
  return issues.length ? { pack: null, issues } : { pack: raw as PresetPack, issues: [] };
}

// The valid packs by name, and a report of every file that was refused and why. A second pack with an id already taken
// is refused too, so the first one stays what the chooser shows.
export function loadPacks(files: Record<string, unknown>): { packs: PresetPack[]; rejected: RejectedPack[] } {
  const packs: PresetPack[] = [];
  const rejected: RejectedPack[] = [];
  for (const [file, raw] of Object.entries(files).sort(([a], [b]) => a.localeCompare(b))) {
    const issues = packIssues(raw);
    if (issues.length === 0 && packs.some(pack => pack.id === (raw as PresetPack).id)) issues.push(`The id "${(raw as PresetPack).id}" is already used by another pack.`);
    if (issues.length) rejected.push({ file: file.split('/').pop() ?? file, issues });
    else packs.push(raw as PresetPack);
  }
  packs.sort((a, b) => a.name.localeCompare(b.name));
  return { packs, rejected };
}

// The packs shipped in `presets/`, read at build time.
export const bundledPacks = loadPacks(import.meta.glob('../presets/*.json', { eager: true, import: 'default' }));

export const packModules = (pack: PresetPack): ServiceModule[] => profileModules(pack.profile);
