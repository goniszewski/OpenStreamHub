import { describe, expect, it } from 'vitest';
import { maximumLabelBytes } from './draft-model';
import { copyName, ImportError, importName, maximumImportBytes, parseImportedProfile, readImportFile } from './profile-library';

const bytes = (text: string) => new TextEncoder().encode(text).length;

describe('copyName', () => {
  it('appends copy, then numbers, among existing names', () => {
    expect(copyName('Desk', ['Desk'])).toBe('Desk copy');
    expect(copyName('Desk', ['Desk', 'Desk copy'])).toBe('Desk copy 2');
    expect(copyName('Desk', ['Desk', 'Desk copy', 'desk COPY 2'])).toBe('Desk copy 3');
    expect(copyName('Desk copy', ['Desk copy'])).toBe('Desk copy copy');
  });

  it('keeps the result within the label byte limit', () => {
    const long = 'é'.repeat(64);
    const name = copyName(long, [long, copyName(long, [long])]);
    expect(bytes(name)).toBeLessThanOrEqual(maximumLabelBytes);
    expect(name.endsWith(' copy 2')).toBe(true);
  });
});

describe('importName', () => {
  it('keeps a free name and suffixes conflicts', () => {
    expect(importName('Desk', ['Other'])).toBe('Desk');
    expect(importName('Desk', ['Desk'])).toBe('Desk (imported)');
    expect(importName('Desk', ['Desk', 'Desk (imported)'])).toBe('Desk (imported 2)');
    expect(importName('Desk', ['Desk', 'Desk (imported)', 'Desk (imported 2)'])).toBe('Desk (imported 3)');
  });
});

describe('parseImportedProfile', () => {
  const legacy = { schema_version: 1, name: 'Old', bindings: [{ module: 'm4315', control: 1, gesture: 'pressed', action: { type: 'toggle_mute' } }] };

  it('rejects oversized text, bad JSON and non-profiles', () => {
    expect(() => parseImportedProfile('x'.repeat(maximumImportBytes + 1))).toThrow(/at most 1 MiB/);
    expect(() => parseImportedProfile('{nope')).toThrow(/not valid JSON/);
    expect(() => parseImportedProfile('[]')).toThrow(/expected a JSON object/);
    expect(() => parseImportedProfile('null')).toThrow(ImportError);
    expect(() => parseImportedProfile('{"schema_version":2}')).toThrow(/name and a bindings/);
    expect(() => parseImportedProfile(JSON.stringify({ schema_version: 6, name: 'x', bindings: [] }))).toThrow(/newer version of Open Stream Hub/);
    expect(() => parseImportedProfile(JSON.stringify({ schema_version: 0, name: 'x', bindings: [] }))).toThrow(/Unsupported profile version/);
    expect(() => parseImportedProfile(JSON.stringify({ schema_version: 2, name: 'x', bindings: [] }))).toThrow(/missing its keypad/);
  });

  it('upgrades a legacy profile to the current schema without executing anything', () => {
    const profile = parseImportedProfile(JSON.stringify(legacy));
    expect(profile.schema_version).toBe(5);
    expect(profile.bindings).toEqual([]);
    expect(profile.keypad!.pages[0].slots[1]).toMatchObject({ type: 'action' });
  });

  it('keeps unknown fields so the backend can reject them', () => {
    const profile = parseImportedProfile(JSON.stringify({ ...legacy, extra: true })) as unknown as Record<string, unknown>;
    expect(profile.extra).toBe(true);
  });
});

describe('readImportFile', () => {
  it('rejects a big file before reading it', async () => {
    let read = false;
    await expect(readImportFile({ size: maximumImportBytes + 1, text: async () => { read = true; return ''; } })).rejects.toThrow(/at most 1 MiB/);
    expect(read).toBe(false);
    await expect(readImportFile({ size: 2, text: async () => '{}' })).resolves.toBe('{}');
  });
});
