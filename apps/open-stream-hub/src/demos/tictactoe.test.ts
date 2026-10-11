import { describe, expect, it } from 'vitest';
import { computerMove, emptyBoard, isOver, nextMark, noScore, openCells, play, rankMoves, tally, winLines, winner, type Board, type Cell, type Mark } from './tictactoe';

const board = (rows: string): Board => rows.replace(/\s/g, '').split('').map((c): Cell => c === 'X' || c === 'O' ? c : null);

describe('tic-tac-toe rules', () => {
  it('finds every one of the eight winning lines for both marks', () => {
    expect(winLines).toHaveLength(8);
    for (const mark of ['X', 'O'] as Mark[]) for (const line of winLines) {
      const cells = emptyBoard().map((_, index) => line.includes(index) ? mark : null);
      expect(winner(cells)).toEqual({ mark, line });
    }
  });
  it('alternates X then O, and refuses a taken cell, a bad index or a move after the end', () => {
    let b = play(emptyBoard(), 4);
    expect(b[4]).toBe('X');
    expect(nextMark(b)).toBe('O');
    expect(play(b, 4)).toBe(b);
    expect(play(b, 9)).toBe(b);
    expect(play(b, -1)).toBe(b);
    b = board('XXX OO. ...');
    expect(isOver(b)).toBe(true);
    expect(play(b, 8)).toBe(b);
  });
  it('recognises a draw', () => {
    const b = board('XOX XOO OXX');
    expect(winner(b)).toBeNull();
    expect(isOver(b)).toBe(true);
    expect(tally(noScore, b)).toEqual({ X: 0, O: 0, draws: 1 });
  });
  it('scores a win for the winner only', () => {
    expect(tally(noScore, board('XXX OO. ...'))).toEqual({ X: 1, O: 0, draws: 0 });
    expect(tally(noScore, board('OOO XX. X..'))).toEqual({ X: 0, O: 1, draws: 0 });
    expect(tally(noScore, board('X.. ... ...'))).toEqual(noScore);
  });
});

describe('the computer', () => {
  it('takes a win, then blocks a loss', () => {
    expect(computerMove(board('XX. OO. ...'), 'perfect')).toBe(2);
    expect(computerMove(board('OO. X.. X..'), 'perfect')).toBe(2);
    expect(computerMove(board('XX. O.. ...'), 'perfect')).toBe(2);
  });
  it('returns nothing once the game is over', () => {
    expect(computerMove(board('XXX OO. ...'), 'perfect')).toBeNull();
  });
  it('a perfect opponent never loses, whoever starts and whatever the other side plays', () => {
    // Every line of play where the computer plays `me` perfectly and the other side tries every open cell.
    const explore = (b: Board, me: Mark): void => {
      const won = winner(b);
      if (won) { expect(won.mark).toBe(me); return; }
      if (isOver(b)) return;
      if (nextMark(b) === me) explore(play(b, computerMove(b, 'perfect', () => 0)!), me);
      else for (const cell of openCells(b)) explore(play(b, cell), me);
    };
    explore(emptyBoard(), 'X');
    explore(emptyBoard(), 'O');
  });
  it('ranks the winning move first', () => {
    const ranked = rankMoves(board('XX. OO. ...'));
    expect(ranked[0].cell).toBe(2);
    expect(ranked[0].score).toBeGreaterThan(0);
  });
  it('on easy sometimes plays a worse move, but always a legal one', () => {
    const b = board('XX. OO. ...');
    const picks = new Set<number>();
    for (const roll of [0.1, 0.2, 0.3]) picks.add(computerMove(b, 'easy', sequence([roll, 0.99]))!);
    expect(picks.has(2)).toBe(false);
    expect(computerMove(b, 'easy', sequence([0.9, 0]))).toBe(2);
    for (let i = 0; i < 200; i++) expect(openCells(b)).toContain(computerMove(b, 'easy')!);
  });
});

function sequence(values: number[]): () => number {
  let i = 0;
  return () => values[Math.min(i++, values.length - 1)];
}
