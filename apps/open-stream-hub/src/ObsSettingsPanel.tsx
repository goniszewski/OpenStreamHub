import { useEffect, useState, useSyncExternalStore } from 'react';
import { X } from 'lucide-react';
import type { ObsHost } from './contracts';
import { IntegerField, SelectField } from './controls';
import { appliesNote } from './DisplaySettingsPanel';
import { idleObsSettingsState, obsEndpoint, obsHostOptions } from './obs-settings';
import type { ObsSettingsStore } from './obs-settings';
import { idleServiceState, isActiveRun } from './service-runner';
import type { ServiceStoreState } from './service-runner';

const noopSubscribe = () => () => {};
const idle = () => idleObsSettingsState;
const idleRunner = () => idleServiceState;

export const obsPasswordNote = 'The password is kept only in memory while OpenStreamHub is open, so enter it again after you restart the app. It is never saved to disk, to a profile, to logs or diagnostics, and it is passed to the service privately, not as a command-line argument. Use a long random OBS password: while OBS is closed, another program on this Mac could pose as OBS and try to guess a weak one.';

// Settings panel for obs_scene actions: a loopback OBS WebSocket endpoint and an in-memory password. Off by default.
export function ObsSettingsPanel({ store, runner }: { store?: ObsSettingsStore; runner?: { subscribe(listener: () => void): () => void; getState(): ServiceStoreState } }) {
  const state = useSyncExternalStore(store?.subscribe ?? noopSubscribe, store?.getState ?? idle, store?.getState ?? idle);
  const runs = useSyncExternalStore(runner?.subscribe ?? noopSubscribe, runner?.getState ?? idleRunner, runner?.getState ?? idleRunner).runs;
  const activeRuns = Object.values(runs).filter(isActiveRun).length;
  // Only the field's own text; cleared as soon as it is handed to the backend.
  const [password, setPassword] = useState('');
  useEffect(() => { if (store && state.desktop && !state.loaded) void store.load(); }, [store, state.desktop, state.loaded]);
  const off = !store || !state.desktop || state.busy || !state.loaded;
  const { enabled, host, port, password_set: passwordSet } = state.settings;
  const submit = () => {
    const value = password;
    setPassword('');
    void store?.setPassword(value);
  };

  return <section className="panel obs-settings-panel" aria-labelledby="obs-settings-title">
    <h2 id="obs-settings-title">OBS Studio</h2>
    <label className="setting-toggle"><input type="checkbox" checked={enabled} disabled={off} aria-describedby="obs-settings-applies" onChange={event => { void store?.setEnabled(event.target.checked); }} />Let “Switch OBS scene” actions connect to OBS on this Mac</label>
    <p className="fine-print">Uses the WebSocket server built into OBS 28 and later (Tools › WebSocket Server Settings). Each action opens one short connection to {obsEndpoint(state.settings)}, switches the program scene and closes it. Only this Mac is allowed; the address is a setting of this app and never part of a profile. Off by default.</p>
    <div className="obs-endpoint-fields">
      <label htmlFor="obs-host">Host</label>
      <SelectField id="obs-host" value={host} disabled={off} onChange={event => { void store?.setHost(event.target.value as ObsHost); }}>
        {obsHostOptions.map(option => <option key={option.value} value={option.value}>{option.label}</option>)}
      </SelectField>
      <label htmlFor="obs-port">Port</label>
      <IntegerField key={`${host}:${port}`} id="obs-port" value={port} minimum={1} maximum={65535} disabled={off} onCommit={value => { void store?.setPort(value); }} />
    </div>
    <form className="obs-password-form" onSubmit={event => { event.preventDefault(); submit(); }}>
      <label htmlFor="obs-password">WebSocket password</label>
      <input id="obs-password" type="password" autoComplete="off" spellCheck={false} value={password} disabled={off} aria-describedby="obs-password-note" placeholder={passwordSet ? 'Set for this app session' : 'Not set'} onChange={event => setPassword(event.target.value)} />
      <div className="settings-actions">
        <button type="submit" className="button compact" disabled={off || password === ''}>Use password</button>
        <button type="button" className="button compact" disabled={off || !passwordSet} onClick={() => { setPassword(''); void store?.clearPassword(); }}>Forget password</button>
      </div>
      <p id="obs-password-note" className="fine-print" role="note">{passwordSet ? 'A password is set. ' : 'No password is set; leave it unset if OBS has authentication turned off. '}{obsPasswordNote}</p>
    </form>
    <p id="obs-settings-applies" className="fine-print" role="status">{appliesNote(activeRuns)}</p>
    {!state.desktop && <p className="fine-print">OBS needs the desktop app.</p>}
    {state.error && <div className="service-failure" role="alert"><span>{state.error}</span><button type="button" className="icon-button" aria-label="Dismiss OBS settings error" onClick={() => store?.dismissError()}><X size={15} /></button></div>}
  </section>;
}
