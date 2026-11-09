import type { PluginAction, PluginValue } from './contracts';
import { invisibleTextCharacter, variableReferences } from './action-model';

// What `list_plugins` returns (plugins.rs): the plugins folder and each folder in it, with its manifest or why it
// cannot be used, whether the user enabled it, its secrets (set or not, never the value) and the host's state for the
// ones that run.
export type PluginParamType = 'text' | 'number' | 'bool' | 'choice';
export interface PluginChoice { id: string; label: string }
export interface PluginParam {
  id: string; label: string; type: PluginParamType; required?: boolean;
  // Text only: {{name}} references may be filled in when pressed; a secret's value lives in the Keychain, one per
  // plugin and parameter id, never in the profile.
  variables?: boolean; secret?: boolean;
  min?: number; max?: number; choices?: PluginChoice[]; default?: PluginValue;
}
export interface PluginManifestAction { id: string; label: string; params?: PluginParam[] }
export interface PluginManifest {
  schema: number; id: string; name: string; version: string; command: string[]; actions: PluginManifestAction[];
  permissions?: { network?: string[]; variables?: string[]; programs?: boolean };
}
export type PluginState = 'running' | 'starting' | 'restarting' | 'stopped' | 'unusable' | 'not_enabled';
export interface PluginStatus { id: string; state: PluginState; error?: string; logs: string[]; dropped_logs: number }
// enabled: the approved fingerprint is the manifest's; changed: plugin.json changed since it was enabled.
export type PluginApproval = 'enabled' | 'changed' | 'not_enabled';
// set is null when the Keychain could not tell.
export interface PluginSecretStatus { id: string; label: string; set: boolean | null }
export interface ListedPlugin {
  folder: string; manifest?: PluginManifest; fingerprint?: string; error?: string;
  approval?: PluginApproval; secrets?: PluginSecretStatus[]; status?: PluginStatus;
}
export interface PluginListing { folder: string; plugins: ListedPlugin[] }

// The plugins with a usable manifest, by name.
export function usablePlugins(listing: PluginListing | null): PluginManifest[] {
  return (listing?.plugins ?? []).flatMap(plugin => plugin.manifest ? [plugin.manifest] : [])
    .sort((a, b) => a.name.localeCompare(b.name));
}

// MAJOR.MINOR.PATCH as numbers, or null (parse_version in plugin.rs).
export function parseVersion(version: string): [number, number, number] | null {
  const parts = version.split('.');
  if (parts.length !== 3 || !parts.every(part => /^\d{1,9}$/.test(part))) return null;
  return parts.map(Number) as [number, number, number];
}

export function versionAtLeast(installed: string, wanted: string): boolean {
  const [a, b] = [parseVersion(installed), parseVersion(wanted)];
  if (!a || !b) return false;
  for (let index = 0; index < 3; index += 1) if (a[index] !== b[index]) return a[index] > b[index];
  return true;
}

// The installed manifest and its action for a bound plugin action, or why the key will not run.
export function resolvePluginAction(listing: PluginListing | null, action: PluginAction):
  { manifest?: PluginManifest; declared?: PluginManifestAction; problem?: string } {
  if (!listing) return {};
  const listed = listing.plugins.find(plugin => plugin.folder === action.plugin);
  if (!listed) return { problem: `${action.plugin} is not installed. Put its folder in the plugins folder (Settings > Plugins).` };
  const manifest = listed.manifest;
  if (!manifest) return { problem: `${action.plugin} cannot be used: ${listed.error ?? 'its manifest is not valid'}.` };
  const declared = manifest.actions.find(item => item.id === action.action);
  if (!declared) return { manifest, problem: `${manifest.name} ${manifest.version} has no action ${action.action}.` };
  if (!versionAtLeast(manifest.version, action.min_version)) {
    return { manifest, declared, problem: `${manifest.name} ${manifest.version} is older than ${action.min_version}, which this key needs. Update the plugin.` };
  }
  return { manifest, declared };
}

const inRange = (value: number, param: PluginParam) =>
  (param.min === undefined || value >= param.min) && (param.max === undefined || value <= param.max);

export const isSecretParam = (param: PluginParam): boolean => param.type === 'text' && param.secret === true;

// Whether a secret of the plugin holds a value in the Keychain: null when the Keychain could not tell, undefined when
// the listing does not say.
export function pluginSecretSet(listing: PluginListing | null, plugin: string, param: string): boolean | null | undefined {
  return listing?.plugins.find(item => item.folder === plugin)?.secrets?.find(secret => secret.id === param)?.set;
}

