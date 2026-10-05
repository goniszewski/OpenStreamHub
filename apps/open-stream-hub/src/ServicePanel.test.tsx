import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { Profile } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { confirmCopy, countdownSeconds, hasKeypadFolders, isMacPlatform, phaseLabel, relativeTime, runBlocker, ServicePanel } from './ServicePanel';
import { emptyCounters, ServiceRunner, serviceIdFor } from './service-runner';
import { DeviceFeed } from './device-feed';

const flat: Profile = { schema_version: 1, name: 'Flat', bindings: [] };
const root = emptyKeypadPage();
root.slots[1] = { type: 'folder', page_id: 'sub' };
const nested: Profile = { schema_version: 2, name: 'Nested', bindings: [], keypad: { pages: [root, emptyKeypadPage('sub', 'Sub')] } };
const dial = { path_hex: 'aabb', module: 'm131e' as const, serial_number: 'S1' };
const keypad = { ...dial, module: 'm4315' as const };
const mac = { platform: 'MacIntel' };

describe('platform and blockers', () => {
  it('detects macOS', () => {
    expect(isMacPlatform(mac)).toBe(true);
    expect(isMacPlatform({ userAgent: 'Mozilla/5.0 (Macintosh; Intel Mac OS X)' })).toBe(true);
    expect(isMacPlatform({ platform: 'Win32', userAgent: 'Windows' })).toBe(false);
    expect(isMacPlatform(undefined)).toBe(false);
  });
  it('refuses nested keypad profiles on the 15-key module only', () => {
    expect(hasKeypadFolders(nested)).toBe(true);
    expect(hasKeypadFolders(flat)).toBe(false);
    expect(runBlocker({ desktop: true, device: keypad, profile: nested })).toContain('folders');
    expect(runBlocker({ desktop: true, device: dial, profile: nested })).toBeNull();
    expect(runBlocker({ desktop: true, device: keypad, profile: flat })).toBeNull();
    expect(runBlocker({ desktop: true, device: undefined, profile: flat })).toContain('No module');
    expect(runBlocker({ desktop: false, device: dial, profile: flat })).toContain('desktop app');
  });
  it('formats relative time', () => {
    expect(relativeTime(1000, 2000)).toBe('now');
    expect(relativeTime(0, 30_000)).toBe('30s ago');
    expect(relativeTime(0, 125_000)).toBe('2m ago');
    expect(relativeTime(0, 7_300_000)).toBe('2h ago');
  });
});

function fixtures(execute: boolean, state: 'running' | 'failed' = 'running', extra: Record<string, unknown> = {}) {
  const feed = new DeviceFeed(async () => [dial] as never, async () => () => {}, true);
  (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [dial] };
  const runner = new ServiceRunner(async () => undefined as never, async () => () => {}, true);
  (runner as unknown as { state: unknown }).state = { desktop: true, error: null, runs: { [serviceIdFor('aabb')]: {
    serviceId: serviceIdFor('aabb'), state, execute, entries: [{ kind: 'action', text: 'planned volume +5', at: 1000 }], counters: { ...emptyCounters(), events: 3 },
    startedAt: 0, profileName: 'Flat', snapshot: '{}', draftUnsaved: true, ...extra } } };
  return { feed, runner };
}
const render = (props: Partial<Parameters<typeof ServicePanel>[0]> & { feed?: DeviceFeed; runner?: ServiceRunner }) =>
  renderToStaticMarkup(<ServicePanel profile={flat} desktop unsaved={false} platform={mac} now={2000} {...props} />);

describe('ServicePanel markup', () => {
  it('shows an unmistakable banner while executing', () => {
    const { feed, runner } = fixtures(true);
    const markup = render({ feed, runner });
    expect(markup).toContain('Actions are running on this Mac');
    expect(markup).toContain('service-banner execute');
    expect(markup).toContain('Live preview paused while actions run.');
    expect(markup).toContain('Profile changed since start. Restart to apply.');
    expect(markup).toContain('(unsaved draft)');
    expect(markup).toContain('>Stop<');
    expect(markup).toContain('role="log"');
    expect(markup).toContain('<time>now</time>');
  });
  it('shows a dry run without the execute banner', () => {
    const { feed, runner } = fixtures(false);
    const markup = render({ feed, runner });
    expect(markup).not.toContain('Actions are running on this Mac');
    expect(markup).toContain('Dry run: actions are logged only');
  });
  it('shows failures with the stderr tail', () => {
    const { feed, runner } = fixtures(false, 'failed', { error: 'device busy', stderrTail: 'hid error' });
    const markup = render({ feed, runner });
    expect(markup).toContain('device busy');
    expect(markup).toContain('hid error');
    expect(markup).toContain('>Failed<');
  });
  it('confirmation names the profile and device with Cancel focused first', () => {
    const { feed, runner } = fixtures(false, 'failed');
    const markup = render({ feed, runner, profile: { ...flat, name: 'Gaming' } });
    expect(markup).toContain('Profile &quot;Gaming&quot; will run on Encoder IPS display S1');
    expect(markup).toContain('really run on this Mac');
    expect(markup.indexOf('>Cancel<')).toBeLessThan(markup.indexOf('>Run actions<'));
    expect(markup).toMatch(/autofocus=""[^>]*>Cancel</);
  });
  it('disables Execute off macOS and explains it', () => {
    const { feed, runner } = fixtures(false, 'failed');
    const markup = render({ feed, runner, platform: { platform: 'Win32' } });
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Execute actions</);
    expect(markup).toContain('only available on macOS');
  });
  it('disables Run with the m4315 reason', () => {
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [keypad] };
    const runner = new ServiceRunner(async () => undefined as never, async () => () => {}, true);
    const markup = render({ feed, runner, profile: nested });
    expect(markup).toContain('keypad folders');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>[^<]*<svg[^>]*>.*Run</s);
  });
  it('is disabled in the browser preview', () => {
    const markup = render({ desktop: false });
    expect(markup).toContain('needs the desktop app');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Dry run/);
  });
});

