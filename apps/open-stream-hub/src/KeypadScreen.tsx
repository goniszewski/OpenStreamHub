import { ChevronRight } from 'lucide-react';
import { KeyDisplayContent } from './DisplayContent';
import type { KeypadPage } from './contracts';
import { keypadDepth, keypadIsBack } from './keypad-model';

interface Props {
  pages: KeypadPage[]; allPages: KeypadPage[]; path: string[]; selected: number;
  onSelect(control: number): void; onNavigate(path: string[]): void;
}

export function KeypadScreen({ pages, allPages, path, selected, onSelect, onNavigate }: Props) {
  const page = pages.at(-1)!;
  const depth = keypadDepth(path);
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
        return <button key={control} className={`keypad-tile${selected === control ? ' selected' : ''}${back ? ' back' : ''}${slot?.type === 'folder' ? ' folder' : ''}`}
          aria-label={back ? `Back one level, L${depth}` : slot?.type === 'folder' ? `Open folder ${folderName}, Key ${control + 1}` : `Select Key ${control + 1}${slot?.type === 'action' ? `, ${slot.label}` : ', empty'}`}
          aria-pressed={!back && slot?.type !== 'folder' ? selected === control : undefined}
          onClick={() => back ? onNavigate(path.slice(0, -1)) : slot?.type === 'folder' ? onNavigate([...path, slot.page_id]) : onSelect(control)}>
          {!back && <span className="keypad-key-number">{control + 1}</span>}
          <KeyDisplayContent slot={slot} pages={allPages} depth={depth} back={back} />
        </button>;
      })}
    </div>
    <p className="fine-print">Screen concept · hardware display uploads remain unavailable.</p>
  </section>;
}
