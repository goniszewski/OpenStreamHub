"""Tests for the Now Playing plugin. Run: /usr/bin/python3 -m unittest discover -s tests (from the plugin folder).

Players are faked, so no test plays, pauses or skips anything. One test draws a real key with render.js.
"""

import base64
import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import plugin  # noqa: E402


class Clock:
    """Timers that fire only when the test says so."""

    def __init__(self):
        self.timers = []

    def schedule(self, seconds, callback):
        timer = Timer(seconds, callback)
        self.timers.append(timer)
        return timer

    def fire(self, seconds):
        """Fires every pending timer due within `seconds` of being scheduled."""
        for timer in list(self.timers):
            if not timer.cancelled and not timer.fired and timer.seconds <= seconds:
                timer.fired = True
                timer.callback()

    def pending(self):
        return [timer.seconds for timer in self.timers if not timer.cancelled and not timer.fired]


class Timer:
    def __init__(self, seconds, callback):
        self.seconds, self.callback = seconds, callback
        self.cancelled = self.fired = False

    def cancel(self):
        self.cancelled = True


KEY = ("m4315", 3)


class GestureTests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.done = []
        self.gestures = plugin.Gestures(lambda verb, choice: self.done.append((verb, choice)), self.clock.schedule)

    def test_a_short_press_toggles_once_the_double_press_window_passes(self):
        self.gestures.press(KEY, {"player": "music"})
        self.gestures.release(KEY)
        self.assertEqual(self.done, [])
        self.assertEqual(self.clock.pending(), [0.3])
        self.clock.fire(0.3)
        self.assertEqual(self.done, [("playpause", "music")])

    def test_a_long_press_goes_back_while_held_and_its_release_does_nothing(self):
        self.gestures.press(KEY, {})
        self.clock.fire(0.5)
        self.assertEqual(self.done, [("previous", "auto")])
        self.gestures.release(KEY)
        self.clock.fire(10)
        self.assertEqual(self.done, [("previous", "auto")])

    def test_a_double_press_goes_next_at_the_second_press_and_never_toggles(self):
        self.gestures.press(KEY, {})
        self.gestures.release(KEY)
        self.gestures.press(KEY, {})
        self.assertEqual(self.done, [("next", "auto")])
        self.gestures.release(KEY)
        self.clock.fire(10)
        self.assertEqual(self.done, [("next", "auto")])

    def test_after_a_double_press_the_next_press_starts_afresh(self):
        for _ in range(2):
            self.gestures.press(KEY, {})
            self.gestures.release(KEY)
        self.gestures.press(KEY, {})
        self.gestures.release(KEY)
        self.clock.fire(0.3)
        self.assertEqual(self.done, [("next", "auto"), ("playpause", "auto")])

    def test_the_times_come_from_the_key_and_are_kept_in_range(self):
        self.gestures.press(KEY, {"long_ms": 900, "double_ms": 5000})
        self.assertEqual(self.clock.pending(), [0.9])
        self.gestures.release(KEY)
        self.assertEqual(self.clock.pending(), [1.0])
        self.gestures.press(("m4315", 4), {"long_ms": "soon", "double_ms": True})
        self.assertEqual(self.clock.pending(), [1.0, 0.5])

    def test_no_double_press_window_toggles_at_the_release(self):
        self.gestures.press(KEY, {"double_ms": 0})
        self.gestures.release(KEY)
        self.assertEqual(self.done, [("playpause", "auto")])

    def test_a_release_without_its_press_is_ignored(self):
        self.gestures.release(KEY)
        self.clock.fire(10)
        self.assertEqual(self.done, [])

    def test_keys_are_told_apart(self):
        self.gestures.press(KEY, {})
        self.gestures.release(KEY)
        self.gestures.press(("m4315", 4), {})
        self.gestures.release(("m4315", 4))
        self.clock.fire(0.3)
        self.assertEqual(self.done, [("playpause", "auto"), ("playpause", "auto")])

    def test_a_key_whose_release_never_comes_plays_or_pauses_from_its_second_press(self):
        # A key the action draws, but whose Release runs something else: the first press can only look long.
        self.gestures.press(KEY, {})
        self.clock.fire(0.5)
        self.gestures.press(KEY, {})
        self.gestures.press(KEY, {})
        self.clock.fire(10)
        self.assertEqual(self.done, [("previous", "auto"), ("playpause", "auto"), ("playpause", "auto")])
        self.gestures.release(KEY)  # a release came after all: the key tells presses apart again
        self.gestures.press(KEY, {})
        self.assertEqual(len(self.done), 3)  # timed again, not played at once
        self.clock.fire(0.5)
        self.assertEqual(self.done[-1], ("previous", "auto"))

    def test_dial_clicks_toggle_or_go_next_when_doubled(self):
        self.gestures.tap(("m131e", 0), {})
        self.clock.fire(0.3)
        self.gestures.tap(("m131e", 0), {})
        self.gestures.tap(("m131e", 0), {})
        self.clock.fire(10)
        self.assertEqual(self.done, [("playpause", "auto"), ("next", "auto")])


