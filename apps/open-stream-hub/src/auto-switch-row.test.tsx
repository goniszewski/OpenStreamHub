import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { AutoSwitchRow } from './AutoSwitchRow';
import { FocusRulesStore, appChoices, appName, rulesForProfile } from './focus-rules';
import type { FocusRule, RunningApp } from './contracts';

const rules: FocusRule[] = [
  { id: 'r1', app_bundle_id: 'com.apple.Safari', profile_id: 'p1' },
  { id: 'r2', app_bundle_id: 'com.example.Zed', profile_id: 'p2' },
];
const apps: RunningApp[] = [{ bundle_id: 'com.apple.Safari', name: 'Safari' }, { bundle_id: 'com.apple.Notes', name: 'Notes' }, { bundle_id: 'com.example.Zed', name: 'Zed' }];

describe('auto-switch helpers', () => {
  it('finds the rules of one profile, and none for an unsaved one', () => {
    expect(rulesForProfile(rules, 'p1').map(rule => rule.id)).toEqual(['r1']);
    expect(rulesForProfile(rules, null)).toEqual([]);
  });
  it('offers only running apps that have no rule, ignoring bundle id case', () => {
    expect(appChoices(apps, rules).map(app => app.bundle_id)).toEqual(['com.apple.Notes']);
    expect(appChoices(apps, [{ id: 'x', app_bundle_id: 'COM.APPLE.NOTES', profile_id: 'p3' }]).map(app => app.name)).toEqual(['Safari', 'Zed']);
  });
  it('names an app that is running and falls back to its bundle id', () => {
    expect(appName('com.apple.safari', apps)).toBe('Safari');
    expect(appName('com.gone.App', apps)).toBe('com.gone.App');
  });
});

describe('FocusRulesStore.runningApps', () => {
  const store = (invoke: (command: string) => Promise<unknown>, desktop = true) => new FocusRulesStore(invoke as never, (async () => () => {}) as never, desktop);
  it('returns the apps the backend lists, drops malformed entries, and never throws', async () => {
    expect(await store(async () => [{ bundle_id: 'a.b', name: 'AB' }, { bundle_id: 1 }, null]).runningApps()).toEqual([{ bundle_id: 'a.b', name: 'AB' }]);
    expect(await store(async () => 'nope').runningApps()).toEqual([]);
    expect(await store(async () => { throw new Error('no'); }).runningApps()).toEqual([]);
    expect(await store(async () => [{ bundle_id: 'a.b', name: 'AB' }], false).runningApps()).toEqual([]);
  });
});

describe('AutoSwitchRow', () => {
  const render = (props: Partial<Parameters<typeof AutoSwitchRow>[0]> = {}) => renderToStaticMarkup(<AutoSwitchRow profileId="p1" profileName="Everyday" {...props} />);

  it('asks to save an unsaved profile first and offers no picker', () => {
    const markup = render({ profileId: null });
    expect(markup).toContain('Switch automatically when an app is in front');
    expect(markup).toContain('Save this profile first');
    expect(markup).not.toContain('id="auto-switch-app"');
  });
  it('shows the picker for a saved profile, disabled without the desktop app', () => {
    const markup = render();
    expect(markup).toContain('aria-label="Switch automatically to Everyday"');
    expect(markup).toContain('id="auto-switch-app"');
    expect(markup).toContain('Choose an app…');
    expect(markup).toContain('Another app (type its bundle ID)…');
    expect(markup).toMatch(/<select[^>]*id="auto-switch-app"[^>]*disabled/);
  });
});
