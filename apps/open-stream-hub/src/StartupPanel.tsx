import { useEffect, useRef, useState } from 'react';
import { setKeepRunningInMenubar } from './menubar';
import type { Invoke } from './profile-session';

export interface AutostartStatus { enabled: boolean; available: boolean; reason: string }

// Settings panel for what happens when the app opens: starting at login and the permission check.
export function StartupPanel({ desktop, invoke, checkPermissions, onCheckPermissions }: {
  desktop: boolean; invoke?: Invoke; checkPermissions: boolean; onCheckPermissions(on: boolean): void;
}) {
  const [status, setStatus] = useState<AutostartStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [menubar, setMenubar] = useState<boolean | null>(null);
  const call = useRef(invoke);
  call.current = invoke;
  const available = desktop && !!invoke;
  useEffect(() => {
    if (!available) return;
    call.current!<AutostartStatus>('autostart_status').then(setStatus).catch(reason => setError(String(reason)));
    call.current!<boolean>('menubar_status').then(setMenubar).catch(reason => setError(String(reason)));
  }, [available]);
  const toggle = async (enabled: boolean) => {
    if (!call.current) return;
    setBusy(true);
    try {
      setStatus(await call.current<AutostartStatus>('set_autostart', { enabled }));
      setError(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  const toggleMenubar = async (enabled: boolean) => {
    if (!call.current) return;
    setBusy(true);
    try {
      const saved = await call.current<boolean>('set_menubar', { enabled });
      setMenubar(saved);
      setKeepRunningInMenubar(saved);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  return <section className="panel startup-panel">
    <h2>Startup</h2>
    <label className="setting-toggle">
      <input type="checkbox" checked={status?.enabled ?? false} disabled={!available || busy || !status?.available} onChange={event => void toggle(event.target.checked)} />
      Start OpenStreamHub when I log in
    </label>
    <p className="fine-print">{!available ? 'Available in the desktop app.' : status && !status.available ? status.reason : 'Adds a login item so your modules work right after you log in. Turn it off here at any time.'}</p>
    <label className="setting-toggle">
      <input type="checkbox" checked={menubar ?? false} disabled={!available || busy || menubar === null} onChange={event => void toggleMenubar(event.target.checked)} />
      Keep running in the menu bar when I close the window
    </label>
    <p className="fine-print">Closing the window hides it and your modules keep working. Use the menu bar icon to show the window again or to quit.</p>
    <label className="setting-toggle">
      <input type="checkbox" checked={checkPermissions} onChange={event => onCheckPermissions(event.target.checked)} />
      Check permissions when the app opens
    </label>
    <p className="fine-print">If macOS has not allowed something the app needs, a short prompt walks you to the right place in System Settings.</p>
    {error && <p className="fine-print permission-error" role="alert">{error}</p>}
  </section>;
}
