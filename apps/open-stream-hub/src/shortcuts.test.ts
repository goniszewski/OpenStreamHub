import { describe, expect, it, vi } from 'vitest';
import { handleShortcut, hint, installShortcuts, isMacPlatform, shortcutAction, withHint } from './shortcuts';
import type { KeyEventLike } from './shortcuts';

const key = (name: string, extra: Partial<KeyEventLike> = {}): KeyEventLike => ({ key: name, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...extra });

describe('shortcutAction', () => {
  it('maps Cmd on macOS and Ctrl elsewhere', () => {
    expect(shortcutAction(key('s', { metaKey: true }), true)).toEqual({ type: 'save' });
    expect(shortcutAction(key('S', { metaKey: true }), true)).toEqual({ type: 'save' });
    expect(shortcutAction(key('d', { metaKey: true }), true)).toEqual({ type: 'duplicate' });
    expect(shortcutAction(key('3', { metaKey: true }), true)).toEqual({ type: 'page', page: 'compatibility' });
    expect(shortcutAction(key('s', { ctrlKey: true }), false)).toEqual({ type: 'save' });
    expect(shortcutAction(key('1', { ctrlKey: true }), false)).toEqual({ type: 'page', page: 'devices' });
    expect(shortcutAction(key('4', { ctrlKey: true }), false)).toEqual({ type: 'page', page: 'settings' });
  });

  it('ignores the wrong modifier, extra modifiers, other keys and 5+', () => {
    expect(shortcutAction(key('s', { ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true }), false)).toBeNull();
    expect(shortcutAction(key('s'), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, shiftKey: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, altKey: true }), true)).toBeNull();
    expect(shortcutAction(key('s', { metaKey: true, ctrlKey: true }), true)).toBeNull();
    expect(shortcutAction(key('q', { metaKey: true }), true)).toBeNull();
    expect(shortcutAction(key('5', { metaKey: true }), true)).toBeNull();
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
  const handlers = () => ({ mac: true, isPageEnabled: vi.fn((page: string) => page !== 'settings'), setPage: vi.fn(), save: vi.fn(), duplicate: vi.fn() });

  it('prevents the default and runs the action, including from inputs', () => {
    const h = handlers();
    const preventDefault = vi.fn();
    expect(handleShortcut({ ...key('s', { metaKey: true }), preventDefault }, h, false)).toBe(true);
    expect(preventDefault).toHaveBeenCalled();
    expect(h.save).toHaveBeenCalledOnce();
    handleShortcut({ ...key('d', { metaKey: true }), preventDefault }, h, false);
    expect(h.duplicate).toHaveBeenCalledOnce();
    handleShortcut({ ...key('2', { metaKey: true }), preventDefault }, h, false);
    expect(h.setPage).toHaveBeenCalledWith('profiles');
  });

  it('respects disabled pages and open modals', () => {
    const h = handlers();
    const preventDefault = vi.fn();
    handleShortcut({ ...key('4', { metaKey: true }), preventDefault }, h, false);
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
    const remove = installShortcuts(target as never, () => ({ mac: false, isPageEnabled: () => true, setPage: vi.fn(), save, duplicate: vi.fn() }));
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
