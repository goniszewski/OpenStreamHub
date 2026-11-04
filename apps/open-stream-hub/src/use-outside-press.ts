import { useEffect } from 'react';
import type { RefObject } from 'react';

// While `active`, a press outside `root` calls `close`. A button is not always focused by a click (WebKit), so losing focus
// alone cannot be relied on to close a popup.
export function useOutsidePress(root: RefObject<HTMLElement | null>, active: boolean, close: () => void) {
  useEffect(() => {
    if (!active) return;
    const outside = (event: PointerEvent) => { if (!root.current?.contains(event.target as Node)) close(); };
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [active]); // eslint-disable-line react-hooks/exhaustive-deps
}
