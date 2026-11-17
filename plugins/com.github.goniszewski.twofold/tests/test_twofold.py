"""Tests for Twofold: the rules, the pictures, and the plugin with a fake app, clock and timers."""

import os
import random
import struct
import sys
import tempfile
import unittest
import zlib

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

import game  # noqa: E402
import picture  # noqa: E402
import plugin  # noqa: E402


def board(*rows):
    return [value for row in rows for value in row]


class Rng:
    """Puts each new tile in the first empty cell, always a 2."""

    def choice(self, items):
        return items[0]

    def random(self):
        return 0.5


class RulesTests(unittest.TestCase):
    def test_a_line_slides_and_each_tile_merges_once(self):
        self.assertEqual(game.slide([2, 2, 2, 2, 0]), ([4, 4, 0, 0, 0], 8))
        self.assertEqual(game.slide([0, 2, 0, 2, 4]), ([4, 4, 0, 0, 0], 4))
        self.assertEqual(game.slide([4, 4, 8, 0, 0]), ([8, 8, 0, 0, 0], 8))
        self.assertEqual(game.slide([2, 2, 2, 0, 0]), ([4, 2, 0, 0, 0], 4))
        self.assertEqual(game.slide([2, 4, 8, 16, 32]), ([2, 4, 8, 16, 32], 0))

    def test_moves_in_each_direction(self):
        start = board([2, 0, 2, 0, 4],
                      [0, 0, 0, 0, 4],
                      [2, 0, 0, 0, 8])
        self.assertEqual(game.move(start, "left"), (board([4, 4, 0, 0, 0], [4, 0, 0, 0, 0], [2, 8, 0, 0, 0]), 4))
        self.assertEqual(game.move(start, "right"), (board([0, 0, 0, 4, 4], [0, 0, 0, 0, 4], [0, 0, 0, 2, 8]), 4))
        self.assertEqual(game.move(start, "up"), (board([4, 0, 2, 0, 8], [0, 0, 0, 0, 8], [0, 0, 0, 0, 0]), 12))
        self.assertEqual(game.move(start, "down"), (board([0, 0, 0, 0, 0], [0, 0, 0, 0, 8], [4, 0, 2, 0, 8]), 12))
        with self.assertRaises(ValueError):
            game.move(start, "sideways")

    def test_a_move_that_changes_nothing_does_not_count(self):
        play = game.Game(Rng())
        play.board = board([2, 4, 0, 0, 0], [0] * 5, [0] * 5)
        self.assertIsNone(play.play("left"))
        self.assertEqual(play.moves, 0)
        self.assertEqual(play.play("right"), "moved")
        self.assertEqual(play.board, board([2, 0, 0, 2, 4], [0] * 5, [0] * 5), "a 2 in the first empty cell")
        self.assertEqual(play.moves, 1)

    def test_a_new_tile_is_a_four_one_time_in_ten(self):
        class Fours(Rng):
            def random(self):
                return 0.05
        self.assertEqual(game.spawn([0] * 15, Fours())[0], 4)
        self.assertEqual(game.spawn([0] * 15, Rng())[0], 2)
        rng = random.Random(1)
        fours = sum(game.spawn([0] * 15, rng).count(4) for _ in range(2000))
        self.assertTrue(120 < fours < 280, fours)

    def test_score_best_win_once_and_game_over(self):
        play = game.Game(Rng(), best=10)
        play.board = board([1024, 1024, 0, 0, 0], [0] * 5, [0] * 5)
        self.assertEqual(play.play("left"), "won")
        self.assertEqual((play.score, play.best), (2048, 2048))
        play.board = board([1024, 1024, 0, 0, 0], [0] * 5, [0] * 5)
        self.assertEqual(play.play("left"), "moved", "2048 is announced once a game")
        full = board([2, 4, 2, 4, 2], [4, 2, 4, 2, 4], [2, 4, 2, 4, 2])
        play.board = full
        self.assertTrue(play.over)
        play.board = board([2, 4, 2, 4, 2], [4, 2, 4, 2, 4], [2, 4, 2, 4, 4])
        self.assertFalse(play.over, "two equal neighbours can still merge")
        play.new()
        self.assertEqual((play.score, play.won, play.best), (0, False, 4096))
        self.assertEqual(sum(1 for value in play.board if value), 2)

    def test_a_saved_game_comes_back_and_a_bad_one_is_ignored(self):
        play = game.Game(Rng())
        saved = {"board": board([2, 0, 0, 0, 2048], [0] * 5, [0] * 5), "score": 30000, "best": 40000, "moves": 9}
        self.assertTrue(play.restore(saved))
        self.assertEqual((play.score, play.best, play.moves, play.won), (30000, 40000, 9, True))
        for bad in (None, [], {"board": [3] * 15, "score": 0, "best": 0, "moves": 0},
                    {"board": [0] * 14, "score": 0, "best": 0, "moves": 0},
                    {"board": [0] * 15, "score": -1, "best": 0, "moves": 0},
                    {"board": [True] * 15, "score": 0, "best": 0, "moves": 0}):
            self.assertFalse(game.Game(Rng()).restore(bad), bad)
        empty = game.Game(Rng())
        self.assertFalse(empty.restore({"board": [0] * 15, "score": 0, "best": 900, "moves": 0}))
        self.assertEqual(empty.best, 900, "a best score is kept from a save whose board is not usable")
        play = game.Game(Rng())
        self.assertTrue(play.restore({"board": [2] + [0] * 14, "score": 0, "best": 0, "moves": 0, "won": "false"}))
        self.assertFalse(play.won)


