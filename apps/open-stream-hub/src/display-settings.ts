import type { DialContent, DisplaySettings, ModuleControl, Profile, ServiceModule } from './contracts';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

// The "Module control" setting and the dial content. Rust stores them (src-tauri/src/display.rs, in
// <app data>/display-settings.json, which also migrates the earlier experimental toggle) and the session manager reads
// them once when a profile is started. This store only shows and edits them; changing them never touches a running
// profile.

export const defaultDisplaySettings: DisplaySettings = { module_control: 'app', dial_content: 'function' };

export const moduleControlOptions: readonly { value: ModuleControl; label: string }[] = [
  { value: 'app', label: 'App' },
  { value: 'built_in_animation', label: 'Built-in animation' },
];

export const dialContentOptions: readonly { value: DialContent; label: string }[] = [
  { value: 'function', label: 'Active dial function name' },
  { value: 'clock', label: 'Digital clock (HH:MM)' },
  { value: 'analog_clock', label: 'Analog clock' },
  { value: 'cpu', label: 'CPU usage (%, macOS only)' },
  { value: 'off', label: 'Nothing (keys only)' },
];

const dialArgs: Record<DialContent, string> = { off: 'off', function: 'function', clock: 'clock', analog_clock: 'analog-clock', cpu: 'cpu' };

// The modules "App" module control drives in software mode: the 15-key module and the dial. Mirrors display::app_controls.
export const appControls = (module: ServiceModule) => module === 'm4315' || module === 'm131e';

// Mirrors display::spawn_display in Rust: the masterhub-service flags a managed run started with `settings` gets.
// Nothing unless module control is App and the run is in software mode; the 15-key module for a keypad profile and for a
// flat one, which the backend writes as one page (a flat profile it cannot convert runs without them); the dial unless its
// content is off, with the analog clock instead of the function name for a profile with nothing on the dial.
export function displayFlags(settings: DisplaySettings, module: ServiceModule, softwareMode: boolean, profile: Profile): string[] {
  if (settings.module_control !== 'app' || !softwareMode) return [];
  if (module === 'm4315' && (profile.schema_version === 1 || profile.keypad !== undefined)) return ['--display-sync'];
  if (module === 'm131e' && settings.dial_content !== 'off') {
    const bindsDial = profile.dial_functions !== undefined || profile.bindings.some(binding => binding.module === 'm131e');
    const content = settings.dial_content === 'function' && !bindsDial ? 'analog_clock' : settings.dial_content;
    return ['--display-sync', '--dial-content', dialArgs[content]];
  }
  return [];
}

export function isDisplaySettings(value: unknown): value is DisplaySettings {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 2
    && moduleControlOptions.some(option => option.value === record.module_control)
    && dialContentOptions.some(option => option.value === record.dial_content);
}

export interface DisplaySettingsState { desktop: boolean; loaded: boolean; busy: boolean; error: string | null; settings: DisplaySettings }
export const idleDisplaySettingsState: DisplaySettingsState = { desktop: false, loaded: false, busy: false, error: null, settings: defaultDisplaySettings };

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
  setDialContent(dialContent: DialContent) { return this.save({ ...this.state.settings, dial_content: dialContent }); }
  dismissError() { this.set({ error: null }); }
}
