import { useEffect, useRef, useState } from 'react';
import { ChevronDown, History } from 'lucide-react';
import { translateIn, useT } from './i18n-react';
import type { Translate } from './i18n-react';

export const historyTipStorageKey = 'open-stream-hub.history-tip';

// The one-time explanation in the list: shown until the list has been opened once. Storage may be missing or refuse
// access; the tip then shows every time rather than never.
export function readHistoryTipShown(storage: Pick<Storage, 'getItem'> | null): boolean {
  try { return storage?.getItem(historyTipStorageKey) === 'shown'; } catch { return false; }
}

export function markHistoryTipShown(storage: Pick<Storage, 'setItem'> | null): void {
  try { storage?.setItem(historyTipStorageKey, 'shown'); } catch { /* the tip may show again next time */ }
}

// The list the popover shows: redo entries above the current state, undo entries below it, each the label of a change.
// Picking an undo entry goes back to just before that change, a redo entry forward through it.
export function HistoryList({ undo, redo, onUndo, onRedo, undoHint, redoHint, showTip, t = translateIn('en') }: {
  undo: string[]; redo: string[]; onUndo(steps: number): void; onRedo(steps: number): void;
  undoHint: string; redoHint: string; showTip: boolean;
  // Hook-free so tests can call it as a function; the menu passes its own translator.
  t?: Translate;
}) {
  return <div className="history-list">
    {showTip && <p className="history-tip" role="note">{t('history.tip', { undoKey: undoHint, redoKey: redoHint })}</p>}
    {redo.length > 0 && <>
      <h3>{t('history.redo')}</h3>
      <ol className="history-entries redo" aria-label={t('history.redoList')}>
        {redo.map((label, index) => <li key={`r${index}`}><button type="button" className="history-entry" onClick={() => onRedo(index + 1)}>{label}</button></li>)}
      </ol>
    </>}
    <h3>{t('history.undo')}</h3>
    {undo.length > 0
      ? <ol className="history-entries" aria-label={t('history.undoList')}>
        {undo.map((label, index) => <li key={`u${index}`}><button type="button" className="history-entry" onClick={() => onUndo(index + 1)}>{label}</button></li>)}
      </ol>
      : <p className="fine-print">{t('history.nothing')}</p>}
  </div>;
}

// The header's history button: opens the list of recent changes. Undo and redo themselves stay as their own buttons
// beside it, with their shortcuts, so the list adds a way to see and jump rather than replacing them.
export function HistoryMenu({ undo, redo, disabled, onUndo, onRedo, undoHint, redoHint, storage }: {
  undo: string[]; redo: string[]; disabled: boolean; onUndo(steps: number): void; onRedo(steps: number): void;
  undoHint: string; redoHint: string; storage: (Pick<Storage, 'getItem'> & Pick<Storage, 'setItem'>) | null;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  const [tip, setTip] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent | KeyboardEvent) => {
      if (event instanceof KeyboardEvent) { if (event.key === 'Escape') setOpen(false); return; }
      if (root.current && !root.current.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener('mousedown', close);
    document.addEventListener('keydown', close);
    return () => { document.removeEventListener('mousedown', close); document.removeEventListener('keydown', close); };
  }, [open]);
  const toggle = () => {
    if (!open) { setTip(!readHistoryTipShown(storage)); markHistoryTipShown(storage); }
    setOpen(value => !value);
  };
  const count = undo.length + redo.length;
  return <div className="history-menu" ref={root}>
    <button type="button" className="icon-button" disabled={disabled} aria-label={t('history.button')} aria-expanded={open} aria-haspopup="dialog"
      data-tip={open ? undefined : count ? t('history.buttonTip') : t('history.buttonTipEmpty')} onClick={toggle}>
      <History size={17} aria-hidden="true" /><ChevronDown size={11} aria-hidden="true" />
    </button>
    {open && <div className="history-popover panel" role="dialog" aria-label={t('history.button')}>
      <HistoryList undo={undo} redo={redo} undoHint={undoHint} redoHint={redoHint} showTip={tip} t={t}
        onUndo={steps => { onUndo(steps); setOpen(false); }} onRedo={steps => { onRedo(steps); setOpen(false); }} />
    </div>}
  </div>;
}