def now(state="playing", track="T1", player="music"):
    return plugin.Now(player, state, track, "Title", "Artist", None)


class FakePlayers:
    def __init__(self):
        self.state = now()
        self.commands = []
        self.reads = 0

    def resolve(self, choice, deadline=None, skipped=None):
        self.reads += 1
        self.deadline = deadline
        return self.state

    def command(self, choice, verb):
        self.commands.append((choice, verb))


class FakeArtist:
    def image(self, state, dial=False):
        if dial:
            return (state.state, state.track, "dial"), "dial-%s-%s" % (state.state, state.track)
        return (state.state, state.track), "%s-%s" % (state.state, state.track)


class FakeClock:
    def __init__(self):
        self.now = 100.0

    def __call__(self):
        return self.now


class PluginTests(unittest.TestCase):
    def setUp(self):
        self.sent = []
        self.clock = Clock()
        self.players = FakePlayers()
        self.time = FakeClock()
        self.plugin = plugin.Plugin(self.sent.append, self.players, FakeArtist(), self.clock.schedule,
                                    start_threads=False, clock=self.time)

    def commands(self):
        return [(verb, choice) for _, verb, choice in self.plugin.commands]

    def appear(self, key="k1", index=3, params=None):
        self.plugin.handle({"type": "appear", "key": key, "action": "control", "params": params or {},
                            "control": {"module": "m4315", "index": index}})

    def images(self):
        return [(m["key"], m["image"]) for m in self.sent if m["type"] == "set_image"]

    def test_a_press_is_answered_at_once_and_its_release_comes_as_an_event(self):
        self.appear()
        press = {"gesture": "pressed", "control": {"module": "m4315", "index": 3}}
        self.plugin.handle({"type": "invoke", "request": 7, "action": "control", "params": {}, "input": press})
        self.assertEqual(self.sent[-1], {"type": "result", "request": 7, "ok": True})
        self.plugin.handle({"type": "event", "action": "control", "params": {}, "gesture": "released",
                            "control": {"module": "m4315", "index": 3}, "extra": "ignored"})
        self.clock.fire(0.3)
        self.assertEqual(self.commands(), [("playpause", "auto")])

    def test_a_press_without_input_toggles(self):
        self.plugin.handle({"type": "invoke", "request": 1, "action": "control", "params": {"player": "spotify"}})
        self.assertEqual(self.commands(), [("playpause", "spotify")])

    def test_an_unknown_action_fails_the_press(self):
        self.plugin.handle({"type": "invoke", "request": 2, "action": "nope", "params": {}})
        self.assertFalse(self.sent[0]["ok"])
        self.assertEqual(self.commands(), [])

    def test_turns_skip_and_raw_dial_presses_are_ignored(self):
        for gesture, module in [("clockwise", "m003k"), ("counter_clockwise", "m003k"), ("down", "m032le"),
                                ("pressed", "m131e"), ("released", "m131e"), ("long_press", "m131e")]:
            self.plugin.handle({"type": "event", "action": "control", "params": {}, "gesture": gesture,
                                "control": {"module": module, "index": 0}})
        self.assertEqual(self.commands(), [("next", "auto"), ("previous", "auto"), ("next", "auto"),
                                           ("previous", "auto")])

    def test_a_press_on_a_key_not_on_screen_toggles_at_once_and_never_goes_back(self):
        # A step of an if / else or multi-action, or Built-in animation: no appear, so no release is counted on.
        press = {"gesture": "pressed", "control": {"module": "m4315", "index": 3}}
        self.plugin.handle({"type": "invoke", "request": 1, "action": "control", "params": {}, "input": press})
        self.plugin.handle({"type": "event", "action": "control", "params": {}, "gesture": "released",
                            "control": {"module": "m4315", "index": 3}})
        self.clock.fire(10)
        self.assertEqual(self.commands(), [("playpause", "auto")])

    def test_a_key_that_leaves_the_screen_while_held_does_not_go_back(self):
        self.appear()
        self.plugin.handle({"type": "invoke", "request": 1, "action": "control", "params": {},
                            "input": {"gesture": "pressed", "control": {"module": "m4315", "index": 3}}})
        self.plugin.handle({"type": "disappear", "key": "k1"})
        self.clock.fire(10)
        self.assertEqual(self.commands(), [])

    def test_a_press_that_waited_too_long_behind_the_player_is_dropped(self):
        self.plugin.queue_command("playpause", "auto")
        self.time.now += 5
        self.plugin.queue_command("next", "auto")
        self.assertEqual(self.plugin.next_command(), ("next", "auto"))
        self.assertIsNone(self.plugin.next_command(block=False))
        self.assertTrue(any(m["type"] == "log" and "dropped 1" in m["message"] for m in self.sent))

    def test_a_check_keeps_to_its_time_with_a_margin_and_at_most_three_seconds(self):
        for timeout_ms, budget in ((1000, 1.0), (10000, plugin.CHECK_SECONDS)):
            before = plugin.time.monotonic()
            self.plugin.check({"type": "check", "request": 9, "condition": "is_playing", "params": {},
                               "timeout_ms": timeout_ms})
            after = plugin.time.monotonic()
            expected = budget - plugin.CHECK_MARGIN_SECONDS
            self.assertGreaterEqual(self.players.deadline, before + expected)
            self.assertLessEqual(self.players.deadline, after + expected)

    def test_a_check_after_a_press_reads_the_player_again(self):
        self.plugin.check({"type": "check", "request": 1, "condition": "is_playing", "params": {}})
        self.plugin.latest["auto"] = (self.time(), self.players.state)  # as a poll would leave it
        original, plugin.AFTER_COMMAND_SECONDS = plugin.AFTER_COMMAND_SECONDS, 0
        try:
            self.plugin.run_command("playpause", "auto")
        finally:
            plugin.AFTER_COMMAND_SECONDS = original
        reads = self.players.reads
        self.plugin.check({"type": "check", "request": 2, "condition": "is_playing", "params": {}})
        self.assertEqual(self.players.reads, reads + 1)

    def test_a_cover_that_fails_reaches_the_plugin_log(self):
        class Covers:
            def cover(self, now, path):
                raise OSError("no route to host")

        artist = plugin.Artist(Covers(), "/nonexistent")
        plugin.Plugin(self.sent.append, self.players, artist, self.clock.schedule, start_threads=False,
                      clock=self.time)
        artist.cover(now("playing", "S1", "spotify"))
        self.assertTrue(any(m["type"] == "log" and "no route to host" in m["message"] for m in self.sent))

    def test_a_key_on_screen_is_drawn_once_per_change(self):
        self.appear(index=0)
        self.plugin.handle({"type": "appear", "key": "k2", "action": "other", "params": {}})
        self.plugin.poll_once()
        self.plugin.poll_once()
        self.assertEqual(self.images(), [("k1", "playing-T1")])
        self.players.state = now("paused")
        self.plugin.poll_once()
        self.assertEqual(self.images()[-1], ("k1", "paused-T1"))
        self.plugin.handle({"type": "disappear", "key": "k1"})
        self.players.state = now("playing", "T2")
        self.plugin.poll_once()
        self.assertEqual(len(self.images()), 2)

    def test_a_refused_image_is_sent_again_a_few_times(self):
        self.appear()
        self.plugin.poll_once()
        for _ in range(5):
            last = [m for m in self.sent if m["type"] == "set_image"][-1]
            self.plugin.handle({"type": "reply", "id": last["id"], "ok": False, "error": "too many"})
            self.plugin.poll_once()
        self.assertEqual(len(self.images()), 1 + plugin.IMAGE_RETRIES)
        self.assertEqual(sum(m["type"] == "log" for m in self.sent), 1)  # the same refusal is logged once
        # A new picture gets its own retries.
        self.players.state = now("playing", "T2")
        self.plugin.poll_once()
        last = [m for m in self.sent if m["type"] == "set_image"][-1]
        self.plugin.handle({"type": "reply", "id": last["id"], "ok": False, "error": "too many"})
        self.plugin.poll_once()
        self.assertEqual(len(self.images()), 3 + plugin.IMAGE_RETRIES)

    def test_keys_of_different_players_are_read_separately(self):
        self.appear("k1", 0, {"player": "music"})
        self.appear("k2", 1, {"player": "bogus"})
        self.plugin.poll_once()
        self.assertEqual(self.players.reads, 2)
        self.assertEqual(sorted(key for key, _ in self.images()), ["k1", "k2"])

    def test_a_dial_showing_the_same_player_gets_the_dial_picture(self):
        self.appear("k1", 0)
        self.plugin.handle({"type": "appear", "key": "k2", "action": "control", "params": {},
                            "control": {"module": "m131e", "index": 0}})
        self.plugin.poll_once()
        self.assertEqual(self.players.reads, 1, "one read for both")
        self.assertEqual(sorted(self.images()), [("k1", "playing-T1"), ("k2", "dial-playing-T1")])
        self.plugin.poll_once()
        self.assertEqual(len(self.images()), 2, "unchanged: not sent again")

    def test_a_check_answers_whether_music_is_playing(self):
        self.plugin.check({"type": "check", "request": 9, "condition": "is_playing", "params": {}})
        self.players.state = now("paused")
        self.plugin.latest.clear()
        self.plugin.check({"type": "check", "request": 10, "condition": "is_playing", "params": {}})
        self.plugin.check({"type": "check", "request": 11, "condition": "other", "params": {}})
        results = [(m["request"], m["ok"], m.get("value")) for m in self.sent]
        self.assertEqual(results, [(9, True, True), (10, True, False), (11, False, None)])

    def test_a_ping_is_answered(self):
        self.plugin.handle({"type": "ping"})
        self.assertEqual(self.sent, [{"type": "pong"}])


