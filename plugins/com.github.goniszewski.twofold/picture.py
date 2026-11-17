"""Twofold's key pictures: 72x72 PNGs drawn with the standard library (a 5x7 pixel font, written out with zlib).

A face is a tuple, which is also how pictures are told apart and cached:
- ("tile", value): a cell, empty when value is 0.
- ("word", text, tone): one word of a message (letters, digits, "?"; empty for a blank), tone "plain", "number" or
  "yes".
"""

import struct
import zlib

SIDE = 72
INSET = 3
RADIUS = 10
TEXT_WIDTH = 66
TEXT_HEIGHT = 40

BACKGROUND = (12, 13, 18)
EMPTY = (38, 42, 54)
# Per value, from 2 up: cool to warm. Past the last, every tile is the last colour.
TILES = [
    (46, 128, 140), (40, 110, 178), (58, 84, 196), (98, 72, 204), (138, 62, 196), (178, 54, 164),
    (210, 62, 118), (226, 88, 74), (234, 124, 52), (240, 162, 44), (246, 200, 56), (236, 236, 236),
]
LIGHT_TEXT = (250, 250, 252)
DARK_TEXT = (24, 24, 32)
WORD = (30, 33, 44)
YES = (44, 150, 92)

# 5x7 glyphs, one string of 5 cells per row, "#" lit.
GLYPHS = {
    "0": [".###.", "#...#", "#..##", "#.#.#", "##..#", "#...#", ".###."],
    "1": ["..#..", ".##..", "..#..", "..#..", "..#..", "..#..", ".###."],
    "2": [".###.", "#...#", "....#", "...#.", "..#..", ".#...", "#####"],
    "3": ["####.", "....#", "....#", ".###.", "....#", "....#", "####."],
    "4": ["...#.", "..##.", ".#.#.", "#..#.", "#####", "...#.", "...#."],
    "5": ["#####", "#....", "####.", "....#", "....#", "#...#", ".###."],
    "6": ["..##.", ".#...", "#....", "####.", "#...#", "#...#", ".###."],
    "7": ["#####", "....#", "...#.", "..#..", ".#...", ".#...", ".#..."],
    "8": [".###.", "#...#", "#...#", ".###.", "#...#", "#...#", ".###."],
    "9": [".###.", "#...#", "#...#", ".####", "....#", "...#.", ".##.."],
    "A": [".###.", "#...#", "#...#", "#####", "#...#", "#...#", "#...#"],
    "B": ["####.", "#...#", "#...#", "####.", "#...#", "#...#", "####."],
    "C": [".###.", "#...#", "#....", "#....", "#....", "#...#", ".###."],
    "D": ["####.", "#...#", "#...#", "#...#", "#...#", "#...#", "####."],
    "E": ["#####", "#....", "#....", "####.", "#....", "#....", "#####"],
    "G": [".###.", "#...#", "#....", "#.###", "#...#", "#...#", ".###."],
    "K": ["#...#", "#..#.", "#.#..", "##...", "#.#..", "#..#.", "#...#"],
    "M": ["#...#", "##.##", "#.#.#", "#.#.#", "#...#", "#...#", "#...#"],
    "N": ["#...#", "##..#", "#.#.#", "#..##", "#...#", "#...#", "#...#"],
    "O": [".###.", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "P": ["####.", "#...#", "#...#", "####.", "#....", "#....", "#...."],
    "R": ["####.", "#...#", "#...#", "####.", "#.#..", "#..#.", "#...#"],
    "S": [".####", "#....", "#....", ".###.", "....#", "....#", "####."],
    "T": ["#####", "..#..", "..#..", "..#..", "..#..", "..#..", "..#.."],
    "U": ["#...#", "#...#", "#...#", "#...#", "#...#", "#...#", ".###."],
    "V": ["#...#", "#...#", "#...#", "#...#", "#...#", ".#.#.", "..#.."],
    "W": ["#...#", "#...#", "#...#", "#.#.#", "#.#.#", "##.##", "#...#"],
    "Y": ["#...#", "#...#", ".#.#.", "..#..", "..#..", "..#..", "..#.."],
    "?": [".###.", "#...#", "....#", "...#.", "..#..", ".....", "..#.."],
}


def tile_colour(value):
    if not value:
        return EMPTY
    step = value.bit_length() - 2  # 2 is the first
    return TILES[min(step, len(TILES) - 1)]


def text_colour(fill):
    # Dark text on light tiles, by perceived brightness.
    return DARK_TEXT if 0.3 * fill[0] + 0.59 * fill[1] + 0.11 * fill[2] > 170 else LIGHT_TEXT


def layout(text):
    """(scale, gap) for `text`: as large as fits TEXT_WIDTH by TEXT_HEIGHT."""
    for scale in range(6, 0, -1):
        gap = 1 if scale <= 2 else scale // 2
        if len(text) * 5 * scale + (len(text) - 1) * gap <= TEXT_WIDTH and 7 * scale <= TEXT_HEIGHT:
            return scale, gap
    return 1, 1


class Canvas:
    def __init__(self, colour):
        self.pixels = [list(colour) * SIDE for _ in range(SIDE)]

    def dot(self, x, y, colour):
        if 0 <= x < SIDE and 0 <= y < SIDE:
            self.pixels[y][x * 3:x * 3 + 3] = colour

    def rounded(self, colour):
        """A rounded square inset from the key's edges."""
        low, high = INSET, SIDE - 1 - INSET
        for y in range(low, high + 1):
            for x in range(low, high + 1):
                cx = min(max(x, low + RADIUS), high - RADIUS)
                cy = min(max(y, low + RADIUS), high - RADIUS)
                if (x - cx) ** 2 + (y - cy) ** 2 <= RADIUS * RADIUS:
                    self.dot(x, y, colour)

    def text(self, text, colour):
        scale, gap = layout(text)
        width = len(text) * 5 * scale + (len(text) - 1) * gap
        left, top = (SIDE - width) // 2, (SIDE - 7 * scale) // 2
        for place, character in enumerate(text):
            glyph = GLYPHS.get(character)
            if glyph is None:
                continue
            x0 = left + place * (5 * scale + gap)
            for row, cells in enumerate(glyph):
                for column, cell in enumerate(cells):
                    if cell == "#":
                        for dy in range(scale):
                            for dx in range(scale):
                                self.dot(x0 + column * scale + dx, top + row * scale + dy, colour)

    def png(self):
        raw = b"".join(b"\0" + bytes(row) for row in self.pixels)

        def chunk(kind, data):
            return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

        header = struct.pack(">IIBBBBB", SIDE, SIDE, 8, 2, 0, 0, 0)  # 8-bit RGB
        return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(raw, 9)) +
                chunk(b"IEND", b""))


def draw(face):
    """The PNG for a face."""
    canvas = Canvas(BACKGROUND)
    if face[0] == "tile":
        _, value = face
        fill = tile_colour(value)
        ink = text_colour(fill)
        canvas.rounded(fill)
        if value:
            canvas.text(str(value), ink)
    elif face[0] == "word":
        _, text, tone = face
        fill = YES if tone == "yes" else WORD
        canvas.rounded(fill)
        canvas.text(text.upper(), (246, 200, 56) if tone == "number" else LIGHT_TEXT)
    else:
        raise ValueError("unknown face %r" % (face,))
    return canvas.png()
