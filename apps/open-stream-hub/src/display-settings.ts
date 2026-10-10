import type { DefaultProfile, DisplaySettings, ModuleControl, Profile, RunMode, ServiceModule } from './contracts';
import type { Invoke } from './profile-session';
import { beginEditingHold as beginEditingHoldStep } from './editing-hold';
import { dialDisplayArgs, effectiveDialDisplay } from './dial-display-model';
import { errorMessage } from './ui-model';

// The "Module control" setting and the one run mode. Rust stores both
// (src-tauri/src/display.rs, in <app data>/display-settings.json, which also
// migrates the earlier shapes) and the session manager reads them when a
// profile is started. This store shows and edits them, and keeps the editing
// hold ("Switch to Test mode while editing"): the flag and the snapshot of
// executing runs behind the "Test (editing)" badge. Saving a new run mode
// lets the backend apply it to every running module. What the dial screen
// shows is each profile's own dial_display setting, so it is not stored here.

// A fresh install (no settings file): Live, with the one-time what-will-run
// confirmation still owed. Mirrors DisplaySettings::default() in Rust.
export const defaultDisplaySettings: DisplaySettings = { module_control: 'app', run_mode: 'live', live_offered: false };

export const moduleControlOptions: readonly { value: ModuleControl; label: string }[] = [
  { value: 'app', label: 'App' },
  { value: 'built_in_animation', label: 'Built-in animation' },
];

// The modules "App" module control drives in software mode: the 15-key module and the dial. Mirrors display::app_controls.
export const appControls = (module: ServiceModule) => module === 'm4315' || module === 'm131e';
// The modules the session manager starts on its own under App: those two and the knobs, which get an input-only session
// (no software mode, no display writes). Mirrors display::auto_takes_over.
export const autoTakesOver = (module: ServiceModule) => appControls(module) || module === 'm003k';

// Mirrors display::spawn_display in Rust: the masterhub-service flags a managed run started with `settings` gets.
// Nothing unless module control is App and the run is in software mode; the 15-key module for a keypad profile and for a
// flat one, which the backend writes as one page (a flat profile it cannot convert runs without them); the dial unless
// the profile's dial display is off, with the effective content the backend derives from the profile (the function name
// by default, the analog clock for a profile with nothing on the dial).
export function displayFlags(settings: DisplaySettings, module: ServiceModule, softwareMode: boolean, profile: Profile): string[] {
  if (settings.module_control !== 'app' || !softwareMode) return [];
  if (module === 'm4315' && (profile.schema_version === 1 || profile.keypad !== undefined)) return ['--display-sync'];
  if (module === 'm131e' && effectiveDialDisplay(profile) !== 'off') {
    return ['--display-sync', '--dial-content', dialDisplayArgs[effectiveDialDisplay(profile)]];
  }
  return [];
}

export function isDisplaySettings(value: unknown): value is DisplaySettings {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 3
    && moduleControlOptions.some(option => option.value === record.module_control)
    && (record.run_mode === 'live' || record.run_mode === 'test')
    && typeof record.live_offered === 'boolean';
}

const isDefaultProfile = (value: unknown): value is DefaultProfile => !!value && typeof value === 'object'
  && Object.keys(value).length === 1 && (((value as DefaultProfile).profile_id === null) || typeof (value as DefaultProfile).profile_id === 'string');

// defaultProfile: the global default profile for automatic starts (Rust startup.rs), loaded on its own; an unreadable
// file shows its error and means no default.
export interface DisplaySettingsState {
  desktop: boolean; loaded: boolean; busy: boolean; error: string | null; settings: DisplaySettings;
  // The editing hold ("Switch to Test mode while editing"): true while the
  // hold keeps the mode at Test on the user's behalf, so the badge can say
  // why. The user's own switch (setRunMode) clears it.
  heldByEditing: boolean;
  defaultProfile: { loaded: boolean; busy: boolean; error: string | null; profileId: string | null };
}
export const idleDisplaySettingsState: DisplaySettingsState = { desktop: false, loaded: false, busy: false, error: null, settings: defaultDisplaySettings,
  heldByEditing: false, defaultProfile: { loaded: false, busy: false, error: null, profileId: null } };

export class DisplaySettingsStore {
  private state: DisplaySettingsState;
  // The run keys that were executing when the editing hold began: the restore
  // may return exactly these to Live without asking again. Only the hold's
  // callers read it, so it stays untyped beyond its keys.
  private editingSnapshot: ReadonlySet<string> = new Set();
  private readonly listeners = new Set<() => void>();

