import type { Tile } from './tictactoe-game';

// Twofold: a sliding number game on the 15 keys, 3 rows of 5, numbered row by row from the top left (as the module's
// keys are). A move slides every tile as far as it goes toward one edge; two tiles of the same value that meet merge
// into one of twice the value, which scores that value, and a tile made by a merge does not merge again in the same
// move. A move that changes nothing does not count. After one that does, a 2 (one time in ten a 4) appears in a random
// empty cell. Reaching 2048 is shown once a game, and the game goes on until no move is left.
export const rows = 3;
export const columns = 5;
export const cellCount = rows * columns;
export const goal = 2048;
export const fourChance = 0.1;
const startTiles = 2;

export type Direction = 'up' | 'down' | 'left' | 'right';
export const directions: readonly Direction[] = ['up', 'down', 'left', 'right'];
export type Board = readonly number[];

// The board's lines for a move, each listed from the edge the tiles move toward.
export function lines(direction: Direction): number[][] {
  const range = (count: number) => Array.from({ length: count }, (_, index) => index);
  switch (direction) {
    case 'left': return range(rows).map(row => range(columns).map(column => row * columns + column));
    case 'right': return range(rows).map(row => range(columns).reverse().map(column => row * columns + column));
    case 'up': return range(columns).map(column => range(rows).map(row => row * columns + column));
    case 'down': return range(columns).map(column => range(rows).reverse().map(row => row * columns + column));
  }
}

// One line slid toward its start, and the score that gained.
export function slide(line: readonly number[]): { line: number[]; gained: number } {
  const tiles = line.filter(value => value !== 0);
  const merged: number[] = [];
  let gained = 0;
  for (let at = 0; at < tiles.length; at += 1) {
    if (at + 1 < tiles.length && tiles[at] === tiles[at + 1]) {
      merged.push(tiles[at] * 2);
      gained += tiles[at] * 2;
      at += 1;
    } else merged.push(tiles[at]);
  }
  return { line: [...merged, ...Array<number>(line.length - merged.length).fill(0)], gained };
}

// The board after a move, and the score it gained. Equal to the board when nothing moved.
export function move(board: Board, direction: Direction): { board: number[]; gained: number } {
  const after = [...board];
  let gained = 0;
  for (const line of lines(direction)) {
    const slid = slide(line.map(index => board[index]));
    gained += slid.gained;
    line.forEach((index, at) => { after[index] = slid.line[at]; });
  }
  return { board: after, gained };
}

const same = (a: Board, b: Board) => a.every((value, index) => value === b[index]);

export const canMove = (board: Board) => directions.some(direction => !same(move(board, direction).board, board));

// The board with a new 2 or 4 in a random empty cell (the same board when it is full).
export function spawn(board: Board, random: () => number): number[] {
  const empty = board.flatMap((value, index) => value === 0 ? [index] : []);
  const after = [...board];
  if (empty.length === 0) return after;
  const cell = empty[Math.min(empty.length - 1, Math.floor(random() * empty.length))];
  after[cell] = random() < fourChance ? 4 : 2;
  return after;
}

export interface Game {
  board: Board;
  score: number;
  best: number;
  moves: number;
  // 2048 was reached (and shown) in this game.
  won: boolean;
  over: boolean;
}

export function newGame(random: () => number, best = 0): Game {
  let board: number[] = Array<number>(cellCount).fill(0);
  for (let count = 0; count < startTiles; count += 1) board = spawn(board, random);
  return { board, score: 0, best, moves: 0, won: false, over: false };
}

// A move: the game after it, and whether it moved anything and reached 2048 for the first time.
export function play(game: Game, direction: Direction, random: () => number): { game: Game; moved: boolean; reached: boolean } {
  if (game.over) return { game, moved: false, reached: false };
  const slid = move(game.board, direction);
  if (same(slid.board, game.board)) return { game, moved: false, reached: false };
  const board = spawn(slid.board, random);
  const score = game.score + slid.gained;
  const reached = !game.won && Math.max(...board) >= goal;
  return {
    game: { board, score, best: Math.max(game.best, score), moves: game.moves + 1, won: game.won || reached, over: !canMove(board) },
    moved: true,
    reached,
  };
}

// The keys: one in the top row moves up, one in the bottom row down, and one in the middle row toward its side. The
// centre key shows the score; pressed again while it shows, or once the game is over, it starts a new game. While the
// score shows, the rest of the middle row puts the board back without a move.
export const centreKey = 7;
const cornerKeys: readonly number[] = [0, columns - 1, cellCount - columns, cellCount - 1];

