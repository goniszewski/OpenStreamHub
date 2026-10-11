import { hint, isMacPlatform } from './shortcuts';
import { useState } from 'react';
import { Search, Trash2, TriangleAlert, X } from 'lucide-react';
import type { Binding, Profile } from './contracts';
import { ContextMenu } from './controls';
import { bindingWarnings, entryOnControl, filterBindings, listBindings, warningsForControl } from './binding-model';
import type { BindingEntry, BindingWarning, SelectedControl, WarningKind } from './binding-model';
import { removeDialBinding } from './dial-function-model';
import { dialFunctionNameId } from './DialFunctionsPanel';
import { focusWhenRendered } from './NoticeRegion';
import { profileBindingCount } from './keypad-model';
import type { ProfileSession } from './profile-session';
import { gestureLabels, controlLabel } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

export const bindingSearchId = 'binding-search';

// Opens the binding in the workspace: the module and control are selected. A dial function's
// binding selects the encoder first so its editor is on screen, then focuses the function's name
// field; a flat encoder binding under dial functions lands on the dial functions editor, which
// offers to move or remove it.
export function showEntry(editor: DeviceEditor, entry: BindingEntry) {
  if (entry.keypad && entry.keypadBinding) editor.selectKeypadBinding(entry.keypad.pageId, entry.control, entry.keypadBinding);
  else if (entry.dialFunction) {
    editor.chooseModule('m131e');
    const index = entry.dialFunction.index;
    focusWhenRendered(() => document.getElementById(dialFunctionNameId(index)));
  }
  else if (entry.flat) editor.selectBinding(entry.flat);
}

function WarningList({ warnings, entries, editor, label, current }: {
  warnings: BindingWarning[]; entries: BindingEntry[]; editor: DeviceEditor; label: string; current?: { control: SelectedControl; gesture: string; serial: string };
}) {
  if (!warnings.length) return null;
  const byId = new Map(entries.map(entry => [entry.id, entry]));
  return <div className="binding-warnings" role="region" aria-label={label}>
    <h4><TriangleAlert size={16} aria-hidden="true" />{label} <span className="small-label">{warnings.length}</span></h4>
    <ul>{warnings.map(warning => {
      // In the inspector, "Show" goes to the other binding involved.
      const targets = warning.entryIds.map(id => byId.get(id)).filter((entry): entry is BindingEntry => entry !== undefined);
      const isCurrent = (entry: BindingEntry) => current !== undefined && entryOnControl(entry, current.control) && entry.gesture === current.gesture && entry.serial === current.serial;
      const target = targets.find(entry => !isCurrent(entry)) ?? targets[0];
      return <li key={warning.id}><span>{warning.message}</span>
        {target && <button type="button" className="button compact" onClick={() => showEntry(editor, target)}>Show</button>}</li>;
    })}</ul>
  </div>;
}

// A note in the Inspector listing the warnings that involve the selected control. `excludeKinds`
// drops warnings another editor renders better: while the dial functions editor is open below,
// its own block names the flat encoder bindings and carries the Move action.
export function ControlWarnings({ profile, editor, selected, excludeKinds }: { profile: Profile; editor: DeviceEditor; selected: SelectedControl; excludeKinds?: readonly WarningKind[] }) {
  const entries = listBindings(profile);
  const warnings = warningsForControl(bindingWarnings(profile), entries, selected)
    .filter(warning => !excludeKinds?.includes(warning.kind));
  return <WarningList warnings={warnings} entries={entries} editor={editor} label="Binding warnings" current={{ control: selected, gesture: editor.selection.gesture, serial: editor.selection.serial }} />;
}

