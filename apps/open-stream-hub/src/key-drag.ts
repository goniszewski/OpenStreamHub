// The keypad tile drag payload: where a dragged key came from, so a drop can move, swap or
// copy it (with Alt). A custom MIME type, like the action picker's, so unrelated page drags
// (text, images) are ignored by the tiles.
import { keypadSlotCount } from './keypad-model';

export const keyDragMime = 'application/x-open-stream-hub-key';

export interface KeyDragSource { path: string[]; control: number }

export function writeKeyDrag(dataTransfer: DataTransfer, source: KeyDragSource): void {
  dataTransfer.setData(keyDragMime, JSON.stringify(source));
  dataTransfer.effectAllowed = 'copyMove';
}

// During dragover, data is unreadable but the offered types are visible.
export function offersKeyDrag(dataTransfer: { types: readonly string[] }): boolean {
  return dataTransfer.types.includes(keyDragMime);
}

export function readKeyDrag(dataTransfer: { getData(name: string): string }): KeyDragSource | null {
  try {
    const value: unknown = JSON.parse(dataTransfer.getData(keyDragMime));
    if (!value || typeof value !== 'object') return null;
    const { path, control } = value as { path?: unknown; control?: unknown };
    if (!Array.isArray(path) || !path.every(item => typeof item === 'string') || path.length === 0) return null;
    if (typeof control !== 'number' || !Number.isInteger(control) || control < 0 || control >= keypadSlotCount) return null;
    return { path: [...path], control };
  } catch { return null; }
}
