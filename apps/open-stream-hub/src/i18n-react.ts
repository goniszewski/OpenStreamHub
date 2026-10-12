import { translate } from './i18n';
import { en } from './locales/en';
import type { MessageKey } from './locales/en';

export type Translate = (key: MessageKey, values?: Record<string, string | number>) => string;

// The text of one key; what the components' `t` is made of.
export const english: Translate = (key, values) => translate(en, key, values);

// `t` for a component.
export const useT = (): Translate => english;