export function keyDirection(key: number): Direction | null {
  const row = Math.floor(key / columns);
  const column = key % columns;
  if (key < 0 || key >= cellCount) return null;
  if (row === 0) return 'up';
  if (row === rows - 1) return 'down';
  return column < 2 ? 'left' : column > 2 ? 'right' : null;
}

// A turn's move: the first knob or roller moves up and down, the second left and right. Clockwise, right and a
// roller's down are "forward" (down, or right).
export function turnDirection(control: number, turn: string): Direction | null {
  const forward = turn === 'clockwise' || turn === 'right' || turn === 'down';
  const back = turn === 'counter_clockwise' || turn === 'left' || turn === 'up';
  if (!forward && !back) return null;
  if (control === 0) return forward ? 'down' : 'up';
  if (control === 1) return forward ? 'right' : 'left';
  return null;
}

// What the middle row shows over the board: nothing, the score (with a new game offered on the centre key), or the
// moment 2048 is reached. A game that is over always shows that it is.
export type Overlay = 'none' | 'score' | 'won';
export interface Twofold { game: Game; overlay: Overlay }

export const scoreMs = 4000;
export const wonMs = 2500;

export const start = (random: () => number, best = 0): Twofold => ({ game: newGame(random, best), overlay: 'none' });

export function slideTo(state: Twofold, direction: Direction, random: () => number): Twofold {
  const result = play(state.game, direction, random);
  if (!result.moved) return state.overlay === 'score' ? { ...state, overlay: 'none' } : state;
  return { game: result.game, overlay: result.reached ? 'won' : 'none' };
}

export function pressCentre(state: Twofold, random: () => number): Twofold {
  if (state.game.over || state.overlay === 'score') return start(random, state.game.best);
  return { ...state, overlay: 'score' };
}

export function press(state: Twofold, key: number, random: () => number): Twofold {
  if (key === centreKey) return pressCentre(state, random);
  // While the score shows, the middle row's other keys carry its words: a press there only puts the board back.
  if (state.overlay === 'score' && Math.floor(key / columns) === 1) return { ...state, overlay: 'none' };
  const direction = keyDirection(key);
  return direction ? slideTo(state, direction, random) : state;
}

// Per value, from 2 up: cool to warm. Past the last, every tile has the last colour.
const colours = ['#2e808c', '#286eb2', '#3a54c4', '#6248cc', '#8a3ec4', '#b236a4', '#d23e76', '#e2584a', '#ea7c34', '#f0a22c', '#f6c838', '#ececec'];

export const valueColour = (value: number) => colours[Math.min(Math.log2(value) - 1, colours.length - 1)];

const cellTile = (value: number): Tile => value === 0 ? { tone: 'empty' } : { tone: 'value', text: String(value), colour: valueColour(value) };
// Message words and numbers are plain labels; the centre key, the only one that does something then, is a button.
const word = (text: string): Tile => ({ tone: 'label', text });
const number = (label: string, value: number): Tile => ({ tone: 'label', label, text: String(value) });
const action = (text: string, sub: string): Tile => ({ tone: 'action', text, sub });

// The picture on each key. An overlay takes the middle row only, so the rest of the board stays in view (with no
// arrows there: its keys only put the board back).
export function tiles(state: Twofold): Tile[] {
  const { score, best, over } = state.game;
  // Each key that moves the tiles shows which way, in a corner, apart from the board's four corners (which move up or
  // down like the rest of their row, but whose arrow would read as either way); none does once the game is over.
  const result = state.game.board.map((value, key) => {
    const arrow = over || cornerKeys.includes(key) ? null : keyDirection(key);
    return arrow ? { ...cellTile(value), arrow } : cellTile(value);
  });
  const row = over ? [word('Game'), word('over'), action('New', 'game'), number('Score', score), number('Best', best)]
    : state.overlay === 'score' ? [number('Score', score), { tone: 'blank' } as Tile, action('New', 'game?'), { tone: 'blank' } as Tile, number('Best', best)]
    : state.overlay === 'won' ? [word('You'), word('made'), cellTile(goal), word('Keep'), word('going')]
    : null;
  if (row) row.forEach((tile, at) => { result[columns + at] = tile; });
  return result;
}

export const twofoldBestKey = 'open-stream-hub.demo.twofold-best';

export function readTwofoldBest(storage: Pick<Storage, 'getItem'>): number {
  try {
    const value = Number(storage.getItem(twofoldBestKey));
    return Number.isInteger(value) && value > 0 ? value : 0;
  } catch { return 0; }
}

export function keepTwofoldBest(storage: Pick<Storage, 'getItem' | 'setItem'>, score: number): number {
  const best = Math.max(readTwofoldBest(storage), score);
  try { storage.setItem(twofoldBestKey, String(best)); } catch { /* a convenience; keep working without it */ }
  return best;
}
