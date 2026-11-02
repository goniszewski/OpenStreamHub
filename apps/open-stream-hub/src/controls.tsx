import { Children, Component, Fragment, isValidElement, useEffect, useId, useRef, useState } from 'react';
import type { FocusEventHandler, KeyboardEvent, ReactNode } from 'react';
import { Check, ChevronDown, CircleHelp, RotateCcw } from 'lucide-react';
import { labelIssue, parseIntegerInput } from './draft-model';

// A dropdown that looks like the rest of the app. A native select opens the system menu, which a webview cannot style, so
// this is a select-only combobox (the ARIA pattern): a button that shows the choice and a listbox that opens under it.
// Options are declared as <option> children, exactly as for a select, and onChange gets an event-like { target: { value } }.
export type SelectChange = { target: { value: string } };
export interface SelectChoice { value: string; label: string; disabled: boolean }

const textOf = (node: ReactNode): string => Children.toArray(node).map(child => (typeof child === 'string' || typeof child === 'number'
  ? String(child) : isValidElement<{ children?: ReactNode }>(child) ? textOf(child.props.children) : '')).join('');

// The <option> children of a select (also inside fragments), as plain choices.
export function selectChoices(children: ReactNode): SelectChoice[] {
  return Children.toArray(children).flatMap((child): SelectChoice[] => {
    if (!isValidElement<{ value?: string | number; disabled?: boolean; children?: ReactNode }>(child)) return [];
    if (child.type === Fragment) return selectChoices(child.props.children);
    if (child.type !== 'option') return [];
    const label = textOf(child.props.children);
    return [{ value: child.props.value === undefined ? label : String(child.props.value), label, disabled: !!child.props.disabled }];
  });
}

// The next choice that can be picked, stepping through the list and wrapping; stays put when none can.
export function nextChoice(choices: ReadonlyArray<SelectChoice>, from: number, delta: 1 | -1): number {
  const count = choices.length;
  const start = from < 0 && delta < 0 ? count : from;
  for (let step = 1; step <= count; step += 1) {
    const index = (((start + delta * step) % count) + count) % count;
    if (!choices[index].disabled) return index;
  }
  return from;
}

export function SelectField({ id, value, onChange, disabled, children, onFocus, 'aria-describedby': describedBy }: {
  id?: string; value: string | number; onChange(event: SelectChange): void; disabled?: boolean; children?: ReactNode;
  onFocus?: FocusEventHandler<HTMLButtonElement>; 'aria-describedby'?: string;
}) {
  const generated = useId();
  const triggerId = id ?? generated;
  const listId = `${triggerId}-list`;
  const optionId = (index: number) => `${triggerId}-option-${index}`;
  const choices = selectChoices(children);
  const selected = choices.findIndex(choice => choice.value === String(value));
  const [openState, setOpen] = useState(false);
  const open = openState && !disabled; // a select that gets disabled while open closes
  const [activeState, setActive] = useState(-1);
  const active = activeState < choices.length ? activeState : -1; // the choices can change under an open list
  const typed = useRef({ text: '', at: 0 });
  const list = useRef<HTMLUListElement>(null);

  useEffect(() => {
    if (open && active >= 0) list.current?.querySelector(`[id="${optionId(active)}"]`)?.scrollIntoView({ block: 'nearest' });
  }, [open, active]); // eslint-disable-line react-hooks/exhaustive-deps

  const show = () => { setActive(selected >= 0 ? selected : nextChoice(choices, -1, 1)); setOpen(true); };
  const choose = (index: number) => {
    const choice = choices[index];
    if (!choice || choice.disabled) return;
    setOpen(false);
    if (index !== selected) onChange({ target: { value: choice.value } });
  };
  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.altKey || event.ctrlKey || event.metaKey) return;
    if (!open) {
      if (['ArrowDown', 'ArrowUp', 'Enter', ' '].includes(event.key)) { event.preventDefault(); show(); }
      return;
    }
    switch (event.key) {
      case 'ArrowDown': event.preventDefault(); setActive(nextChoice(choices, active, 1)); break;
      case 'ArrowUp': event.preventDefault(); setActive(nextChoice(choices, active, -1)); break;
      case 'Home': event.preventDefault(); setActive(nextChoice(choices, -1, 1)); break;
      case 'End': event.preventDefault(); setActive(nextChoice(choices, -1, -1)); break;
      case 'Enter': case ' ': event.preventDefault(); choose(active); break;
      case 'Escape': event.preventDefault(); event.stopPropagation(); setOpen(false); break;
      case 'Tab': setOpen(false); break;
      default:
        if (event.key.length === 1) {
          // Typing picks the next choice that starts with what was typed in the last moment.
          const now = Date.now();
          typed.current = { text: (now - typed.current.at < 700 ? typed.current.text : '') + event.key.toLowerCase(), at: now };
          const from = active < 0 ? 0 : active;
          const order = [...choices.keys()].map(offset => (from + offset + (typed.current.text.length === 1 ? 1 : 0)) % choices.length);
          const hit = order.find(index => !choices[index].disabled && choices[index].label.toLowerCase().startsWith(typed.current.text));
          if (hit !== undefined) setActive(hit);
        }
    }
  };
  return <div className={`select-field${disabled ? ' disabled' : ''}${open ? ' open' : ''}`}>
    <button type="button" id={triggerId} className="select-trigger" role="combobox" disabled={disabled} aria-haspopup="listbox" aria-expanded={open} aria-controls={listId}
      aria-activedescendant={open && active >= 0 ? optionId(active) : undefined} aria-describedby={describedBy} onFocus={onFocus}
      onClick={() => (open ? setOpen(false) : show())} onBlur={() => setOpen(false)} onKeyDown={onKeyDown}>
      <span className="select-value">{(choices[selected] ?? choices[0])?.label ?? ''}</span>
    </button>
    <ChevronDown size={17} aria-hidden="true" />
    <ul id={listId} ref={list} className="select-list" role="listbox" hidden={!open} onMouseDown={event => event.preventDefault()}>
      {choices.map((choice, index) => <li key={`${index}:${choice.value}`} id={optionId(index)} role="option" data-value={choice.value} aria-selected={index === selected} aria-disabled={choice.disabled || undefined}
        className={`select-option${index === active ? ' active' : ''}`} onMouseEnter={() => !choice.disabled && setActive(index)} onClick={() => choose(index)}>
        <span className="select-option-label">{choice.label}</span>{index === selected && <Check size={15} aria-hidden="true" />}
      </li>)}
    </ul>
  </div>;
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
