import { useSyncExternalStore } from 'react';
import { I18nStore, translate } from './i18n';
import type { I18nState, LanguageSetting, Locale } from './i18n';
import { en } from './locales/en';
import type { MessageKey } from './locales/en';
import { pl } from './locales/pl';

const catalogues = { en, pl };

export type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

// The text of one key in one locale; what the components' `t` is made of.
export const translateIn = (locale: Locale): Translate => (key, values) => translate(catalogues, locale, key, values);

const setDocumentLanguage = (locale: Locale) => { if (typeof document !== 'undefined') document.documentElement.lang = locale; };

// The app's one language store. Tests and the browser preview without storage start in English.
export const i18n = new I18nStore(
  typeof localStorage === 'undefined' ? null : localStorage,
  typeof navigator === 'undefined' ? ['en'] : navigator.languages ?? ['en'],
  setDocumentLanguage,
);

const translators: Record<Locale, Translate> = { en: translateIn('en'), pl: translateIn('pl') };

// `t` in the current language, redrawing the component when the language changes.
export function useT(store: I18nStore = i18n): Translate {
  const state = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  return translators[state.locale];
}

export function useLanguage(store: I18nStore = i18n): { state: I18nState; setLanguage(setting: LanguageSetting): void } {
  const state = useSyncExternalStore(store.subscribe, store.getState, store.getState);
  return { state, setLanguage: setting => store.setLanguage(setting) };
}
