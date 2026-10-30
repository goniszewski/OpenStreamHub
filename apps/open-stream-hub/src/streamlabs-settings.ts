import type { ObsHost, StreamlabsConnectionState, StreamlabsConnectionView, StreamlabsSettings, StreamlabsSettingsView } from './contracts';
import { obsEndpoint, obsHostOptions } from './obs-settings';
import type { Invoke } from './profile-session';
import { errorMessage } from './ui-model';

// The Streamlabs Desktop setting for the streamlabs_* actions, built like the OBS one (obs-settings.ts). Rust stores the
// endpoint (src-tauri/src/streamlabs.rs, in <app data>/streamlabs-settings.json) and keeps the API token in the macOS
// Keychain; the session manager reads both once when a profile is started. This store never holds the token: setToken
// hands it straight to the backend and every reply says only whether one is set.

export const defaultStreamlabsPort = 59650;
export const defaultStreamlabsSettings: StreamlabsSettingsView = { enabled: false, host: '127.0.0.1', port: defaultStreamlabsPort, token_set: false };

// Same loopback hosts and endpoint form as OBS (StreamlabsEndpoint wraps ObsEndpoint in Rust).
export const streamlabsHostOptions = obsHostOptions;
export const streamlabsEndpoint = obsEndpoint;

// Mirrors streamlabs::streamlabs_args in Rust: the masterhub-service flags a managed run started with `settings` gets.
// Streamlabs always needs the token, so without one the run gets no Streamlabs flags at all. Never the token itself.
export function streamlabsFlags(settings: StreamlabsSettingsView): string[] {
  if (!settings.enabled || !settings.token_set) return [];
  return ['--streamlabs-endpoint', streamlabsEndpoint(settings), '--streamlabs-token-stdin'];
}

export function isStreamlabsSettingsView(value: unknown): value is StreamlabsSettingsView {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 4 && typeof record.enabled === 'boolean' && typeof record.token_set === 'boolean'
    && streamlabsHostOptions.some(option => option.value === record.host)
    && Number.isInteger(record.port) && (record.port as number) >= 1 && (record.port as number) <= 65535;
}

export const streamlabsConnectionLabels: Record<StreamlabsConnectionState, string> = {
  off: 'Off', token_needed: 'Token needed', connected: 'Connected', not_running: 'Not running', remote_control_off: 'Remote Control off',
  wrong_port: 'Wrong port', wrong_token: 'Wrong token', failed: 'Not connected',
};

export function isStreamlabsConnectionView(value: unknown): value is StreamlabsConnectionView {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return Object.keys(record).length === 2 && typeof record.message === 'string' && typeof record.state === 'string' && record.state in streamlabsConnectionLabels;
}

export interface StreamlabsSettingsState { desktop: boolean; loaded: boolean; busy: boolean; error: string | null; settings: StreamlabsSettingsView; connection: StreamlabsConnectionView | null; testing: boolean }
export const idleStreamlabsSettingsState: StreamlabsSettingsState = { desktop: false, loaded: false, busy: false, error: null, settings: defaultStreamlabsSettings, connection: null, testing: false };

export class StreamlabsSettingsStore {
  private state: StreamlabsSettingsState;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly invoke: Invoke, desktop: boolean) {
    this.state = { ...idleStreamlabsSettingsState, desktop };
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  private set(patch: Partial<StreamlabsSettingsState>) {
    this.state = { ...this.state, ...patch };
    this.listeners.forEach(listener => listener());
  }

  // Until a reply arrives (or when it is unreadable), the shown value stays off.
  private async run(command: string, args?: Record<string, unknown>): Promise<boolean> {
    if (!this.state.desktop) return false;
    this.set({ busy: true });
    try {
      const settings = await this.invoke<unknown>(command, args);
      if (!isStreamlabsSettingsView(settings)) throw new Error('The Streamlabs settings reply was not understood.');
      // A changed endpoint, switch or token makes the last test stale.
      this.set({ busy: false, loaded: true, error: null, settings, connection: null });
      return true;
    } catch (reason) {
      this.set({ busy: false, error: errorMessage(reason) });
      return false;
    }
  }

  private current(): StreamlabsSettings {
    const { enabled, host, port } = this.state.settings;
    return { enabled, host, port };
  }

  load() { return this.run('streamlabs_settings'); }
  save(settings: StreamlabsSettings) { return this.run('set_streamlabs_settings', { settings }); }
  setEnabled(enabled: boolean) { return this.save({ ...this.current(), enabled }); }
  setHost(host: ObsHost) { return this.save({ ...this.current(), host }); }
  setPort(port: number) { return this.save({ ...this.current(), port }); }
  // The token is passed through and never kept here. An empty entry clears it.
  setToken(token: string) { return this.run('set_streamlabs_token', { token: token === '' ? null : token }); }
  clearToken() { return this.run('set_streamlabs_token', { token: null }); }
  // Probes the saved endpoint (auth, then a read that changes nothing) and returns a state with a fix hint. A failed
  // probe leaves the previous result in place.
  async testConnection(): Promise<boolean> {
    if (!this.state.desktop || this.state.testing) return false;
    this.set({ testing: true });
    try {
      const connection = await this.invoke<unknown>('streamlabs_status');
      if (!isStreamlabsConnectionView(connection)) throw new Error('The Streamlabs connection reply was not understood.');
      this.set({ testing: false, connection });
      return true;
    } catch (reason) {
      this.set({ testing: false, error: errorMessage(reason) });
      return false;
    }
  }
  dismissError() { this.set({ error: null }); }
}
