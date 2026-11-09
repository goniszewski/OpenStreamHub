import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { PluginAction } from './contracts';
import { pluginIssue, pluginReferences } from './action-model';
import { pluginActionIssues, pluginActionWith, pluginParamIssue, pluginSecretSet, resolvePluginAction, usablePlugins, versionAtLeast, withPluginParam } from './plugins-model';
import { lights, listing } from './plugins-test-data';
import { capabilityLines, profileCapabilities, stepLabel } from './profile-capabilities';
import { actionVariableNames, renameVariable } from './variable-model';
import type { Profile } from './contracts';

const fixture = JSON.parse(readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/actions.json', import.meta.url), 'utf8')) as {
  valid: Record<string, unknown>[]; invalid: Record<string, unknown>[];
};

describe('plugin action contract', () => {
  it('agrees with the shared fixture on valid and invalid plugin actions', () => {
    const valid = fixture.valid.flatMap(action => action.type === 'plugin' ? [action]
      : action.type === 'multi_action' ? (action.actions as Record<string, unknown>[]).filter(step => step.type === 'plugin') : []);
    expect(valid.length).toBeGreaterThan(3);
    for (const action of valid) expect(pluginIssue(action), JSON.stringify(action)).toBeNull();
    const invalid = fixture.invalid.filter(action => action.type === 'plugin');
    expect(invalid.length).toBeGreaterThan(8);
    // An unknown field (command) is refused by serde in Rust; the TS check does not look at extra fields.
    for (const action of invalid.filter(action => !('command' in action))) expect(pluginIssue(action), JSON.stringify(action)).not.toBeNull();
  });

  it('reads variable references only from the parameters it lists', () => {
    expect(pluginReferences({ params: { a: '{{x}}-{{y}}', b: '{{z}}' }, variables: ['a'] })).toEqual({ names: ['x', 'y'], malformed: false });
    expect(pluginReferences({ params: { a: '{{ x }}' }, variables: ['a'] }).malformed).toBe(true);
    expect(pluginReferences({ params: { a: 3 }, variables: ['a'] }).malformed).toBe(true);
    expect(pluginReferences({ params: { a: '{{x}}' } }).names).toEqual([]);
  });
});

