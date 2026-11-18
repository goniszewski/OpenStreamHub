import { describe, expect, it } from 'vitest';
import type { Action, ConditionalAction, PluginCondition, Profile } from './contracts';
import { conditionLabel, conditionPluginUses, pluginIssue, pluginUses } from './action-model';
import { capabilityLines, profileCapabilities } from './profile-capabilities';
import { conditionPlugins, pluginActionWith, pluginConditionWith, resolvePluginCondition, usablePlugins, withPluginParam } from './plugins-model';
import { keyFaceIssue } from './draft-model';
import { bundledPacks, packActionIssue } from './preset-packs';
import { starterIssue, starters, typingActionIssue } from './starter-model';
import { actionVariableNames, renameVariable, variableUseCounts } from './variable-model';
import { checkingLights, conditionListing, meetings } from './plugins-test-data';

const isOn: PluginCondition = { plugin: 'com.example.lights', min_version: '1.2.0', condition: 'is_on', params: { entity: 'light.{{room}}' }, variables: ['entity'], value: true };
const ifOn = (when: ConditionalAction['when'] = { plugin: isOn }): ConditionalAction => ({ type: 'conditional', when, then: [{ type: 'toggle_mute' }] });
const flat = (...actions: Action[]): Profile => ({
  schema_version: 3, name: 'Test',
  bindings: actions.map((action, control) => ({ module: 'm003k', control, gesture: 'pressed', action })),
  variables: [{ name: 'room', value: 'desk' }],
} as Profile);

describe('plugin uses', () => {
  it('walks plugin actions and plugin conditions, nested groups and branches included, tests before steps', () => {
    const action: ConditionalAction = {
      type: 'conditional',
      when: { all: [{ output_muted: true }, { any: [{ plugin: isOn }, { plugin: { plugin: 'com.example.meetings', min_version: '2.0.0', condition: 'in_call', value: false } }] }] },
      then: [{ type: 'plugin', plugin: 'com.example.lights', min_version: '1.0.0', action: 'toggle' }],
      else: [ifOn({ plugin: { ...isOn, condition: 'reachable', params: undefined, variables: undefined } })],
    };
    expect(pluginUses(action).map(use => [use.kind, use.plugin, use.id])).toEqual([
      ['condition', 'com.example.lights', 'is_on'], ['condition', 'com.example.meetings', 'in_call'],
      ['action', 'com.example.lights', 'toggle'], ['condition', 'com.example.lights', 'reachable'],
    ]);
    expect(conditionPluginUses({ plugin: isOn })).toEqual([{ kind: 'condition', plugin: 'com.example.lights', min_version: '1.2.0', id: 'is_on', params: { entity: 'light.{{room}}' }, variables: ['entity'], value: true }]);
    // Partial shapes (a run snapshot, an import) are tolerated.
    expect(pluginUses({ type: 'conditional', when: 'nope', then: null })).toEqual([]);
  });

  it('reads as a test in the key label', () => {
    expect(conditionLabel({ plugin: isOn })).toBe('com.example.lights: is_on holds');
    expect(conditionLabel({ any: [{ output_muted: true }, { plugin: { ...isOn, value: false } }] })).toBe('the output is muted or com.example.lights: is_on does not hold');
  });
});

