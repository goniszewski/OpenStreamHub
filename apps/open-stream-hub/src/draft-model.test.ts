import { describe, expect, it } from 'vitest';
import { clampUtf8, closeDecision, labelIssue, maximumLabelBytes, parseIntegerInput, stableStringify } from './draft-model';
import { actionLabel } from './ui-model';

const bytes = (text: string) => new TextEncoder().encode(text).length;

describe('stable dirty check', () => {
  it('ignores object key order but not array order or values', () => {
    expect(stableStringify({ b: 1, a: { d: [1, { y: 2, x: 1 }], c: null } })).toBe(stableStringify({ a: { c: null, d: [1, { x: 1, y: 2 }] }, b: 1 }));
    expect(stableStringify({ a: [1, 2] })).not.toBe(stableStringify({ a: [2, 1] }));
    expect(stableStringify({ a: 1 })).not.toBe(stableStringify({ a: 2 }));
  });
});

describe('UTF-8 label clamping', () => {
  it('keeps short labels and cuts long ones on a code point boundary', () => {
    expect(clampUtf8('Mute')).toBe('Mute');
    const clamped = clampUtf8('é'.repeat(100));
    expect(bytes(clamped)).toBeLessThanOrEqual(maximumLabelBytes);
    expect(clamped).toBe('é'.repeat(64));
    expect(clampUtf8('a' + '😀'.repeat(40))).toBe('a' + '😀'.repeat(31));
  });
  it('fits auto-generated key labels to the Rust 128-byte limit', () => {
    const label = clampUtf8(actionLabel({ type: 'launch_app', bundle_id: `com.example.${'x'.repeat(200)}` }));
    expect(bytes(label)).toBeLessThanOrEqual(128);
    expect(label.startsWith('Open com.example.')).toBe(true);
    expect(labelIssue(label, 'Key label')).toBeNull();
  });
});

describe('inline label validation', () => {
  it('mirrors the Rust nonempty, byte-length and control-character rules', () => {
    expect(labelIssue('', 'Profile name')).toBe('Profile name cannot be empty.');
    expect(labelIssue('   ', 'Key label')).toBe('Key label cannot be empty.');
    expect(labelIssue('x'.repeat(129), 'Page name')).toContain('128 bytes');
    expect(labelIssue('é'.repeat(64), 'Page name')).toBeNull();
    expect(labelIssue('Tab\there', 'Key label')).toContain('control characters');
    expect(labelIssue('Work', 'Profile name')).toBeNull();
  });
});

describe('integer inputs', () => {
  it('accepts only whole numbers within range', () => {
    expect(parseIntegerInput('42', 0, 100)).toEqual({ ok: true, value: 42 });
    expect(parseIntegerInput(' -7 ', -100, 100)).toEqual({ ok: true, value: -7 });
    expect(parseIntegerInput('-0', 0, 100)).toEqual({ ok: true, value: 0 });
    for (const raw of ['', '-', '1.5', '1e2', 'NaN', 'abc', '101', '-1']) expect(parseIntegerInput(raw, 0, 100).ok).toBe(false);
  });
  it('rejects a zero volume change', () => {
    const zero = parseIntegerInput('0', -100, 100, { nonZero: true });
    expect(zero.ok).toBe(false);
    expect(zero.ok ? '' : zero.error).toContain('Zero');
  });
});

describe('window close decision', () => {
  it('closes clean drafts, asks for unsaved ones and waits while busy', () => {
    expect(closeDecision(false, false)).toBe('allow');
    expect(closeDecision(true, false)).toBe('allow');
    expect(closeDecision(false, true)).toBe('confirm');
    expect(closeDecision(true, true)).toBe('wait');
  });
});
