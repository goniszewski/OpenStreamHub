import { useCallback, useEffect, useRef, useState } from 'react';
import { Check } from 'lucide-react';
import type { InputPermissions } from './contracts';
import { ModalDialog } from './controls';
import { missingPermissions, permissionRows } from './PermissionsPanel';
import type { Target } from './PermissionsPanel';
import { PermissionDragTile } from './PermissionDragTile';
import type { Invoke } from './profile-session';

export const permissionPromptPollMs = 2000;
const closeDelayMs = 1200;

// Asked once per launch when macOS lacks a permission the app needs. It follows System Settings: the list
// updates when the user comes back, and the dialog closes itself once everything is allowed.
export function PermissionsPrompt({ invoke, onClose }: { invoke: Invoke; onClose(): void }) {
  const [missing, setMissing] = useState<Target[] | null>(null);
  const [opened, setOpened] = useState(false);
  const call = useRef(invoke);
  call.current = invoke;
  const check = useCallback(async () => {
    try {
      setMissing(missingPermissions(await call.current<InputPermissions>('input_permissions')));
    } catch { /* A failing check says nothing about permissions; keep what is shown. */ }
  }, []);
  useEffect(() => {
    void check();
    const timer = setInterval(() => { void check(); }, permissionPromptPollMs);
    window.addEventListener('focus', check);
    return () => { clearInterval(timer); window.removeEventListener('focus', check); };
  }, [check]);
  const done = missing !== null && missing.length === 0;
  useEffect(() => {
    if (!done) return;
    const timer = setTimeout(onClose, closeDelayMs);
    return () => clearTimeout(timer);
  }, [done, onClose]);
  const open = (target: Target) => { setOpened(true); void call.current('open_permission_settings', { target }).catch(() => {}); };
  return <ModalDialog open labelledBy="permissions-title" describedBy="permissions-message" onCancel={onClose}>
    <h2 id="permissions-title">{done ? 'All set' : 'Allow OpenStreamHub in System Settings'}</h2>
    {done
      ? <p id="permissions-message" className="permission-done"><Check size={16} /> Every permission is on.</p>
      : <>
        <p id="permissions-message">Some actions will not work until these are on:</p>
        <ul className="permission-list">{(missing ?? []).map(target => {
          const row = permissionRows.find(item => item.target === target)!;
          return <li key={target} className="prompt-row">
            <div><strong>{row.title}</strong><small>{row.why}</small></div>
            <button className="button primary" onClick={() => open(target)}>Open</button>
          </li>;
        })}</ul>
        {opened && <PermissionDragTile invoke={invoke} />}
        <p className="fine-print">Switch OpenStreamHub on in the list. If it is already there but still not working, select it, remove it with the minus button, then add it back with the plus button.</p>
      </>}
    <div className="dialog-actions"><button className="button" autoFocus onClick={onClose}>{done ? 'Close' : 'Not now'}</button></div>
  </ModalDialog>;
}
