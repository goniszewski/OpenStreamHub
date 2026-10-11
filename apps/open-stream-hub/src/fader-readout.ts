// What the inspector shows for a physical fader: its live position and the range and jitter seen since the
// readout was last reset, so the endpoints and any wobble can be read off and recorded in the hardware results.

export interface FaderStats { last: number | null; min: number | null; max: number | null; changes: number; jitter: number; step?: number }

export const emptyFaderStats: FaderStats = { last: null, min: null, max: null, changes: 0, jitter: 0 };

// A step that undoes the one before (up then down, or down then up, by one point) counts as jitter: the shape of
// a fader resting between two readings rather than being moved.
export function recordFader(stats: FaderStats, value: number): FaderStats {
  if (stats.last === value) return stats;
  const step = stats.last === null ? 0 : value - stats.last;
  const wobble = stats.step !== undefined && stats.step !== 0 && Math.abs(step) === 1 && Math.abs(stats.step) === 1 && Math.sign(step) !== Math.sign(stats.step);
  return {
    last: value,
    min: stats.min === null ? value : Math.min(stats.min, value),
    max: stats.max === null ? value : Math.max(stats.max, value),
    changes: stats.last === null ? 0 : stats.changes + 1,
    jitter: stats.jitter + (wobble ? 1 : 0),
    step,
  };
}

export function faderStatsText(stats: FaderStats): string {
  if (stats.last === null) return 'Move the fader to see its position.';
  return `Range seen ${stats.min}–${stats.max} · ${stats.changes} ${stats.changes === 1 ? 'change' : 'changes'} · ${stats.jitter} back-and-forth ${stats.jitter === 1 ? 'step' : 'steps'} of one point`;
}
