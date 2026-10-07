import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import type { Action } from './contracts';
import { defaultMultiAction, defaultStepAction, keyLabel, namedKeys, stepActionTypes, textIssue, urlIssue } from './action-model';
import { actionLabel } from './ui-model';

// Shared with the Rust service tests (profile.rs), so both sides agree on action shapes.
const fixture = JSON.parse(readFileSync(new URL('../../../crates/cooltek-sdk-service/tests/fixtures/actions.json', import.meta.url), 'utf8')) as {
  named_keys: string[]; valid: Action[]; invalid: unknown[];
};

describe('action contract', () => {
  it('mirrors the Rust named-key allow-list in order', () => {
    expect(namedKeys.map(([name]) => name)).toEqual(fixture.named_keys);
  });

  it('labels every valid fixture action without throwing', () => {
    for (const action of fixture.valid) expect(actionLabel(action)).not.toBe('');
    expect(actionLabel({ type: 'shortcut', key: 'arrow_left', modifiers: ['command', 'shift'] })).toBe('Command + Shift + Left Arrow');
    expect(actionLabel({ type: 'media', command: 'next_track' })).toBe('Next track');
    expect(actionLabel({ type: 'system', command: 'screenshot' })).toBe('Screenshot to clipboard');
    expect(actionLabel({ type: 'open_url', url: 'https://example.com' })).toBe('Open https://example.com');
    expect(actionLabel({ type: 'text', text: 'Hello\nthere, this is a long snippet', press_enter: true })).toBe('Paste “Hello there, this is a l…” + Return');
    expect(actionLabel(defaultMultiAction())).toBe('2 actions: Toggle output mute, then Command + C');
  });

  it('builds default step actions that are themselves valid fixture shapes', () => {
    for (const [type] of stepActionTypes) {
      const action = defaultStepAction(type);
      expect(action.type).toBe(type);
      if (action.type === 'open_url') expect(urlIssue(action.url)).toBeNull();
      if (action.type === 'text') expect(textIssue(action.text)).toBeNull();
    }
    expect(stepActionTypes.map(([type]) => type)).not.toContain('multi_action');
  });

  it('labels named and literal keys', () => {
    expect(keyLabel('f20')).toBe('F20');
    expect(keyLabel('page_up')).toBe('Page Up');
    expect(keyLabel(' ')).toBe('Space');
    expect(keyLabel('q')).toBe('Q');
  });
});

describe('inline action hints', () => {
  it('flags URLs the service rejects', () => {
    for (const url of ['https://example.com', 'http://localhost:8080/x?y=1#z']) expect(urlIssue(url)).toBeNull();
    for (const url of ['', 'example.com', 'ftp://example.com', 'javascript:alert(1)', 'https://', 'https://user:pw@example.com',
      'https://example.com/"; do shell script "id"', 'https://exa mple.com', `https://example.com/${'a'.repeat(2048)}`]) {
      expect(urlIssue(url), url).not.toBeNull();
    }
  });

  it('flags text the service rejects', () => {
    expect(textIssue('line\n\ttab')).toBeNull();
    expect(textIssue('"; do shell script "id"')).toBeNull();
    for (const text of ['', 'bell\u0007', 'cr\r', 'esc\u001b[0m', 'x'.repeat(2049), 'é'.repeat(1025)]) expect(textIssue(text), JSON.stringify(text)).not.toBeNull();
  });

  it('rejects invisible format and separator characters like the Rust validator', () => {
    const invisible: number[] = [];
    for (let code = 0x200b; code <= 0x200f; code++) invisible.push(code);
    for (let code = 0x202a; code <= 0x202e; code++) invisible.push(code);
    for (let code = 0x2060; code <= 0x2069; code++) invisible.push(code);
    invisible.push(0x2028, 0x2029, 0xfeff, 0x00ad, 0x061c, 0x180e, 0x206f, 0xfff9, 0xe0001, 0xe0041);
    for (const code of invisible) expect(textIssue(`safe${String.fromCodePoint(code)}text`), code.toString(16)).toMatch(/invisible/);
    for (const text of ['Zażółć gęślą jaźń ĄĆĘŁŃÓŚŹŻ', 'Thanks! 🎉👍', 'heart ❤\u{FE0F} (with variation selector)', 'flag 🇵🇱', '日本語 ελληνικά русский', 'tab\tand\nnewline']) {
      expect(textIssue(text), text).toBeNull();
    }
    expect(textIssue('family 👨\u{200D}👩\u{200D}👧')).not.toBeNull();
  });

  it('agrees with the shared fixture on text snippets', () => {
    for (const action of fixture.valid) if (action.type === 'text') expect(textIssue(action.text), action.text).toBeNull();
    const invalid = fixture.invalid.filter((action): action is { type: 'text'; text: string } => (action as { type?: string }).type === 'text');
    expect(invalid.length).toBeGreaterThan(0);
    for (const action of invalid) expect(textIssue(action.text), JSON.stringify(action.text)).not.toBeNull();
  });
});