describe('plugins model', () => {
  it('compares versions numerically', () => {
    expect(versionAtLeast('1.10.0', '1.9.9')).toBe(true);
    expect(versionAtLeast('1.2.0', '1.2.0')).toBe(true);
    expect(versionAtLeast('1.2.0', '1.3.0')).toBe(false);
    expect(versionAtLeast('1.2', '1.0.0')).toBe(false);
  });

  it('lists only usable plugins and says why a key will not run', () => {
    expect(usablePlugins(listing).map(manifest => manifest.id)).toEqual(['com.example.lights']);
    const action = (patch: Partial<PluginAction>): PluginAction => ({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.0.0', action: 'toggle', ...patch });
    expect(resolvePluginAction(listing, action({})).problem).toBeUndefined();
    expect(resolvePluginAction(listing, action({ plugin: 'com.example.gone' })).problem).toContain('is not installed');
    expect(resolvePluginAction(listing, action({ plugin: 'com.example.broken' })).problem).toContain('not a valid manifest');
    expect(resolvePluginAction(listing, action({ action: 'dim' })).problem).toContain('has no action dim');
    expect(resolvePluginAction(listing, action({ min_version: '2.0.0' })).problem).toContain('older than 2.0.0');
    expect(resolvePluginAction(null, action({ plugin: 'com.example.gone' }))).toEqual({});
  });

  it('starts from the defaults and keeps values that still fit when the action changes', () => {
    const fresh = pluginActionWith(lights, 'set');
    expect(fresh).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { level: 50, mode: 'warm' } });
    const edited = withPluginParam(withPluginParam(fresh, lights.actions[1].params![0], 'light.{{room}}'), lights.actions[1].params![2], true);
    expect(edited.variables).toEqual(['entity']);
    expect(pluginActionWith(lights, 'set', { ...edited, params: { ...edited.params, level: 500 } }).params)
      .toEqual({ entity: 'light.{{room}}', level: 50, fade: true, mode: 'warm' });
    expect(pluginActionWith(lights, 'toggle', edited)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'toggle' });
  });

  it('fills in variables exactly when a text parameter that allows it holds {{', () => {
    const [entity, , , , token] = lights.actions[1].params!;
    let action = pluginActionWith(lights, 'set');
    action = withPluginParam(action, entity, 'light.{{room}}');
    expect(action.variables).toEqual(['entity']);
    action = withPluginParam(action, entity, 'light.desk');
    expect(action.variables).toBeUndefined();
    // A secret text does not take variables, so {{ stays plain text there.
    action = withPluginParam(action, token, '{{room}}');
    expect(action.variables).toBeUndefined();
    action = withPluginParam(action, token, undefined);
    expect(action.params).not.toHaveProperty('token');
  });

  it('never calls a secret missing, refuses one the profile carries and does not carry one across actions', () => {
    const token = { ...lights.actions[1].params![4], required: true };
    expect(pluginParamIssue(token, undefined)).toBeNull();
    expect(pluginParamIssue(token, 'old')).toContain('kept in the Keychain, not the profile');
    expect(pluginSecretSet(listing, 'com.example.lights', 'token')).toBe(true);
    expect(pluginSecretSet(listing, 'com.example.lights', 'other')).toBeUndefined();
    expect(pluginActionWith(lights, 'set', { type: 'plugin', plugin: lights.id, min_version: '1.2.0', action: 'set', params: { entity: 'a', token: 'old' } }).params)
      .not.toHaveProperty('token');
  });

  it('checks each value against its parameter', () => {
    const [entity, level, fade, mode] = lights.actions[1].params!;
    expect(pluginParamIssue(entity, undefined)).toBe('Light is required.');
    expect(pluginParamIssue(entity, 'a​b')).toContain('invisible');
    expect(pluginParamIssue(entity, '{{room}}', [])).toBe('Variable room is not declared in this profile.');
    expect(pluginParamIssue(entity, '{{room}}', ['room'])).toBeNull();
    expect(pluginParamIssue(entity, '{{ room }}', ['room'])).toContain('Every {{');
    expect(pluginParamIssue(level, 101)).toContain('between 0 and 100');
    expect(pluginParamIssue(fade, 'yes')).toContain('on or off');
    expect(pluginParamIssue(mode, 'hot')).toContain('Pick one');
    expect(pluginActionIssues(lights.actions[1], { type: 'plugin', plugin: lights.id, min_version: '1.0.0', action: 'set', params: { entity: 'a', bogus: 1 } }, []))
      .toEqual(['The plugin has no parameter bogus; remove it by choosing the action again.']);
  });
});

const flat = (...actions: Profile['bindings'][number]['action'][]): Profile => ({
  schema_version: 5, name: 'p',
  bindings: actions.map((action, control) => ({ module: 'm003k', control, gesture: 'pressed', action })),
  variables: [{ name: 'room', value: 'desk' }],
} as Profile);

describe('plugin actions in the Execute confirmation and the variables panel', () => {
  const set: PluginAction = { type: 'plugin', plugin: 'com.example.lights', min_version: '1.0.0', action: 'set', params: { entity: 'light.{{room}}', level: 40 }, variables: ['entity'] };

  it('is high risk and names the plugin, the action, every parameter and the variables that feed it', () => {
    const capabilities = profileCapabilities(flat(set, { ...set, min_version: '1.4.0' }, { type: 'set_variable', name: 'room', value: 'hall' }));
    expect(capabilities.risk).toBe('high');
    expect(capabilities.riskReasons).toContain('A key asks a plugin to act. A plugin is a program on this Mac that runs as you.');
    // The minimum version does not make a second entry.
    expect(capabilities.plugins).toEqual([
      'com.example.lights: set with {"entity":"light.{{room}}","level":40}, filling in the current value of room when pressed (room starts at “desk”, keys set it to “hall”; other programs on this Mac can change it)',
    ]);
    expect(capabilityLines(capabilities).some(line => line.startsWith('Asks plugins to act.'))).toBe(true);
    expect(stepLabel(set)).toContain('Ask the plugin com.example.lights: set');
  });

  it('counts each reference as a use and renames it', () => {
    expect(actionVariableNames(set)).toEqual(['room']);
    const renamed = renameVariable(flat(set), 'room', 'area');
    expect(renamed.bindings[0].action).toEqual({ ...set, params: { entity: 'light.{{area}}', level: 40 } });
  });
});
