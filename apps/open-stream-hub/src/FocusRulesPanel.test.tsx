import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { ProfileSummary } from './contracts';
import { FocusRulesStore, maxFocusRules } from './focus-rules';
import type { FocusRulesState } from './focus-rules';
import { FocusRulesPanel, ruleFormProfile } from './FocusRulesPanel';

const profiles: ProfileSummary[] = [
  { id: '00000000-0000-4000-8000-00000000000a', name: 'Everyday', binding_count: 2, modified_at_ms: 0, dry_run_only: false },
  { id: '00000000-0000-4000-8000-00000000000b', name: 'Browser', binding_count: 1, modified_at_ms: 0, dry_run_only: false },
];

function storeWith(patch: Partial<FocusRulesState>) {
  const store = new FocusRulesStore(async () => ({ enabled: false, rules: [] }) as never, async () => () => {}, true);
  (store as unknown as { state: FocusRulesState }).state = { ...store.getState(), loaded: true, ...patch };
  return store;
}

describe('FocusRulesPanel markup', () => {
  it('is off by default with an empty, labelled form', () => {
    const markup = renderToStaticMarkup(<FocusRulesPanel store={storeWith({})} profiles={profiles} />);
    expect(markup).toContain('aria-labelledby="focus-rules-title"');
    expect(markup).toMatch(/<input type="checkbox"(?![^>]*checked)[^>]*>Switch profiles by focused app \(off by default\)/);
    expect(markup).toContain('No rules yet.');
    expect(markup).toContain('<label for="focus-rule-bundle-id">App bundle ID</label>');
    expect(markup).toContain('Use current app');
    expect(markup).toContain('<label for="focus-rule-profile">Profile</label>');
    expect(markup).toContain('>Everyday</option>');
    expect(markup).toContain('Add rule');
    expect(markup).not.toContain('aria-invalid="true"');
  });

  it('lists rules with accessible actions and flags deleted profiles', () => {
    const store = storeWith({ enabled: true, focusedApp: 'com.apple.Mail', rules: [
      { id: 'r1', app_bundle_id: 'com.apple.Safari', profile_id: profiles[1].id },
      { id: 'r2', app_bundle_id: 'com.apple.Terminal', profile_id: '00000000-0000-4000-8000-0000000000ff' },
    ] });
    const markup = renderToStaticMarkup(<FocusRulesPanel store={store} profiles={profiles} />);
    expect(markup).toMatch(/<input type="checkbox"[^>]*checked=""/);
    expect(markup).toContain('<code>com.apple.Safari</code></td><td>Browser</td>');
    expect(markup).toContain('aria-label="Edit rule for com.apple.Safari"');
    expect(markup).toContain('aria-label="Delete rule for com.apple.Terminal"');
    expect(markup).toContain('Missing profile (rule is skipped)');
    expect(markup).toContain('Last focused app: <code>com.apple.Mail</code>');
    expect(markup).toContain('2/64');
  });

  it('never silently retargets an edited rule whose profile is missing', () => {
    const missing = '00000000-0000-4000-8000-0000000000ff';
    expect(ruleFormProfile(profiles, missing, true)).toBe('');
    expect(ruleFormProfile(profiles, profiles[1].id, true)).toBe(profiles[1].id);
    // A new rule still defaults to the first profile.
    expect(ruleFormProfile(profiles, '', false)).toBe(profiles[0].id);
    expect(ruleFormProfile(profiles, missing, false)).toBe(profiles[0].id);
    expect(ruleFormProfile([], '', false)).toBe('');
  });

  it('shows errors, the rule limit and the browser preview state', () => {
    const rules = Array.from({ length: maxFocusRules }, (_, index) => ({ id: `r${index}`, app_bundle_id: `app.n${index}`, profile_id: profiles[0].id }));
    const markup = renderToStaticMarkup(<FocusRulesPanel store={storeWith({ rules, error: 'focus rules are unreadable' })} profiles={profiles} />);
    expect(markup).toContain('role="alert"><span>focus rules are unreadable</span>');
    expect(markup).toContain('At most 64 rules.');
    expect(markup).toMatch(/<button type="submit"[^>]*disabled=""/);
    const preview = renderToStaticMarkup(<FocusRulesPanel profiles={[]} />);
    expect(preview).toContain('Focused-app rules need the desktop app.');
    expect(preview).toContain('Save a profile first');
  });
});
