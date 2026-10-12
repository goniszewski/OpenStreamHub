import { describe, expect, it } from 'vitest';
import { keys, newGame, press, start, status, tiles } from './tictactoe-game';
import type { Game } from './tictactoe-game';

const never = () => 0.99;
const friend = (): Game => press(newGame(), keys.mode);
const board = [0, 1, 2, 5, 6, 7, 10, 11, 12];

describe('tic-tac-toe on the keys', () => {
  it('plays a two-player game on the left nine keys and scores the win', () => {
    let game = friend();
    for (const key of [board[0], board[3], board[1], board[4], board[2]]) game = press(game, key);
    expect(game.board.slice(0, 3)).toEqual(['X', 'X', 'X']);
    expect(status(game)).toEqual({ kind: 'won', mark: 'X', yours: null });
    expect(game.score.X).toBe(1);
    // The winning line is lit, and a finished game takes no more moves.
    expect(tiles(game).slice(0, 3).map(tile => tile.tone)).toEqual(['win', 'win', 'win']);
    // The line blinks: lit, then back to plain marks, while the other cells stay as they are.
    expect(tiles(game, false).slice(0, 3).map(tile => tile.tone)).toEqual(['x', 'x', 'last']);
    expect(press(game, board[8])).toBe(game);
    expect(tiles(press(game, keys.newGame))[board[0]].tone).toBe('empty');
  });

  it('ignores a taken cell and a blank key, and lights the last move', () => {
    let game = press(friend(), board[4]);
    expect(press(game, board[4])).toBe(game);
    expect(press(game, board[8])).not.toBe(game);
    expect(press(game, keys.scoreX)).toBe(game);
    expect(tiles(game)[board[4]]).toEqual({ tone: 'last', mark: 'X' });
    game = press(game, board[0]);
    expect(tiles(game)[board[4]].tone).toBe('x');
  });

  it('answers a move at once against the computer, and the computer opens when it plays X', () => {
    let game = press(newGame(), board[0], never);
    expect(game.board.filter(Boolean)).toHaveLength(2);
    game = press(game, keys.newGame, never);
    // Marks swapped: the computer is X and has opened.
    expect(game.human).toBe('O');
    expect(game.board.filter(Boolean)).toEqual(['X']);
    expect(status(game)).toEqual({ kind: 'turn', mark: 'O', yours: true });
    // Its own cell cannot be played over.
    const taken = game.board.findIndex(Boolean);
    expect(press(game, board[taken], never)).toBe(game);
  });

  it('never loses on the hard level, whatever the person plays', () => {
    const play = (order: number[]) => {
      let game = newGame();
      for (const cell of order) game = press(game, board[cell], never);
      return game;
    };
    for (let first = 0; first < 9; first++) for (let second = 0; second < 9; second++) {
      const game = play([first, second, 0, 1, 2, 3, 4, 5, 6, 7, 8]);
      // The person plays X: every game ends with no win for the person.
      expect(game.score.X).toBe(0);
    }
  });

  it('switches level and opponent from the keys, starts the score over, and exits', () => {
    const game = press(newGame(), keys.level);
    expect(game.difficulty).toBe('easy');
    expect(tiles(game)[keys.level].text).toBe('Easy');
    const two = press(game, keys.mode);
    expect(two.opponent).toBe('friend');
    expect(tiles(two)[keys.level].tone).toBe('blank');
    expect(press(two, keys.level)).toBe(two);
    expect(press(game, keys.exit).exit).toBe(true);
  });

  it('lets the computer open a game it starts', () => {
    const game = start({ ...newGame(), human: 'O' }, never);
    expect(game.board.filter(Boolean)).toHaveLength(1);
  });
});
