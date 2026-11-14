// @vitest-environment jsdom
import { act } from 'react';
import { afterEach, beforeAll, describe, expect, it } from 'vitest';
import { createPluginManageStore } from './plugin-manage-store';
import { PluginsPanel } from './PluginsPanel';
import { render } from './dom-test';
import type { Rendered } from './dom-test';
import { formatSize } from './plugins-model';
import type { InstallReview, ListedPlugin, PluginListing, PluginManifest } from './plugins-model';
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

function fakeInvoke(refuse: Record<string, string> = {}, answers: Record<string, unknown> = {}) {
  const calls: [string, unknown][] = [];
  const invoke = (async (command: string, args?: unknown) => {
    calls.push([command, args]);
    if (refuse[command]) throw refuse[command];
    if (command === 'list_plugins') return answers.list_plugins ?? listing;
    if (command in answers) return answers[command];
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

  it('still enables from the first-enable dialog after the review body was shared with the install dialog', async () => {
    const { invoke, calls } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    view.click(button(item(view, 'Hall'), 'Enable…'));
    expect(view.$('dialog').getAttribute('aria-describedby')).toBe('plugin-enable-warning plugin-enable-intro');
    expect(view.container.querySelector('.plugin-install-facts')).toBeNull();
    view.click(button(view.$('dialog'), 'Enable'));
    await settle();
    expect(calls).toContainEqual(['enable_plugin', { id: 'com.example.hall', fingerprint: 'h1' }]);
  });
});

// A plugin chosen in Downloads under another name; it installs as its id.
const notify: PluginManifest = { ...lights, id: 'com.example.notify', name: 'Notify', version: '1.0.0', command: ['bin/notify', '--fast'] };
const fresh: InstallReview = {
  path: '/Users/me/Downloads/notify-main', files: 12, bytes: 3 * 1024 * 1024 + 300 * 1024, installed: null, older: false,
  plugin: { folder: 'com.example.notify', manifest: notify, fingerprint: 'n1', approval: 'not_enabled', secrets: [{ id: 'token', label: 'Token', set: false }] },
};
// approval is what plain Install leaves (review_plugin_install): enabled only when this exact plugin.json is approved.
const update = (installed: InstallReview['installed'], older = false, approval: ListedPlugin['approval'] = 'not_enabled'): InstallReview =>
  ({ ...fresh, installed, older, plugin: { ...fresh.plugin, approval, manifest: { ...notify, version: older ? '0.9.0' : '1.1.0' } } });

describe('Installing and removing plugins in the DOM', () => {
  const openInstall = async (answers: Record<string, unknown>, refuse: Record<string, string> = {}) => {
    const fake = fakeInvoke(refuse, answers);
    view = render(<PluginsPanel desktop invoke={fake.invoke} />);
    await settle();
    view.click(button(view.$('.settings-actions'), 'Install…'));
    await settle();
    return fake;
  };

  for (const [label, enable] of [['Install', false], ['Install and enable', true]] as const) {
    it(`reviews the chosen folder and installs it with ${label}`, async () => {
      const { calls } = await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: fresh });
      expect(calls).toContainEqual(['review_plugin_install', { path: '/Users/me/Downloads/notify-main' }]);
      const dialog = view!.$('dialog');
      expect(view!.$('#plugin-install-title').textContent).toBe('Install Notify?');
      expect(dialog.getAttribute('aria-describedby')).toBe('plugin-install-warning plugin-install-intro');
      expect(view!.$('#plugin-install-warning').textContent).toContain('A plugin runs as you and can do anything you can');
      const text = dialog.textContent ?? '';
      for (const expected of [
        'From: /Users/me/Downloads/notify-main', `12 files, ${formatSize(fresh.bytes)}`, 'Id: com.example.notify', 'Version: 1.0.0',
        'Folder: /Users/me/plugins/com.example.notify', 'Starts the program bin/notify with the arguments "--fast"',
        'Toggle', 'Set a light', 'Sets the variables room.', 'Secrets it uses: Token.',
      ]) expect(text).toContain(expected);
      expect(text).not.toContain('Replaces');
      view!.click(button(dialog, label));
      await settle();
      expect(calls).toContainEqual(['install_plugin', { path: '/Users/me/Downloads/notify-main', fingerprint: 'n1', enable }]);
      expect(commands(calls).at(-1)).toBe('list_plugins');
      expect(view!.container.querySelector('dialog')).toBeNull();
      expect(view!.container.querySelector('.plugin-manage-error')).toBeNull();
    });
  }

  it('does nothing when the folder picker is cancelled', async () => {
    const { calls } = await openInstall({ choose_plugin_folder: null });
    expect(commands(calls)).not.toContain('review_plugin_install');
    expect(view!.container.querySelector('dialog')).toBeNull();
    expect(view!.container.querySelector('.plugin-manage-error')).toBeNull();
  });

  it('shows a refused review in the panel', async () => {
    await openInstall({ choose_plugin_folder: '/Users/me/Downloads/stuff' }, { review_plugin_install: 'plugin.json was not found in /Users/me/Downloads/stuff' });
    expect(view!.container.querySelector('dialog')).toBeNull();
    const alert = view!.$('.plugin-manage-error');
    expect(alert.getAttribute('role')).toBe('alert');
    expect(alert.textContent).toBe('The plugin was not installed: plugin.json was not found in /Users/me/Downloads/stuff');
  });

  it('says what an update replaces, warns when it is older and when its approval ends', async () => {
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: '1.0.0', approval: 'enabled', same_manifest: false }, true) });
    const text = view!.$('dialog').textContent ?? '';
    expect(text).toContain('Replaces version 1.0.0, which is installed now.');
    expect(view!.$('.plugin-install-older').textContent).toBe('Version 0.9.0 is older than the installed 1.0.0.');
    expect(view!.$('.plugin-install-approval').textContent).toContain('so it needs enabling again');
  });

  it('says an identical plugin.json stays enabled, and names a replaced plugin whose manifest cannot be read', async () => {
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: '1.0.0', approval: 'enabled', same_manifest: true }, false, 'enabled') });
    expect(view!.container.querySelector('.plugin-install-older')).toBeNull();
    expect(view!.$('.plugin-install-approval').textContent).toBe('This exact plugin.json is enabled already, so it stays enabled.');
    view!.click(button(view!.$('dialog'), 'Cancel'));
    expect(view!.container.querySelector('dialog')).toBeNull();

    view!.unmount();
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: null, approval: null, same_manifest: false }) });
    expect(view!.$('.plugin-install-replaces').textContent).toBe('Replaces an installed plugin whose manifest cannot be read.');
    expect(view!.container.querySelector('.plugin-install-approval')).toBeNull();
  });

  it('closes the dialog with the reason when plugin.json changed since the review', async () => {
    const { calls } = await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: fresh },
      { install_plugin: 'plugin.json changed since you reviewed it; review it again' });
    view!.click(button(view!.$('dialog'), 'Install and enable'));
    await settle();
    expect(view!.container.querySelector('dialog')).toBeNull();
    expect(view!.$('.plugin-manage-error').textContent).toBe('The plugin was not installed: plugin.json changed since you reviewed it; review it again');
    expect(commands(calls).at(-1)).toBe('list_plugins');
  });

  it('removes a plugin after a confirmation that names it, and not when cancelled', async () => {
    const { invoke, calls } = fakeInvoke();
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    view.click(button(item(view, 'Lights'), 'Remove…'));
    expect(view.$('#plugin-remove-title').textContent).toBe('Remove Lights?');
    expect(view.$('dialog').getAttribute('role')).toBe('alertdialog');
    expect(view.$('#plugin-remove-message').textContent).toContain('keep their keys, which then fail with "not installed"');
    expect(view.$('#plugin-remove-message').textContent).toContain('its approval, the secrets its plugin.json declares in the Keychain, and its data');
    view.click(button(view.$('dialog'), 'Cancel'));
    expect(view.container.querySelector('dialog')).toBeNull();
    expect(commands(calls)).not.toContain('remove_plugin');

    view.click(button(item(view, 'Lights'), 'Remove…'));
    view.click(button(view.$('dialog'), 'Remove'));
    await settle();
    expect(calls).toContainEqual(['remove_plugin', { id: 'com.example.lights' }]);
    expect(commands(calls).at(-1)).toBe('list_plugins');
    expect(view.container.querySelector('dialog')).toBeNull();
  });

  it('removes a folder whose manifest cannot be used', async () => {
    const broken: PluginListing = { ...listing, plugins: [{ folder: 'notify-main', error: 'plugin.json is not a valid manifest' }, ...listing.plugins] };
    const { invoke, calls } = fakeInvoke({}, { list_plugins: broken });
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    const before = commands(calls).filter(command => command === 'list_plugins').length;
    view.click(button(item(view, 'notify-main'), 'Remove…'));
    view.click(button(view.$('dialog'), 'Remove'));
    await settle();
    expect(calls).toContainEqual(['remove_plugin', { id: 'notify-main' }]);
    expect(commands(calls).at(-1)).toBe('list_plugins');
    expect(commands(calls).filter(command => command === 'list_plugins').length).toBe(before + 1);
    expect(view.container.querySelector('dialog')).toBeNull();
    expect(view.container.querySelector('.plugin-manage-error')).toBeNull();
  });

  it('shows a refused remove of a folder whose manifest cannot be used', async () => {
    const broken: PluginListing = { ...listing, plugins: [{ folder: 'notify-main', error: 'plugin.json is not a valid manifest' }, ...listing.plugins] };
    const { invoke, calls } = fakeInvoke({ remove_plugin: 'the folder could not be deleted' }, { list_plugins: broken });
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    view!.click(button(item(view!, 'notify-main'), 'Remove…'));
    expect(view!.$('#plugin-remove-title').textContent).toBe('Remove notify-main?');
    view!.click(button(view!.$('dialog'), 'Remove'));
    await settle();
    expect(calls).toContainEqual(['remove_plugin', { id: 'notify-main' }]);
    expect(view!.$('.plugin-manage-error').textContent).toBe('notify-main was not removed: the folder could not be deleted');
  });

  it('takes whether it stays enabled from what plain Install leaves, not from the installed approval alone', async () => {
    // Installed and enabled, but with other bytes: plain Install leaves it not enabled.
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: '1.0.0', approval: 'enabled', same_manifest: false }) });
    expect(view!.$('.plugin-install-approval').textContent).toContain('so it needs enabling again');
    view!.unmount();
    // Installed and enabled with exactly this plugin.json: plain Install keeps it enabled.
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: '1.0.0', approval: 'enabled', same_manifest: true }, false, 'enabled') });
    expect(view!.$('.plugin-install-approval').textContent).toContain('so it stays enabled');
    view!.unmount();
    // Installed with a changed plugin.json and the same bytes chosen again: it still needs enabling.
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: '1.0.0', approval: 'changed', same_manifest: true }) });
    expect(view!.$('.plugin-install-approval').textContent).toContain('so it needs enabling again');
    view!.unmount();
    // Installed but never enabled: nothing to say about the approval.
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: update({ version: '1.0.0', approval: 'not_enabled', same_manifest: false }) });
    expect(view!.container.querySelector('.plugin-install-approval')).toBeNull();
  });

  it('installs and shows install errors when the plugin listing could not be read', async () => {
    const { calls } = await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: fresh },
      { list_plugins: 'permission denied', install_plugin: 'a symbolic link is not allowed: bin/notify' });
    expect(view!.container.textContent).toContain('The plugins folder could not be read: permission denied');
    expect(view!.$('#plugin-install-title').textContent).toBe('Install Notify?');
    expect(view!.$('dialog').textContent).toContain('Folder: com.example.notify');
    view!.click(button(view!.$('dialog'), 'Install'));
    await settle();
    expect(calls).toContainEqual(['install_plugin', { path: fresh.path, fingerprint: 'n1', enable: false }]);
    expect(view!.container.querySelector('dialog')).toBeNull();
    expect(view!.$('.plugin-manage-error').textContent).toBe('The plugin was not installed: a symbolic link is not allowed: bin/notify');
  });

  it('says a plugin was installed when only a later step failed, and not installed otherwise', async () => {
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: fresh },
      { install_plugin: 'installed: Notify could not be enabled: the Keychain is locked' });
    view!.click(button(view!.$('dialog'), 'Install and enable'));
    await settle();
    expect(view!.$('.plugin-manage-error').textContent).toBe('The plugin was installed, but: Notify could not be enabled: the Keychain is locked');
    view!.unmount();
    await openInstall({ choose_plugin_folder: fresh.path, review_plugin_install: fresh }, { install_plugin: 'plugin.json changed since you reviewed it; review it again' });
    view!.click(button(view!.$('dialog'), 'Install'));
    await settle();
    expect(view!.$('.plugin-manage-error').textContent).toBe('The plugin was not installed: plugin.json changed since you reviewed it; review it again');
  });

  it('keeps the install dialog open while installing, so closing it cannot hide an install still running', async () => {
    let finish!: () => void;
    const fake = fakeInvoke({}, { choose_plugin_folder: fresh.path, review_plugin_install: fresh });
    const invoke = (async (command: string, args?: unknown) => command === 'install_plugin'
      ? new Promise<void>(resolve => { finish = resolve; }) : fake.invoke(command, args as Record<string, unknown> | undefined)) as typeof fake.invoke;
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    view.click(button(view.$('.settings-actions'), 'Install…'));
    await settle();
    view.click(button(view.$('dialog'), 'Install'));
    await settle();
    expect(button(view.$('dialog'), 'Cancel').disabled).toBe(true);
    expect(button(view.$('.settings-actions'), 'Install…').disabled).toBe(true);
    view.$('dialog').dispatchEvent(new Event('cancel', { cancelable: true }));
    await settle();
    expect(view.container.querySelector('dialog')).not.toBeNull();
    finish();
    await settle();
    expect(view.container.querySelector('dialog')).toBeNull();
    expect(button(view.$('.settings-actions'), 'Install…').disabled).toBe(false);
  });

  it('keeps a folder picked after leaving the page, and opens no second picker meanwhile', async () => {
    let pick!: (path: string) => void;
    const fake = fakeInvoke({}, { review_plugin_install: fresh });
    const invoke = (async (command: string, args?: unknown) => command === 'choose_plugin_folder'
      ? new Promise<string>(resolve => { pick = resolve; }) : fake.invoke(command, args as Record<string, unknown> | undefined)) as typeof fake.invoke;
    const store = createPluginManageStore();
    view = render(<PluginsPanel desktop invoke={invoke} manageStore={store} />);
    await settle();
    view.click(button(view.$('.settings-actions'), 'Install…'));
    await settle();
    // The picker does not block the app: the user goes to another page and back while it is open.
    view.unmount();
    view = render(<PluginsPanel desktop invoke={invoke} manageStore={store} />);
    await settle();
    expect(button(view.$('.settings-actions'), 'Install…').disabled).toBe(true);
    view.unmount();
    pick(fresh.path);
    await settle();
    view = render(<PluginsPanel desktop invoke={invoke} manageStore={store} />);
    await settle();
    expect(view.$('#plugin-install-title').textContent).toBe('Install Notify?');
    expect(fake.calls.filter(([command]) => command === 'choose_plugin_folder')).toHaveLength(0);
    expect(fake.calls).toContainEqual(['review_plugin_install', { path: fresh.path }]);
    view.click(button(view.$('dialog'), 'Cancel'));
    await settle();
    expect(view.container.querySelector('dialog')).toBeNull();
    expect(button(view.$('.settings-actions'), 'Install…').disabled).toBe(false);
  });

  it('says a plugin was removed when only a later step failed', async () => {
    const { invoke, calls } = fakeInvoke({ remove_plugin: 'removed: the Keychain kept the secret token' });
    view = render(<PluginsPanel desktop invoke={invoke} />);
    await settle();
    view.click(button(item(view, 'Lights'), 'Remove…'));
    view.click(button(view.$('dialog'), 'Remove'));
    await settle();
    expect(view.$('.plugin-manage-error').textContent).toBe('Lights was removed, but: the Keychain kept the secret token');
    expect(commands(calls).at(-1)).toBe('list_plugins');
  });
});
