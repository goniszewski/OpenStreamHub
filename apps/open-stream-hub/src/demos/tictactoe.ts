export type Mark = 'X' | 'O';
export type Cell = Mark | null;
export type Board = readonly Cell[];

export const winLines: readonly (readonly [number, number, number])[] = [
  [0, 1, 2], [3, 4, 5], [6, 7, 8],
  [0, 3, 6], [1, 4, 7], [2, 5, 8],
  [0, 4, 8], [2, 4, 6],
];

export const emptyBoard = (): Board => Array<Cell>(9).fill(null);

export function winner(board: Board): { mark: Mark; line: readonly [number, number, number] } | null {
  for (const line of winLines) {
    const mark = board[line[0]];
    if (mark && mark === board[line[1]] && mark === board[line[2]]) return { mark, line };
  }
  return null;
}

export const isFull = (board: Board): boolean => board.every(cell => cell !== null);
export const isOver = (board: Board): boolean => winner(board) !== null || isFull(board);
export const nextMark = (board: Board): Mark => board.filter(Boolean).length % 2 === 0 ? 'X' : 'O';
export const otherMark = (mark: Mark): Mark => mark === 'X' ? 'O' : 'X';
export const openCells = (board: Board): number[] => board.flatMap((cell, index) => cell === null ? [index] : []);

// Play `cell` for whoever is to move; a taken cell or a finished game leaves the board as it was.
export function play(board: Board, cell: number): Board {
  if (!Number.isInteger(cell) || cell < 0 || cell > 8 || board[cell] !== null || isOver(board)) return board;
  const next = [...board];
  next[cell] = nextMark(board);
  return next;
}

// Minimax score from `me`'s point of view: positive is a win, shallower wins and deeper losses score higher.
function score(board: Board, me: Mark, depth: number): number {
  const won = winner(board);
  if (won) return won.mark === me ? 10 - depth : depth - 10;
  if (isFull(board)) return 0;
  const scores = openCells(board).map(cell => score(play(board, cell), me, depth + 1));
  return nextMark(board) === me ? Math.max(...scores) : Math.min(...scores);
}

// Every open cell with its minimax score for the side to move, best first; ties keep board order.
export function rankMoves(board: Board): { cell: number; score: number }[] {
  const me = nextMark(board);
  return openCells(board).map(cell => ({ cell, score: score(play(board, cell), me, 1) })).sort((a, b) => b.score - a.score || a.cell - b.cell);
}

export type Difficulty = 'perfect' | 'easy';

// The computer's move: always the best one, or on "easy" a random open cell about one time in three.
// `random` is injected so tests are deterministic. Returns null when the game is over.
export function computerMove(board: Board, difficulty: Difficulty, random: () => number = Math.random): number | null {
  if (isOver(board)) return null;
  const ranked = rankMoves(board);
  if (difficulty === 'easy' && random() < 1 / 3) {
    const open = openCells(board);
    return open[Math.min(open.length - 1, Math.floor(random() * open.length))];
  }
  const best = ranked.filter(move => move.score === ranked[0].score);
  return best[Math.min(best.length - 1, Math.floor(random() * best.length))].cell;
}

export type Score = { X: number; O: number; draws: number };
export const noScore: Score = { X: 0, O: 0, draws: 0 };

export function tally(score: Score, board: Board): Score {
  const won = winner(board);
  if (won) return { ...score, [won.mark]: score[won.mark] + 1 };
  return isFull(board) ? { ...score, draws: score.draws + 1 } : score;
}
