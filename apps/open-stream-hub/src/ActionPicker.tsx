import { useEffect, useRef, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';
import { Check, ChevronDown, Trash2 } from 'lucide-react';
import type { BindingAction } from './contracts';
import {
  entriesForScope, pickerEntry, pickerEntryIsCurrent, pickerEntryLabel, pickerGroups, pickerWebStorage, readRecentActions, recordRecentAction, writeActionDrag,
} from './action-picker-model';
import type { PickerActionType, PickerEntry, PickerScope, StorageLike } from './action-picker-model';

// Open state of one picker: the popover is closed, or open with the current search text.
export interface PickerState { open: boolean; query: string }
export const closedPicker: PickerState = { open: false, query: '' };

// Moves keyboard focus among the panel's action entries, wrapping at both ends. With no entry
// focused (the search box has it), ArrowDown moves to the first entry and ArrowUp to the last.
export function cycleEntryFocus(root: ParentNode, active: Element | null, delta: 1 | -1): void {
  const entries = [...root.querySelectorAll<HTMLButtonElement>('[data-action-entry]')];
  if (!entries.length) return;
  const index = active ? entries.indexOf(active as HTMLButtonElement) : delta === 1 ? -1 : 0;
  entries[(index + delta + entries.length) % entries.length]?.focus();
}

type Assign = (type: PickerActionType | null) => void;

// Returns focus to the trigger after a popover-initiated close (an entry choice, Remove action,
// Escape, or a trigger toggle), so keyboard users are not left on the page body. An outside
// press closes without refocusing: the user's click decides where focus belongs.
export function focusTrigger(triggerId: string) {
  if (typeof document === 'undefined') return;
  document.getElementById(triggerId)?.focus();
}

// The action chooser that replaces the flat action select: a trigger showing the bound action and
// a popover with a search box, the recently used actions and the catalogue grouped by category.
// Entries assign on click and carry a drag payload for dropping onto a control. Fully controlled,
// so it renders and interacts identically wherever a type is chosen.
export function ActionPicker({ state, onStateChange, scope, action, triggerId, idPrefix = '', onAssign, recent, storage = pickerWebStorage, allowDrag = true }: {
  state: PickerState; onStateChange(next: PickerState): void; scope: PickerScope; action: BindingAction | null;
  triggerId: string; idPrefix?: string; onAssign: Assign; recent: PickerActionType[]; storage?: StorageLike; allowDrag?: boolean;
}) {
  const currentType = action?.type as PickerActionType | undefined;
  const searchId = `${triggerId}-search`;
  const validTypes = new Set(entriesForScope(scope).map(entry => entry.type));
  const recentEntries = recent.filter(type => validTypes.has(type));
  const groups = pickerGroups(state.query, scope);

  const close = () => { focusTrigger(triggerId); onStateChange(closedPicker); };

  const assign = (type: PickerActionType | null) => {
    if (type) recordRecentAction(storage, type);
    onAssign(type);
    close();
  };

  const entryButton = (entry: PickerEntry) => <button key={entry.type} type="button" className={`picker-entry${pickerEntryIsCurrent(entry.type, currentType) ? ' current' : ''}`}
    data-action-entry="" data-action-type={entry.type} draggable={allowDrag} aria-current={pickerEntryIsCurrent(entry.type, currentType) || undefined}
    onClick={() => assign(entry.type)}
    onDragStart={allowDrag ? event => writeActionDrag(event.dataTransfer, entry.type) : undefined}>
    {pickerEntryIsCurrent(entry.type, currentType) && <Check size={15} aria-hidden="true" />}
    <span>{entry.label}</span>
  </button>;

  const panelKeyDown = (event: ReactKeyboardEvent<HTMLElement>) => {
    if (event.key === 'Escape') { event.preventDefault(); close(); return; }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      cycleEntryFocus(event.currentTarget, typeof document === 'undefined' ? null : document.activeElement, event.key === 'ArrowDown' ? 1 : -1);
    }
  };

  return <span className="action-picker">
    <button type="button" id={triggerId} className="action-picker-trigger" aria-haspopup="dialog" aria-expanded={state.open}
      title={currentType ? pickerEntryLabel(currentType) : 'Choose an action'}
      onClick={() => { if (state.open) close(); else onStateChange({ open: true, query: '' }); }}>
      <span className="action-picker-trigger-label">{currentType ? pickerEntryLabel(currentType) : 'Choose an action'}</span>
      <ChevronDown size={17} aria-hidden="true" />
    </button>
    {state.open && <div className="action-picker-panel" role="dialog" aria-label="Choose an action" onKeyDown={panelKeyDown}>
      <label className="sr-only" htmlFor={searchId}>Search actions</label>
      <input id={searchId} type="search" placeholder="Search actions" autoComplete="off" autoFocus value={state.query}
        onChange={event => onStateChange({ ...state, query: event.target.value })} />
      {groups.length === 0 ? <p className="picker-empty" role="status">No action matches “{state.query}”.</p> : <>
        {!state.query.trim() && recentEntries.length > 0 && <section className="picker-group recent" aria-label="Recently used actions">
          <p className="picker-group-label">Recent</p>
          <div className="picker-entries">{recentEntries.map(type => entryButton(pickerEntry(type)!))}</div>
        </section>}
        {groups.map(group => <section key={group.category} aria-labelledby={`${idPrefix}action-group-${group.category}`}>
          <p className="picker-group-label" id={`${idPrefix}action-group-${group.category}`}>{group.label}</p>
          <div className="picker-entries">{group.entries.map(entryButton)}</div>
        </section>)}
      </>}
      {action && scope !== 'step' && <button type="button" className="picker-unbind" data-action-type="unbound"
        onClick={() => assign(null)}><Trash2 size={15} aria-hidden="true" />Remove action</button>}
    </div>}
  </span>;
}

// The picker for one field, owning its open state and closing when a press lands outside it.
export function ActionPickerField(props: Omit<Parameters<typeof ActionPicker>[0], 'state' | 'onStateChange' | 'recent'>) {
  const [state, setState] = useState<PickerState>(closedPicker);
  const wrapper = useRef<HTMLSpanElement>(null);
  useEffect(() => {
    if (!state.open) return;
    const close = (event: PointerEvent) => { if (!wrapper.current?.contains(event.target as Node)) setState(closedPicker); };
    document.addEventListener('pointerdown', close);
    return () => document.removeEventListener('pointerdown', close);
  }, [state.open]);
  return <span className="action-picker-field" ref={wrapper}>
    <ActionPicker {...props} state={state} onStateChange={setState} recent={state.open ? readRecentActions(pickerWebStorage) : []} />
  </span>;
}
