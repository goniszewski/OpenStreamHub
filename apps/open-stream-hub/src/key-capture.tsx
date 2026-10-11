// The key chooser and the "Press keys…" capture field of the shortcut editor.
// Like the action picker: a pure, controlled picker (renderable and callable as
// a function in tests) plus thin stateful field wrappers.
import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Check, ChevronDown } from 'lucide-react';
import type { Modifier } from './contracts';
import { chordFromEvent, chordLabel, heldLabel, keyCatalog, searchKeys } from './key-capture-model';
import type { CaptureEvent, Chord } from './key-capture-model';
import { isMediaKey } from './action-model';

export interface KeyPickerState { open: boolean; query: string }
export const closedKeyPicker: KeyPickerState = { open: false, query: '' };

// How many key-picker popovers are open anywhere. The count is a plain
// number, so it is a stable snapshot for useSyncExternalStore (the editing
// hold of the run mode reads it, exactly like the action pickers).
let openKeyPickers = 0;
const keyPickerListeners = new Set<() => void>();
const notifyKeyPickerListeners = () => keyPickerListeners.forEach(listener => listener());
export const subscribeOpenKeyPickers = (listener: () => void) => {
  keyPickerListeners.add(listener);
  return () => { keyPickerListeners.delete(listener); };
};
export const readOpenKeyPickers = (): number => openKeyPickers;

// The key chooser: a trigger naming the chosen key and a popover with a search
// box and the catalog grouped (letters and digits, keys, function keys,
// keypad, media keys). Media entries render only when includeMedia; macros
// cannot hold them. `triggerLabel` overrides the trigger text, so the same
// picker also serves as an "Add keystroke" chooser.
export function KeyPicker({ state, onStateChange, triggerId, value, onPick, includeMedia = true, disabled = false, triggerLabel }: {
  state: KeyPickerState; onStateChange(next: KeyPickerState): void; triggerId: string; value: string;
  onPick(key: string): void; includeMedia?: boolean; disabled?: boolean; triggerLabel?: string;
}) {
  const searchId = `${triggerId}-search`;
  const groups = keyCatalog().map(group => ({ group, entries: searchKeys(state.query, group) }))
    .filter(({ group, entries }) => entries.length > 0 && (includeMedia || !group.media));
  // Returns focus to the trigger after a popover-initiated close, so keyboard
  // users are not left on the page body; an outside press closes without it.
  const close = () => {
    onStateChange(closedKeyPicker);
    if (typeof document !== 'undefined') document.getElementById(triggerId)?.focus();
  };
  const panelKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === 'Escape') { event.preventDefault(); close(); }
  };
  return <span className="action-picker">
    <button type="button" id={triggerId} className="action-picker-trigger" aria-haspopup="dialog" aria-expanded={state.open} disabled={disabled}
      title={triggerLabel ?? 'Choose a key'} onClick={() => { if (state.open) close(); else onStateChange({ open: true, query: '' }); }}>
      <span className="action-picker-trigger-label">{triggerLabel ?? chordLabel({ key: value, modifiers: [] })}</span>
      <ChevronDown size={17} aria-hidden="true" />
    </button>
    {state.open && <div className="action-picker-panel" role="dialog" aria-label="Choose a key" onKeyDown={panelKeyDown}>
      <label className="sr-only" htmlFor={searchId}>Search keys</label>
      <input id={searchId} type="search" placeholder="Search keys" autoComplete="off" autoFocus value={state.query}
        onChange={event => onStateChange({ ...state, query: event.target.value })} />
      {groups.length === 0 ? <p className="picker-empty" role="status">No key matches “{state.query}”.</p> : groups.map(({ group, entries }) => <section key={group.id} aria-labelledby={`${triggerId}-key-group-${group.id}`}>
        <p className="picker-group-label" id={`${triggerId}-key-group-${group.id}`}>{group.label}</p>
        <div className="picker-entries">{entries.map(entry => <button key={entry.key} type="button"
          className={`picker-entry${entry.key === value ? ' current' : ''}`} data-key-entry={entry.key}
          aria-current={entry.key === value || undefined}
          onClick={() => { onPick(entry.key); close(); }}>
          {entry.key === value && <Check size={15} aria-hidden="true" />}
          <span>{entry.label}</span>
        </button>)}</div>
      </section>)}
    </div>}
  </span>;
}

// The picker for one field, owning its open state and closing when a press
// lands outside it.
export function KeyPickerField(props: Omit<Parameters<typeof KeyPicker>[0], 'state' | 'onStateChange'>) {
  const [state, setState] = useState<KeyPickerState>(closedKeyPicker);
  const wrapper = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!state.open) return;
    openKeyPickers += 1;
    notifyKeyPickerListeners();
    const close = (event: PointerEvent) => { if (!wrapper.current?.contains(event.target as Node)) setState(closedKeyPicker); };
    document.addEventListener('pointerdown', close);
    return () => { openKeyPickers -= 1; notifyKeyPickerListeners(); document.removeEventListener('pointerdown', close); };
  }, [state.open]);
  return <span className="action-picker-field" ref={wrapper}>
    <KeyPicker {...props} state={state} onStateChange={setState} />
  </span>;
}

// The chord capture field: a read-only input that listens while focused
// ("Press keys…"), shows the chord it holds, and says why a pressed key was
// refused. Escape cancels; a complete chord ends the capture. The browser and
// the app's own shortcuts stay out through preventDefault + stopPropagation,
// so a captured Command+S is captured, not a save. The key picker sits
// alongside, so every key is choosable without pressing it.
export function HotkeyField({ id, value, modifiers, disabled, onChange }: {
  id: string; value: string; modifiers: Modifier[]; disabled?: boolean; onChange(chord: Chord): void;
}) {
  const [capturing, setCapturing] = useState(false);
  const [held, setHeld] = useState<Modifier[]>([]);
  const [note, setNote] = useState<string | null>(null);
  const read = (event: ReactKeyboardEvent<HTMLInputElement>): CaptureEvent => ({
    key: event.key, code: event.code, metaKey: event.metaKey, ctrlKey: event.ctrlKey, altKey: event.altKey, shiftKey: event.shiftKey,
  });
  const keyDown = (event: ReactKeyboardEvent<HTMLInputElement>) => {
    if (!capturing) return;
    event.preventDefault();
    event.stopPropagation();
    if (event.key === 'Escape') {
      setCapturing(false);
      setHeld([]);
      return;
    }
    const result = chordFromEvent(read(event));
    if ('pending' in result) {
      setHeld(result.held);
      return;
    }
    setHeld([]);
    setCapturing(false);
    if ('unsupported' in result) {
      setNote(`“${result.unsupported}” is not a key OpenStreamHub can send. Press another key or choose one from the list.`);
      return;
    }
    setNote(null);
    onChange(result.chord);
  };
  const display = capturing && held.length ? heldLabel(held) : chordLabel({ key: value, modifiers });
  return <span className="hotkey-field">
    <input id={id} className="hotkey-input" type="text" readOnly value={display} disabled={disabled}
      placeholder="Press keys…" aria-describedby={note ? `${id}-note` : undefined}
      onFocus={() => { if (!disabled) { setCapturing(true); setNote(null); } }}
      onBlur={() => { setCapturing(false); setHeld([]); }}
      onKeyDown={keyDown} />
    <KeyPickerField triggerId={`${id}-choose`} value={value} includeMedia disabled={disabled}
      onPick={key => { setNote(null); onChange({ key, modifiers: isMediaKey(key) ? [] : modifiers }); }} />
    {note && <p id={`${id}-note`} className="field-warning" role="note">{note}</p>}
  </span>;
}
