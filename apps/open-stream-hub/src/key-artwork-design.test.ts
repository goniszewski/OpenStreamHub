import { describe, expect, it } from 'vitest';
import type { BindingAction } from './contracts';
import {
  appIconId, buildKeyArtworkDesignRequest, defaultKeyArtworkDesign, designTextIssue, isAppIconId,
  keyDesignLayout, keyTextMaxLines, keyTextSizes, readRecentKeyIcons,
  recentKeyIconsLimit, recordRecentKeyIcon, searchKeyIcons, suggestedIconIds, wrapKeyText,
} from './key-artwork-design';
import type { KeyArtworkDesign } from './key-artwork-design';
import { keyIcons } from './key-icons';
import { FakeStorage } from './fake-storage';

// Monospace fake measure: every character (the ellipsis included) is 7 px wide,
// so wrapping is deterministic and independent of the font pixel size.
const measure = (text: string) => text.length * 7;
const design = (overrides: Partial<KeyArtworkDesign> = {}): KeyArtworkDesign => ({ ...defaultKeyArtworkDesign, ...overrides });

describe('bundled key icons', () => {
  it('is a committed offline catalog with unique ids and bare stroke markup', () => {
    expect(keyIcons.length).toBeGreaterThan(100);
    expect(new Set(keyIcons.map(icon => icon.id)).size).toBe(keyIcons.length);
    for (const icon of keyIcons) {
      expect(icon.svg).toMatch(/^<(path|rect|circle|ellipse|line|polyline|polygon)\s/);
      expect(icon.svg).not.toContain('http');
      expect(icon.svg).not.toMatch(/stroke=/);
    }
  });

  it('searches ids, names and keywords, and returns the catalog order for empty queries', () => {
    expect(searchKeyIcons('louder')).toEqual(['volume-2']);
    expect(searchKeyIcons('obs stream scene')).toEqual(['clapperboard']);
    expect(searchKeyIcons('nothing-matches-this-query')).toEqual([]);
    expect(searchKeyIcons('   ')).toEqual(keyIcons.map(icon => icon.id));
  });
});

describe('suggested icons from the assigned action', () => {
  it('maps each action family, honouring media and system commands', () => {
    expect(suggestedIconIds({ type: 'toggle_mute' })).toEqual(['volume-x', 'mic-off']);
    expect(suggestedIconIds({ type: 'set_volume', percent: 40 })).toEqual(['volume-2', 'volume-1']);
    expect(suggestedIconIds({ type: 'open_url', url: 'https://example.com' })).toEqual(['globe', 'link']);
    expect(suggestedIconIds({ type: 'media', command: 'next_track' })).toEqual(['skip-forward']);
    expect(suggestedIconIds({ type: 'system', command: 'screenshot' })).toEqual(['camera']);
    expect(suggestedIconIds({ type: 'obs_scene', scene: 'Scene' })).toEqual(['clapperboard']);
    expect(suggestedIconIds({ type: 'obs_source_visibility', scene: 'S', source: 'So', op: 'toggle' })).toEqual(['eye', 'eye-off']);
    expect(suggestedIconIds({ type: 'shortcut', key: 'c', modifiers: ['command'] })).toEqual(['keyboard']);
  });

  it('offers the real app icon for Open application and nothing without an action', () => {
    const launch: BindingAction = { type: 'launch_app', bundle_id: 'com.apple.finder' };
    expect(suggestedIconIds(launch)).toEqual([appIconId('com.apple.finder'), 'app-window']);
    expect(isAppIconId(suggestedIconIds(launch)[0])).toBe(true);
    expect(suggestedIconIds(null)).toEqual([]);
    expect(suggestedIconIds(undefined)).toEqual([]);
  });
});

describe('label text rules', () => {
  it('accepts empty text (icon-only artwork) and mirrors the key label byte cap', () => {
    expect(designTextIssue('')).toBeNull();
    expect(designTextIssue('Mute')).toBeNull();
    expect(designTextIssue('x'.repeat(129))).toContain('128 bytes');
    expect(designTextIssue('line\nbreak')).toContain('control characters');
  });
});

describe('text wrapping', () => {
  it('wraps greedily within the key padding and reports the size line caps', () => {
    // 60 px of usable width at this measure (72 − 2 × 6 padding).
    expect(keyTextMaxLines(keyTextSizes[0].px)).toBe(3);
    expect(keyTextMaxLines(keyTextSizes[2].px)).toBe(2);
    expect(keyTextMaxLines(keyTextSizes[3].px)).toBe(1);
    expect(wrapKeyText('mute mic', 14, 3, measure)).toEqual(['mute mic']);
    // A dropped word that still fits the last line is absorbed, not ellipsised.
    expect(wrapKeyText('mute microphone again', 14, 3, measure)).toEqual(['mute', 'micropho', 'ne again']);
  });

  it('hard-breaks words wider than a line and ellipsises dropped text', () => {
    expect(wrapKeyText('extraordinary', 14, 3, measure)).toEqual(['extraord', 'inary']);
    expect(wrapKeyText('one two three four five', 14, 2, measure)).toEqual(['one two', 'three…']);
  });
});

