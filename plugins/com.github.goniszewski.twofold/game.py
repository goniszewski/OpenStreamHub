"""Twofold's rules, on a board of 3 rows of 5 cells, without any input or drawing.

A move slides every tile as far as it goes toward one edge. Two tiles of the same value that meet merge into one of
twice the value, which scores that value; a tile made by a merge does not merge again in the same move. A move that
changes nothing does not count. After a move that counts, a 2 (or, one time in ten, a 4) appears in a random empty
cell. Reaching 2048 is announced once, and the game goes on. The game is over when the board is full and no two
neighbouring tiles are equal.
"""

ROWS = 3
COLUMNS = 5
CELLS = ROWS * COLUMNS
DIRECTIONS = ("up", "down", "left", "right")
GOAL = 2048
FOUR_CHANCE = 0.1
START_TILES = 2


def lines(direction):
    """The board's lines for a move, each listed from the edge the tiles move toward."""
    if direction == "left":
        return [[row * COLUMNS + column for column in range(COLUMNS)] for row in range(ROWS)]
    if direction == "right":
        return [[row * COLUMNS + column for column in reversed(range(COLUMNS))] for row in range(ROWS)]
    if direction == "up":
        return [[row * COLUMNS + column for row in range(ROWS)] for column in range(COLUMNS)]
    if direction == "down":
        return [[row * COLUMNS + column for row in reversed(range(ROWS))] for column in range(COLUMNS)]
    raise ValueError("unknown direction %r" % (direction,))


def slide(line):
    """One line slid toward its start: (new line, score gained)."""
    tiles = [value for value in line if value]
    merged, gained, at = [], 0, 0
    while at < len(tiles):
        if at + 1 < len(tiles) and tiles[at] == tiles[at + 1]:
            merged.append(tiles[at] * 2)
            gained += tiles[at] * 2
            at += 2
        else:
            merged.append(tiles[at])
            at += 1
    return merged + [0] * (len(line) - len(merged)), gained


def move(board, direction):
    """The board after a move toward `direction`, and the score it gained. The same board when nothing moved."""
    after = list(board)
    gained = 0
    for line in lines(direction):
        slid, points = slide([board[index] for index in line])
        gained += points
        for index, value in zip(line, slid):
            after[index] = value
    return after, gained


def can_move(board):
    return any(move(board, direction)[0] != list(board) for direction in DIRECTIONS)


def spawn(board, rng):
    """The board with a new 2 or 4 in a random empty cell (the same board when it is full)."""
    empty = [index for index, value in enumerate(board) if not value]
    if not empty:
        return list(board)
    after = list(board)
    after[rng.choice(empty)] = 4 if rng.random() < FOUR_CHANCE else 2
    return after


def valid_board(board):
    return (isinstance(board, list) and len(board) == CELLS and
            all(isinstance(v, int) and not isinstance(v, bool) and (v == 0 or (v >= 2 and v & (v - 1) == 0))
                for v in board))


class Game:
    """One game and the best score so far."""

    def __init__(self, rng, best=0):
        self.rng = rng
        self.best = best
        self.new()

    def new(self):
        self.best = max(self.best, getattr(self, "score", 0))
        board = [0] * CELLS
        for _ in range(START_TILES):
            board = spawn(board, self.rng)
        self.board = board
        self.score = 0
        self.moves = 0
        self.won = False  # 2048 was reached (and announced) in this game

    @property
    def over(self):
        return not can_move(self.board)

    def play(self, direction):
        """Makes a move. Returns "moved", "won" (it reached 2048 for the first time) or None when nothing moved."""
        after, gained = move(self.board, direction)
        if after == self.board:
            return None
        self.board = spawn(after, self.rng)
        self.score += gained
        self.best = max(self.best, self.score)
        self.moves += 1
        if not self.won and max(self.board) >= GOAL:
            self.won = True
            return "won"
        return "moved"

    def to_dict(self):
        return {"board": self.board, "score": self.score, "best": self.best, "moves": self.moves, "won": self.won}

    def restore(self, saved):
        """Takes a saved game back, if it is one; returns whether it was. A best score is kept even from a save whose
        board is not usable."""
        if not isinstance(saved, dict):
            return False

        def count(name):
            value = saved.get(name)
            return value if isinstance(value, int) and not isinstance(value, bool) and value >= 0 else None

        if count("best") is not None:
            self.best = max(self.best, count("best"))
        board, numbers = saved.get("board"), [count(name) for name in ("score", "moves")]
        if not valid_board(board) or not any(board) or None in numbers:
            return False
        self.board = list(board)
        self.score, self.moves = numbers
        self.best = max(self.best, self.score)
        self.won = saved.get("won") is True or max(board) >= GOAL
        return True
