import { describe, expect, it } from 'vitest';
import type { Message } from './i18n';
import { en } from './locales/en';
import { pl } from './locales/pl';

const texts = (message: Message): string[] => typeof message === 'string' ? [message] : Object.values(message);
const placesOf = (text: string) => [...text.matchAll(/\{(\w+)\}/g)].map(match => match[1]).sort();
const catalogues: Record<string, Record<string, Message>> = { en, pl };

describe('the catalogues', () => {
  it('have the same keys in every language, so nothing is left untranslated', () => {
    expect(Object.keys(pl).sort()).toEqual(Object.keys(en).sort());
  });
  it('hold no empty message', () => {
    for (const [locale, catalogue] of Object.entries(catalogues)) {
      for (const [key, message] of Object.entries(catalogue)) {
        for (const text of texts(message)) expect(text.trim(), `${locale} ${key}`).not.toBe('');
      }
    }
  });
  it('use the same {places} in every language, in every plural form', () => {
    for (const [key, source] of Object.entries(en) as [string, Message][]) {
      const wanted = [...new Set(texts(source).flatMap(placesOf))].sort();
      for (const text of texts(pl[key as keyof typeof pl])) {
        const found = [...new Set(placesOf(text))].sort();
        // A plural form may leave the count out when its wording does not need it, but never adds a place.
        expect(wanted, `pl ${key}: ${text}`).toEqual(expect.arrayContaining(found));
      }
    }
  });
  it('give Polish all four plural forms wherever it has a plural, and wherever English has one', () => {
    for (const [key, polish] of Object.entries(pl) as [string, Message][]) {
      if (typeof polish === 'string') { expect(typeof en[key as keyof typeof en], `${key} is a plural in English`).toBe('string'); continue; }
      expect(Object.keys(polish).sort(), key).toEqual(['few', 'many', 'one', 'other']);
    }
  });
  it('give English plural messages a one form and an other form', () => {
    for (const [key, message] of Object.entries(en) as [string, Message][]) {
      if (typeof message !== 'string') expect(Object.keys(message).sort(), key).toEqual(['one', 'other']);
    }
  });
});
