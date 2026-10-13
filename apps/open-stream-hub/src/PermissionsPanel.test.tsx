import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { PermissionsPanel, missingPermissions, permissionLabels, permissionState } from './PermissionsPanel';
import { PermissionsPrompt } from './PermissionsPrompt';
import { StartupPanel } from './StartupPanel';
import { UpdatePrompt, UpdatesPanel, updateResultText } from './UpdatesPanel';
import { dayMs, markUpdateChecked, markUpdateSeen, permissionCheckStorageKey, readPermissionCheckSetting, readUpdateCheckSetting, updateAlreadySeen, updateCheckDue, writePermissionCheckSetting, writeUpdateCheckSetting } from './ui-model';

describe('permissions panel', () => {
  it('reports what macOS says and never claims a state for Automation', () => {
    expect(permissionState('post_events', { post_events: true, listen_events: false })).toBe('granted');
    expect(permissionState('listen_events', { post_events: true, listen_events: false })).toBe('missing');
    expect(permissionState('post_events', null)).toBe('unknown');
    expect(permissionState('automation', { post_events: false, listen_events: false })).toBe('ask');
    expect(permissionLabels.ask).toBe('Asked when used');
  });

  it('lists the three permissions with their buttons on the desktop', () => {
    const markup = renderToStaticMarkup(<PermissionsPanel desktop invoke={(async () => undefined) as never} />);
    for (const title of ['Accessibility', 'Input Monitoring', 'Automation']) expect(markup).toContain(title);
    expect(markup.match(/>Open</g)).toHaveLength(3);
    expect(markup).toContain('Check again');
  });

  it('says the check needs the desktop app in the browser preview', () => {
    const markup = renderToStaticMarkup(<PermissionsPanel desktop={false} />);
    expect(markup).toContain('desktop app only');
    expect(markup).not.toContain('Check again');
    expect(markup).not.toContain('permission-list');
  });

  it('lists only what macOS reports as off, and never Automation', () => {
    expect(missingPermissions(null)).toEqual([]);
    expect(missingPermissions({ post_events: true, listen_events: true })).toEqual([]);
    expect(missingPermissions({ post_events: false, listen_events: true })).toEqual(['post_events']);
    expect(missingPermissions({ post_events: false, listen_events: false })).toEqual(['post_events', 'listen_events']);
  });
});

describe('startup panel and permission prompt', () => {
  it('checks permissions on launch unless it was turned off', () => {
    expect(readPermissionCheckSetting(null)).toBe(true);
    const store = new Map<string, string>();
    const storage = { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => { store.set(key, value); } };
    expect(readPermissionCheckSetting(storage)).toBe(true);
    writePermissionCheckSetting(storage, false);
    expect(store.get(permissionCheckStorageKey)).toBe('off');
    expect(readPermissionCheckSetting(storage)).toBe(false);
  });

  it('shows both startup options, with the login item waiting for its status', () => {
    const markup = renderToStaticMarkup(<StartupPanel desktop invoke={(async () => undefined) as never} checkPermissions onCheckPermissions={() => {}} />);
    expect(markup).toContain('Start OpenStreamHub when I log in');
    expect(markup).toContain('Check permissions when the app opens');
    expect(markup).toMatch(/Check permissions when the app opens/);
  });

  it('says the login item needs the desktop app in the browser preview', () => {
    const markup = renderToStaticMarkup(<StartupPanel desktop={false} checkPermissions onCheckPermissions={() => {}} />);
    expect(markup).toContain('Available in the desktop app');
  });

  it('opens as a short dialog with a way out', () => {
    const markup = renderToStaticMarkup(<PermissionsPrompt invoke={(async () => undefined) as never} onClose={() => {}} />);
    expect(markup).toContain('Allow OpenStreamHub in System Settings');
    expect(markup).toContain('Not now');
  });
});

describe('updates', () => {
  const memory = () => {
    const store = new Map<string, string>();
    return { getItem: (key: string) => store.get(key) ?? null, setItem: (key: string, value: string) => { store.set(key, value); } };
  };

  it('is off until turned on, and asks at most once a day', () => {
    const storage = memory();
    expect(readUpdateCheckSetting(null)).toBe(false);
    expect(readUpdateCheckSetting(storage)).toBe(false);
    writeUpdateCheckSetting(storage, true);
    expect(readUpdateCheckSetting(storage)).toBe(true);
    expect(updateCheckDue(storage, 1_000_000)).toBe(true);
    markUpdateChecked(storage, 1_000_000);
    expect(updateCheckDue(storage, 1_000_000 + dayMs - 1)).toBe(false);
    expect(updateCheckDue(storage, 1_000_000 + dayMs)).toBe(true);
    // A clock set back counts as due rather than waiting a year.
    expect(updateCheckDue(storage, 500_000)).toBe(true);
  });

  it('remembers the release it already told the user about', () => {
    const storage = memory();
    expect(updateAlreadySeen(storage, '0.1.1')).toBe(false);
    markUpdateSeen(storage, '0.1.1');
    expect(updateAlreadySeen(storage, '0.1.1')).toBe(true);
    expect(updateAlreadySeen(storage, '0.1.2')).toBe(false);
  });

  it('words the result and the prompt', () => {
    expect(updateResultText({ current: '0.1.0', latest: '0.1.1', available: true })).toBe('OpenStreamHub 0.1.1 is available. You have 0.1.0.');
    expect(updateResultText({ current: '0.1.0', latest: '0.1.0', available: false })).toBe('You are up to date (0.1.0).');
    const markup = renderToStaticMarkup(<UpdatePrompt info={{ current: '0.1.0', latest: '0.1.1', available: true }} onOpen={() => {}} onClose={() => {}} />);
    expect(markup).toContain('OpenStreamHub 0.1.1 is available');
    expect(markup).toContain('Later');
  });

  it('offers the check in the desktop app only', () => {
    const desktop = renderToStaticMarkup(<UpdatesPanel desktop invoke={(async () => undefined) as never} version="0.1.0" daily={false} onDaily={() => {}} />);
    expect(desktop).toContain('Check for updates');
    expect(desktop).toContain('Check once a day when the app opens');
    expect(desktop).toContain('You have version 0.1.0');
    const browser = renderToStaticMarkup(<UpdatesPanel desktop={false} version={null} daily={false} onDaily={() => {}} />);
    expect(browser).toContain('Available in the desktop app');
    expect(browser).not.toContain('Check for updates');
  });
});
