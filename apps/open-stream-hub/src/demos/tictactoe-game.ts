import { computerMove, emptyBoard, isFull, nextMark, noScore, otherMark, play, tally, winner } from './tictactoe';
import type { Board, Difficulty, Mark, Score } from './tictactoe';

// The 15 keys are 5 columns by 3 rows, numbered row by row from the top left (verified on hardware). The board takes
// the left three columns; the right two hold the controls and the score.
export const keyCount = 15;
const boardKeys = [0, 1, 2, 5, 6, 7, 10, 11, 12] as const;
export const keys = { newGame: 3, exit: 4, mode: 8, level: 9, scoreX: 13, scoreO: 14 } as const;

export type Opponent = 'computer' | 'friend';

export interface Game {
  board: Board;
  opponent: Opponent;
  difficulty: Difficulty;
  // The mark the person plays against the computer; the computer plays the other. Alternates each game, so each side starts in turn.
  human: Mark;
  score: Score;
  // The cell played last, shown lit.
  last: number | null;
  // True once the person asked to leave.
  exit: boolean;
}

export const newGame = (): Game => ({ board: emptyBoard(), opponent: 'computer', difficulty: 'perfect', human: 'X', score: noScore, last: null, exit: false });

const cellOf = (key: number) => boardKeys.indexOf(key as typeof boardKeys[number]);

// The computer moves whenever it is its turn against a person and the game is not over.
function reply(game: Game, random: () => number): Game {
  if (game.opponent !== 'computer' || winner(game.board) || isFull(game.board) || nextMark(game.board) === game.human) return game;
  const cell = computerMove(game.board, game.difficulty, random);
  return cell === null ? game : finish(game, cell);
}

function finish(game: Game, cell: number): Game {
  const board = play(game.board, cell);
  if (board === game.board) return game;
  const over = winner(board) !== null || isFull(board);
  return { ...game, board, last: cell, score: over ? tally(game.score, board) : game.score };
}

// What a key press does. A taken cell, a finished game and a press on a blank key change nothing.
export function press(game: Game, key: number, random: () => number = Math.random): Game {
  if (key === keys.exit) return { ...game, exit: true };
  if (key === keys.newGame) return reply({ ...game, board: emptyBoard(), last: null, human: game.opponent === 'computer' ? otherMark(game.human) : game.human }, random);
  if (key === keys.mode) return reply({ ...game, board: emptyBoard(), last: null, opponent: game.opponent === 'computer' ? 'friend' : 'computer', human: 'X', score: noScore }, random);
  if (key === keys.level) return game.opponent === 'computer' ? { ...game, difficulty: game.difficulty === 'perfect' ? 'easy' : 'perfect' } : game;
  const cell = cellOf(key);
  if (cell < 0) return game;
  if (game.opponent === 'computer' && nextMark(game.board) !== game.human) return game;
  return reply(finish(game, cell), random);
}

// The computer opens a game in which it plays X.
export const start = (game: Game, random: () => number = Math.random): Game => reply(game, random);

export type Tone = 'empty' | 'x' | 'o' | 'win' | 'last' | 'button' | 'score' | 'blank';
export interface Tile { tone: Tone; mark?: Mark; text?: string; sub?: string }

const blank: Tile = { tone: 'blank' };

// The picture on each of the 15 keys.
export function tiles(game: Game): Tile[] {
  const won = winner(game.board);
  const result: Tile[] = Array.from({ length: keyCount }, () => blank);
  boardKeys.forEach((key, cell) => {
    const mark = game.board[cell];
    const inLine = won?.line.includes(cell as never) ?? false;
    result[key] = mark === null ? { tone: 'empty' } : { tone: inLine ? 'win' : game.last === cell ? 'last' : mark === 'X' ? 'x' : 'o', mark };
  });
  result[keys.newGame] = { tone: 'button', text: 'New', sub: 'game' };
  result[keys.exit] = { tone: 'button', text: 'Exit' };
  result[keys.mode] = { tone: 'button', text: game.opponent === 'computer' ? 'vs CPU' : '2 players' };
  result[keys.level] = game.opponent === 'computer' ? { tone: 'button', text: game.difficulty === 'perfect' ? 'Hard' : 'Easy' } : blank;
  result[keys.scoreX] = { tone: 'score', mark: 'X', text: String(game.score.X) };
  result[keys.scoreO] = { tone: 'score', mark: 'O', text: String(game.score.O) };
  return result;
}

// The state the person reads in the app: whose turn it is, or how the game ended.
export type Status = { kind: 'turn'; mark: Mark; yours: boolean | null } | { kind: 'won'; mark: Mark; yours: boolean | null } | { kind: 'draw' };
export function status(game: Game): Status {
  const won = winner(game.board);
  const yours = (mark: Mark) => game.opponent === 'computer' ? mark === game.human : null;
  if (won) return { kind: 'won', mark: won.mark, yours: yours(won.mark) };
  if (isFull(game.board)) return { kind: 'draw' };
  const mark = nextMark(game.board);
  return { kind: 'turn', mark, yours: yours(mark) };
}
