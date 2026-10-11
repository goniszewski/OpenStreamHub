import { describe, expect, it } from 'vitest';
import { emptyFaderStats, faderStatsText, recordFader } from './fader-readout';

describe('fader readout', () => {
  it('tracks the last value, the range and the number of changes', () => {
    let stats = recordFader(emptyFaderStats, 40);
    expect(stats).toMatchObject({ last: 40, min: 40, max: 40, changes: 0 });
    stats = recordFader(stats, 40);
    expect(stats.changes).toBe(0);
    stats = recordFader(recordFader(stats, 0), 100);
    expect(stats).toMatchObject({ last: 100, min: 0, max: 100, changes: 2, jitter: 0 });
  });
  it('counts one-point reversals as jitter and large moves as none', () => {
    let stats = recordFader(emptyFaderStats, 50);
    for (const value of [51, 50, 51, 50]) stats = recordFader(stats, value);
    expect(stats.jitter).toBe(3);
    stats = recordFader(recordFader(stats, 70), 30);
    expect(stats.jitter).toBe(3);
  });
  it('words the result for the hardware notes', () => {
    expect(faderStatsText(emptyFaderStats)).toBe('Move the fader to see its position.');
    expect(faderStatsText(recordFader(recordFader(emptyFaderStats, 3), 97))).toBe('Range seen 3–97 · 1 change · 0 back-and-forth steps of one point');
  });
});
