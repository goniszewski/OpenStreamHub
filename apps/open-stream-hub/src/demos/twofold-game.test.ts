import { describe, expect, it } from 'vitest';
import { canMove, centreKey, keepTwofoldBest, keyDirection, lines, move, newGame, play, press, readTwofoldBest, slide, slideTo, spawn, start, tiles, turnDirection, twofoldBestKey, valueColour } from './twofold-game';
import type { Game, Twofold } from './twofold-game';

// A fixed random source: it always draws a 2.
const first = () => 0.2;
const game = (board: number[], extra: Partial<Game> = {}): Game => ({ board, score: 0, best: 0, moves: 0, won: false, over: false, ...extra });
const state = (board: number[], extra: Partial<Game> = {}): Twofold => ({ game: game(board, extra), overlay: 'none' });

describe('twofold rules', () => {
  it('lists every line from the edge the tiles move toward', () => {
    expect(lines('left')[0]).toEqual([0, 1, 2, 3, 4]);
    expect(lines('right')[2]).toEqual([14, 13, 12, 11, 10]);
    expect(lines('up')[4]).toEqual([4, 9, 14]);
    expect(lines('down')[0]).toEqual([10, 5, 0]);
    expect(lines('up')).toHaveLength(5);
  });

  it('merges a pair once, and a tile made by a merge not again in the same move', () => {
    expect(slide([2, 2, 2, 2, 0])).toEqual({ line: [4, 4, 0, 0, 0], gained: 8 });
    expect(slide([4, 2, 2, 0, 0])).toEqual({ line: [4, 4, 0, 0, 0], gained: 4 });
    expect(slide([2, 0, 2, 0, 4])).toEqual({ line: [4, 4, 0, 0, 0], gained: 4 });
    expect(slide([2, 2, 2, 0, 0])).toEqual({ line: [4, 2, 0, 0, 0], gained: 4 });
    expect(slide([0, 0, 0])).toEqual({ line: [0, 0, 0], gained: 0 });
  });

  it('moves a whole board and keeps a board nothing could move', () => {
    const board = [2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0];
    expect(move(board, 'left')).toEqual({ board: [4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0], gained: 4 });
    expect(move(board, 'down').board.slice(10)).toEqual([4, 0, 0, 0, 2]);
    const stuck = [2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2];
    expect(canMove(stuck)).toBe(false);
    expect(canMove([...stuck.slice(0, 14), 0])).toBe(true);
  });

  it('puts a 2, or one time in ten a 4, in an empty cell', () => {
    const board = Array<number>(15).fill(2);
    board[3] = 0; board[9] = 0;
    const draws = (values: number[]) => { let at = 0; return () => values[at++]; };
    expect(spawn(board, draws([0, 0.5]))[3]).toBe(2);
    expect(spawn(board, draws([0.99, 0.05]))[9]).toBe(4);
    expect(spawn(Array<number>(15).fill(2), draws([0, 0]))).toEqual(Array<number>(15).fill(2));
  });

  it('starts with two tiles and keeps the best score', () => {
    const fresh = newGame(Math.random, 40);
    expect(fresh.board.filter(value => value !== 0)).toHaveLength(2);
    expect(fresh).toMatchObject({ score: 0, best: 40, moves: 0, won: false, over: false });
  });

  it('counts only a move that changes the board, and reaching 2048 once', () => {
    const still = game([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    expect(play(still, 'left', first)).toEqual({ game: still, moved: false, reached: false });
    const near = game([1024, 1024, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], { score: 10, best: 5 });
    const won = play(near, 'left', first);
    expect(won).toMatchObject({ moved: true, reached: true, game: { score: 2058, best: 2058, moves: 1, won: true } });
    expect(won.game.board[0]).toBe(2048);
    const again = play({ ...won.game, board: [2048, 2048, ...Array<number>(13).fill(0)] }, 'left', first);
    expect(again.reached).toBe(false);
  });

  it('ends the game when the move leaves no other', () => {
    const almost = game([2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 8, 8]);
    const result = play(almost, 'right', first);
    expect(result.moved).toBe(true);
    expect(result.game.board).toEqual([2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 2, 4, 2, 16]);
    expect(result.game.over).toBe(false);
    const stuck = game([2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2], { over: true });
    expect(play(stuck, 'left', first).moved).toBe(false);
  });
});

describe('twofold controls', () => {
  it('maps each key to the edge it is on, the middle row by its side, and the centre to nothing', () => {
    expect([0, 1, 2, 3, 4].map(keyDirection)).toEqual(['up', 'up', 'up', 'up', 'up']);
    expect([10, 11, 12, 13, 14].map(keyDirection)).toEqual(['down', 'down', 'down', 'down', 'down']);
    expect([5, 6, 7, 8, 9].map(keyDirection)).toEqual(['left', 'left', null, 'right', 'right']);
    expect(keyDirection(15)).toBeNull();
    expect(centreKey).toBe(7);
  });

  it('maps the first turn to up and down and the second to left and right, for knobs, the dial and rollers', () => {
    expect(turnDirection(0, 'clockwise')).toBe('down');
    expect(turnDirection(0, 'counter_clockwise')).toBe('up');
    expect(turnDirection(0, 'up')).toBe('up');
    expect(turnDirection(1, 'down')).toBe('right');
    expect(turnDirection(1, 'left')).toBe('left');
    expect(turnDirection(2, 'clockwise')).toBeNull();
    expect(turnDirection(0, 'pressed')).toBeNull();
  });

  it('shows the score on the centre key, and starts again on a second press', () => {
    const playing = state([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], { score: 12, best: 30 });
    const shown = press(playing, centreKey, first);
    expect(shown.overlay).toBe('score');
    expect(shown.game).toBe(playing.game);
    const again = press(shown, centreKey, first);
    expect(again.overlay).toBe('none');
    expect(again.game).toMatchObject({ score: 0, best: 30, moves: 0 });
  });

  it('lets a move dismiss the score, even one that moves nothing, and the middle row only dismiss it', () => {
    const shown: Twofold = { ...state([2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]), overlay: 'score' };
    expect(slideTo(shown, 'left', first)).toEqual({ ...shown, overlay: 'none' });
    const moved = press(shown, 0, first);
    expect(moved.overlay).toBe('none');
    expect(moved.game.moves).toBe(0);
    const right = press(shown, 9, first);
    expect(right).toEqual({ ...shown, overlay: 'none' });
    expect(press({ ...shown, game: { ...shown.game, board: [0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0] } }, 13, first).game.moves).toBe(1);
  });

  it('starts a new game from the centre key once the game is over, and ignores edge keys then', () => {
    const over = state([2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2, 4, 2], { over: true, score: 90, best: 90 });
    expect(press(over, 0, first)).toBe(over);
    expect(press(over, centreKey, first).game).toMatchObject({ over: false, score: 0, best: 90 });
  });

  it('starts a game', () => {
    expect(start(first, 7)).toMatchObject({ overlay: 'none', game: { best: 7 } });
  });
});

describe('twofold pictures', () => {
  it('draws each cell and lays a message over the middle row only', () => {
    const board = [2, 4, 0, 0, 0, 8, 0, 0, 0, 0, 0, 0, 0, 0, 4096];
    const plain = tiles(state(board));
    expect(plain[0]).toEqual({ tone: 'value', text: '2', colour: valueColour(2) });
    expect(plain[2]).toEqual({ tone: 'empty', arrow: 'up' });
    expect(plain.map(tile => tile.arrow ?? '-').join(' ')).toBe('- up up up - left left - right right - down down down -');
    expect(plain[14]).toMatchObject({ tone: 'value', text: '4096' });
    const scored = tiles({ game: game(board, { score: 12, best: 40 }), overlay: 'score' });
    expect(scored.slice(0, 5)).toEqual(plain.slice(0, 5));
    expect(scored.slice(10)).toEqual(plain.slice(10));
    expect(scored[5]).toEqual({ tone: 'label', label: 'Score', text: '12' });
    expect(scored[1].arrow).toBe('up');
    expect(scored[7]).toEqual({ tone: 'action', text: 'New', sub: 'game?' });
    expect(scored[9]).toEqual({ tone: 'label', label: 'Best', text: '40' });
    const over = tiles({ game: game(board, { over: true }), overlay: 'won' });
    expect(over.some(tile => tile.arrow)).toBe(false);
    expect(over[5]).toEqual({ tone: 'label', text: 'Game' });
    expect(over[7]).toEqual({ tone: 'action', text: 'New', sub: 'game' });
    expect(over.filter(tile => tile.tone === 'action')).toHaveLength(1);
    const won = tiles({ game: game(board), overlay: 'won' });
    expect(won[7]).toMatchObject({ tone: 'value', text: '2048' });
  });

  it('gives every value a colour, the largest ones the last', () => {
    expect(valueColour(2)).not.toBe(valueColour(4));
    expect(valueColour(8192)).toBe(valueColour(65536));
  });
});

describe('twofold best score', () => {
  const memory = (values: Record<string, string> = {}) => ({ getItem: (key: string) => values[key] ?? null, setItem: (key: string, value: string) => { values[key] = value; } });
  it('keeps the better score and reads a bad one as none', () => {
    const storage = memory();
    expect(readTwofoldBest(storage)).toBe(0);
    expect(keepTwofoldBest(storage, 120)).toBe(120);
    expect(keepTwofoldBest(storage, 80)).toBe(120);
    expect(readTwofoldBest(memory({ [twofoldBestKey]: 'lots' }))).toBe(0);
    expect(readTwofoldBest({ getItem: () => { throw new Error('blocked'); } })).toBe(0);
  });
});