describe('plugin conditions against the installed manifests', () => {
  it('offers the conditions of enabled plugins only, and plugin actions only from manifests that have actions', () => {
    expect(conditionPlugins(conditionListing).map(manifest => manifest.id)).toEqual(['com.example.lights', 'com.example.meetings']);
    expect(usablePlugins(conditionListing).map(manifest => manifest.id)).toEqual(['com.example.hall', 'com.example.lights']);
  });

  it('starts from the defaults, keeps fitting values and the answer, and keeps variables in step with the text', () => {
    const fresh = pluginConditionWith(checkingLights, 'is_on');
    expect(fresh).toEqual({ plugin: 'com.example.lights', min_version: '1.2.0', condition: 'is_on', value: true });
    const typed = withPluginParam(fresh, checkingLights.conditions![0].params![0], 'light.{{room}}');
    expect(typed).toEqual({ ...fresh, params: { entity: 'light.{{room}}' }, variables: ['entity'] });
    expect(withPluginParam(typed, checkingLights.conditions![0].params![0], 'light.desk')).toEqual({ ...fresh, params: { entity: 'light.desk' } });
    expect(pluginConditionWith(checkingLights, 'is_on', { ...isOn, value: false })).toEqual({ ...isOn, value: false });
    expect(pluginConditionWith(meetings, 'in_call')).toEqual({ plugin: 'com.example.meetings', min_version: '2.0.0', condition: 'in_call', value: true });
  });

  it('does not crash building an action from a manifest that offers only conditions', () => {
    const previous = { type: 'plugin' as const, plugin: 'com.example.meetings', min_version: '1.0.0', action: 'mute' };
    expect(pluginActionWith(meetings, 'mute', previous)).toBe(previous);
    expect(pluginActionWith(meetings, '')).toEqual({ type: 'plugin', plugin: 'com.example.meetings', min_version: '2.0.0', action: '' });
  });

  it('names a condition as a condition in its shape issues', () => {
    const tooMany = Object.fromEntries(Array.from({ length: 33 }, (_, index) => [`p${index}`, 1]));
    expect(pluginIssue({ ...isOn, action: 'Bad id' }, 'condition')).toBe('Choose one of the plugin\'s conditions.');
    expect(pluginIssue({ ...isOn, action: isOn.condition, params: tooMany, variables: [] }, 'condition')).toBe('A plugin condition takes at most 32 parameters.');
    expect(pluginIssue({ ...isOn, action: 'Bad id' })).toBe('Choose one of the plugin\'s actions.');
    expect(pluginIssue({ ...isOn, action: isOn.condition, params: tooMany, variables: [] })).toBe('A plugin action takes at most 32 parameters.');
  });

  it('says what is wrong: not installed, older, unknown condition, not enabled', () => {
    expect(resolvePluginCondition(conditionListing, isOn).problem).toBeUndefined();
    expect(resolvePluginCondition(conditionListing, { ...isOn, plugin: 'com.example.gone' }).problem).toContain('com.example.gone is not installed');
    expect(resolvePluginCondition(conditionListing, { ...isOn, min_version: '1.3.0' }).problem).toContain('Lights 1.2.0 is older than 1.3.0');
    expect(resolvePluginCondition(conditionListing, { ...isOn, condition: 'is_off' }).problem).toBe('Lights 1.2.0 has no condition is_off.');
    expect(resolvePluginCondition(conditionListing, { ...isOn, plugin: 'com.example.hall' }).problem).toContain('Hall is not enabled');
    // An action id is not a condition.
    expect(resolvePluginCondition(conditionListing, { ...isOn, condition: 'toggle' }).problem).toContain('has no condition toggle');
  });
});

describe('plugin conditions in the Execute confirmation', () => {
  it('names each check with its parameters and the variables that feed it, and is high risk', () => {
    const capabilities = profileCapabilities(flat(ifOn(), ifOn({ plugin: { ...isOn, min_version: '1.3.0', value: false } }), { type: 'set_variable', name: 'room', value: 'hall' }));
    expect(capabilities.risk).toBe('high');
    expect(capabilities.riskReasons).toContain('A key asks a plugin to check something before an if / else chooses what to do. A plugin is a program on this Mac that runs as you.');
    // The answer it holds for and the minimum version are named, so the two checks read as two lines.
    const fed = ', filling in the current value of room when pressed (room starts at “desk”, keys set it to “hall”; other programs on this Mac can change it)';
    expect(capabilities.pluginChecks).toEqual([
      `com.example.lights: check is_on (holds; needs 1.2.0 or later) with {"entity":"light.{{room}}"}${fed}`,
      `com.example.lights: check is_on (does not hold; needs 1.3.0 or later) with {"entity":"light.{{room}}"}${fed}`,
    ]);
    expect(capabilities.plugins).toEqual([]);
    expect(capabilityLines(capabilities).find(line => line.startsWith('Asks plugins to check something'))).toContain('com.example.lights: check is_on');
  });

  it('gives a changed check a new line, so a Live run whose saved profile changes one asks again', () => {
    const before = capabilityLines(profileCapabilities(flat(ifOn())));
    const after = capabilityLines(profileCapabilities(flat(ifOn({ plugin: { ...isOn, params: { entity: 'light.hall' }, variables: undefined } }))));
    expect(after.filter(line => !before.includes(line))).toHaveLength(1);
    const added = capabilityLines(profileCapabilities(flat(ifOn({ all: [{ plugin: isOn }, { plugin: { ...isOn, condition: 'reachable', params: undefined, variables: undefined } }] }))));
    expect(added.find(line => line.startsWith('Asks plugins to check'))).toContain('com.example.lights: check reachable');
    // A flipped answer or a raised minimum version (both of which the service's re-confirmation pauses on) is a new line too.
    for (const changed of [{ ...isOn, value: false }, { ...isOn, min_version: '1.3.0' }]) {
      const lines = capabilityLines(profileCapabilities(flat(ifOn({ plugin: changed }))));
      expect(lines.filter(line => !before.includes(line))).toHaveLength(1);
    }
  });
});

