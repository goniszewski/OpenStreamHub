import { describe, expect, it, vi } from 'vitest';
import { handleShortcut, hint, installShortcuts, isMacPlatform, isTextEntryTarget, redoHint, shortcutAction, shortcutList, withHint } from './shortcuts';
import type { KeyEventLike } from './shortcuts';

const key = (name: string, extra: Partial<KeyEventLike> = {}): KeyEventLike => ({ key: name, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...extra });

describe('shortcutAction', () => {
  it('maps Cmd on macOS and Ctrl elsewhere', () => {
    expect(shortcutAction(key('s', { metaKey: true }), true)).toEqual({ type: 'save' });
    expect(shortcutAction(key('S', { metaKey: true }), true)).toEqual({ type: 'save' });
    expect(shortcutAction(key('d', { metaKey: true }), true)).toEqual({ type: 'duplicate' });
    expect(shortcutAction(key('3', { metaKey: true }), true)).toEqual({ type: 'page', page: 'lighting' });
    expect(shortcutAction(key('s', { ctrlKey: true }), false)).toEqual({ type: 'save' });
    expect(shortcutAction(key('1', { ctrlKey: true }), false)).toEqual({ type: 'page', page: 'workspace' });
    expect(shortcutAction(key('2', { ctrlKey: true }), false)).toEqual({ type: 'page', page: 'artwork' });
    expect(shortcutAction(key('4', { ctrlKey: true }), false)).toEqual({ type: 'page', page: 'compatibility' });
    expect(shortcutAction(key('5', { ctrlKey: true }), false)).toEqual({ type: 'page', page: 'settings' });
  });

  it('ignores the wrong modifier, extra modifiers, other keys and 6+', () => {
    expect(shortcutAction(key('s', { ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true }), false)).toBeNull();
    expect(shortcutAction(key('s'), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, shiftKey: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, altKey: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(key('q', { metaKey: true }), true)).toBeNull();
    expect(shortcutAction(key('6', { metaKey: true }), true)).toBeNull();
    expect(shortcutAction(key('Escape'), true)).toBeNull();
  });

  it('ignores events while a modal is open or an IME composition is active', () => {
    expect(shortcutAction(key('s', { metaKey: true }), true, { modalOpen: true })).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, isComposing: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, keyCode: 229 }), true)).toBeNull();
  });

  it('does not repeat duplicate while the key is held', () => {
    expect(shortcutAction(key('d', { metaKey: true, repeat: true }), true)).toBeNull();
  });
});

describe('handleShortcut', () => {
  const handlers = () => ({ mac: true, isPageEnabled: vi.fn((page: string) => page !== 'settings'), setPage: vi.fn(), save: vi.fn(), duplicate: vi.fn(), undo: vi.fn(), redo: vi.fn(), find: vi.fn(() => true), copy: vi.fn(() => true), paste: vi.fn(() => true), delete: vi.fn(() => true) });

  it('prevents the default and runs the action, including from inputs', () => {
    const h = handlers();
    const preventDefault = vi.fn();
    expect(handleShortcut({ ...key('s', { metaKey: true }), preventDefault }, h, false)).toBe(true);
    expect(preventDefault).toHaveBeenCalled();
    expect(h.save).toHaveBeenCalledOnce();
    handleShortcut({ ...key('d', { metaKey: true }), preventDefault }, h, false);
    expect(h.duplicate).toHaveBeenCalledOnce();
    handleShortcut({ ...key('2', { metaKey: true }), preventDefault }, h, false);
    expect(h.setPage).toHaveBeenCalledWith('artwork');
  });

  it('respects disabled pages and open modals', () => {
    const h = handlers();
    const preventDefault = vi.fn();
    handleShortcut({ ...key('5', { metaKey: true }), preventDefault }, h, false);
    expect(h.setPage).not.toHaveBeenCalled();
    expect(handleShortcut({ ...key('s', { metaKey: true }), preventDefault }, h, true)).toBe(false);
    expect(h.save).not.toHaveBeenCalled();
  });
});

describe('installShortcuts', () => {
  it('listens for keydown, detects open dialogs and removes the listener', () => {
    let listener: ((event: Event) => void) | undefined;
    let dialogOpen = false;
    const target = {
      addEventListener: vi.fn((_: string, fn: (event: Event) => void) => { listener = fn; }), removeEventListener: vi.fn(),
      querySelector: vi.fn(() => dialogOpen ? {} : null),
    };
    const save = vi.fn();
    const remove = installShortcuts(target as never, () => ({ mac: false, isPageEnabled: () => true, setPage: vi.fn(), save, duplicate: vi.fn(), undo: vi.fn(), redo: vi.fn(), find: () => true, copy: () => true, paste: () => true, delete: () => true }));
    const event = { ...key('s', { ctrlKey: true }), preventDefault: vi.fn() };
    listener!(event as unknown as Event);
    expect(save).toHaveBeenCalledOnce();
    dialogOpen = true;
    listener!(event as unknown as Event);
    expect(save).toHaveBeenCalledOnce();
    expect(target.querySelector).toHaveBeenCalledWith('dialog[open]');
    remove();
    expect(target.removeEventListener).toHaveBeenCalledWith('keydown', listener);
  });
});

