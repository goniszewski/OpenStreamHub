import { Component, useEffect, useRef, useState } from 'react';
import type { ReactNode, SelectHTMLAttributes } from 'react';
import { ChevronDown, CircleHelp, RotateCcw } from 'lucide-react';
import { labelIssue, parseIntegerInput } from './draft-model';

export function SelectField({ children, ...props }: SelectHTMLAttributes<HTMLSelectElement>) {
  return <div className={`select-field${props.disabled ? ' disabled' : ''}`}><select {...props}>{children}</select><ChevronDown size={17} aria-hidden="true" /></div>;
}

// Keeps the typed text locally and commits only whole numbers within range,
// so an empty or partial entry never reaches the draft as NaN or 0.
export function IntegerField({ id, value, minimum, maximum, nonZero, disabled, onCommit }: {
  id: string; value: number; minimum: number; maximum: number; nonZero?: boolean; disabled?: boolean; onCommit(value: number): void;
}) {
  const [raw, setRaw] = useState(String(value));
  const [committed, setCommitted] = useState(value);
  if (committed !== value) { setCommitted(value); setRaw(String(value)); }
  const parsed = parseIntegerInput(raw, minimum, maximum, { nonZero });
  return <>
    <input id={id} type="number" min={minimum} max={maximum} step={1} value={raw} disabled={disabled}
      aria-invalid={!parsed.ok} aria-describedby={parsed.ok ? undefined : `${id}-error`}
      onChange={event => {
        setRaw(event.target.value);
        const next = parseIntegerInput(event.target.value, minimum, maximum, { nonZero });
        if (next.ok && next.value !== value) onCommit(next.value);
      }} />
    {!parsed.ok && <p id={`${id}-error`} className="field-error" role="alert">{parsed.error}</p>}
  </>;
}

// Shows an inline problem for a label the Rust service would reject.
export function LabelError({ id, value, field }: { id: string; value: string; field: string }) {
  const issue = labelIssue(value, field);
  return issue ? <p id={id} className="field-error" role="alert">{issue}</p> : null;
}

// A modal driven by state. showModal() runs only when the dialog is closed,
// which avoids the InvalidStateError thrown for an already-open dialog.
export function ModalDialog({ open, labelledBy, describedBy, onCancel, className, role, children }: {
  open: boolean; labelledBy: string; describedBy?: string; onCancel(): void; className?: string; role?: 'alertdialog'; children: ReactNode;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const element = dialog.current;
    if (!element) return;
    if (open && !element.open) element.showModal();
    else if (!open && element.open) element.close();
  }, [open]);
  return <dialog ref={dialog} role={role} className={`unsaved-dialog${className ? ` ${className}` : ''}`} aria-labelledby={labelledBy} aria-describedby={describedBy}
    onCancel={event => { event.preventDefault(); onCancel(); }}>{children}</dialog>;
}

// A small right-click menu, positioned at the pointer. It takes the focus when it opens, so
// Escape closes it and a click anywhere else moves the focus away and closes it — no listeners.
export interface ContextMenuItem { label: string; hint?: string; disabled?: boolean; danger?: boolean; onSelect(): void }

export function ContextMenu({ x, y, label, items, onClose }: { x: number; y: number; label: string; items: ContextMenuItem[]; onClose(): void }) {
  return <div className="context-menu" role="menu" aria-label={label} style={{ left: x, top: y }} tabIndex={-1} autoFocus
    onKeyDown={event => { if (event.key === 'Escape') { event.stopPropagation(); onClose(); } }}
    onBlur={event => { if (!event.currentTarget.contains(event.relatedTarget as Node | null)) onClose(); }}
    onContextMenu={event => event.preventDefault()}>
    {items.map(item => <button key={item.label} type="button" role="menuitem" disabled={item.disabled} className={item.danger ? 'danger' : undefined}
      onClick={() => { onClose(); item.onSelect(); }}>{item.label}{item.hint && <kbd>{item.hint}</kbd>}</button>)}
  </div>;
}

export function CrashScreen({ error }: { error: Error }) {
  return <div className="crash-screen" role="alert">
    <CircleHelp size={28} />
    <h1>Something went wrong</h1>
    <p>OpenStreamHub hit an unexpected problem and stopped this view. Saved profiles on disk are not affected; unsaved changes may be lost.</p>
    <code>{error.message}</code>
    <button className="button primary" onClick={() => window.location.reload()}><RotateCcw size={17} />Reload window</button>
  </div>;
}

export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state: { error: Error | null } = { error: null };
  static getDerivedStateFromError(error: unknown) { return { error: error instanceof Error ? error : new Error(String(error)) }; }
  render() { return this.state.error ? <CrashScreen error={this.state.error} /> : this.props.children; }
}