export function BindingsPanel({ profile, session, editor, disabled, initialQuery = '' }: {
  profile: Profile; session: ProfileSession; editor: DeviceEditor; disabled: boolean; initialQuery?: string;
}) {
  const [query, setQuery] = useState(initialQuery);
  // The right-click menu of one row, named by the row's stable entry id (a fresh profile may have
  // moved or dropped it by the time the menu item is clicked).
  const [rowMenu, setRowMenu] = useState<{ id: string; x: number; y: number } | null>(null);
  const entries = listBindings(profile);
  const shown = filterBindings(entries, query);
  const warnings = bindingWarnings(profile);
  const searching = query.trim() !== '';
  const remove = (entry: BindingEntry) => {
    if (entry.keypad && entry.keypadBinding) editor.removeKeypadBinding(entry.keypad.pageId, entry.control, entry.keypadBinding);
    else if (entry.dialFunction) { const index = entry.dialFunction.index; session.update(latest => removeDialBinding(latest, index, entry.index)); }
    else session.update(latest => ({ ...latest, bindings: latest.bindings.filter((_, position) => position !== entry.index) }));
  };
  // Copying a row copies the whole key (label, bindings, artwork, folder subtree) or the one
  // binding; dial function bindings copy like flat encoder bindings.
  const copyEntry = (entry: BindingEntry) => {
    if (entry.keypad && entry.keypadBinding) editor.copyKeypadKey(entry.keypad.pageId, entry.control);
    else if (entry.dialBinding) editor.copyBinding({ module: 'm131e', control: 0, gesture: entry.gesture, ...(entry.serial ? { serial_number: entry.serial } : {}), action: entry.dialBinding.action } as Binding);
    else if (entry.flat) editor.copyBinding(entry.flat);
  };
  const menuEntry = rowMenu ? entries.find(item => item.id === rowMenu.id) : undefined;
  return <>
    {profileBindingCount(profile) > 0 && <div className="binding-search" role="search" data-shortcut={hint('F', isMacPlatform(typeof navigator === 'undefined' ? undefined : navigator))}>
      <Search size={16} aria-hidden="true" />
      <label className="sr-only" htmlFor={bindingSearchId}>Search bindings</label>
      <input id={bindingSearchId} type="search" autoComplete="off" spellCheck={false} placeholder="Search bindings" value={query} aria-describedby="binding-search-count"
        onChange={event => setQuery(event.target.value)} onKeyDown={event => { if (event.key === 'Escape' && query) { event.preventDefault(); setQuery(''); } }} />
      {query && <button type="button" className="icon-button" aria-label="Clear search" title="Clear search" onClick={() => setQuery('')}><X size={16} /></button>}
      <span id="binding-search-count" className="small-label" role="status" aria-live="polite">{searching ? `${shown.length} of ${entries.length} bindings` : `${entries.length} bindings`}</span>
    </div>}
    <WarningList warnings={warnings} entries={entries} editor={editor} label="Binding warnings" />
    {shown.length > 0 && <div className="binding-list">{shown.map(entry => <div key={entry.id} className="binding-row"
      onContextMenu={event => { event.preventDefault(); setRowMenu({ id: entry.id, x: event.clientX, y: event.clientY }); }}><div>
      <strong>{entry.title}</strong><span>{entry.actionText}</span><small>{entry.serial ? `Serial: ${entry.serial}` : 'Any device serial'}</small></div>
      <button className="button compact" onClick={() => showEntry(editor, entry)}>Edit</button>
      <button className="icon-button" aria-label={`Remove ${entry.keypad ? `${entry.keypad.pageName} Key ${entry.control + 1}` : entry.dialFunction ? `Encoder ${entry.dialFunction.name}` : controlLabel(entry.module, entry.control)} ${gestureLabels[entry.gesture]} binding`} disabled={disabled} onClick={() => remove(entry)}><Trash2 size={17} /></button>
    </div>)}</div>}
    {menuEntry && rowMenu && <ContextMenu x={rowMenu.x} y={rowMenu.y} label="Binding menu" onClose={() => setRowMenu(null)}
      items={[{ label: 'Copy', onSelect: () => copyEntry(menuEntry) }, { label: 'Remove', danger: true, disabled, onSelect: () => remove(menuEntry) }]} />}
    {entries.length === 0 && <p className="empty-state">No bindings yet. Select a module and choose an action to add one.</p>}
    {entries.length > 0 && shown.length === 0 && <p className="empty-state" role="status">No bindings match “{query.trim()}”. <button type="button" className="button compact" onClick={() => setQuery('')}>Clear search</button></p>}
  </>;
}
