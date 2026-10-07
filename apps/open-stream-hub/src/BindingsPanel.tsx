import { useState } from 'react';
import { Search, Trash2, TriangleAlert, X } from 'lucide-react';
import type { Profile } from './contracts';
import { bindingWarnings, entryOnControl, filterBindings, listBindings, warningsForControl } from './binding-model';
import type { BindingEntry, BindingWarning, SelectedControl } from './binding-model';
import { removeDialBinding } from './dial-function-model';
import { focusDialFunction } from './DialFunctionsPanel';
import { profileBindingCount } from './keypad-model';
import type { ProfileSession } from './profile-session';
import { gestureLabels, controlLabel } from './ui-model';
import type { DeviceEditor } from './useDeviceEditor';

export const bindingSearchId = 'binding-search';

// Opens the binding in the Devices editor.
export function showEntry(editor: DeviceEditor, entry: BindingEntry) {
  if (entry.keypad && entry.keypadBinding) editor.selectKeypadBinding(entry.keypad.pageId, entry.control, entry.keypadBinding);
  else if (entry.dialFunction) focusDialFunction(entry.dialFunction.index);
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

// A note in the Inspector listing the warnings that involve the selected control.
export function ControlWarnings({ profile, editor, selected }: { profile: Profile; editor: DeviceEditor; selected: SelectedControl }) {
  const entries = listBindings(profile);
  const warnings = warningsForControl(bindingWarnings(profile), entries, selected);
  return <WarningList warnings={warnings} entries={entries} editor={editor} label="Binding warnings" current={{ control: selected, gesture: editor.selection.gesture, serial: editor.selection.serial }} />;
}

export function BindingsPanel({ profile, session, editor, disabled, initialQuery = '' }: {
  profile: Profile; session: ProfileSession; editor: DeviceEditor; disabled: boolean; initialQuery?: string;
}) {
  const [query, setQuery] = useState(initialQuery);
  const entries = listBindings(profile);
  const shown = filterBindings(entries, query);
  const warnings = bindingWarnings(profile);
  const searching = query.trim() !== '';
  const remove = (entry: BindingEntry) => {
    if (entry.keypad && entry.keypadBinding) editor.removeKeypadBinding(entry.keypad.pageId, entry.control, entry.keypadBinding);
    else if (entry.dialFunction) { const index = entry.dialFunction.index; session.update(latest => removeDialBinding(latest, index, entry.index)); }
    else session.update(latest => ({ ...latest, bindings: latest.bindings.filter((_, position) => position !== entry.index) }));
  };
  return <>
    {profileBindingCount(profile) > 0 && <div className="binding-search" role="search">
      <Search size={16} aria-hidden="true" />
      <label className="sr-only" htmlFor={bindingSearchId}>Search bindings</label>
      <input id={bindingSearchId} type="search" autoComplete="off" spellCheck={false} placeholder="Search bindings" value={query} aria-describedby="binding-search-count"
        onChange={event => setQuery(event.target.value)} onKeyDown={event => { if (event.key === 'Escape' && query) { event.preventDefault(); setQuery(''); } }} />
      {query && <button type="button" className="icon-button" aria-label="Clear search" title="Clear search" onClick={() => setQuery('')}><X size={16} /></button>}
      <span id="binding-search-count" className="small-label" role="status" aria-live="polite">{searching ? `${shown.length} of ${entries.length} bindings` : `${entries.length} bindings`}</span>
    </div>}
    <WarningList warnings={warnings} entries={entries} editor={editor} label="Binding warnings" />
    {shown.length > 0 && <div className="binding-list">{shown.map(entry => <div key={entry.id} className="binding-row"><div>
      <strong>{entry.title}</strong><span>{entry.actionText}</span><small>{entry.serial ? `Serial: ${entry.serial}` : 'Any device serial'}</small></div>
      <button className="button compact" onClick={() => showEntry(editor, entry)}>Edit</button>
      <button className="icon-button" aria-label={`Remove ${entry.keypad ? `${entry.keypad.pageName} Key ${entry.control + 1}` : entry.dialFunction ? `Encoder ${entry.dialFunction.name}` : controlLabel(entry.module, entry.control)} ${gestureLabels[entry.gesture]} binding`} disabled={disabled} onClick={() => remove(entry)}><Trash2 size={17} /></button>
    </div>)}</div>}
    {entries.length === 0 && <p className="empty-state">No bindings yet. Select a module in Devices and choose an action to add one.</p>}
    {entries.length > 0 && shown.length === 0 && <p className="empty-state" role="status">No bindings match “{query.trim()}”. <button type="button" className="button compact" onClick={() => setQuery('')}>Clear search</button></p>}
  </>;
}
