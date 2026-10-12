import { describe, expect, it } from 'vitest';
import { appear, bestKey, clearFlash, escaped, hit, keepBest, light, moleDuration, moleEndMs, moleStartMs, newReaction, newWhack, pickKey, randomWait, reactionDone, reactionPress, reactionRounds, reactionTiles, readBest, roundMs, summarise, whackTiles } from './whack-game';

const memory = (initial: Record<string, string> = {}) => {
  const data = { ...initial };
  return { getItem: (key: string) => data[key] ?? null, setItem: (key: string, value: string) => { data[key] = value; } };
};

describe('placing moles', () => {
  it('never picks the previous key, whatever the random number', () => {
    for (let previous = 0; previous < 15; previous++) for (const random of [0, 0.3, 0.5, 0.999999, 1]) expect(pickKey(previous, () => random)).not.toBe(previous);
    expect(pickKey(null, () => 0)).toBe(0);
    expect(pickKey(0, () => 0)).toBe(1);
    expect(pickKey(14, () => 0.999999)).toBe(13);
  });

  it('reaches every key and never repeats one over a long run', () => {
    let state = newWhack();
    const seen = new Set<number>();
    let seed = 7;
    const random = () => { seed = (seed * 16807) % 2147483647; return seed / 2147483647; };
    for (let step = 0; step < 500; step++) {
      const next = appear(state, random);
      expect(next.mole).not.toBe(state.last);
      seen.add(next.mole!);
      state = escaped(next);
    }
    expect(seen.size).toBe(15);
  });

  it('speeds up over the round and stays within its ends', () => {
    expect(moleDuration(0)).toBe(moleStartMs);
    expect(moleDuration(roundMs)).toBe(moleEndMs);
    expect(moleDuration(roundMs * 2)).toBe(moleEndMs);
    expect(moleDuration(-5)).toBe(moleStartMs);
    expect(moleDuration(roundMs / 2)).toBeLessThan(moleDuration(roundMs / 4));
  });
});

describe('scoring', () => {
  it('scores a hit on the mole and sends it away', () => {
    const shown = appear(newWhack(), () => 0);
    const after = hit(shown, shown.mole!);
    expect(after).toMatchObject({ score: 1, hits: 1, misses: 0, mole: null, last: shown.mole, flash: { key: shown.mole, kind: 'hit' } });
  });

  it('costs a point for an empty key, and the mole stays', () => {
    let state = hit(appear(newWhack(), () => 0), 0);
    state = appear(state, () => 0.5);
    const wrong = state.mole === 3 ? 4 : 3;
    const after = hit(state, wrong);
    expect(after).toMatchObject({ score: 0, misses: 1, mole: state.mole, flash: { key: wrong, kind: 'miss' } });
  });

  it('costs a point when the mole escapes, never goes below zero, and ignores an escape with no mole', () => {
    let state = newWhack();
    state = escaped(appear(state, () => 0));
    expect(state.score).toBe(0);
    expect(state.misses).toBe(1);
    expect(state.flash?.kind).toBe('miss');
    expect(escaped(state)).toBe(state);
    const scored = hit(appear(newWhack(), () => 0), 0);
    expect(escaped(appear(scored, () => 0.9)).score).toBe(0);
  });

  it('presses with no mole showing cost a point too', () => {
    const point = hit(appear(newWhack(), () => 0), 0);
    expect(hit(point, 5).score).toBe(0);
  });

  it('shows the mole, its hole and the verdict on the keys', () => {
    const shown = appear(newWhack(), () => 0);
    const tiles = whackTiles(shown);
    expect(tiles).toHaveLength(15);
    expect(tiles[shown.mole!].tone).toBe('mole');
    expect(tiles.filter(tile => tile.tone === 'empty')).toHaveLength(14);
    const after = hit(shown, shown.mole!);
    expect(whackTiles(after)[shown.mole!].tone).toBe('hit');
    expect(whackTiles(clearFlash(after)).every(tile => tile.tone === 'empty')).toBe(true);
    expect(whackTiles(hit(shown, 14))[14].tone).toBe('miss');
    expect(clearFlash(newWhack())).toEqual(newWhack());
  });
});

describe('the best score', () => {
  it('keeps the best and never lowers it', () => {
    const storage = memory();
    expect(readBest(storage)).toBe(0);
    expect(keepBest(storage, 12)).toBe(12);
    expect(keepBest(storage, 5)).toBe(12);
    expect(readBest(storage)).toBe(12);
    expect(keepBest(storage, 20)).toBe(20);
  });

  it('survives storage that is missing, broken or holds nonsense', () => {
    expect(readBest(memory({ [bestKey]: 'lots' }))).toBe(0);
    expect(readBest(memory({ [bestKey]: '-4' }))).toBe(0);
    const broken = { getItem: () => { throw new Error('no'); }, setItem: () => { throw new Error('no'); } };
    expect(readBest(broken)).toBe(0);
    expect(keepBest(broken, 3)).toBe(3);
  });
});

describe('reaction test', () => {
  it('waits between one second and three, whatever the random number', () => {
    expect(randomWait(() => 0)).toBe(1000);
    expect(randomWait(() => 0.999999)).toBe(3000);
    expect(randomWait(() => 0.5)).toBe(2000);
  });

  it('records the time of a press on the lit key over five rounds, never lighting the same key twice', () => {
    let state = newReaction();
    let previous: number | null = null;
    [310, 250, 410, 280, 330].forEach(time => {
      expect(reactionDone(state)).toBe(false);
      state = light(state, () => 0);
      expect(state.lit).not.toBe(previous);
      previous = state.lit;
      state = reactionPress(state, state.lit!, time);
      expect(state.lit).toBeNull();
    });
    expect(state.times).toEqual([310, 250, 410, 280, 330]);
    expect(reactionDone(state)).toBe(true);
    expect(state.times).toHaveLength(reactionRounds);
  });

  it('counts a press before the light as early and a press on another key as nothing', () => {
    const waiting = reactionPress(newReaction(), 4, 0);
    expect(waiting).toMatchObject({ early: 1, times: [] });
    const lit = light(waiting, () => 0);
    expect(reactionPress(lit, lit.lit! + 1, 100)).toBe(lit);
    expect(reactionPress(lit, lit.lit!, 99.6).times).toEqual([100]);
  });

  it('lights only the chosen key, in colour', () => {
    const tiles = reactionTiles(light(newReaction(), () => 0));
    expect(tiles.filter(tile => tile.tone === 'fill')).toHaveLength(1);
    expect(tiles.filter(tile => tile.tone === 'blank')).toHaveLength(14);
    expect(reactionTiles(newReaction()).every(tile => tile.tone === 'blank')).toBe(true);
  });

  it('summarises the spread of the times', () => {
    expect(summarise([])).toBeNull();
    expect(summarise([310, 250, 410, 280, 330])).toEqual({ best: 250, worst: 410, spread: 160, mean: 316, median: 310 });
    expect(summarise([200, 300])).toMatchObject({ median: 250, spread: 100 });
  });
});