describe('layout', () => {
  it('centres an icon-only design', () => {
    const layout = keyDesignLayout(design({ iconId: 'volume-2', text: '' }), measure);
    expect(layout.icon).toEqual({ x: 14, y: 14, size: 44 });
    expect(layout.lines).toEqual([]);
  });

  it('anchors text per alignment and gives the icon the remaining band', () => {
    // 'mute mic' fits one 16.8 px line at the medium size.
    const bottom = keyDesignLayout(design({ iconId: 'volume-2', text: 'mute mic', textSize: 1, align: 'bottom' }), measure);
    expect(bottom.lines).toEqual([{ text: 'mute mic', y: 72 - 6 - 16.8 }]);
    expect(bottom.icon!.size).toBeCloseTo(72 - 12 - 16.8 - 3);
    expect(bottom.icon!.y).toBeLessThan(bottom.lines[0].y);
    const top = keyDesignLayout(design({ iconId: 'volume-2', text: 'mute mic', textSize: 1, align: 'top' }), measure);
    expect(top.lines[0].y).toBe(6);
    expect(top.icon!.y).toBeGreaterThan(top.lines[0].y + 16.8);
    const middle = keyDesignLayout(design({ iconId: 'volume-2', text: 'mute mic', textSize: 1, align: 'middle' }), measure);
    expect(middle.lines[0].y).toBeCloseTo((72 - 16.8) / 2);
  });

  it('squeezes the icon into whatever band the text leaves', () => {
    // Three medium lines are centred and leave only a sliver below.
    const squeezed = keyDesignLayout(design({ iconId: 'volume-2', text: 'eeee ffff gggg', textSize: 1, align: 'middle' }), measure);
    expect(squeezed.lines).toHaveLength(3);
    expect(squeezed.icon!.size).toBeCloseTo(1.8);
    const textOnly = keyDesignLayout(design({ text: 'Mute', iconId: null }), measure);
    expect(textOnly.icon).toBeNull();
    expect(textOnly.lines[0]).toEqual({ text: 'Mute', y: 72 - 6 - 16.8 });
  });
});

describe('prepared-image requests', () => {
  it('passes identity geometry and the background colour to the key pipeline', () => {
    const built = buildKeyArtworkDesignRequest(design({ background: '#204060' }), 'QUJD');
    expect(built.ok).toBe(true);
    if (built.ok) {
      expect(built.request).toEqual({
        image_base64: 'QUJD', target: 'keypad', quality: 85, fit: 'contain', quarter_turns: 0, background: [32, 64, 96],
      });
    }
    expect(buildKeyArtworkDesignRequest(design({ background: 'nope' }), 'QUJD')).toMatchObject({ ok: false });
    expect(buildKeyArtworkDesignRequest(design(), '').ok).toBe(false);
  });
});

describe('recently used icons', () => {
  it('records deduped ids newest first, bounded, and survives junk storage', () => {
    const storage = new FakeStorage();
    expect(readRecentKeyIcons(storage)).toEqual([]);
    for (const id of ['volume-2', 'globe', 'volume-2', 'play', 'star']) recordRecentKeyIcon(storage, id);
    expect(readRecentKeyIcons(storage)).toEqual(['star', 'play', 'volume-2', 'globe']);
    for (let index = 0; index < recentKeyIconsLimit + 3; index++) recordRecentKeyIcon(storage, `icon-${index}`);
    const stored: unknown = JSON.parse(storage.getItem('open-stream-hub.recent-key-icons')!);
    // The rewrite keeps only ids that still resolve, newest first, and stays bounded.
    expect(stored).toEqual(['icon-10', 'star', 'play', 'volume-2', 'globe']);
    // Unknown ids are dropped on read; app icons stay (they render once fetched).
    recordRecentKeyIcon(storage, 'app:com.apple.finder');
    expect(readRecentKeyIcons(storage)).toContain('app:com.apple.finder');
    expect(readRecentKeyIcons(storage)).not.toContain('icon-999');
    storage.setItem('open-stream-hub.recent-key-icons', '{oops');
    expect(readRecentKeyIcons(storage)).toEqual([]);
  });
});
