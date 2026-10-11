// The app's translation layer: message catalogues per language, a store that holds the chosen language and switches it
// while the app runs, and `translate`, which picks the message and fills in its values. English is the source; every
// other catalogue is typed to have exactly English's keys, so a missing translation is a compile error, not a blank.
// Nothing is fetched at run time: the catalogues are part of the build.

export type Locale = 'en' | 'pl';
export type LanguageSetting = 'system' | Locale;

// The languages the picker offers, named in themselves so a reader who cannot read the current language can find theirs.
export const languages: ReadonlyArray<{ id: Locale; name: string }> = [{ id: 'en', name: 'English' }, { id: 'pl', name: 'Polski' }];

// A message is text with {name} places for values, or one text per plural category (`other` is required).
export type PluralMessage = Partial<Record<Intl.LDMLPluralRule, string>> & { other: string };
export type Message = string | PluralMessage;

export const languageStorageKey = 'open-stream-hub.language';

export function isLocale(value: unknown): value is Locale {
  return languages.some(language => language.id === value);
}

export function isLanguageSetting(value: unknown): value is LanguageSetting {
  return value === 'system' || isLocale(value);
}

// The first language of the system's list that the app has a catalogue for, else English.
export function resolveLocale(setting: LanguageSetting, systemLanguages: readonly string[]): Locale {
  if (setting !== 'system') return setting;
  for (const tag of systemLanguages) {
    const base = tag.toLowerCase().split('-')[0];
    const match = languages.find(language => language.id === base);
    if (match) return match.id;
  }
  return 'en';
}

// Fills {name} places. A place with no value stays as written, so a missing value shows up in testing rather than
// vanishing; a value that is a number is shown as given (no digit grouping), which suits counts.
export function fill(text: string, values: Record<string, string | number> = {}): string {
  return text.replace(/\{(\w+)\}/g, (place, name: string) => name in values ? String(values[name]) : place);
}

// The text of a message for these values: a plural message picks its form by `count` for the locale's plural rules.
export function format(message: Message, locale: Locale, values?: Record<string, string | number>): string {
  if (typeof message === 'string') return fill(message, values);
  const count = Number(values?.count);
  const category = Number.isFinite(count) ? new Intl.PluralRules(locale).select(count) : 'other';
  return fill(message[category] ?? message.other, values);
}

// Looks a key up in the locale's catalogue, then English's, then falls back to the key itself.
export function translate(catalogues: Record<Locale, Record<string, Message>>, locale: Locale, key: string, values?: Record<string, string | number>): string {
  const message = catalogues[locale][key] ?? catalogues.en[key];
  return message === undefined ? key : format(message, catalogues[locale][key] === undefined ? 'en' : locale, values);
}

export interface I18nState { setting: LanguageSetting; locale: Locale }

// Holds the chosen language. Changing it notifies every subscriber, so components redraw in the new language at once,
// and it is remembered locally. `systemLanguages` is the browser's language list (navigator.languages).
export class I18nStore {
  private state: I18nState;
  private readonly listeners = new Set<() => void>();

  constructor(private readonly storage: Pick<Storage, 'getItem' | 'setItem'> | null, private readonly systemLanguages: readonly string[], private readonly onLocale?: (locale: Locale) => void) {
    let setting: LanguageSetting = 'system';
    try { const stored = storage?.getItem(languageStorageKey); if (isLanguageSetting(stored)) setting = stored; } catch { /* the choice is not remembered */ }
    this.state = { setting, locale: resolveLocale(setting, systemLanguages) };
    onLocale?.(this.state.locale);
  }

  readonly subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  readonly getState = () => this.state;

  setLanguage(setting: LanguageSetting) {
    if (!isLanguageSetting(setting) || setting === this.state.setting) return;
    try { this.storage?.setItem(languageStorageKey, setting); } catch { /* the choice lasts for this session only */ }
    const locale = resolveLocale(setting, this.systemLanguages);
    this.state = { setting, locale };
    this.onLocale?.(locale);
    this.listeners.forEach(listener => listener());
  }
}
