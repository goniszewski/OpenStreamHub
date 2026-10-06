// Keyboard shortcuts: Cmd on macOS, Ctrl elsewhere. The mapping is pure; the
// App installs it with `installShortcuts`.
import type { Page } from './ui-model';

export type ShortcutAction = { type: 'save' } | { type: 'duplicate' } | { type: 'undo' } | { type: 'redo' } | { type: 'find' } | { type: 'page'; page: Page };

export interface KeyEventLike {
  key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; isComposing?: boolean; keyCode?: number; repeat?: boolean;
  // Where focus was when the key was pressed (the DOM event target).
  target?: unknown;
}

export const shortcutPages: readonly Page[] = ['devices', 'profiles', 'compatibility', 'settings'];

export function isMacPlatform(navigatorLike: { platform?: string; userAgent?: string } | undefined) {
  return /mac/i.test(navigatorLike?.platform || navigatorLike?.userAgent || '');
}

// Inputs whose value the user types, so the browser's own undo applies.
const textInputTypes = new Set(['text', 'search', 'email', 'url', 'tel', 'password', 'number', 'date', 'datetime-local', 'month', 'time', 'week']);

// True when focus is in a field with its own native undo (text input, textarea,
// contenteditable). A select has no native undo, so Cmd+Z still undoes the draft. Undo and redo are then left to the browser so Cmd+Z
// inside a field undoes the typing, not the last profile edit.
export function isTextEntryTarget(target: unknown): boolean {
  if (!target || typeof target !== 'object') return false;
  const element = target as { tagName?: string; type?: string; isContentEditable?: boolean };
  if (element.isContentEditable) return true;
  const tag = element.tagName?.toUpperCase();
  if (tag === 'TEXTAREA') return true;
  return tag === 'INPUT' && textInputTypes.has((element.type ?? 'text').toLowerCase());
}

export function shortcutAction(event: KeyEventLike, mac: boolean, options: { modalOpen?: boolean } = {}): ShortcutAction | null {
  if (options.modalOpen || event.isComposing || event.keyCode === 229) return null;
  const modifier = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  if (!modifier || event.altKey) return null;
  const key = event.key.toLowerCase();
  // Shift only changes Z (redo); every other shortcut is unshifted.
  if (key === 'z') return isTextEntryTarget(event.target) ? null : { type: event.shiftKey ? 'redo' : 'undo' };
  if (key === 'y' && !mac && !event.shiftKey) return isTextEntryTarget(event.target) ? null : { type: 'redo' };
  if (event.shiftKey) return null;
  if (key === 's') return { type: 'save' };
  if (key === 'f') return { type: 'find' };
  if (key === 'd') return event.repeat ? null : { type: 'duplicate' };
  const index = ['1', '2', '3', '4'].indexOf(key);
  return index >= 0 ? { type: 'page', page: shortcutPages[index] } : null;
}

export function hint(key: string, mac: boolean) { return mac ? `⌘${key}` : `Ctrl+${key}`; }
export function withHint(label: string, key: string, mac: boolean) { return `${label} (${hint(key, mac)})`; }
export function redoHint(mac: boolean) { return mac ? '⌘⇧Z' : 'Ctrl+Y'; }

export const shortcutList: { keys: string; label: string }[] = [
  { keys: 'S', label: 'Save profile' },
  { keys: 'D', label: 'Duplicate profile' },
  { keys: 'Z', label: 'Undo draft edit (not while typing in a field)' },
  { keys: 'Shift+Z', label: 'Redo draft edit (Ctrl+Y also works on Windows and Linux)' },
  { keys: 'F', label: 'Search bindings (Profiles page)' },
  { keys: '1', label: 'Devices' },
  { keys: '2', label: 'Profiles' },
  { keys: '3', label: 'Compatibility' },
  { keys: '4', label: 'Settings' },
];

export interface ShortcutHandlers {
  mac: boolean;
  isPageEnabled(page: Page): boolean;
  setPage(page: Page): void;
  save(): void;
  duplicate(): void;
  undo(): void;
  redo(): void;
  // Returns false when search is not available (not on the Profiles page).
  find(): boolean;
}

// Returns true when the event was handled (and its default prevented).
export function handleShortcut(event: KeyEventLike & { preventDefault(): void }, handlers: ShortcutHandlers, modalOpen: boolean) {
  const action = shortcutAction(event, handlers.mac, { modalOpen });
  if (!action) return false;
  if (action.type === 'find' && !handlers.find()) return false;
  event.preventDefault();
  if (action.type === 'save') handlers.save();
  else if (action.type === 'undo') handlers.undo();
  else if (action.type === 'redo') handlers.redo();
  else if (action.type === 'duplicate') handlers.duplicate();
  else if (action.type === 'find') { /* handled above */ }
  else if (handlers.isPageEnabled(action.page)) handlers.setPage(action.page);
  return true;
}

export function installShortcuts(target: Pick<Document, 'addEventListener' | 'removeEventListener' | 'querySelector'>, handlers: () => ShortcutHandlers) {
  const listener = (event: Event) => handleShortcut(event as KeyboardEvent, handlers(), target.querySelector('dialog[open]') !== null);
  target.addEventListener('keydown', listener);
  return () => target.removeEventListener('keydown', listener);
}
