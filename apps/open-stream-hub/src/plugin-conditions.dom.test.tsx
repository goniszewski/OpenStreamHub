// @vitest-environment jsdom
import { act, useState } from 'react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { ActionFields } from './ActionFields';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { BindingAction, ConditionalAction, PluginCondition } from './contracts';
import { PluginEnableDialog } from './PluginEnableDialog';
import { PluginsContext } from './plugins-context';
import type { PluginListing } from './plugins-model';
import type { Invoke } from './profile-session';
import { VariablesContext } from './variables-context';
import { checkingLights, conditionListing, meetings } from './plugins-test-data';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };
});

function Harness({ initial, seen, plugins = conditionListing, invoke }: { initial: BindingAction; seen: BindingAction[]; plugins?: PluginListing | null; invoke?: Invoke }) {
  const [action, setAction] = useState<BindingAction | null>(initial);
  return <VariablesContext.Provider value={[{ name: 'room', value: 'desk' }]}>
    <PluginsContext.Provider value={{ listing: plugins, error: null, refresh: () => {} }}>
      <ActionFields action={action} fader={false} selectorKey="m4315:root:0:pressed:" invoke={invoke} onChange={next => { if (next) seen.push(next); setAction(next); }} />
    </PluginsContext.Provider>
  </VariablesContext.Provider>;
}

function type(input: HTMLInputElement, value: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, value);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  });
}

const choose = (view: Rendered, select: string, value: string) => {
  view.click(view.$(`#${select}`));
  view.click(view.$(`#${select}-list [data-value="${value}"]`));
};
const options = (view: Rendered, select: string) => {
  view.click(view.$(`#${select}`));
  const found = view.$$(`#${select}-list [data-value]`).map(item => [item.getAttribute('data-value'), item.textContent, item.getAttribute('aria-disabled') === 'true']);
  view.click(view.$(`#${select}`));
  return found;
};

const muted: ConditionalAction = { type: 'conditional', when: { output_muted: true }, then: [{ type: 'toggle_mute' }] };
const whenOf = (action: BindingAction | undefined) => (action as ConditionalAction).when;
const isOn: PluginCondition = { plugin: 'com.example.lights', min_version: '1.2.0', condition: 'is_on', params: { entity: 'light.desk' }, value: true };

