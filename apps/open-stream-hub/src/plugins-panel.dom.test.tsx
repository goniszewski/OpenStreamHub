// @vitest-environment jsdom
import { act } from 'react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { PluginsPanel } from './PluginsPanel';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import type { PluginListing, PluginManifest } from './plugins-model';
import type { Invoke } from './profile-session';
import { lights } from './plugins-test-data';

let view: Rendered | null = null;
afterEach(() => { view?.unmount(); view = null; });

// jsdom has <dialog> without showModal.
beforeAll(() => {
  HTMLDialogElement.prototype.showModal ??= function (this: HTMLDialogElement) { this.open = true; };
  HTMLDialogElement.prototype.close ??= function (this: HTMLDialogElement) { this.open = false; };
});

const hall: PluginManifest = { ...lights, id: 'com.example.hall', name: 'Hall', command: ['/usr/bin/python3', 'plugin.py', '--quiet'],
  actions: [{ ...lights.actions[0], events: true }, ...lights.actions.slice(1)],
  permissions: { network: ['homeassistant.local:8123'], variables: ['room'], programs: true } };
const desk: PluginManifest = { ...lights, id: 'com.example.desk', name: 'Desk', actions: [lights.actions[0]] };

const listing: PluginListing = {
  folder: '/Users/me/plugins',
  plugins: [
    { folder: 'com.example.desk', manifest: { ...desk, actions: lights.actions }, fingerprint: 'd1', approval: 'changed',
      secrets: [{ id: 'token', label: 'Token', set: true }, { id: 'pin', label: 'PIN', set: null }] },
    { folder: 'com.example.hall', manifest: hall, fingerprint: 'h1', approval: 'not_enabled', secrets: [{ id: 'token', label: 'Token', set: false }] },
    { folder: 'com.example.lights', manifest: lights, fingerprint: 'l1', approval: 'enabled', secrets: [{ id: 'token', label: 'Token', set: true }] },
  ],
};

function fakeInvoke(refuse: Record<string, string> = {}) {
  const calls: [string, unknown][] = [];
  const invoke = (async (command: string, args?: unknown) => {
    calls.push([command, args]);
    if (refuse[command]) throw refuse[command];
    if (command === 'list_plugins') return listing;
    return undefined;
  }) as Invoke;
  return { invoke, calls };
}

const settle = () => act(async () => { for (let index = 0; index < 4; index += 1) await Promise.resolve(); });

const item = (view: Rendered, name: string) => view.$$('.plugin-item').find(element => element.querySelector('strong')?.textContent?.startsWith(name))!;
const button = (root: Element, text: string) => {
  const found = [...root.querySelectorAll<HTMLButtonElement>('button')].find(element => element.textContent === text);
  if (!found) throw new Error(`no button ${text}`);
  return found;
};
const commands = (calls: [string, unknown][]) => calls.map(([command]) => command);

