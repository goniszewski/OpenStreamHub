// Keyboard shortcuts: Cmd on macOS, Ctrl elsewhere. The mapping is pure; the
// App installs it with `installShortcuts`.
import type { Page } from './ui-model';

export type ShortcutAction = { type: 'save' } | { type: 'duplicate' } | { type: 'page'; page: Page };

export interface KeyEventLike {
  key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; isComposing?: boolean; keyCode?: number; repeat?: boolean;
}

export const shortcutPages: readonly Page[] = ['devices', 'profiles', 'compatibility', 'settings'];

export function isMacPlatform(navigatorLike: { platform?: string; userAgent?: string } | undefined) {
  return /mac/i.test(navigatorLike?.platform || navigatorLike?.userAgent || '');
}

export function shortcutAction(event: KeyEventLike, mac: boolean, options: { modalOpen?: boolean } = {}): ShortcutAction | null {
  if (options.modalOpen || event.isComposing || event.keyCode === 229) return null;
  const modifier = mac ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
  if (!modifier || event.altKey || event.shiftKey) return null;
  const key = event.key.toLowerCase();
  if (key === 's') return { type: 'save' };
  if (key === 'd') return event.repeat ? null : { type: 'duplicate' };
  const index = ['1', '2', '3', '4'].indexOf(key);
  return index >= 0 ? { type: 'page', page: shortcutPages[index] } : null;
}

export function hint(key: string, mac: boolean) { return mac ? `⌘${key}` : `Ctrl+${key}`; }
export function withHint(label: string, key: string, mac: boolean) { return `${label} (${hint(key, mac)})`; }

export const shortcutList: { keys: string; label: string }[] = [
  { keys: 'S', label: 'Save profile' },
  { keys: 'D', label: 'Duplicate profile' },
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
}

// Returns true when the event was handled (and its default prevented).
export function handleShortcut(event: KeyEventLike & { preventDefault(): void }, handlers: ShortcutHandlers, modalOpen: boolean) {
  const action = shortcutAction(event, handlers.mac, { modalOpen });
  if (!action) return false;
  event.preventDefault();
  if (action.type === 'save') handlers.save();
  else if (action.type === 'duplicate') handlers.duplicate();
  else if (handlers.isPageEnabled(action.page)) handlers.setPage(action.page);
  return true;
}

export function installShortcuts(target: Pick<Document, 'addEventListener' | 'removeEventListener' | 'querySelector'>, handlers: () => ShortcutHandlers) {
  const listener = (event: Event) => handleShortcut(event as KeyboardEvent, handlers(), target.querySelector('dialog[open]') !== null);
  target.addEventListener('keydown', listener);
  return () => target.removeEventListener('keydown', listener);
}
