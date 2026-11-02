import { useEffect, useId, useRef, useState } from 'react';
import { ListChecks } from 'lucide-react';
import type { ModuleAssignments as Assignments } from './binding-model';
import { controlLabel, gestureLabels } from './ui-model';

// "3 of 15 keys assigned" for the module on show. Pressing it opens a small list of every hardware trigger that has an
// action and the action attached to it, so what a module does can be read at a glance without opening each control.
export function ModuleAssignmentsChip({ assignments, moduleName }: { assignments: Assignments; moduleName: string }) {
  const { assigned, total, noun, entries, folders } = assignments;
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const panelId = useId();
  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) setOpen(false); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open]);
  const rows = [
    ...entries.map(entry => ({ key: entry.id, control: entry.control, trigger: `${entry.dialFunction ? `${entry.dialFunction.name} · ` : ''}${controlLabel(entry.module, entry.control)} · ${gestureLabels[entry.gesture]}`, action: entry.actionText })),
    ...folders.map(folder => ({ key: `folder:${folder.control}`, control: folder.control, trigger: controlLabel('m4315', folder.control), action: `Opens folder “${folder.name}”` })),
  ].sort((a, b) => a.control - b.control);
  return <div className="assignments" ref={root} onKeyDown={event => { if (event.key === 'Escape' && open) { event.stopPropagation(); setOpen(false); (root.current?.querySelector('button') as HTMLElement | null)?.focus(); } }}>
    <button type="button" className={`button compact assignments-chip${assigned === 0 ? ' none' : ''}`} aria-haspopup="dialog" aria-expanded={open} aria-controls={panelId} onClick={() => setOpen(!open)}>
      <ListChecks size={15} aria-hidden="true" />{assigned} of {total} {noun} assigned
    </button>
    {open && <div id={panelId} className="assignments-panel" role="dialog" aria-label={`Actions assigned on ${moduleName}`}>
      {entries.length + folders.length === 0
        ? <p className="fine-print">Nothing is assigned on this module yet. Pick a control and choose an action for it.</p>
        : <ul className="assignments-list">
          {rows.map(row => <li key={row.key}>
            <span className="assignments-trigger">{row.trigger}</span>
            <span className="assignments-action">{row.action}</span>
          </li>)}
        </ul>}
    </div>}
  </div>;
}