describe('Settings > Plugins in the DOM', () => {
  it('groups plugins into Enabled (open) and Disabled (closed), and shows ids, paths and commands as code', async () => {
    const { invoke } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    const enabled = view.$<HTMLDetailsElement>('[data-group="enabled"]');
    const disabled = view.$<HTMLDetailsElement>('[data-group="disabled"]');
    expect(enabled.open).toBe(true);
    expect(disabled.open).toBe(false);
    expect(enabled.querySelector('summary')!.textContent).toBe('Enabled (1)');
    expect(disabled.querySelector('summary')!.textContent).toBe('Disabled (2)1 to review');
    expect([...enabled.querySelectorAll('.plugin-title strong')].map(name => name.textContent)).toEqual(['Lights']);
    expect([...disabled.querySelectorAll('.plugin-title strong')].map(name => name.textContent)).toEqual(['Desk', 'Hall']);
    const hall = item(view, 'Hall');
    const codes = [...hall.querySelectorAll('code')].map(code => code.textContent);
    expect(codes).toEqual(['com.example.hall', '/usr/bin/python3 plugin.py --quiet', 'homeassistant.local:8123', 'room']);
    expect(view.$('.plugins-panel > .fine-print code').textContent).toBe('plugin.json');
  });

  it('offers Enable…, Review… or Disable by approval', async () => {
    const { invoke, calls } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    expect(item(view, 'Hall').textContent).toContain('Not enabled. It does not run until you review');
    expect(button(item(view, 'Hall'), 'Enable…')).toBeTruthy();
    expect(item(view, 'Desk').textContent).toContain('plugin.json changed since you enabled it');
    expect(button(item(view, 'Desk'), 'Review…')).toBeTruthy();
    expect(item(view, 'Lights').textContent).toContain('Enabled.');
    view.click(button(item(view, 'Lights'), 'Disable'));
    await settle();
    expect(calls).toContainEqual(['disable_plugin', { id: 'com.example.lights' }]);
    expect(commands(calls).at(-1)).toBe('list_plugins');
  });

  it('lists what the manifest declares in the first-enable dialog and enables the fingerprint it was opened with', async () => {
    const { invoke, calls } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    expect(view.container.querySelector('dialog')).toBeNull();
    view.click(button(item(view, 'Hall'), 'Enable…'));
    const dialog = view.$('dialog');
    const text = dialog.textContent ?? '';
    expect(view.$('#plugin-enable-title').textContent).toBe('Enable Hall?');
    expect(view.$('#plugin-enable-warning').textContent).toContain('A plugin runs as you and can do anything you can');
    expect(view.$('#plugin-enable-warning').textContent).toContain('not limits, except the variables it may set');
    for (const expected of [
      'Id: com.example.hall', 'Version: 1.2.0', 'Folder: /Users/me/plugins/com.example.hall',
      'Starts the program /usr/bin/python3 with the arguments "plugin.py" "--quiet"',
      'Toggle', 'Set a light', 'Says it connects to homeassistant.local:8123.', 'Sets the variables room.', 'Starts other programs.',
      'Secrets it uses: Token.',
      // Each action's parameters, with their types and marks.
      'Light: text, takes variables, required', 'Level: number', 'Fade: on or off', 'Mode: choice', 'Token: text, secret',
    ]) expect(text).toContain(expected);
    expect(text).toContain('Toggle (no parameters)');
    // Only the action that takes events says it hears the other input.
    const events = view.$$('.plugin-enable-events');
    expect(events).toHaveLength(1);
    expect(events[0].closest('li')?.textContent).toContain('Toggle');
    expect(events[0].textContent).toContain('hears the other input on keys and controls bound to this action');
    // Hall's secret is not set, so none is named as one that will be sent.
    expect(text).not.toContain('Stored secrets');
    view.click(button(dialog, 'Enable'));
    await settle();
    expect(calls).toContainEqual(['enable_plugin', { id: 'com.example.hall', fingerprint: 'h1' }]);
    expect(commands(calls).at(-1)).toBe('list_plugins');
    expect(view.container.querySelector('dialog')).toBeNull();
  });

  it('names the stored secrets a changed plugin will be sent, and closes on a refused enable with the reason', async () => {
    const { invoke, calls } = fakeInvoke({ enable_plugin: 'plugin.json changed since you reviewed it; review it again' });
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    view.click(button(item(view, 'Desk'), 'Review…'));
    expect(view.$('dialog').textContent).toContain('Stored secrets that will be sent to it once you enable it: Token.');
    view.click(button(view.$('dialog'), 'Enable'));
    await settle();
    // What the dialog showed may be stale, so it closes; the listing is read again and the reason stays.
    expect(view.container.querySelector('dialog')).toBeNull();
    expect(view.$('.plugin-enable-error').textContent).toBe('Desk was not enabled: plugin.json changed since you reviewed it; review it again');
    expect(commands(calls).at(-1)).toBe('list_plugins');
  });

  it('says when the Keychain could not tell whether a secret is set', async () => {
    const { invoke } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    const pin = item(view, 'Desk').querySelector('[data-secret="pin"]')!;
    expect(pin.textContent).toContain('Could not check the Keychain');
    expect(button(pin, 'Set…')).toBeTruthy();
    expect(button(pin, 'Forget')).toBeTruthy();
  });

  it('sets, changes and forgets a secret without echoing it', async () => {
    const { invoke, calls } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    const hallSecret = item(view, 'Hall').querySelector('[data-secret="token"]')!;
    expect(hallSecret.textContent).toContain('Not set');
    expect(() => button(hallSecret, 'Forget')).toThrow();
    view.click(button(hallSecret, 'Set…'));
    const input = hallSecret.querySelector<HTMLInputElement>('input')!;
    expect(input.type).toBe('password');
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')!.set!.call(input, 's3cret');
      input.dispatchEvent(new Event('input', { bubbles: true }));
    });
    view.click(button(hallSecret, 'Save in the Keychain'));
    await settle();
    expect(calls).toContainEqual(['set_plugin_secret', { plugin: 'com.example.hall', param: 'token', value: 's3cret' }]);
    expect(hallSecret.querySelector('input')).toBeNull();
    expect(view.container.textContent).not.toContain('s3cret');

    const lightsSecret = item(view, 'Lights').querySelector('[data-secret="token"]')!;
    expect(lightsSecret.textContent).toContain('Set');
    expect(button(lightsSecret, 'Change…')).toBeTruthy();
    view.click(button(lightsSecret, 'Forget'));
    await settle();
    expect(calls).toContainEqual(['forget_plugin_secret', { plugin: 'com.example.lights', param: 'token' }]);
  });
});