  constructor(private readonly invoke: Invoke, desktop: boolean) {
    this.state = { ...idleDisplaySettingsState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<DisplaySettingsState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Until a reply arrives (or when it is unreadable), the shown value stays the default; the backend decides at Start.
  private async run(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!this.state.desktop) return false;
    this.set({ busy: true });
    try {
      const settings = await this.invoke<unknown>(command, args);
      if (!isDisplaySettings(settings)) throw new Error('The display settings reply was not understood.');
      this.set({ busy: false, loaded: true, error: null, settings });
      return true;
    } catch (reason) {
      this.set({ busy: false, loaded: false, error: errorMessage(reason) });
      return false;
    }
  }

  load() { return this.run('display_settings'); }
  save(settings: DisplaySettings) { return this.run('set_display_settings', { settings }); }
  setModuleControl(moduleControl: ModuleControl) { return this.save({ ...this.state.settings, module_control: moduleControl }); }
  // Switching the run mode applies it to every running module: the backend
  // restarts the sessions that change, a running dry one executing only
  // after the caller confirmed the switch (the header dialog). Switching is
  // itself the one-time confirmation, whichever way it goes. A switch during
  // the editing hold is the user's own: the hold ends, so nothing is
  // restored underneath them when the editing ends.
  setRunMode(runMode: RunMode) {
    if (this.state.heldByEditing) this.set({ heldByEditing: false });
    return this.save({ ...this.state.settings, run_mode: runMode, live_offered: true });
  }
  // The editing hold's saves ("Switch to Test mode while editing"): they
  // change the mode alone and never answer the one-time confirmation, which
  // stays owed exactly as it was.
  setHeldMode(runMode: RunMode) { return this.save({ ...this.state.settings, run_mode: runMode }); }
  // Begins the editing hold: the mode is saved Test (reaching the backend, so
  // a press on any module is logged, never run) and the badge says why.
  // `executing` names the runs that were executing right now — the snapshot
  // the restore may return to Live without asking again. False when there is
  // nothing to do: the hold is on, the setting is off, or the mode is Test.
  beginEditingHold(whileEditing: boolean, executing: readonly string[]): boolean {
    const step = beginEditingHoldStep(this.state.heldByEditing, this.state.settings.run_mode, whileEditing);
    if (!step.save) return false;
    this.editingSnapshot = new Set(executing);
    this.set({ heldByEditing: true });
    void this.setHeldMode(step.save);
    return true;
  }
  // Ends the hold without restoring (the caller decides whether the restore
  // needs the switch-to-Live dialog first). False when no hold was on.
  dropEditingHold(): boolean {
    if (!this.state.heldByEditing) return false;
    this.set({ heldByEditing: false });
    return true;
  }
  get editingHeld(): boolean { return this.state.heldByEditing; }
  get editingSnapshotRuns(): ReadonlySet<string> { return this.editingSnapshot; }
  // The one-time confirmation: Live stays (or becomes) the mode and the
  // fresh-install hold is lifted, so automatic sessions start executing.
  confirmLive() { return this.save({ ...this.state.settings, run_mode: 'live', live_offered: true }); }
  // Declined once: Test stands and the offer never nags again (the header
  // switch is always there).
  declineLiveOffer() { return this.save({ ...this.state.settings, run_mode: 'test', live_offered: true }); }
  // A confirmed Run with actions is a confirmation of the mode too.
  markLiveConfirmed() { return this.save({ ...this.state.settings, live_offered: true }); }
  dismissError() { this.set({ error: null }); }

  private async runDefault(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!this.state.desktop) return false;
    this.set({ defaultProfile: { ...this.state.defaultProfile, busy: true } });
    try {
      const reply = await this.invoke<unknown>(command, args);
      if (!isDefaultProfile(reply)) throw new Error('The default profile reply was not understood.');
      this.set({ defaultProfile: { loaded: true, busy: false, error: null, profileId: reply.profile_id } });
      return true;
    } catch (reason) {
      this.set({ defaultProfile: { ...this.state.defaultProfile, busy: false, error: errorMessage(reason) } });
      return false;
    }
  }

  loadDefaultProfile() { return this.runDefault('default_profile'); }
  setDefaultProfile(profileId: string | null) { return this.runDefault('set_default_profile', { profileId }); }

  // Browser-preview QA only (the App dev hook calls it; the desktop app
  // never does): the preview cannot reach Tauri, so captures seed the
  // settings the header badge and the settings panel show.
  seedPreview(settings: DisplaySettings) {
    if (this.state.desktop) return;
    this.set({ loaded: true, error: null, settings });
  }
}
