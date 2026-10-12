import type { Tile } from './tictactoe-game';

// Whack-a-mole and the reaction test share the 15 keys, numbered row by row from the top left.
export const keyCount = 15;
export const roundMs = 30000;
// How long a mole stays: it starts slow and speeds up over the round. Every change is two key writes at most, well within
// what the display speed test measures.
export const moleStartMs = 1000;
export const moleEndMs = 450;
// The pause between one mole leaving and the next appearing, and how long a hit or miss mark stays on its key.
export const gapMs = 250;
export const flashMs = 300;
// A press this soon after a mole left still counts: the key took time to draw and the press took time to arrive.
export const graceMs = 200;

// A key other than `previous`, so the same key never comes up twice in a row.
export function pickKey(previous: number | null, random: () => number = Math.random): number {
  const choices = Array.from({ length: keyCount }, (_, key) => key).filter(key => key !== previous);
  return choices[Math.min(choices.length - 1, Math.floor(random() * choices.length))];
}

// How long a mole stays at `elapsed` ms into the round.
export const moleDuration = (elapsed: number): number => Math.round(moleStartMs - (moleStartMs - moleEndMs) * Math.min(Math.max(elapsed, 0), roundMs) / roundMs);

export interface Whack {
  score: number;
  hits: number;
  misses: number;
  // The key the mole is on, and the one it was last on.
  mole: number | null;
  last: number | null;
  flash: { key: number; kind: 'hit' | 'miss' } | null;
  // The mole that has just escaped and the score before it cost a point, while a late press can still count.
  late: { key: number; score: number } | null;
}

export const newWhack = (): Whack => ({ score: 0, hits: 0, misses: 0, mole: null, last: null, flash: null, late: null });

// A point is lost, but the score never drops below zero.
const lose = (score: number) => Math.max(0, score - 1);

export const appear = (whack: Whack, random: () => number = Math.random): Whack => ({ ...whack, mole: pickKey(whack.last, random), flash: null, late: null });

// A press: the mole's key scores; any other key (or any key while no mole shows) costs a point.
export function hit(whack: Whack, key: number): Whack {
  if (whack.mole === null && whack.late?.key === key) return { ...whack, score: whack.late.score + 1, hits: whack.hits + 1, misses: whack.misses - 1, last: key, flash: { key, kind: 'hit' }, late: null };
  if (key === whack.mole) return { ...whack, score: whack.score + 1, hits: whack.hits + 1, mole: null, last: key, flash: { key, kind: 'hit' } };
  return { ...whack, score: lose(whack.score), misses: whack.misses + 1, flash: { key, kind: 'miss' } };
}

// The mole left before it was hit.
export const escaped = (whack: Whack): Whack => whack.mole === null ? whack
  : { ...whack, score: lose(whack.score), misses: whack.misses + 1, mole: null, last: whack.mole, flash: { key: whack.mole, kind: 'miss' }, late: { key: whack.mole, score: whack.score } };

export const clearLate = (whack: Whack): Whack => whack.late ? { ...whack, late: null } : whack;

export const clearFlash = (whack: Whack): Whack => whack.flash ? { ...whack, flash: null } : whack;

const hole: Tile = { tone: 'empty' };

export function whackTiles(whack: Whack): Tile[] {
  return Array.from({ length: keyCount }, (_, key) => whack.flash?.key === key ? { tone: whack.flash.kind } : key === whack.mole ? { tone: 'mole' } : hole);
}

export const bestKey = 'open-stream-hub.demo.whack-best';

export function readBest(storage: Pick<Storage, 'getItem'>): number {
  try {
    const value = Number(storage.getItem(bestKey));
    return Number.isInteger(value) && value > 0 ? value : 0;
  } catch { return 0; }
}

// Keeps the better of the stored best and `score`. Returns the best now.
export function keepBest(storage: Pick<Storage, 'getItem' | 'setItem'>, score: number): number {
  const best = Math.max(readBest(storage), score);
  try { storage.setItem(bestKey, String(best)); } catch { /* a convenience; keep working without it */ }
  return best;
}

// The reaction test: one key lights up after a random wait; the time to press it is the result.
export const reactionRounds = 5;
export const minWaitMs = 1000;
export const maxWaitMs = 3000;
export const randomWait = (random: () => number = Math.random): number => minWaitMs + Math.floor(random() * (maxWaitMs - minWaitMs + 1));
export const reactionColour = '#2fe06b';

export interface Reaction {
  // The key that is lit now; null while waiting.
  lit: number | null;
  last: number | null;
  times: number[];
  // Presses before any key lit.
  early: number;
}

export const newReaction = (): Reaction => ({ lit: null, last: null, times: [], early: 0 });

export const light = (reaction: Reaction, random: () => number = Math.random): Reaction => ({ ...reaction, lit: pickKey(reaction.last, random) });

// A press while waiting is early (and not counted as a time); on the lit key it records `elapsed` ms; other keys do nothing.
export function reactionPress(reaction: Reaction, key: number, elapsed: number): Reaction {
  if (reaction.lit === null) return { ...reaction, early: reaction.early + 1 };
  if (key !== reaction.lit) return reaction;
  return { ...reaction, lit: null, last: key, times: [...reaction.times, Math.max(0, Math.round(elapsed))] };
}

export const reactionDone = (reaction: Reaction): boolean => reaction.times.length >= reactionRounds;

export function reactionTiles(reaction: Reaction): Tile[] {
  return Array.from({ length: keyCount }, (_, key) => key === reaction.lit ? { tone: 'fill', colour: reactionColour } : { tone: 'blank' });
}

export interface ReactionSummary { best: number; mean: number; median: number; worst: number; spread: number }

export function summarise(times: readonly number[]): ReactionSummary | null {
  if (times.length === 0) return null;
  const sorted = [...times].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  const median = sorted.length % 2 ? sorted[middle] : Math.round((sorted[middle - 1] + sorted[middle]) / 2);
  return { best: sorted[0], worst: sorted[sorted.length - 1], spread: sorted[sorted.length - 1] - sorted[0], mean: Math.round(sorted.reduce((sum, time) => sum + time, 0) / sorted.length), median };
}
