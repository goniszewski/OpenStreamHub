// @vitest-environment jsdom
import { act, useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import type { Invoke } from './profile-session';
import { ActionFields } from './ActionFields';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { BindingAction } from './contracts';
import { PluginsContext } from './plugins-context';
import type { PluginListing } from './plugins-model';
import { VariablesContext } from './variables-context';
import { lights, listing } from './plugins-test-data';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

function Harness({ initial, seen, plugins = listing, invoke }: { initial: BindingAction; seen: BindingAction[]; plugins?: PluginListing | null; invoke?: Invoke }) {
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

describe('plugin action fields in the DOM', () => {
  it('takes the first installed plugin for a fresh pick and generates the form from its manifest', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: '', min_version: '1.0.0', action: '' }} seen={seen} />);
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'toggle' });
    expect(view.container.querySelector('#plugin-param-entity')).toBeNull();

    choose(view, 'plugin-action', 'set');
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { level: 50, mode: 'warm' } });
    // The required light is missing until typed.
    expect(view.$('#plugin-param-entity-error').textContent).toBe('Light is required.');
    type(view.$<HTMLInputElement>('#plugin-param-entity'), 'light.{{room}}');
    expect(view.container.querySelector('#plugin-param-entity-error')).toBeNull();
    expect(seen.at(-1)).toMatchObject({ params: { entity: 'light.{{room}}' }, variables: ['entity'] });
    type(view.$<HTMLInputElement>('#plugin-param-level'), '140');
    expect(view.$('#plugin-param-level-error').textContent).toContain('between 0 and 100');
    view.click(view.$('#plugin-param-fade'));
    expect(seen.at(-1)).toMatchObject({ params: { fade: true } });
    // A secret has no input: it is in the Keychain.
    expect(view.$('#plugin-param-token').tagName).toBe('P');
    expect(view.$('#plugin-param-token').textContent).toContain('kept in the Keychain, never in the profile');
    expect(view.$('#plugin-param-token').textContent).toContain('It is set.');
    expect(view.container.querySelector('input[type="password"]')).toBeNull();
  });

  it('does not call a required secret missing, and moves a secret the profile still holds to the Keychain', async () => {
    const required: PluginListing = { ...listing, plugins: listing.plugins.map(plugin => plugin.manifest ? {
      ...plugin, secrets: [{ id: 'token', label: 'Token', set: false }],
      manifest: { ...lights, actions: lights.actions.map(action => ({ ...action, params: action.params?.map(param => param.id === 'token' ? { ...param, required: true } : param) })) },
    } : plugin) };
    const calls: [string, unknown][] = [];
    const invoke = (async (command: string, args?: unknown) => { calls.push([command, args]); }) as Invoke;
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a', mode: 'warm', token: 'old-secret' } }}
      seen={seen} plugins={required} invoke={invoke} />);
    expect(view.$('#plugin-param-token').textContent).toContain('It is not set yet.');
    expect(view.container.textContent).not.toContain('Token is required');
    expect(view.container.textContent).toContain('This profile still holds a value for Token');
    const move = view.$$<HTMLButtonElement>('button').find(button => button.textContent === 'Move to the Keychain')!;
    view.click(move);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(calls).toEqual([['set_plugin_secret', { plugin: 'com.example.lights', param: 'token', value: 'old-secret' }]]);
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a', mode: 'warm' } });
    expect(view.container.textContent).toContain('Moved to the Keychain. Save the profile');
    expect(view.container.textContent).not.toContain('This profile still holds');
  });

  it('takes a moved secret out of the draft at the click, so edits made while the Keychain answers are kept', async () => {
    const calls: [string, unknown][] = [];
    let store: { resolve(): void; reject(reason: unknown): void } | null = null;
    const invoke = ((command: string, args?: unknown) => {
      calls.push([command, args]);
      return new Promise<void>((resolve, reject) => { store = { resolve, reject }; });
    }) as Invoke;
    const seen: BindingAction[] = [];
    const required: PluginListing = { ...listing, plugins: listing.plugins.map(plugin => plugin.manifest ? { ...plugin, secrets: [{ id: 'token', label: 'Token', set: false }] } : plugin) };
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a', token: 'old' } }}
      seen={seen} plugins={required} invoke={invoke} />);
    view.click(view.$$<HTMLButtonElement>('button').find(button => button.textContent === 'Move to the Keychain')!);
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a' } });
    expect(view.container.textContent).toContain('storing it in the Keychain');
    // Edited while the Keychain has not answered yet (a prompt is open).
    type(view.$<HTMLInputElement>('#plugin-param-entity'), 'light.hall');
    const edited = seen.length;
    await act(async () => { store!.resolve(); await Promise.resolve(); await Promise.resolve(); });
    expect(seen.length).toBe(edited);
    expect(seen.at(-1)).toMatchObject({ params: { entity: 'light.hall' } });
    expect(view.container.textContent).toContain('Moved to the Keychain');
    expect(calls).toEqual([['set_plugin_secret', { plugin: 'com.example.lights', param: 'token', value: 'old' }]]);
  });

  it('says Undo brings a secret back when the Keychain refuses it', async () => {
    const invoke = (async () => { throw 'the Keychain is locked'; }) as Invoke;
    const seen: BindingAction[] = [];
    const required: PluginListing = { ...listing, plugins: listing.plugins.map(plugin => plugin.manifest ? { ...plugin, secrets: [{ id: 'token', label: 'Token', set: false }] } : plugin) };
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a', token: 'old' } }}
      seen={seen} plugins={required} invoke={invoke} />);
    view.click(view.$$<HTMLButtonElement>('button').find(button => button.textContent === 'Move to the Keychain')!);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(view.container.textContent).toContain('Could not store it in the Keychain: the Keychain is locked. It is no longer in this draft; Undo puts it back.');
    expect(view.container.textContent).not.toContain('Moved to the Keychain');
  });

  it('asks before replacing a value the Keychain already holds', async () => {
    const calls: [string, unknown][] = [];
    const invoke = (async (command: string, args?: unknown) => { calls.push([command, args]); }) as Invoke;
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a', token: 'old' } }}
      seen={seen} invoke={invoke} />);
    const named = (text: string) => view!.$$<HTMLButtonElement>('button').find(button => button.textContent === text);
    // The listing says Token is set: removing is the primary action and nothing offers a plain move.
    expect(named('Remove from the profile')!.className).toContain('primary');
    expect(named('Move to the Keychain')).toBeUndefined();
    view.click(named('Replace the stored value…')!);
    expect(view.container.textContent).toContain('The Keychain already holds a value for Token. Replace it');
    expect(calls).toEqual([]);
    view.click(named('Keep the stored value')!);
    expect(named('Replace')).toBeUndefined();
    view.click(named('Replace the stored value…')!);
    view.click(named('Replace')!);
    await act(async () => { await Promise.resolve(); await Promise.resolve(); });
    expect(calls).toEqual([['set_plugin_secret', { plugin: 'com.example.lights', param: 'token', value: 'old' }]]);
    expect(seen.at(-1)).toMatchObject({ params: { entity: 'a' } });
  });

  it('can drop a secret from the profile without moving it', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a', token: 'old' } }} seen={seen} />);
    expect(view.$$<HTMLButtonElement>('button').some(button => button.textContent === 'Move to the Keychain')).toBe(false);
    view.click(view.$$<HTMLButtonElement>('button').find(button => button.textContent === 'Remove from the profile')!);
    expect(seen.at(-1)).toEqual({ type: 'plugin', plugin: 'com.example.lights', min_version: '1.2.0', action: 'set', params: { entity: 'a' } });
    expect(view.container.textContent).toContain('Removed. Save the profile');
  });

  it('keeps a key whose plugin is missing and says why it will not run', () => {
    const seen: BindingAction[] = [];
    view = render(<Harness initial={{ type: 'plugin', plugin: 'com.example.gone', min_version: '1.0.0', action: 'x' }} seen={seen} />);
    expect(view.$('#plugin-problem').textContent).toContain('com.example.gone is not installed');
    expect(view.$('#plugin-id').textContent).toContain('com.example.gone (not installed)');
    expect(seen).toEqual([]);
  });

  it('says so when no plugin is installed', () => {
    view = render(<Harness initial={{ type: 'plugin', plugin: '', min_version: '1.0.0', action: '' }} seen={[]}
      plugins={{ folder: '/p', plugins: [] }} />);
    expect(view.container.textContent).toContain('No plugins are installed.');
    expect(view.container.textContent).toContain('Plugins folder: /p');
    expect(lights.actions.length).toBe(2);
  });
});
