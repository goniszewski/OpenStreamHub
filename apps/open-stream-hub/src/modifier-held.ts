import { useEffect } from 'react';

// Marks the document while the shortcut modifier (Command on a Mac, Control elsewhere) is held, so the controls that
// have a shortcut can show it as a chip. Losing focus clears it, since the key-up would go to another window.
export function useModifierHeld(mac: boolean, root: HTMLElement | undefined = typeof document === 'undefined' ? undefined : document.documentElement) {
  useEffect(() => {
    if (!root) return;
    const attribute = 'data-mod-held';
    const set = (held: boolean) => { if (held) root.setAttribute(attribute, ''); else root.removeAttribute(attribute); };
    const key = mac ? 'Meta' : 'Control';
    const down = (event: KeyboardEvent) => { if (event.key === key) set(true); };
    const up = (event: KeyboardEvent) => { if (event.key === key) set(false); };
    const clear = () => set(false);
    window.addEventListener('keydown', down);
    window.addEventListener('keyup', up);
    window.addEventListener('blur', clear);
    return () => { window.removeEventListener('keydown', down); window.removeEventListener('keyup', up); window.removeEventListener('blur', clear); set(false); };
  }, [mac, root]);
}
