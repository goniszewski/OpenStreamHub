import { ChevronRight } from 'lucide-react';
import type { DragEvent } from 'react';
import { KeyDisplayContent } from './DisplayContent';
import { isValidForControl, offersActionDrag, readActionDrag } from './action-picker-model';
import type { PickerActionType } from './action-picker-model';
import type { KeypadPage } from './contracts';
import { offersKeyDrag, readKeyDrag, writeKeyDrag } from './key-drag';
import { keypadDepth, keypadIsBack } from './keypad-model';

interface Props {
  pages: KeypadPage[]; allPages: KeypadPage[]; path: string[]; selected: number;
  onSelect(control: number): void; onNavigate(path: string[]): void;
  // When set, keys accept a dragged picker entry (never the reserved Back key).
  onDropAction?(control: number, type: PickerActionType): void;
  // When set, occupied keys can be dragged to another key (a move, a swap onto content, with Alt
  // a copy onto an empty key) and right-click opens the Copy/Paste/Clear menu.
  onMoveKey?(source: { path: string[]; control: number }, to: { path: string[]; control: number }, copy: boolean): void;
  onKeyContext?(control: number, at: { x: number; y: number }): void;
}

export function KeypadScreen({ pages, allPages, path, selected, onSelect, onNavigate, onDropAction, onMoveKey, onKeyContext }: Props) {
  const page = pages.at(-1)!;
  const depth = keypadDepth(path);
  const acceptsDrop = (control: number) => (event: DragEvent<HTMLElement>) => {
    if (offersKeyDrag(event.dataTransfer)) {
      if (!onMoveKey) return;
      event.preventDefault();
      event.dataTransfer.dropEffect = event.altKey ? 'copy' : 'move';
      return;
    }
    if (onDropAction && offersActionDrag(event.dataTransfer)) event.preventDefault();
  };
  const acceptDrop = (control: number) => (event: DragEvent<HTMLElement>) => {
    if (offersKeyDrag(event.dataTransfer)) {
      const source = readKeyDrag(event.dataTransfer);
      if (onMoveKey && source) { event.preventDefault(); onMoveKey(source, { path, control }, event.altKey); }
      return;
    }
    if (!onDropAction) return;
    const type = readActionDrag(event.dataTransfer);
    if (!type || !isValidForControl(type, 'm4315')) return;
    event.preventDefault();
    onDropAction(control, type);
  };
  return <section className="keypad-screen" aria-label="Keypad screen preview">
    <nav className="keypad-breadcrumbs" aria-label="Keypad folders">
      {/* The root page is the first crumb; no separate profile-name crumb. */}
      {pages.map((item, index) => <span key={item.id}>{index > 0 && <ChevronRight size={13} aria-hidden="true" />}<button aria-current={index === pages.length - 1 ? 'page' : undefined} onClick={() => onNavigate(path.slice(0, index + 1))}>{item.name}</button></span>)}
    </nav>
    <div className="keypad-tiles" role="group" aria-label={`${page.name} keys`}>
      {page.slots.map((slot, control) => {
        const back = keypadIsBack(path, control);
        const target = slot?.type === 'folder' ? allPages.find(item => item.id === slot.page_id) : undefined;
        const folderName = slot?.type === 'folder' ? target?.name ?? slot.page_id : '';
        const draggable = !back && !!onMoveKey && slot !== null;
        const dropProps = !back && (onDropAction || onMoveKey) ? { onDragOver: acceptsDrop(control), onDrop: acceptDrop(control) } : {};
        return <button key={control} className={`keypad-tile${selected === control ? ' selected' : ''}${back ? ' back' : ''}${slot?.type === 'folder' ? ' folder' : ''}`}
          aria-label={back ? `Back one level, L${depth}` : slot?.type === 'folder' ? `Open folder ${folderName}, Key ${control + 1}` : `Select Key ${control + 1}${slot?.type === 'action' ? `, ${slot.label}` : ', empty'}`}
          aria-pressed={!back && slot?.type !== 'folder' ? selected === control : undefined}
          onClick={() => back ? onNavigate(path.slice(0, -1)) : slot?.type === 'folder' ? onNavigate([...path, slot.page_id]) : onSelect(control)}
          draggable={draggable || undefined}
          onDragStart={draggable ? event => writeKeyDrag(event.dataTransfer, { path, control }) : undefined}
          onContextMenu={onKeyContext && !back ? event => { event.preventDefault(); onKeyContext(control, { x: event.clientX, y: event.clientY }); } : undefined}
          {...dropProps}>
          {!back && <span className="keypad-key-number">{control + 1}</span>}
          <KeyDisplayContent key={page.id} slot={slot} pages={allPages} depth={depth} back={back} artwork={page.artwork?.[String(control)]} />
        </button>;
      })}
    </div>
    <p className="fine-print">Artwork follows the active page while OpenStreamHub drives the display.</p>
  </section>;
}