def png_size(data):
    assert data[:8] == b"\x89PNG\r\n\x1a\n"
    return struct.unpack(">II", data[16:24])


def png_pixels(data):
    raw = zlib.decompress(data[41:data.index(b"IEND") - 4])
    rows = [raw[row * (1 + 72 * 3) + 1:(row + 1) * (1 + 72 * 3)] for row in range(72)]
    return lambda x, y: tuple(rows[y][x * 3:x * 3 + 3])


class PictureTests(unittest.TestCase):
    def test_every_face_is_a_small_72_pixel_png(self):
        for face in [("tile", 0), ("tile", 2), ("tile", 2048), ("tile", 131072),
                     ("word", "SCORE", "plain"), ("word", "999999", "number"), ("word", "GAME?", "plain"),
                     ("word", "YES", "yes")]:
            data = picture.draw(face)
            self.assertEqual(png_size(data), (72, 72), face)
            self.assertLess(len(data), 2048, face)

    def test_colours_follow_the_value(self):
        at = png_pixels(picture.draw(("tile", 2)))
        self.assertEqual(at(0, 0), picture.BACKGROUND)
        self.assertEqual(at(10, 10), picture.TILES[0])
        self.assertEqual(png_pixels(picture.draw(("tile", 2048)))(10, 10), picture.TILES[10])
        self.assertEqual(png_pixels(picture.draw(("tile", 1 << 20)))(10, 10), picture.TILES[-1])

    def test_every_word_the_plugin_shows_has_its_letters(self):
        for kind in ("score", "confirm", "over", "won"):
            for face in plugin.message_faces(kind, 1234567890, 0).values():
                self.assertTrue(set(face[1]) <= set(picture.GLYPHS), face)


class Timers:
    def __init__(self):
        self.pending = []

    def __call__(self, seconds, callback):
        timers = self

        class Timer:
            cancelled = False

            def cancel(self):
                self.cancelled = True
                if self in timers.pending:
                    timers.pending.remove(self)

        timer = Timer()
        timer.seconds, timer.callback = seconds, callback
        self.pending.append(timer)
        return timer

    def fire(self, seconds=None):
        """Runs the timers due within `seconds` (all, without it)."""
        for timer in list(self.pending):
            if seconds is None or timer.seconds <= seconds:
                self.pending.remove(timer)
                timer.callback()


class Clock:
    def __init__(self):
        self.now = 100.0

    def __call__(self):
        return self.now


class MemoryStore:
    def __init__(self, saved=None):
        self.saved = saved

    def load(self):
        return self.saved

    def save(self, value):
        self.saved = dict(value, board=list(value["board"]))


def key_press(index):
    return {"gesture": "pressed", "control": {"module": "m4315", "index": index}}


