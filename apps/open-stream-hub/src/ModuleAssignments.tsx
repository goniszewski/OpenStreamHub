import { useId, useRef, useState } from 'react';
import { ListChecks } from 'lucide-react';
import type { ModuleAssignments as Assignments } from './binding-model';
import { useT } from './i18n-react';
import { controlLabel, gestureLabels } from './ui-model';
import { useOutsidePress } from './use-outside-press';

// "3 of 15 keys assigned" for the module on show. Pressing it opens a small list of every hardware trigger that has an
// action and the action attached to it, so what a module does can be read at a glance without opening each control.
export function ModuleAssignmentsChip({ assignments, moduleName }: { assignments: Assignments; moduleName: string }) {
  const t = useT();
  const { assigned, total, noun, entries, folders } = assignments;
  const [open, setOpen] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const panelId = useId();
  useOutsidePress(root, open, () => setOpen(false));
  // Only needed while the list is open.
  const rows = !open ? [] : [
    ...entries.map(entry => ({ key: entry.id, control: entry.control, trigger: `${entry.dialFunction ? `${entry.dialFunction.name} · ` : ''}${controlLabel(entry.module, entry.control)} · ${gestureLabels[entry.gesture]}`, action: entry.actionText })),
    ...folders.map(folder => ({ key: `folder:${folder.control}`, control: folder.control, trigger: controlLabel('m4315', folder.control), action: t('assignments.folder', { name: folder.name }) })),
  ].sort((a, b) => a.control - b.control);
  return <div className="assignments" ref={root} onKeyDown={event => { if (event.key === 'Escape' && open) { event.stopPropagation(); setOpen(false); (root.current?.querySelector('button') as HTMLElement | null)?.focus(); } }}>
    <button type="button" className={`button compact assignments-chip${assigned === 0 ? ' none' : ''}`} aria-haspopup="dialog" aria-expanded={open} aria-controls={panelId} onClick={() => setOpen(!open)}>
      <ListChecks size={15} aria-hidden="true" />{t('assignments.chip', { assigned, total, noun })}
    </button>
    {open && <div id={panelId} className="assignments-panel" role="dialog" aria-label={t('assignments.panel', { module: moduleName })}>
      {entries.length + folders.length === 0
        ? <p className="fine-print">{t('assignments.empty')}</p>
        : <ul className="assignments-list">
          {rows.map(row => <li key={row.key}>
            <span className="assignments-trigger">{row.trigger}</span>
            <span className="assignments-action">{row.action}</span>
          </li>)}
        </ul>}
    </div>}
  </div>;
}
