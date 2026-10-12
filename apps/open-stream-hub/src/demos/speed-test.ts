import type { Tile } from './tictactoe-game';

// The colours the test cycles through on the keys; the same eight as `speed.rs`.
export const palette = ['#ff4040', '#ffa020', '#f0e030', '#40dc60', '#30d2dc', '#4080ff', '#a060ff', '#ff60c8'] as const;
export const keyCount = 15;

export type Pass = 'grid' | 'single';
export interface Progress { writes: number; frames: number; elapsed_ms: number }
export interface Quantiles { count: number; mean_ms: number; p50_ms: number; p95_ms: number; max_ms: number }
export interface PassResult {
  pass: Pass; seconds: number; writes: number; failed: number; frames: number; writes_per_second: number; frames_per_second: number;
  round_trip: Quantiles; device: Quantiles | null; cancelled: boolean; stopped: string | null;
}

// What the 15 keys show after `writes` writes of a pass: the grid pass draws key after key, each frame shifting every
// colour; the single pass moves one key on at a time. Keys not yet written are dark.
export function previewTiles(pass: Pass, writes: number): Tile[] {
  const tiles: Tile[] = Array.from({ length: keyCount }, () => ({ tone: 'blank' }));
  for (let write = Math.max(0, writes - keyCount); write < writes; write++) {
    const key = write % keyCount;
    const image = pass === 'grid' ? (Math.floor(write / keyCount) + key) % palette.length : write % palette.length;
    tiles[key] = { tone: 'fill', colour: palette[image] };
  }
  return tiles;
}

export function quantiles(samples: readonly number[]): Quantiles {
  if (!samples.length) return { count: 0, mean_ms: 0, p50_ms: 0, p95_ms: 0, max_ms: 0 };
  const sorted = [...samples].sort((a, b) => a - b);
  const rank = (share: number) => sorted[Math.min(sorted.length, Math.max(1, Math.ceil(share * sorted.length))) - 1];
  return { count: sorted.length, mean_ms: sorted.reduce((sum, value) => sum + value, 0) / sorted.length, p50_ms: rank(0.5), p95_ms: rank(0.95), max_ms: sorted[sorted.length - 1] };
}

const one = (value: number) => value.toFixed(1);
const row = (label: string, q: Quantiles | null) => q && q.count ? `${label} p50 ${one(q.p50_ms)} ms, p95 ${one(q.p95_ms)} ms, max ${one(q.max_ms)} ms` : `${label} not reported`;

export function describePass(result: PassResult): string {
  const name = result.pass === 'grid' ? 'Full grid (15 keys per frame)' : 'One key at a time';
  const rate = result.pass === 'grid'
    ? `${one(result.frames_per_second)} frames/s (${one(result.writes_per_second)} images/s)`
    : `${one(result.writes_per_second)} images/s`;
  const notes = [result.cancelled ? 'cancelled early' : '', result.stopped ?? ''].filter(Boolean).join('; ');
  return [
    `${name}, ${one(result.seconds)} s: ${rate}; ${result.writes} images, ${result.failed} failed${notes ? ` (${notes})` : ''}`,
    `  ${row('round trip', result.round_trip)}`,
    `  ${row('device write', result.device)}`,
  ].join('\n');
}

// Everything the page reports, as plain text for docs/masterhub-hardware-results.md.
export function describeResults(results: readonly PassResult[], latency: readonly number[], date: string): string {
  const lines = [`Display speed test, 15-key module, ${date}`, ...results.map(describePass)];
  if (latency.length) {
    const q = quantiles(latency);
    lines.push(`Press to redraw (${q.count} presses, one key written): median ${one(q.p50_ms)} ms, max ${one(q.max_ms)} ms`);
  }
  return lines.join('\n');
}

// A safe frame rate for other demos: the grid pass's sustained rate, rounded down, never above 10 (the artwork GIF limit).
export const safeFrameRate = (grid: PassResult | undefined): number | null =>
  grid && !grid.cancelled && grid.failed === 0 && grid.frames_per_second > 0 ? Math.max(1, Math.min(10, Math.floor(grid.frames_per_second * 0.8))) : null;
