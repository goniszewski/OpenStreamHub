import { describe, expect, it } from 'vitest';
import fixture from '../fixtures/key-faces.json';
import { drawActionKey, drawBackKey, drawFolderKey, keyFaceFingerprint, keyLabelLines, tileIconOf } from './key-face';
import type { TileIcon } from './key-face';

const icons: TileIcon[] = ['generic', 'volume', 'mute', 'play', 'next', 'previous', 'stop', 'screenshot', 'undo', 'redo', 'timer', 'web', 'application', 'text', 'sequence', 'profile', 'obs'];
const labels = ['', 'Record', 'Mute mic', 'Toggle microphone', 'A label far longer than the key can ever hold in full', 'Żółw 9'];

describe('key pictures match the ones the service draws', () => {
  it('draws every icon with every label as the Rust fixture fingerprints it', () => {
    const mismatched: string[] = [];
    for (const icon of icons) for (const label of labels) {
      const key = `${icon}|${label}`;
      if (keyFaceFingerprint(drawActionKey(icon, label)) !== (fixture as Record<string, string>)[key]) mismatched.push(key);
    }
    expect(mismatched).toEqual([]);
  });

  it('draws the folder and the Back key at every depth as the fixture does', () => {
    const expected = fixture as Record<string, string>;
    expect(keyFaceFingerprint(drawFolderKey())).toBe(expected.folder);
    for (let depth = 1; depth <= 8; depth += 1) expect(keyFaceFingerprint(drawBackKey(depth))).toBe(expected[`back|${depth}`]);
  });

  it('has a fingerprint for exactly the pictures it draws', () => {
    expect(Object.keys(fixture)).toHaveLength(icons.length * labels.length + 1 + 8);
  });
});

describe('keyLabelLines', () => {
  it('wraps a label as the module does: large for two short lines, else small, cut with a dot', () => {
    expect(keyLabelLines('Record')).toEqual({ lines: ['Record'], scale: 2 });
    expect(keyLabelLines('Mute mic')).toEqual({ lines: ['Mute', 'mic'], scale: 2 });
    expect(keyLabelLines('Toggle microphone')).toEqual({ lines: ['Toggle', 'microphone'], scale: 1 });
    expect(keyLabelLines('a'.repeat(40)).lines.at(-1)).toMatch(/\.$/);
    expect(keyLabelLines('').lines).toEqual([]);
  });
});

describe('tileIconOf', () => {
  it('picks the icon from what the first binding does, as TileIcon::of does', () => {
    expect(tileIconOf()).toBe('generic');
    expect(tileIconOf({ type: 'media', command: 'next_track' })).toBe('next');
    expect(tileIconOf({ type: 'system', command: 'redo' })).toBe('redo');
    expect(tileIconOf({ type: 'obs_record', op: 'toggle' })).toBe('obs');
    expect(tileIconOf({ type: 'toggle_mute' })).toBe('mute');
    expect(tileIconOf({ type: 'shortcut', key: 'v', modifiers: ['command'] })).toBe('generic');
    expect(tileIconOf({ type: 'macro', steps: [] })).toBe('sequence');
  });
});