class PluginTests(unittest.TestCase):
    def setUp(self):
        self.sent = []
        self.timers = Timers()
        self.clock = Clock()
        self.store = MemoryStore()
        self.plugin = plugin.Plugin(self.sent.append, self.store, rng=Rng(), schedule=self.timers,
                                    clock=self.clock, draw=lambda face: repr(face).encode())
        self.request = 0

    def appear_all(self):
        for index in range(15):
            self.plugin.handle({"type": "appear", "key": "k%d" % index, "action": "tile", "params": {},
                                "control": {"module": "m4315", "index": index}})

    def images(self):
        """key -> the face it was last sent, and forget what was sent."""
        shown = {}
        for message in self.sent:
            if message["type"] == "set_image":
                import base64
                shown[message["key"]] = eval(base64.b64decode(message["image"]))
        self.sent.clear()
        return shown

    def settle(self):
        """All keys shown and answered, a second later."""
        self.appear_all()
        self.images()
        self.answer_all()
        self.clock.now += 1.0

    def answer_all(self, ok=True, error="the picture is bad"):
        for number in list(self.plugin.requests):
            self.plugin.handle({"type": "reply", "id": number, "ok": ok, "error": "" if ok else error})

    def invoke(self, action, params=None, source=None):
        self.request += 1
        message = {"type": "invoke", "request": self.request, "action": action, "params": params or {}}
        if source:
            message["input"] = source
        self.plugin.handle(message)

    def press(self, index):
        self.invoke("tile", source=key_press(index))

    def release(self, index):
        self.plugin.handle({"type": "event", "action": "tile", "gesture": "released",
                            "control": {"module": "m4315", "index": index}})

    def test_each_key_shows_its_cell_and_only_changed_keys_are_sent_again(self):
        self.plugin.game.board = board([2, 0, 0, 0, 0], [0] * 5, [0] * 5)
        self.appear_all()
        shown = self.images()
        self.assertEqual(len(shown), 15)
        self.assertEqual(shown["k0"], ("tile", 2))
        self.assertEqual(shown["k14"], ("tile", 0))
        self.answer_all()
        self.clock.now += 1.0
        self.invoke("move", {"direction": "right"})
        self.assertEqual(self.sent[0], {"type": "result", "request": 1, "ok": True})
        # The 2 moved from cell 0 to 4; the new 2 appeared in cell 0, which so looks the same.
        self.assertEqual(self.images(), {"k4": ("tile", 2)})
        self.assertEqual(self.store.saved["board"][:5], [2, 0, 0, 0, 2])

    def test_pictures_are_held_to_the_rate_limit_and_the_newest_one_goes_out(self):
        self.appear_all()
        self.assertEqual(len(self.images()), plugin.MAX_IMAGES_PER_SECOND)
        self.clock.now += 1.0
        self.timers.fire()
        self.images()
        self.plugin.game.board = board([4] * 5, [4] * 5, [4] * 5)
        self.plugin.redraw()
        self.assertEqual(len(self.images()), plugin.MAX_IMAGES_PER_SECOND)
        self.plugin.game.board = board([8] * 5, [8] * 5, [8] * 5)
        self.plugin.redraw()
        self.assertEqual(self.images(), {})
        self.clock.now += 1.0
        self.timers.fire()
        later = self.images()
        self.assertEqual(len(later), 15, "every key, since none shows an 8 yet")
        self.assertEqual(set(later.values()), {("tile", 8)})

    def test_a_refused_picture_is_sent_again_a_few_times(self):
        self.plugin.handle({"type": "appear", "key": "k0", "action": "tile", "params": {},
                            "control": {"module": "m4315", "index": 0}})
        for attempt in range(plugin.IMAGE_RETRIES):
            self.assertEqual(len(self.images()), 1)
            self.answer_all(ok=False)
            self.clock.now += 1.0
            self.timers.fire()
        self.assertEqual(len(self.images()), 1)
        self.answer_all(ok=False)
        self.clock.now += 1.0
        self.timers.fire()
        self.assertEqual(self.images(), {}, "given up on that picture")

    def test_a_picture_the_apps_rate_limit_refused_is_always_sent_again(self):
        self.plugin.handle({"type": "appear", "key": "k0", "action": "tile", "params": {},
                            "control": {"module": "m4315", "index": 0}})
        for attempt in range(plugin.IMAGE_RETRIES + 3):
            self.assertEqual(len(self.images()), 1)
            self.answer_all(ok=False, error="at most 20 set_image and clear_image a second")
            self.clock.now += 1.0
            self.timers.fire()

    def test_only_the_reply_to_the_last_picture_counts(self):
        self.plugin.handle({"type": "appear", "key": "k0", "action": "tile", "params": {},
                            "control": {"module": "m4315", "index": 0}})
        self.plugin.game.board[0] = 2
        self.plugin.redraw()
        self.plugin.game.board[0] = 0
        self.plugin.redraw()
        self.images()
        first = min(self.plugin.requests)
        self.plugin.handle({"type": "reply", "id": first, "ok": False, "error": "late"})
        self.clock.now += 1.0
        self.timers.fire()
        self.assertEqual(self.images(), {}, "a late refusal of an older picture sends nothing")

    def test_holding_a_key_shows_the_score_until_shortly_after_it_is_let_go(self):
        self.plugin.game.score, self.plugin.game.best = 1234, 5678
        self.settle()
        self.press(3)
        self.assertEqual(self.images(), {})
        self.timers.fire(plugin.HOLD_SECONDS)
        shown = self.images()
        self.assertEqual([shown["k%d" % cell] for cell in plugin.MIDDLE],
                         [("word", "SCORE", "plain"), ("word", "1234", "number"), ("word", "", "plain"),
                          ("word", "BEST", "plain"), ("word", "5678", "number")])
        self.assertEqual(len(shown), 5, "only the middle row changes")
        self.release(3)
        self.assertEqual(self.timers.pending[0].seconds, plugin.AFTER_RELEASE_SECONDS)
        self.clock.now += 2
        self.timers.fire()
        self.assertEqual(self.images()["k5"], ("tile", 0))

    def test_the_score_goes_when_its_key_leaves_another_is_pressed_or_a_move_is_made(self):
        self.settle()
        for end in ("disappear", "other", "move"):
            self.plugin.game.board = board([2, 0, 0, 0, 0], [0] * 5, [0] * 5)
            self.press(3)
            self.timers.fire(plugin.HOLD_SECONDS)
            self.assertEqual(self.plugin.message, "score", end)
            if end == "disappear":
                self.plugin.handle({"type": "disappear", "key": "k3"})
            elif end == "other":
                self.clock.now += 1
                self.press(4)
            else:
                self.invoke("move", {"direction": "right"})
            self.assertIsNone(self.plugin.message, end)
            self.assertIsNone(self.plugin.held, end)
            self.clock.now += 1
            self.timers.fire()
            self.appear_all()

    def test_a_held_score_goes_away_by_itself_after_a_while(self):
        self.settle()
        self.press(3)
        self.timers.fire(plugin.HOLD_SECONDS)
        self.timers.fire(plugin.SCORE_MAX_SECONDS)
        self.assertIsNone(self.plugin.message)

    def test_a_short_press_shows_nothing(self):
        self.appear_all()
        self.images()
        self.press(3)
        self.release(3)
        self.timers.fire()
        self.assertEqual(self.images(), {})

    def test_a_double_press_asks_and_only_yes_starts_a_new_game(self):
        self.plugin.game.board = board([2, 4, 8, 16, 32], [0] * 5, [0] * 5)
        self.plugin.game.score = 500
        self.settle()
        self.press(0)
        self.release(0)
        self.clock.now += 0.2
        self.press(9)
        self.clock.now += plugin.CONFIRM_GRACE_SECONDS + 0.01
        shown = self.images()
        self.assertEqual(shown["k7"], ("word", "YES", "yes"))
        self.assertEqual(shown["k6"], ("word", "GAME?", "plain"))
        self.press(3)  # anything but YES goes on
        self.assertEqual(self.plugin.game.score, 500)
        self.assertEqual(self.images()["k7"], ("tile", 0))
        self.clock.now += 1
        self.press(0)
        self.clock.now += 0.1
        self.press(0)
        self.clock.now += 0.1
        self.press(plugin.YES_CELL)
        self.assertEqual(self.plugin.game.score, 500, "too soon to have read the question")
        self.clock.now += plugin.CONFIRM_GRACE_SECONDS + 0.01
        self.press(plugin.YES_CELL)
        self.assertEqual(self.plugin.game.score, 0)
        self.assertEqual(self.store.saved["score"], 0)
        self.assertEqual(self.plugin.game.best, 500)

    def test_the_question_goes_away_by_itself_or_with_a_move(self):
        self.appear_all()
        self.press(0)
        self.clock.now += 0.1
        self.press(0)
        self.assertEqual(self.plugin.message, "confirm")
        self.timers.fire(plugin.CONFIRM_SECONDS)
        self.assertIsNone(self.plugin.message)
        self.clock.now += 1
        self.press(0)
        self.clock.now += 0.1
        self.press(0)
        self.plugin.game.board = board([2, 0, 0, 0, 0], [0] * 5, [0] * 5)
        self.invoke("move", {"direction": "right"})
        self.assertIsNone(self.plugin.message)
        self.assertEqual(self.plugin.game.board[:5], [2, 0, 0, 0, 0], "the move only answered the question")

    def test_game_over_shows_and_a_press_after_a_moment_starts_again(self):
        self.plugin.game.board = board([2, 4, 2, 4, 2], [4, 2, 4, 2, 4], [4, 2, 4, 2, 0])
        self.plugin.game.score = 77
        self.settle()
        self.invoke("move", {"direction": "right"})  # a 2 then fills the first cell: over
        self.assertTrue(self.plugin.game.over)
        shown = self.images()
        self.assertEqual((shown["k6"], shown["k7"], shown["k8"]),
                         (("word", "GAME", "plain"), ("word", "OVER", "plain"), ("word", "77", "number")))
        self.press(4)
        self.assertTrue(self.plugin.game.over, "too soon: maybe a press meant for the last move")
        self.clock.now += plugin.OVER_GRACE_SECONDS
        self.press(4)
        self.assertFalse(self.plugin.game.over)

    def test_2048_is_shown_even_when_that_move_ends_the_game(self):
        self.plugin.game.board = board([1024, 1024, 4, 8, 16], [4, 8, 16, 32, 64], [8, 16, 32, 64, 128])
        self.settle()
        self.invoke("move", {"direction": "left"})
        self.assertTrue(self.plugin.game.over)
        self.assertEqual(self.images()["k8"], ("word", "2048", "number"))
        self.timers.fire(plugin.WON_SECONDS)
        self.assertEqual(self.images()["k7"], ("word", "OVER", "plain"))

    def test_reaching_2048_is_shown_for_a_moment(self):
        self.plugin.game.board = board([1024, 1024, 0, 0, 0], [0] * 5, [0] * 5)
        self.settle()
        self.invoke("move", {"direction": "left"})
        self.assertEqual(self.images()["k8"], ("word", "2048", "number"))
        self.timers.fire(plugin.WON_SECONDS)
        self.assertEqual(self.images()["k8"], ("tile", 0))

    def test_a_move_without_a_direction_is_logged_and_other_actions_are_refused(self):
        self.invoke("move", {})
        self.assertTrue(any(m["type"] == "log" and "direction" in m["message"] for m in self.sent))
        self.invoke("jump")
        self.assertEqual(self.sent[-1]["ok"], False)

    def test_keys_off_the_keypad_draw_nothing_and_a_gone_key_is_forgotten(self):
        self.plugin.handle({"type": "appear", "key": "d", "action": "tile", "params": {},
                            "control": {"module": "m131e", "index": 0}})
        self.plugin.handle({"type": "appear", "key": "x", "action": "tile", "params": {},
                            "control": {"module": "m4315", "index": 15}})
        self.assertEqual(self.images(), {})
        self.appear_all()
        self.images()
        self.plugin.handle({"type": "disappear", "key": "k3"})
        self.plugin.game.board = board([4] * 5, [4] * 5, [4] * 5)
        self.plugin.redraw()
        self.assertNotIn("k3", self.images())

    def test_without_a_data_folder_the_game_is_not_kept(self):
        loose = plugin.Plugin(self.sent.append, plugin.Store(None), rng=Rng(), schedule=self.timers,
                              clock=self.clock)
        loose.move("left")

    def test_the_game_is_kept_in_a_file(self):
        with tempfile.TemporaryDirectory() as folder:
            store = plugin.Store(os.path.join(folder, "game.json"))
            first = plugin.Plugin(self.sent.append, store, rng=Rng(), schedule=self.timers, clock=self.clock)
            first.game.board = board([2, 0, 0, 0, 0], [0] * 5, [0] * 5)
            first.move("right")
            again = plugin.Plugin(self.sent.append, store, rng=Rng(), schedule=self.timers, clock=self.clock)
            self.assertEqual(again.game.board, first.game.board)
            self.assertEqual(again.game.moves, 1)


if __name__ == "__main__":
    unittest.main()
