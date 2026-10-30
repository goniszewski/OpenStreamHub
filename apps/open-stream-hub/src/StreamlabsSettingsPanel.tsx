import { useEffect, useState, useSyncExternalStore } from 'react';
import { X } from 'lucide-react';
import type { ObsHost } from './contracts';
import { IntegerField, SelectField } from './controls';
import { appliesNote } from './DisplaySettingsPanel';
import { defaultStreamlabsPort, idleStreamlabsSettingsState, streamlabsConnectionLabels, streamlabsEndpoint, streamlabsHostOptions } from './streamlabs-settings';
import type { StreamlabsSettingsStore } from './streamlabs-settings';
import { idleServiceState, isActiveRun } from './service-runner';
import type { ServiceStoreState } from './service-runner';

export const streamlabsStatusIntervalMs = 10_000;
const noopSubscribe = () => () => {};
const idle = () => idleStreamlabsSettingsState;
const idleRunner = () => idleServiceState;

export const streamlabsTokenHelp = 'In Streamlabs Desktop open Settings › Remote Control, choose Show details and copy the API token shown there.';
export const streamlabsTokenNote = 'The token is kept in your macOS Keychain, so you enter it once. After an update macOS may ask whether OpenStreamHub can use it: choose Always Allow. It is never saved to a file, a profile, logs or diagnostics, it is never shown again here, and it is passed to the background helper privately, not as a command-line argument.';

// Settings panel for the Streamlabs Desktop actions (streamlabs_*): a loopback Remote Control endpoint and a write-only
// API token. Off by default; laid out like ObsSettingsPanel.
export function StreamlabsSettingsPanel({ store, runner }: { store?: StreamlabsSettingsStore; runner?: { subscribe(listener: () => void): () => void; getState(): ServiceStoreState } }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const runs = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner).runs;
  const activeRuns = Object.values(runs).filter(isActiveRun).length;
  // Only the field's own text; cleared as soon as it is handed to the backend.
  const [token, setToken] = useState('');
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  // Re-test while the panel is open so the chip follows Streamlabs Desktop starting and stopping.
  useEffect(() => {
    if (!store || !state.desktop || !state.loaded || !state.settings.enabled) return;
    void store.testConnection();
    const timer = setInterval(() => { void store.testConnection(); }, streamlabsStatusIntervalMs);
    return () => clearInterval(timer);
  }, [store, state.desktop, state.loaded, state.settings]);
  const off = !store || !state.desktop || state.busy || !state.loaded;
  const { enabled, host, port, token_set: tokenSet } = state.settings;
  const submit = () => {
    const value = token;
    setToken('');
    void store?.setToken(value);
  };

  return <section className="panel obs-settings-panel streamlabs-settings-panel" aria-labelledby="streamlabs-settings-title">
    <h2 id="streamlabs-settings-title">Streamlabs Desktop</h2>
    <label className="setting-toggle"><input type="checkbox" checked={enabled} disabled={off} aria-describedby="streamlabs-settings-applies" onChange={event => { void store?.setEnabled(event.target.checked); }} />Let Streamlabs actions connect to Streamlabs Desktop on this Mac</label>
    <p className="fine-print">Uses the Remote Control API of Streamlabs Desktop (Settings › Remote Control). Each action opens one short connection to {streamlabsEndpoint(state.settings)}, signs in with the API token, sends its requests (for example switch the scene, toggle recording, mute a source) and closes it. Only this Mac is allowed: the connection must stay on loopback, and the address is a setting of this app, never part of a profile. Off by default. Not yet tested against a real Streamlabs Desktop.</p>
    <div className="obs-endpoint-fields">
      <label htmlFor="streamlabs-host">Host</label>
      <SelectField id="streamlabs-host" value={host} disabled={off} onChange={event => { void store?.setHost(event.target.value as ObsHost); }}>
        {streamlabsHostOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
      </SelectField>
      <label htmlFor="streamlabs-port">Port</label>
      <IntegerField key={`${host}:${port}`} id="streamlabs-port" value={port} minimum={1} maximum={65535} disabled={off} onCommit={value => { void store?.setPort(value); }} />
    </div>
    <form className="obs-password-form" onSubmit={event => { event.preventDefault(); submit(); }}>
      <label htmlFor="streamlabs-token">API token</label>
      <input id="streamlabs-token" type="password" autoComplete="off" spellCheck={false} value={token} disabled={off} aria-describedby="streamlabs-token-note" placeholder={tokenSet ? 'Set for this app' : 'Not set'} onChange={event => setToken(event.target.value)} />
      <div className="settings-actions">
        <button type="submit" className="button compact" disabled={off || token === ''}>Use token</button>
        <button type="button" className="button compact" disabled={off || !tokenSet} onClick={() => { setToken(''); void store?.clearToken(); }}>Forget token</button>
      </div>
      <p id="streamlabs-token-note" className="fine-print" role="note">{tokenSet ? 'A token is set. ' : 'No token is set; Streamlabs actions need one. '}{streamlabsTokenHelp} {streamlabsTokenNote}</p>
    </form>
    {enabled && <div className="obs-status-row" role="status">
      <span className={`obs-chip obs-chip-${state.connection?.state ?? 'unknown'}`}>{state.connection ? streamlabsConnectionLabels[state.connection.state] : 'Checking…'}</span>
      <span className="fine-print">{state.connection?.message ?? ''}</span>
      {state.connection && ['wrong_port', 'remote_control_off', 'not_running'].includes(state.connection.state) && port !== defaultStreamlabsPort && <button type="button" className="button compact" disabled={off} onClick={() => { void store?.setPort(defaultStreamlabsPort); }}>Use default port {defaultStreamlabsPort}</button>}
      <button type="button" className="button compact" disabled={off || state.testing} onClick={() => { void store?.testConnection(); }}>{state.testing ? 'Testing…' : 'Test connection'}</button>
    </div>}
    <p id="streamlabs-settings-applies" className="fine-print" role="status">{appliesNote(activeRuns)}</p>
    {!state.desktop && <p className="fine-print">Streamlabs needs the desktop app.</p>}
    {state.error && <div className="service-failure" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss Streamlabs settings error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
  </section>;
}
