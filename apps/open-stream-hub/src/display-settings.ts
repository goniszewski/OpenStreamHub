import type { DefaultProfile, DisplaySettings, ModuleControl, Profile, ServiceModule } from './contracts';
import type { Invoke } from './profile-session';
import { dialDisplayArgs, effectiveDialDisplay } from './dial-display-model';
import { errorMessage } from './ui-model';

// The "Module control" setting. Rust stores it (src-tauri/src/display.rs, in
// <app data>/display-settings.json, which also migrates the earlier shapes)
// and the session manager reads it once when a profile is started. This store
// only shows and edits it; changing it never touches a running profile. What
// the dial screen shows is each profile's own dial_display setting, so it is
// not stored here.

export const defaultDisplaySettings: DisplaySettings = { module_control: 'app', auto_execute: false };

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
  return Object.keys(record).length === 2
    && moduleControlOptions.some(option => option.value === record.module_control)
    && typeof record.auto_execute === 'boolean';
}

const isDefaultProfile = (value: unknown): value is DefaultProfile => !!value && typeof value === 'object'
  && Object.keys(value).length === 1 && (((value as DefaultProfile).profile_id === null) || typeof (value as DefaultProfile).profile_id === 'string');

// defaultProfile: the global default profile for automatic starts (Rust startup.rs), loaded on its own; an unreadable
// file shows its error and means no default.
export interface DisplaySettingsState {
  desktop: boolean; loaded: boolean; busy: boolean; error: string | null; settings: DisplaySettings;
  defaultProfile: { loaded: boolean; busy: boolean; error: string | null; profileId: string | null };
}
export const idleDisplaySettingsState: DisplaySettingsState = { desktop: false, loaded: false, busy: false, error: null, settings: defaultDisplaySettings,
  defaultProfile: { loaded: false, busy: false, error: null, profileId: null } };

export class DisplaySettingsStore {
  private state: DisplaySettingsState;
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
  // Turning it off restarts every automatic session that runs actions as a dry run (the backend does that on save).
  setAutoExecute(autoExecute: boolean) { return this.save({ ...this.state.settings, auto_execute: autoExecute }); }
  // The indicator's one-click stop. Never saves defaults over settings it has not read: it loads them first. If they cannot
  // be read, the backend treats them as not App and has already stopped every automatic session.
  async stopAutomaticActions() {
    if (!this.state.loaded && !(await this.load())) return false;
    return this.setAutoExecute(false);
  }
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
}