describe('hints', () => {
  it('formats per platform', () => {
    expect(hint('S', true)).toBe('⌘S');
    expect(hint('S', false)).toBe('Ctrl+S');
    expect(withHint('Save profile', 'S', true)).toBe('Save profile (⌘S)');
    expect(isMacPlatform({ platform: 'MacIntel' })).toBe(true);
    expect(isMacPlatform({ platform: 'Win32', userAgent: 'Windows' })).toBe(false);
    expect(isMacPlatform(undefined)).toBe(false);
  });
});

describe('undo, redo and find', () => {
  it('maps Cmd+Z / Cmd+Shift+Z on macOS and Ctrl+Z / Ctrl+Shift+Z / Ctrl+Y elsewhere', () => {
    expect(shortcutAction(key('z', { metaKey: true }), true)).toEqual({ type: 'undo' });
    expect(shortcutAction(key('Z', { metaKey: true, shiftKey: true }), true)).toEqual({ type: 'redo' });
    expect(shortcutAction(key('z', { ctrlKey: true }), false)).toEqual({ type: 'undo' });
    expect(shortcutAction(key('Z', { ctrlKey: true, shiftKey: true }), false)).toEqual({ type: 'redo' });
    expect(shortcutAction(key('y', { ctrlKey: true }), false)).toEqual({ type: 'redo' });
    expect(shortcutAction(key('f', { metaKey: true }), true)).toEqual({ type: 'find' });
    expect(shortcutAction(key('f', { ctrlKey: true }), false)).toEqual({ type: 'find' });
  });

  it('ignores the wrong modifier, Ctrl+Y on macOS, Alt and shifted non-Z keys', () => {
    expect(shortcutAction(key('z', { ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(key('z', { metaKey: true }), false)).toBeNull();
    expect(shortcutAction(key('y', { metaKey: true }), true)).toBeNull();
    expect(shortcutAction(key('y', { ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(key('y', { ctrlKey: true, shiftKey: true }), false)).toBeNull();
    expect(shortcutAction(key('z', { metaKey: true, altKey: true }), true)).toBeNull();
    expect(shortcutAction(key('f', { metaKey: true, shiftKey: true }), true)).toBeNull();
    expect(shortcutAction(key('z'), true)).toBeNull();
    expect(shortcutAction(key('z', { metaKey: true }), true, { modalOpen: true })).toBeNull();
  });

  it('leaves undo and redo to the browser while focus is in a text field', () => {
    for (const target of [{ tagName: 'INPUT', type: 'text' }, { tagName: 'INPUT' }, { tagName: 'INPUT', type: 'number' }, { tagName: 'INPUT', type: 'search' }, { tagName: 'TEXTAREA' }, { tagName: 'DIV', isContentEditable: true }]) {
      expect(isTextEntryTarget(target)).toBe(true);
      expect(shortcutAction(key('z', { metaKey: true, target }), true)).toBeNull();
      expect(shortcutAction(key('z', { metaKey: true, shiftKey: true, target }), true)).toBeNull();
      expect(shortcutAction(key('y', { ctrlKey: true, target }), false)).toBeNull();
    }
  });

  it('still intercepts on buttons, selects, checkboxes, ranges, the body and with no target', () => {
    for (const target of [{ tagName: 'BUTTON' }, { tagName: 'BODY' }, { tagName: 'SELECT' }, { tagName: 'select' }, { tagName: 'INPUT', type: 'checkbox' }, { tagName: 'INPUT', type: 'range' }, { tagName: 'INPUT', type: 'radio' }, null, undefined]) {
      expect(isTextEntryTarget(target)).toBe(false);
      expect(shortcutAction(key('z', { metaKey: true, target }), true)).toEqual({ type: 'undo' });
    }
  });

  it('other shortcuts keep working from inside a text field', () => {
    expect(shortcutAction(key('s', { metaKey: true, target: { tagName: 'INPUT', type: 'text' } }), true)).toEqual({ type: 'save' });
  });

  it('handleShortcut runs undo/redo and only prevents default when it acts', () => {
    const handlers = { mac: true, isPageEnabled: () => true, setPage: vi.fn(), save: vi.fn(), duplicate: vi.fn(), undo: vi.fn(), redo: vi.fn(), find: vi.fn(() => false), copy: vi.fn(() => true), paste: vi.fn(() => true), delete: vi.fn(() => true) };
    const preventDefault = vi.fn();
    expect(handleShortcut({ ...key('z', { metaKey: true }), preventDefault }, handlers, false)).toBe(true);
    expect(handlers.undo).toHaveBeenCalledOnce();
    expect(handleShortcut({ ...key('z', { metaKey: true, shiftKey: true }), preventDefault }, handlers, false)).toBe(true);
    expect(handlers.redo).toHaveBeenCalledOnce();
    expect(preventDefault).toHaveBeenCalledTimes(2);
    const input = { tagName: 'INPUT', type: 'text' };
    expect(handleShortcut({ ...key('z', { metaKey: true, target: input }), preventDefault }, handlers, false)).toBe(false);
    expect(handlers.undo).toHaveBeenCalledOnce();
    expect(preventDefault).toHaveBeenCalledTimes(2);
    // Find outside the workspace keeps the browser's own Cmd+F.
    expect(handleShortcut({ ...key('f', { metaKey: true }), preventDefault }, handlers, false)).toBe(false);
    expect(preventDefault).toHaveBeenCalledTimes(2);
    handlers.find.mockReturnValue(true);
    expect(handleShortcut({ ...key('f', { metaKey: true }), preventDefault }, handlers, false)).toBe(true);
    expect(preventDefault).toHaveBeenCalledTimes(3);
  });

  it('documents the shortcuts and hints', () => {
    expect(withHint('Undo', 'Z', true)).toBe('Undo (⌘Z)');
    expect(withHint('Undo', 'Z', false)).toBe('Undo (Ctrl+Z)');
    expect(redoHint(true)).toBe('⌘⇧Z');
    expect(redoHint(false)).toBe('Ctrl+Y');
    expect(shortcutList.map(item => item.keys)).toEqual(expect.arrayContaining(['Z', 'Shift+Z', 'F', 'C', 'V', '⌫']));
  });
});

describe('copy, paste and clear', () => {
  it('maps Cmd+C / Cmd+V / Backspace and Delete', () => {
    expect(shortcutAction(key('c', { metaKey: true }), true)).toEqual({ type: 'copy' });
    expect(shortcutAction(key('c', { ctrlKey: true }), false)).toEqual({ type: 'copy' });
    expect(shortcutAction(key('v', { metaKey: true }), true)).toEqual({ type: 'paste' });
    expect(shortcutAction(key('v', { ctrlKey: true }), false)).toEqual({ type: 'paste' });
    expect(shortcutAction(key('Backspace', { metaKey: true }), true)).toEqual({ type: 'delete' });
    expect(shortcutAction(key('Delete', { metaKey: true }), true)).toEqual({ type: 'delete' });
  });

  it('leaves copy, paste and backspace to the browser inside a text field, and ignores repeats', () => {
    const input = { tagName: 'INPUT', type: 'text' };
    expect(shortcutAction(key('c', { metaKey: true, target: input }), true)).toBeNull();
    expect(shortcutAction(key('v', { metaKey: true, target: input }), true)).toBeNull();
    expect(shortcutAction(key('Backspace', { metaKey: true, target: input }), true)).toBeNull();
    expect(shortcutAction(key('c', { metaKey: true, target: { tagName: 'BUTTON' } }), true)).toEqual({ type: 'copy' });
    expect(shortcutAction(key('Backspace', { metaKey: true, repeat: true }), true)).toBeNull();
    expect(shortcutAction(key('c', { metaKey: true, shiftKey: true }), true)).toBeNull();
  });

  it('only prevents the default when copy or delete act, and always acts on paste', () => {
    const handlers = { mac: true, isPageEnabled: () => true, setPage: vi.fn(), save: vi.fn(), duplicate: vi.fn(), undo: vi.fn(), redo: vi.fn(), find: () => true, copy: vi.fn(() => false), paste: vi.fn(() => true), delete: vi.fn(() => true) };
    const preventDefault = vi.fn();
    expect(handleShortcut({ ...key('c', { metaKey: true }), preventDefault }, handlers, false)).toBe(false);
    expect(handlers.copy).toHaveBeenCalledOnce();
    expect(preventDefault).not.toHaveBeenCalled();
    expect(handleShortcut({ ...key('v', { metaKey: true }), preventDefault }, handlers, false)).toBe(true);
    expect(handlers.paste).toHaveBeenCalledOnce();
    expect(handleShortcut({ ...key('Backspace', { metaKey: true }), preventDefault }, handlers, false)).toBe(true);
    expect(handlers.delete).toHaveBeenCalledOnce();
    expect(preventDefault).toHaveBeenCalledTimes(2);
  });
});
