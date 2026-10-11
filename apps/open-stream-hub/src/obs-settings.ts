import type { ObsConnectionState, ObsConnectionView, ObsHost, ObsSettings, ObsSettingsView } from './contracts';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

// The OBS WebSocket setting for obs_scene actions. Rust stores the endpoint (src-tauri/src/obs.rs, in
// <app data>/obs-settings.json) and keeps the password in the app's memory only; the session manager reads both once
// when a profile is started. This store shows and edits the setting. It never holds the password: setPassword hands it
// straight to the backend and every reply says only whether one is set.

export const defaultObsSettings: ObsSettingsView = { enabled: false, host: '127.0.0.1', port: 4455, password_set: false };

// Loopback only: the backend refuses every other host.
export const obsHostOptions: readonly { value: ObsHost; label: string }[] = [
  { value: '127.0.0.1', label: '127.0.0.1 (this Mac, IPv4)' },
  { value: '::1', label: '::1 (this Mac, IPv6)' },
  { value: 'localhost', label: 'localhost (this Mac)' },
];

// Mirrors the endpoint form ObsEndpoint prints (crates/cooltek-sdk-service/src/obs.rs).
export function obsEndpoint(settings: ObsSettings): string {
  return settings.host === '::1' ? `[::1]:${settings.port}` : `${settings.host}:${settings.port}`;
}

// Mirrors obs::obs_args in Rust: the masterhub-service flags a managed run started with `settings` gets. Never the
// password, which goes to the service's stdin.
export function obsFlags(settings: ObsSettingsView): string[] {
  if (!settings.enabled) return [];
  return ['--obs-endpoint', obsEndpoint(settings), ...(settings.password_set ? ['--obs-password-stdin'] : [])];
}

export function isObsSettingsView(value: unknown): value is ObsSettingsView {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 4 && typeof record.enabled === 'boolean' && typeof record.password_set === 'boolean'
    && obsHostOptions.some(option => option.value === record.host)
    && Number.isInteger(record.port) && (record.port as number) >= 1 && (record.port as number) <= 65535;
}

export const obsConnectionLabels: Record<ObsConnectionState, string> = {
  off: 'Off', connected: 'Connected', not_running: 'Not running', websocket_off: 'WebSocket off', wrong_port: 'Wrong port',
  password_needed: 'Password needed', wrong_password: 'Wrong password', failed: 'Not connected',
};

export function isObsConnectionView(value: unknown): value is ObsConnectionView {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 2 && typeof record.message === 'string' && typeof record.state === 'string' && record.state in obsConnectionLabels;
}

export interface ObsSettingsState { desktop: boolean; loaded: boolean; busy: boolean; error: string | null; settings: ObsSettingsView; connection: ObsConnectionView | null; testing: boolean }
export const idleObsSettingsState: ObsSettingsState = { desktop: false, loaded: false, busy: false, error: null, settings: defaultObsSettings, connection: null, testing: false };

export class ObsSettingsStore {
  private state: ObsSettingsState;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly invoke: Invoke, desktop: boolean) {
    this.state = { ...idleObsSettingsState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<ObsSettingsState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Until a reply arrives (or when it is unreadable), the shown value stays off.
  private async run(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!this.state.desktop) return false;
    this.set({ busy: true });
    try {
      const settings = await this.invoke<unknown>(command, args);
      if (!isObsSettingsView(settings)) throw new Error('The OBS settings reply was not understood.');
      // A changed endpoint, switch or password makes the last test stale.
      this.set({ busy: false, loaded: true, error: null, settings, connection: null });
      return true;
    } catch (reason) {
      this.set({ busy: false, error: errorMessage(reason) });
      return false;
    }
  }

  private current(): ObsSettings {
    const { enabled, host, port } = this.state.settings;
    return { enabled, host, port };
  }

  load() { return this.run('obs_settings'); }
  save(settings: ObsSettings) { return this.run('set_obs_settings', { settings }); }
  setEnabled(enabled: boolean) { return this.save({ ...this.current(), enabled }); }
  setHost(host: ObsHost) { return this.save({ ...this.current(), host }); }
  setPort(port: number) { return this.save({ ...this.current(), port }); }
  // The password is passed through and never kept here. An empty entry clears it.
  setPassword(password: string) { return this.run('set_obs_password', { password: password === '' ? null : password }); }
  clearPassword() { return this.run('set_obs_password', { password: null }); }
  // Probes the saved endpoint; the backend sends no OBS request and returns a state with a fix hint. A failed probe
  // leaves the previous result in place.
  async testConnection(): Promise<boolean> {
    if (!this.state.desktop || this.state.testing) return false;
    this.set({ testing: true });
    try {
      const connection = await this.invoke<unknown>('obs_status');
      if (!isObsConnectionView(connection)) throw new Error('The OBS connection reply was not understood.');
      this.set({ testing: false, connection });
      return true;
    } catch (reason) {
      this.set({ testing: false, error: errorMessage(reason) });
      return false;
    }
  }
  dismissError() { this.set({ error: null }); }
}
