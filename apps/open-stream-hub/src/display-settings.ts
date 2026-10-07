import type { DialContent, DisplaySettings, Profile, ServiceModule } from './contracts';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

// The experimental "send images to module displays" setting. Rust stores it (src-tauri/src/display.rs, in
// <app data>/display-settings.json) and the session manager reads it once when a profile is started. This store only
// shows and edits it; changing it never touches a running profile.

export const defaultDisplaySettings: DisplaySettings = { enabled: false, dial_content: 'function' };

export const dialContentOptions: readonly { value: DialContent; label: string }[] = [
  { value: 'function', label: 'Active dial function name' },
  { value: 'clock', label: 'Digital clock (HH:MM)' },
  { value: 'analog_clock', label: 'Analog clock' },
  { value: 'off', label: 'Nothing (keys only)' },
];

const dialArgs: Record<DialContent, string> = { off: 'off', function: 'function', clock: 'clock', analog_clock: 'analog-clock' };

// Mirrors display::display_args in Rust: the masterhub-service flags a managed run started with `settings` gets.
// Nothing unless the setting is on and the run requests software mode; the 15-key module only for a schema-2 keypad
// profile; the dial unless its content is off.
export function displayFlags(settings: DisplaySettings, module: ServiceModule, softwareMode: boolean, profile: Profile): string[] {
  if (!settings.enabled || !softwareMode) return [];
  if (module === 'm4315' && profile.schema_version === 2 && profile.keypad !== undefined) return ['--display-sync'];
  if (module === 'm131e' && settings.dial_content !== 'off') return ['--display-sync', '--dial-content', dialArgs[settings.dial_content]];
  return [];
}

export function isDisplaySettings(value: unknown): value is DisplaySettings {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 2 && typeof record.enabled === 'boolean'
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

  // Until a reply arrives (or when it is unreadable), the shown value stays off.
  private async run(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!this.state.desktop) return false;
    this.set({ busy: true });
    try {
      const settings = await this.invoke<unknown>(command, args);
      if (!isDisplaySettings(settings)) throw new Error('The display settings reply was not understood.');
      this.set({ busy: false, loaded: true, error: null, settings });
      return true;
    } catch (reason) {
      this.set({ busy: false, error: errorMessage(reason) });
      return false;
    }
  }

  load() { return this.run('display_settings'); }
  save(settings: DisplaySettings) { return this.run('set_display_settings', { settings }); }
  setEnabled(enabled: boolean) { return this.save({ ...this.state.settings, enabled }); }
  setDialContent(dialContent: DialContent) { return this.save({ ...this.state.settings, dial_content: dialContent }); }
  dismissError() { this.set({ error: null }); }
}
