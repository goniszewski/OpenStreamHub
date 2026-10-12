import { describe, expect, it } from 'vitest';
import { fill, format, translate } from './i18n';
import type { Message } from './i18n';

describe('fill and format', () => {
  it('fills {name} places and leaves a place without a value as written', () => {
    expect(fill('{a} then {b}', { a: 'x', b: 2 })).toBe('x then 2');
    expect(fill('{a} then {b}', { a: 'x' })).toBe('x then {b}');
    expect(fill('no places')).toBe('no places');
  });
  it('picks a plural form by count', () => {
    const message: Message = { one: '{count} file', other: '{count} files' };
    expect([1, 0, 3].map(count => format(message, { count }))).toEqual(['1 file', '0 files', '3 files']);
  });
  it('falls back to the other form when there is no count', () => {
    expect(format({ one: 'one', other: 'other' })).toBe('other');
  });
});

describe('translate', () => {
  it('uses the message, then the key', () => {
    const catalogue = { hello: 'Hello {name}' };
    expect(translate(catalogue, 'hello', { name: 'Ola' })).toBe('Hello Ola');
    expect(translate(catalogue, 'missing')).toBe('missing');
  });
});
