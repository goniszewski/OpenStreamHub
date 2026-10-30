import { useCallback, useEffect, useRef, useState } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import type { InputPermissions } from './contracts';
import type { Invoke } from './profile-session';

export type Target = 'post_events' | 'listen_events' | 'automation';

export const permissionRows: { target: Target; title: string; detail: string; why: string }[] = [
  { target: 'post_events', title: 'Accessibility', detail: 'Needed to send keyboard shortcuts, text and media keys to other apps.', why: 'Sends shortcuts, text and media keys.' },
  { target: 'listen_events', title: 'Input Monitoring', detail: 'Needed to record key macros. Reading the modules\' own input may need it too.', why: 'Records key macros and reads module input.' },
  { target: 'automation', title: 'Automation', detail: 'Needed to control System Events and media players such as Music and Spotify. macOS cannot report this one: it asks the first time an action needs it, and the list in System Settings shows what you allowed.', why: 'Controls other apps.' },
];

export type PermissionStatus = 'granted' | 'missing' | 'unknown' | 'ask';
export const permissionLabels: Record<PermissionStatus, string> = { granted: 'Granted', missing: 'Not granted', unknown: 'Checking', ask: 'Asked when used' };

// Automation has no passive check, so it is never reported as granted or missing.
// The permissions macOS can report that are off, in list order. Automation has no passive check, so it never counts.
export function missingPermissions(permissions: InputPermissions | null): Target[] {
  if (!permissions) return [];
  return permissionRows.map(row => row.target).filter(target => permissionState(target, permissions) === 'missing');
}

export function permissionState(target: Target, permissions: InputPermissions | null): PermissionStatus {
  if (target === 'automation') return 'ask';
  if (!permissions) return 'unknown';
  return (target === 'post_events' ? permissions.post_events : permissions.listen_events) ? 'granted' : 'missing';
}

// Settings panel that shows which macOS privacy permissions OpenStreamHub has and opens the matching System Settings pane.
// Accessibility and Input Monitoring are read without prompting; Automation has no passive check.
export function PermissionsPanel({ desktop, invoke }: { desktop: boolean; invoke?: Invoke }) {
  const [permissions, setPermissions] = useState<InputPermissions | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  // The row whose System Settings pane was just opened; its drag tile shows until the permission is granted.
  const [dragging, setDragging] = useState<Target | null>(null);
  const available = desktop && !!invoke;
  // The parent passes a new function on every render; keep the latest in a ref so a re-render never re-runs the check.
  const call = useRef(invoke);
  call.current = invoke;
  // The check takes a moment of osascript; a button press holds the busy state a little longer so it reads as done.
  const check = useCallback(async (manual = false) => {
    const run = call.current;
    if (!run) return;
    // Only a button press shows the busy state; the automatic checks stay silent.
    if (manual) setBusy(true);
    const minimum = manual ? new Promise<void>(resolve => setTimeout(resolve, 500)) : Promise.resolve();
    try {
      setPermissions(await run<InputPermissions>('input_permissions'));
      setError(null);
    } catch (reason) {
      setError(String(reason));
    } finally {
      await minimum;
      if (manual) setBusy(false);
    }
  }, []);
  useEffect(() => {
    if (!available) return;
    void check();
    // The user changes a permission in System Settings and comes back to the app.
    const onFocus = () => { void check(); };
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [available, check]);
  // While a drag tile waits for its drop, re-check so the row turns green without a click.
  const waiting = dragging !== null && permissionState(dragging, permissions) !== 'granted';
  useEffect(() => {
    if (!waiting) return;
    const timer = setInterval(() => { void check(); }, 1500);
    return () => clearInterval(timer);
  }, [waiting, check]);
  const startDrag = async () => {
    if (!call.current) return;
    try {
      await call.current('start_permission_drag');
    } catch (reason) {
      setError(String(reason));
    }
  };
  const open = async (target: Target) => {
    if (!call.current) return;
    try {
      await call.current('open_permission_settings', { target });
      setDragging(target === 'automation' ? null : target);
      setError(null);
    } catch (reason) {
      setError(String(reason));
    }
  };
  return <section className="panel permissions-panel">
    <h2>Permissions</h2>
    <p>macOS keeps these per app. Allow OpenStreamHub in System Settings, then come back; this list updates by itself.</p>
    {!available && <p className="fine-print">Permissions can be checked in the desktop app only.</p>}
    {available && <ul className="permission-list">{permissionRows.map(row => {
      const status = permissionState(row.target, permissions);
      return <li key={row.target}>
        <div><strong>{row.title}</strong><small>{row.detail}</small></div>
        <span className={`permission-status ${status}`} role="status">{permissionLabels[status]}</span>
        <button className="button" onClick={() => void open(row.target)}>Open</button>
        {dragging === row.target && status !== 'granted' && <div className="permission-drag" role="group" aria-label="Drag OpenStreamHub into System Settings">
          <button className="permission-drag-tile" onMouseDown={() => void startDrag()}><img src="/app-icon.png" alt="" width={32} height={32} />OpenStreamHub</button>
          <small>Drag this onto the list in System Settings, then switch it on. This row turns green by itself.</small>
        </div>}
      </li>;
    })}</ul>}
    {available && <div className="settings-actions"><button className="button" disabled={busy} aria-busy={busy} onClick={() => void check(true)}>{busy ? <LoaderCircle size={14} className="spinning" /> : <RefreshCw size={14} />}{busy ? 'Checking…' : 'Check again'}</button></div>}
    {error && <p className="fine-print permission-error" role="alert">{error}</p>}
  </section>;
}