// Whether `value` fits the parameter (check_invoke in plugins.rs), and why not. A secret is never in the profile: the
// app fills it in from the Keychain, so a missing one is not an issue here and one the profile carries is.
export function pluginParamIssue(param: PluginParam, value: PluginValue | undefined, declared: ReadonlyArray<string> = []): string | null {
  if (isSecretParam(param)) return value === undefined ? null : `${param.label} is a secret and is kept in the Keychain, not the profile. Move it to the Keychain, then save.`;
  if (value === undefined) return param.required ? `${param.label} is required.` : null;
  switch (param.type) {
    case 'text': {
      if (typeof value !== 'string') return `${param.label} must be text.`;
      if (new TextEncoder().encode(value).length > 2048) return `${param.label} must be at most 2048 bytes.`;
      if (/\p{Cc}/u.test(value) || invisibleTextCharacter.test(value)) return `${param.label} must be one line without control or invisible characters.`;
      if (param.variables && value.includes('{{')) {
        const names = variableReferences(value);
        if (!names) return 'Every {{ must begin a variable reference such as {{name}}, with a variable name and no spaces.';
        const missing = names.find(name => !declared.includes(name));
        if (missing) return `Variable ${missing} is not declared in this profile.`;
      }
      return null;
    }
    case 'number':
      if (typeof value !== 'number' || !Number.isFinite(value)) return `${param.label} must be a number.`;
      return inRange(value, param) ? null : `${param.label} must be between ${param.min ?? '…'} and ${param.max ?? '…'}.`;
    case 'bool': return typeof value === 'boolean' ? null : `${param.label} must be on or off.`;
    case 'choice': return typeof value === 'string' && (param.choices ?? []).some(choice => choice.id === value) ? null : `Pick one of the choices for ${param.label}.`;
  }
}

// A plugin action for `actionId` of `manifest`, starting from the manifest's defaults, keeping the values of
// `previous` that still fit. min_version is the installed version, so an older plugin is caught when it runs.
export function pluginActionWith(manifest: PluginManifest, actionId: string, previous?: PluginAction): PluginAction {
  const declared = manifest.actions.find(item => item.id === actionId) ?? manifest.actions[0];
  let next: PluginAction = { type: 'plugin', plugin: manifest.id, min_version: manifest.version, action: declared.id };
  for (const param of (declared.params ?? []).filter(param => !isSecretParam(param))) {
    const kept = previous?.plugin === manifest.id ? previous.params?.[param.id] : undefined;
    // Kept when it fits the parameter; whether its variables are declared is the form's to say, not a reason to drop it.
    const value = kept !== undefined && pluginParamIssue({ ...param, variables: false }, kept) === null ? kept : param.default;
    if (value !== undefined) next = withPluginParam(next, param, value);
  }
  return next;
}

// Sets (or, with undefined, removes) one parameter. A text parameter the manifest lets take variables is filled in
// exactly when it holds {{, so variables lists it then and only then. Empty params and variables are omitted.
export function withPluginParam(action: PluginAction, param: PluginParam, value: PluginValue | undefined): PluginAction {
  const params = { ...(action.params ?? {}) };
  if (value === undefined) delete params[param.id]; else params[param.id] = value;
  const variables = new Set(action.variables ?? []);
  if (param.type === 'text' && param.variables && typeof value === 'string' && value.includes('{{')) variables.add(param.id);
  else variables.delete(param.id);
  const { params: _params, variables: _variables, ...rest } = action;
  return {
    ...rest,
    ...(Object.keys(params).length ? { params } : {}),
    ...(variables.size ? { variables: [...variables].sort() } : {}),
  };
}

// Every problem a bound plugin action has against the installed manifest, one per parameter at most, plus ones the
// manifest does not declare.
export function pluginActionIssues(declared: PluginManifestAction, action: PluginAction, variables: ReadonlyArray<string>): string[] {
  const params = declared.params ?? [];
  const issues = params.flatMap(param => {
    const issue = pluginParamIssue(param, action.params?.[param.id], variables);
    return issue ? [issue] : [];
  });
  for (const name of Object.keys(action.params ?? {})) {
    if (!params.some(param => param.id === name)) issues.push(`The plugin has no parameter ${name}; remove it by choosing the action again.`);
  }
  return issues;
}