class Done:
    def __init__(self, returncode=0, stdout="", stderr="boom (-1743)"):
        self.returncode, self.stdout, self.stderr = returncode, stdout, stderr if returncode else ""


class PlayersTests(unittest.TestCase):
    def players(self, running, states):
        calls = []

        def run(argv, **_):
            calls.append(argv)
            if argv[0] == plugin.PGREP:
                return Done(0 if argv[-1] in running else 1)
            name = os.path.basename(argv[1]).split(".")[0]
            if argv[2] == "state":
                if states[name] is None:
                    return Done(1)  # not allowed to control it
                return Done(stdout=states[name] + "\n")
            return Done(stdout="ok")
        return plugin.Players(run), calls

    def test_auto_prefers_the_player_that_is_playing(self):
        players, _ = self.players({"Music", "Spotify"}, {
            "music": "paused\tM1\tA\tB",
            "spotify": "playing\tspotify:track:1\tC\tD\thttps://i.scdn.co/image/x",
        })
        state = players.resolve("auto")
        self.assertEqual((state.player, state.state, state.track, state.cover_url),
                         ("spotify", "playing", "spotify:track:1", "https://i.scdn.co/image/x"))

    def test_auto_leaves_out_a_player_it_may_not_control(self):
        players, _ = self.players({"Music", "Spotify"}, {"music": None, "spotify": "paused\tS1\tC\tD\thttps://i.scdn.co/x"})
        skipped = []
        self.assertEqual(players.resolve("auto", skipped=skipped).player, "spotify")
        self.assertIn("Automation", skipped[0])
        players, _ = self.players({"Music"}, {"music": None})
        with self.assertRaises(plugin.PlayerError):
            players.resolve("auto")

    def test_a_tab_in_a_name_keeps_the_cover_and_seeking_counts_as_playing(self):
        players, _ = self.players({"Spotify", "Music"}, {
            "spotify": "paused\tS1\tA\tB\tC\thttps://i.scdn.co/image/y", "music": "fast forwarding\tM1\tA\tB"})
        self.assertEqual(players.read("spotify", plugin.time.monotonic() + 5).cover_url, "https://i.scdn.co/image/y")
        self.assertEqual(players.read("music", plugin.time.monotonic() + 5).state, "playing")

    def test_a_track_without_an_artist_is_still_playing(self):
        players, _ = self.players({"Music", "Spotify"}, {"music": "playing\tM1\tRadio\t",
                                                          "spotify": "playing\tS1\tName\t\t"})
        state = players.read("music", plugin.time.monotonic() + 5)
        self.assertEqual((state.state, state.title, state.artist), ("playing", "Radio", ""))
        self.assertEqual(players.read("spotify", plugin.time.monotonic() + 5).state, "playing")

    def test_a_plain_http_spotify_cover_is_fetched_over_https(self):
        players, _ = self.players({"Spotify"}, {"spotify": "playing\tS1\tA\tB\thttp://i.scdn.co/image/z"})
        self.assertEqual(players.read("spotify", plugin.time.monotonic() + 5).cover_url, "https://i.scdn.co/image/z")

    def test_a_cover_is_never_followed_off_its_host(self):
        handler = plugin.SameHostRedirects()
        request = plugin.urllib.request.Request("https://i.scdn.co/image/a")
        with self.assertRaises(plugin.PlayerError):
            handler.redirect_request(request, None, 302, "Found", {}, "https://example.com/a")
        self.assertIsNotNone(handler.redirect_request(request, None, 302, "Found", {}, "https://i.scdn.co/image/b"))

    def test_a_missing_spotify_is_named(self):
        self.assertEqual(plugin.explain("Spotify", "syntax error: Can’t get application id \"x\". (-1728)"),
                         "Spotify is not installed")

    def test_a_player_that_is_not_running_is_never_asked(self):
        players, calls = self.players(set(), {})
        self.assertEqual(players.resolve("auto").player, None)
        self.assertTrue(all(argv[0] == plugin.PGREP for argv in calls))

    def test_play_pause_with_nothing_running_starts_apple_music_but_skipping_does_not(self):
        players, calls = self.players(set(), {})
        self.assertEqual(players.command("auto", "next"), "next: nothing is playing")
        players.command("auto", "playpause")
        self.assertEqual(calls[-1][1:], [os.path.join(plugin.HERE, "music.applescript"), "playpause"])

    def test_a_refused_automation_says_where_to_allow_it(self):
        players = plugin.Players(lambda argv, **_: Done(0) if argv[0] == plugin.PGREP else Done(1))
        with self.assertRaises(plugin.PlayerError) as raised:
            players.resolve("music")
        self.assertIn("Privacy & Security > Automation", str(raised.exception))


