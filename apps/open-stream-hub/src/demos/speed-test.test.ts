import { describe, expect, it } from 'vitest';
import { describePass, describeResults, palette, previewTiles, quantiles, safeFrameRate } from './speed-test';
import type { PassResult } from './speed-test';

const result = (over: Partial<PassResult> = {}): PassResult => ({
  pass: 'grid', seconds: 30, writes: 3000, failed: 0, frames: 200, writes_per_second: 100, frames_per_second: 6.67,
  round_trip: { count: 3000, mean_ms: 10, p50_ms: 9.5, p95_ms: 14, max_ms: 40 }, device: { count: 3000, mean_ms: 6, p50_ms: 5, p95_ms: 8, max_ms: 20 },
  cancelled: false, stopped: null, ...over,
});

describe('speed test model', () => {
  it('previews the last fifteen writes of each pass on the keys', () => {
    expect(previewTiles('grid', 0).every(tile => tile.tone === 'blank')).toBe(true);
    const part = previewTiles('grid', 3);
    expect(part.slice(0, 3).map(tile => tile.colour)).toEqual([palette[0], palette[1], palette[2]]);
    expect(part[3].tone).toBe('blank');
    // The second frame shifts every key's colour by one.
    expect(previewTiles('grid', 16)[0].colour).toBe(palette[1]);
    expect(previewTiles('single', 16)[0].colour).toBe(palette[0 + 15 % 8]);
  });

  it('works out quantiles by nearest rank', () => {
    const q = quantiles(Array.from({ length: 20 }, (_, index) => index + 1));
    expect([q.p50_ms, q.p95_ms, q.max_ms, q.mean_ms]).toEqual([10, 19, 20, 10.5]);
    expect(quantiles([]).count).toBe(0);
  });

  it('writes the numbers as plain text, with what went wrong', () => {
    const text = describePass(result({ failed: 2, stopped: '10 writes in a row failed' }));
    expect(text).toContain('Full grid (15 keys per frame), 30.0 s: 6.7 frames/s (100.0 images/s); 3000 images, 2 failed (10 writes in a row failed)');
    expect(text).toContain('round trip p50 9.5 ms, p95 14.0 ms, max 40.0 ms');
    expect(describePass(result({ pass: 'single', device: null }))).toContain('device write not reported');
    const all = describeResults([result()], [12, 8, 10], '2026-10-12');
    expect(all.split('\n')[0]).toBe('Display speed test, 15-key module, 2026-10-12');
    expect(all).toContain('Press to redraw (3 presses, one key written): median 10.0 ms, max 12.0 ms');
  });

  it('suggests a frame rate other demos can keep, only from a clean grid pass', () => {
    expect(safeFrameRate(result())).toBe(5);
    expect(safeFrameRate(result({ frames_per_second: 40 }))).toBe(10);
    expect(safeFrameRate(result({ failed: 1 }))).toBeNull();
    expect(safeFrameRate(result({ cancelled: true }))).toBeNull();
    expect(safeFrameRate(undefined)).toBeNull();
  });
});