describe('plugin conditions and variables', () => {
  it('counts each reference in a condition\'s parameters as a use and renames it', () => {
    const profile = flat(ifOn());
    expect(actionVariableNames(profile.bindings[0].action)).toEqual(['room']);
    expect(variableUseCounts(profile).get('room')).toBe(1);
    const renamed = renameVariable(profile, 'room', 'area');
    expect(renamed.bindings[0].action).toEqual(ifOn({ plugin: { ...isOn, params: { entity: 'light.{{area}}' } } }));
  });
});

describe('key faces never ask a plugin', () => {
  it('refuses a plugin condition on a face, which takes variable tests only', () => {
    expect(keyFaceIssue({ when: { plugin: isOn }, label: 'On' }, [{ name: 'room', value: 'desk' }])).toBe('A key face can only test variables.');
  });
});

describe('starters and preset packs never use a plugin condition', () => {
  const terminal = starters.find(item => item.id === 'terminal-agents')!;
  const withCondition = (): Profile => {
    const profile = structuredClone(terminal.profile);
    const slot = profile.keypad!.pages[0].slots.find(item => item?.type === 'action');
    if (slot?.type === 'action') slot.bindings[0].action = { type: 'conditional', when: { plugin: isOn }, then: [{ type: 'text', text: 'yes' }] };
    return profile;
  };

  it('refuses one in a starter even where the action check allows an if / else', () => {
    expect(typingActionIssue({ type: 'conditional', when: { plugin: isOn }, then: [{ type: 'text', text: 'yes' }] })).toBeNull();
    expect(starterIssue(withCondition(), typingActionIssue)).toBe('A starter cannot use a plugin (an if / else tests com.example.lights).');
  });

  it('refuses one in a pack', () => {
    expect(starterIssue(withCondition(), packActionIssue)).not.toBeNull();
  });

  it('checks a pack\'s dial functions too, which are saved with its profile', () => {
    const pack = bundledPacks.packs.find(item => item.id === 'zoom')!;
    expect(starterIssue(pack.profile, packActionIssue)).toBeNull();
    const withDial = (action: Action): Profile => ({ ...structuredClone(pack.profile), dial_functions: [{ name: 'Lights', bindings: [{ gesture: 'right', action }] }] });
    expect(starterIssue(withDial({ type: 'toggle_mute' }), packActionIssue)).toBeNull();
    expect(starterIssue(withDial({ type: 'conditional', when: { plugin: isOn }, then: [{ type: 'toggle_mute' }] }), typingActionIssue)).toBe('A starter cannot use a plugin (an if / else tests com.example.lights).');
    expect(starterIssue(withDial({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.0.0', action: 'toggle' }), packActionIssue)).not.toBeNull();
    expect(starterIssue(withDial({ type: 'text', text: 'rm -rf ~' }), packActionIssue)).not.toBeNull();
    expect(starterIssue(withDial({ type: 'set_variable', name: 'missing', value: true }), typingActionIssue)).toBe('Variable missing is not declared.');
  });
});