class CoverTests(unittest.TestCase):
    def test_a_track_without_a_cover_yet_is_asked_again_for_a_while(self):
        answers = [False, True]

        class Covers:
            asked = 0

            def cover(self, now, path):
                Covers.asked += 1
                return answers[min(Covers.asked, len(answers)) - 1]

        artist = plugin.Artist(Covers(), "/nonexistent")
        track = now("playing", "T9")
        self.assertEqual(artist.cover(track), "")
        self.assertEqual(artist.cover(track), "")  # asked again only after a while
        artist.covered["music"]["last"] -= plugin.COVER_RETRY_SECONDS
        self.assertEqual(artist.cover(track), "/nonexistent/cover-music")
        self.assertEqual(Covers.asked, 2)

    def test_a_cover_that_fails_is_logged(self):
        class Covers:
            def cover(self, now, path):
                raise OSError("certificate verify failed")

        artist = plugin.Artist(Covers(), "/nonexistent")
        logged = []
        artist.log = logged.append
        self.assertEqual(artist.cover(now("playing", "S1", "spotify")), "")
        self.assertIn("certificate verify failed", logged[0])

    def test_two_players_on_screen_keep_their_covers(self):
        class Covers:
            asked = 0

            def cover(self, now, path):
                Covers.asked += 1
                return True

        artist = plugin.Artist(Covers(), "/nonexistent")
        for _ in range(3):
            artist.cover(now("playing", "M1", "music"))
            artist.cover(now("playing", "S1", "spotify"))
        self.assertEqual(Covers.asked, 2)


