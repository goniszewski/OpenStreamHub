import { describe, expect, it, vi } from 'vitest';
import { FakeStorage } from './fake-storage';
import { fill, format, I18nStore, isLanguageSetting, languageStorageKey, languages, resolveLocale, translate } from './i18n';
import type { Message } from './i18n';

describe('resolveLocale', () => {
  it('uses a chosen language as it is, and for System the first system language the app has', () => {
    expect(resolveLocale('pl', ['en-US'])).toBe('pl');
    expect(resolveLocale('en', ['pl-PL'])).toBe('en');
    expect(resolveLocale('system', ['de-DE', 'pl-PL', 'en-US'])).toBe('pl');
    expect(resolveLocale('system', ['PL'])).toBe('pl');
    expect(resolveLocale('system', ['fr-FR', 'de'])).toBe('en');
    expect(resolveLocale('system', [])).toBe('en');
  });
});

describe('fill and format', () => {
  it('fills {name} places and leaves a place without a value as written', () => {
    expect(fill('{a} then {b}', { a: 'x', b: 2 })).toBe('x then 2');
    expect(fill('{a} then {b}', { a: 'x' })).toBe('x then {b}');
    expect(fill('no places')).toBe('no places');
  });
  it('picks a plural form by count with the locale’s rules', () => {
    const message: Message = { one: '{count} plik', few: '{count} pliki', many: '{count} plików', other: '{count} pliku' };
    expect([0, 1, 2, 4, 5, 12, 22, 25].map(count => format(message, 'pl', { count }))).toEqual(
      ['0 plików', '1 plik', '2 pliki', '4 pliki', '5 plików', '12 plików', '22 pliki', '25 plików']);
    const english: Message = { one: '{count} file', other: '{count} files' };
    expect([format(english, 'en', { count: 1 }), format(english, 'en', { count: 0 }), format(english, 'en', { count: 3 })]).toEqual(['1 file', '0 files', '3 files']);
  });
  it('falls back to the other form when a category has no text, and when there is no count', () => {
    expect(format({ other: 'many things' }, 'pl', { count: 1 })).toBe('many things');
    expect(format({ one: 'one', other: 'other' }, 'en')).toBe('other');
  });
});

describe('translate', () => {
  const catalogues = { en: { hello: 'Hello {name}', only: 'English only' }, pl: { hello: 'Cześć {name}' } };
  it('uses the locale’s message, then English’s, then the key', () => {
    expect(translate(catalogues, 'pl', 'hello', { name: 'Ola' })).toBe('Cześć Ola');
    expect(translate(catalogues, 'pl', 'only')).toBe('English only');
    expect(translate(catalogues, 'pl', 'missing')).toBe('missing');
    expect(translate(catalogues, 'en', 'hello', { name: 'Ola' })).toBe('Hello Ola');
  });
});

describe('I18nStore', () => {
  it('starts on System, resolved from the system languages, and tells the locale hook', () => {
    const seen: string[] = [];
    const store = new I18nStore(new FakeStorage(), ['pl-PL'], locale => seen.push(locale));
    expect(store.getState()).toEqual({ setting: 'system', locale: 'pl' });
    expect(seen).toEqual(['pl']);
  });
  it('switches at once, remembers the choice and notifies subscribers once', () => {
    const storage = new FakeStorage();
    const store = new I18nStore(storage, ['en-US']);
    const listener = vi.fn();
    store.subscribe(listener);
    store.setLanguage('pl');
    store.setLanguage('pl');
    expect(listener).toHaveBeenCalledTimes(1);
    expect(store.getState()).toEqual({ setting: 'pl', locale: 'pl' });
    expect(storage.getItem(languageStorageKey)).toBe('pl');
    expect(new I18nStore(storage, ['en-US']).getState().locale).toBe('pl');
    store.setLanguage('system');
    expect(store.getState().locale).toBe('en');
  });
  it('ignores a stored language it does not know, an unknown setting, and storage that refuses', () => {
    const storage = new FakeStorage();
    storage.setItem(languageStorageKey, 'klingon');
    const store = new I18nStore(storage, ['en']);
    expect(store.getState().setting).toBe('system');
    store.setLanguage('klingon' as never);
    expect(store.getState().setting).toBe('system');
    const refusing = { getItem: () => { throw new Error('blocked'); }, setItem: () => { throw new Error('blocked'); } };
    const blocked = new I18nStore(refusing, ['en']);
    blocked.setLanguage('pl');
    expect(blocked.getState().locale).toBe('pl');
    expect(new I18nStore(null, ['en']).getState().locale).toBe('en');
  });
  it('knows its settings', () => {
    expect(['system', ...languages.map(language => language.id)].every(isLanguageSetting)).toBe(true);
    expect(isLanguageSetting('de')).toBe(false);
    expect(isLanguageSetting(undefined)).toBe(false);
  });
});