describe('plugin conditions in the if / else editor', () => {
  it('lists each enabled plugin\'s conditions under its name after the built-in tests', () => {
    view = render(<Harness initial={muted} seen={[]} />);
    const listed = options(view, 'conditional-when');
    const lights = listed.findIndex(([value]) => value === 'heading:com.example.lights');
    expect(listed.slice(lights)).toEqual([
      ['heading:com.example.lights', 'Lights', true],
      ['plugin:com.example.lights:is_on', 'Light is on', false],
      ['plugin:com.example.lights:reachable', 'Hub is reachable', false],
      ['heading:com.example.meetings', 'Meetings', true],
      ['plugin:com.example.meetings:in_call', 'A call is running', false],
    ]);
    // Built-in tests come first; a plugin that is not enabled offers nothing.
    expect(listed[0][0]).toBe('output_muted');
    expect(listed.some(([value]) => String(value).includes('com.example.hall'))).toBe(false);
  });

  it('shows the parameters with the plugin action form and a Holds / Does not hold choice', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={muted} seen={seen} />);
    choose(view, 'conditional-when', 'plugin:com.example.lights:is_on');
    expect(whenOf(seen.at(-1))).toEqual({ plugin: { plugin: 'com.example.lights', min_version: '1.2.0', condition: 'is_on', value: true } });
    expect(view.$('#conditional-plugin-param-entity-error').textContent).toBe('Light is required.');
    type(view.$<HTMLInputElement>('#conditional-plugin-param-entity'), 'light.{{room}}');
    expect(whenOf(seen.at(-1))).toEqual({ plugin: { plugin: 'com.example.lights', min_version: '1.2.0', condition: 'is_on', params: { entity: 'light.{{room}}' }, variables: ['entity'], value: true } });
    type(view.$<HTMLInputElement>('#conditional-plugin-param-entity'), 'light.{{nowhere}}');
    expect(view.$('#conditional-plugin-param-entity-error').textContent).toBe('Variable nowhere is not declared in this profile.');
    // A secret has no input, only whether it is set.
    expect(view.$('#conditional-plugin-param-token').tagName).toBe('P');
    expect(view.$('#conditional-plugin-param-token').textContent).toContain('It is set.');
    choose(view, 'conditional-holds', 'false');
    expect(whenOf(seen.at(-1))).toMatchObject({ plugin: { condition: 'is_on', value: false } });
    expect(view.container.textContent).toContain('Asks Lights 1.2.0 when the key is pressed');
    // Back to a built-in test.
    choose(view, 'conditional-when', 'timer_running');
    expect(whenOf(seen.at(-1))).toEqual({ timer_running: true });
  });

  it('works inside AND / OR groups', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ ...muted, when: { all: [{ output_muted: true }, { plugin: isOn }] } }} seen={seen} />);
    expect(view.$<HTMLInputElement>('#conditional-plugin-param-entity-2').value).toBe('light.desk');
    choose(view, 'conditional-when-2', 'plugin:com.example.meetings:in_call');
    expect(whenOf(seen.at(-1))).toEqual({ all: [{ output_muted: true }, { plugin: { plugin: 'com.example.meetings', min_version: '2.0.0', condition: 'in_call', value: true } }] });
  });

  it('keeps a missing plugin, an older version, an unknown condition or a disabled plugin and says so', () => {
    // The picker names the test it still holds: as offered when the plugin offers it, or as not available.
    const cases: [PluginCondition, string, string][] = [
      [{ ...isOn, plugin: 'com.example.gone' }, 'com.example.gone is not installed', 'com.example.gone: is_on (not available)'],
      [{ ...isOn, min_version: '9.0.0' }, 'Lights 1.2.0 is older than 9.0.0', 'Light is on'],
      [{ ...isOn, condition: 'is_off' }, 'Lights 1.2.0 has no condition is_off.', 'com.example.lights: is_off (not available)'],
      [{ ...isOn, plugin: 'com.example.hall' }, 'Hall is not enabled', 'com.example.hall: is_on (not available)'],
    ];
    for (const [check, problem, shown] of cases) {
      const seen: BindingAction[] = [];
      view = render(<Harness initial={{ ...muted, when: { plugin: check } }} seen={seen} />);
      expect(view.$('#conditional-plugin-problem').textContent).toContain(problem);
      expect(view.$('#conditional-when').textContent).toBe(shown);
      expect(seen).toEqual([]);
      view.unmount(); view = null;
    }
  });

  it('moves a secret the profile still holds to the Keychain', async () => {
    const calls: [string, unknown][] = [];
    const invoke = (async (command: string, args?: unknown) => { calls.push([command, args]); }) as Invoke;
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ ...muted, when: { plugin: { ...isOn, params: { entity: 'light.desk', token: 'old-secret' } } } }} seen={seen} invoke={invoke}
      plugins={{ ...conditionListing, plugins: conditionListing.plugins.map(plugin => plugin.folder === 'com.example.lights' ? { ...plugin, secrets: [{ id: 'token', label: 'Token', set: false }] } : plugin) }} />);
    expect(view.container.textContent).toContain('This profile still holds a value for Token');
    view.click(view.$$<HTMLButtonElement>('button').find(button => button.textContent === 'Move to the Keychain')!);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(calls).toEqual([['set_plugin_secret', { plugin: 'com.example.lights', param: 'token', value: 'old-secret' }]]);
    expect(whenOf(seen.at(-1))).toEqual({ plugin: isOn });
  });
});

describe('the first-enable dialog with conditions', () => {
  it('lists the conditions with their parameters under the actions and says when the plugin is asked', () => {
    view = render(<PluginEnableDialog open manifest={checkingLights} folder="/plugins/com.example.lights" onCancel={() => {}} onEnable={() => {}} />);
    const text = view.$('dialog').textContent ?? '';
    expect(text.indexOf('Actions')).toBeGreaterThan(-1);
    expect(text.indexOf('Conditions')).toBeGreaterThan(text.indexOf('Actions'));
    expect(text).toContain('The plugin is asked each time an if / else using one runs');
    const conditions = view.$$('.plugin-enable-conditions > li').map(item => item.textContent);
    expect(conditions).toEqual(['Light is onLight: text, takes variables, requiredToken: text, secret', 'Hub is reachable (no parameters)']);
    expect(text).toContain('Secrets it uses: Token.');
  });

  it('shows a plugin that has only conditions without an empty actions list', () => {
    view = render(<PluginEnableDialog open manifest={meetings} folder="/plugins/com.example.meetings" onCancel={() => {}} onEnable={() => {}} />);
    expect(view.container.querySelector('.plugin-enable-actions')).toBeNull();
    expect(view.$$('.plugin-enable-conditions > li').map(item => item.textContent)).toEqual(['A call is running (no parameters)']);
  });
});