@unittest.skipUnless(sys.platform == "darwin" and os.path.exists(plugin.OSASCRIPT), "needs macOS")
class RenderTests(unittest.TestCase):
    def test_render_draws_a_72_pixel_png_for_each_state(self):
        with tempfile.TemporaryDirectory() as folder:
            artist = plugin.Artist(FakePlayers(), folder)
            for state in ("playing", "paused", "stopped"):
                data = base64.b64decode(artist.image(now(state, track=None))[1])
                self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")
                self.assertEqual(data[16:24], (72).to_bytes(4, "big") * 2)
                self.assertLessEqual(len(data), plugin.MAX_IMAGE)

    def test_render_draws_a_cover_playing_and_paused(self):
        with tempfile.TemporaryDirectory() as folder:
            source = os.path.join(folder, "source.png")
            subprocess.run([plugin.OSASCRIPT, "-l", "JavaScript", os.path.join(plugin.HERE, "render.js"), source,
                            "stopped"], check=True, capture_output=True, timeout=30)

            class Covers:
                def cover(self, now, path):
                    with open(source, "rb") as file, open(path, "wb") as out:
                        out.write(file.read())
                    return True

            artist = plugin.Artist(Covers(), folder)
            pictures = set()
            for state in ("playing", "paused"):
                signature, image = artist.image(now(state, track="T1"))
                self.assertTrue(signature[3])  # drawn with the cover
                data = base64.b64decode(image)
                self.assertEqual(data[16:24], (72).to_bytes(4, "big") * 2)
                self.assertLessEqual(len(data), plugin.MAX_IMAGE)
                pictures.add(image)
            self.assertEqual(len(pictures), 2)
            # On the dial screen: a 240x240 JPEG, which the app takes as well as a PNG.
            signature, image = artist.image(now("playing", track="T1"), dial=True)
            self.assertEqual(signature[-1], "dial")
            data = base64.b64decode(image)
            self.assertEqual(data[:2], b"\xff\xd8")
            self.assertIn(b"\xff\xc0\x00\x11\x08\x00\xf0\x00\xf0", data)  # baseline, 240x240
            self.assertLessEqual(len(data), plugin.MAX_DIAL_IMAGE)


if __name__ == "__main__":
    unittest.main()
