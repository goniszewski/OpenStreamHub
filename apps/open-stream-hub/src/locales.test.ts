import { describe, expect, it } from 'vitest';
import type { Message } from './i18n';
import { en } from './locales/en';

const texts = (message: Message): string[] => typeof message === 'string' ? [message] : Object.values(message);

describe('the catalogue', () => {
  it('holds no empty message', () => {
    for (const [key, message] of Object.entries(en) as [string, Message][]) {
      for (const text of texts(message)) expect(text.trim(), key).not.toBe('');
    }
  });
  it('gives plural messages a one form and an other form', () => {
    for (const [key, message] of Object.entries(en) as [string, Message][]) {
      if (typeof message !== 'string') expect(Object.keys(message).sort(), key).toEqual(['one', 'other']);
    }
  });
});