describe('session banners', () => {
  const sessionFor = (phase: string, extra: Record<string, unknown> = {}, execute = true) => fixtures(execute, 'running', {
    session: { serviceId: null, phase, reason: null, attempt: 0, module: 'm131e', serialNumber: 'ABCDEF123456', ...extra } });
  const liveBanner = 'Actions are running on this Mac';

  it('shows the execute-live banner only while running', () => {
    const { feed, runner } = sessionFor('running', { serviceId: serviceIdFor('aabb') });
    const markup = render({ feed, runner });
    expect(markup).toContain(liveBanner);
    expect(markup).not.toContain('Resume actions');
  });
  it('needs_confirmation is prominent, offers Resume and Stop, and never says actions are live', () => {
    const { feed, runner } = sessionFor('needs_confirmation', { reason: 'sleep' });
    const markup = render({ feed, runner });
    expect(markup).not.toContain(liveBanner);
    expect(markup).not.toContain('service-banner execute');
    expect(markup).toContain('service-banner paused attention');
    expect(markup).toContain('Actions are paused. Nothing will run until you resume.');
    expect(markup).toContain('Resume actions…');
    expect(markup).toContain('>Stop<');
    expect(markup).toContain('>Needs confirmation<');
  });
  it('suspended in dry-run mode says it resumes automatically', () => {
    const { feed, runner } = sessionFor('suspended', { reason: 'sleep' }, false);
    const markup = render({ feed, runner });
    expect(markup).toContain('Paused while your Mac sleeps');
    expect(markup).toContain('resumes automatically in dry-run mode');
    expect(markup).toContain('>Stop<');
  });
  it('waiting names the module and serial tail', () => {
    const { feed, runner } = sessionFor('waiting_for_device', { reason: 'device_removed' });
    const markup = render({ feed, runner });
    expect(markup).toContain('Waiting for Encoder IPS display …123456 to reconnect…');
    expect(markup).not.toContain(liveBanner);
    expect(markup).toContain('>Stop<');
  });
  it('reconnecting shows the attempt and countdown', () => {
    const { feed, runner } = sessionFor('reconnecting', { attempt: 3, nextAttemptAt: 6500 });
    const markup = render({ feed, runner });
    expect(markup).toContain('attempt 3/5');
    expect(markup).toContain('Next attempt in 5s.');
    expect(markup).not.toContain(liveBanner);
    expect(countdownSeconds(6500, 2000)).toBe(5);
    expect(countdownSeconds(1000, 2000)).toBe(0);
    expect(countdownSeconds(undefined, 2000)).toBeNull();
  });
  it('stopped shows reason and detail and no Stop', () => {
    const { feed, runner } = sessionFor('stopped', { reason: 'service_exited', detail: 'HID read failed' });
    const markup = render({ feed, runner });
    expect(markup).toContain('Session stopped: the service exited');
    expect(markup).toContain('HID read failed');
    expect(markup).not.toContain(liveBanner);
    expect(markup).not.toContain('>Stop<');
    expect(markup).toContain('>Run<');
  });
  it('keeps showing the session when no device is connected', () => {
    const { runner } = sessionFor('waiting_for_device');
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    const markup = render({ feed, runner });
    expect(markup).toContain('to reconnect');
    expect(markup).toContain('>Stop<');
  });
  it('keeps the stale-profile notice against the original snapshot', () => {
    const { feed, runner } = sessionFor('needs_confirmation');
    expect(render({ feed, runner })).toContain('Profile changed since start. Restart to apply.');
  });
  it('labels phases and builds the resume confirmation copy', () => {
    expect(phaseLabel(undefined)).toBe('Stopped');
    const copy = confirmCopy('resume', 'Gaming', 'Encoder …1234');
    expect(copy).toMatchObject({ title: 'Resume actions on this Mac?', confirm: 'Resume actions' });
    expect(copy.message).toContain('Profile "Gaming"');
    expect(copy.message).toContain('Encoder …1234');
    expect(confirmCopy('start', 'Gaming', 'X').confirm).toBe('Run actions');
  });
});
