"""Tests for the Claude Code plugin. Run: /usr/bin/python3 -B -m unittest discover -s tests (from the plugin folder).

Terminals are faked, so no test brings a window to the front or types a key. One test draws real keys with render.js.
"""

import base64
import json
import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import hook  # noqa: E402
import install_hooks  # noqa: E402
import plugin  # noqa: E402

KEY = ("m4315", 3)
PROMPT = """
 Bash command

   npm test
   Run the tests

 Do you want to proceed?
 ❯ 1. Yes
   2. Yes, and don't ask again for npm test commands in /repo
   3. No, and tell Claude what to do differently (esc)

 Esc to cancel · Tab to amend
"""


class Clock:
    def __init__(self):
        self.timers = []
        self.now = 1000.0

    def schedule(self, seconds, callback):
        timer = Timer(seconds, callback)
        self.timers.append(timer)
        return timer

    def fire(self):
        for timer in list(self.timers):
            if not timer.cancelled and not timer.fired:
                timer.fired = True
                timer.callback()

    def pending(self):
        return [t for t in self.timers if not t.cancelled and not t.fired]

    def __call__(self):
        return self.now


class Timer:
    def __init__(self, seconds, callback):
        self.seconds, self.callback = seconds, callback
        self.cancelled = self.fired = False

    def cancel(self):
        self.cancelled = True


class DataFolder(unittest.TestCase):
    """A test with its own data folder for the hooks."""

    def setUp(self):
        self.folder = tempfile.TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        patcher = mock.patch.dict(os.environ, {"OSH_CLAUDE_DATA": self.folder.name})
        patcher.start()
        self.addCleanup(patcher.stop)
        self.sessions = os.path.join(self.folder.name, "sessions")

    def event(self, name, at, **fields):
        event = {"session_id": "abc", "hook_event_name": name, "cwd": "/repo/sub"}
        event.update(fields)
        return hook.handle(event, environ={"CLAUDE_PROJECT_DIR": "/work/OpenStreamHub", "TERM_PROGRAM": "iTerm.app",
                                           "__CFBundleIdentifier": "com.googlecode.iterm2",
                                           "ITERM_SESSION_ID": "w0t0p0:UUID-1"},
                           now=at, find_claude=lambda: (4242, ["/dev/ttys004", "/dev/ttys002"], [4242, 4000]))

    def record(self):
        with open(os.path.join(self.sessions, "abc.json")) as file:
            return json.load(file)


class HookTests(DataFolder):
    def test_a_session_goes_from_idle_to_working_to_waiting_to_done(self):
        self.event("SessionStart", 1, source="startup")
        self.assertEqual(self.record()["state"], "idle")
        self.event("UserPromptSubmit", 2, prompt="hi")
        self.assertEqual((self.record()["state"], self.record()["detail"]), ("working", ""))
        self.event("PreToolUse", 3, tool_name="Bash")
        self.assertEqual((self.record()["state"], self.record()["detail"]), ("working", "Bash"))
        self.event("PermissionRequest", 4, tool_name="Bash")
        self.assertEqual((self.record()["state"], self.record()["detail"]), ("waiting", "Bash"))
        self.event("Notification", 5, notification_type="permission_prompt", message="Claude needs your permission")
        self.assertEqual(self.record()["detail"], "Bash", "the late notification keeps the tool's name")
        self.event("PostToolUse", 6, tool_name="Bash")
        self.assertEqual(self.record()["state"], "working")
        self.event("Stop", 7)
        record = self.record()
        self.assertEqual(record["state"], "done")
        self.assertEqual(record["project"], "OpenStreamHub")
        self.assertEqual((record["pid"], record["ttys"], record["pids"]),
                         (4242, ["/dev/ttys004", "/dev/ttys002"], [4242, 4000]))
        self.assertEqual(record["terminal"], {"program": "iTerm.app", "bundle": "com.googlecode.iterm2",
                                              "iterm": "UUID-1", "multiplexed": False})
        self.assertNotIn("cwd", record)

    def test_an_older_event_that_comes_last_is_ignored(self):
        self.event("Stop", 10)
        self.event("PreToolUse", 9, tool_name="Bash")  # hooks run in the background, in any order
        self.assertEqual(self.record()["state"], "done")

    def test_a_question_waits_and_an_idle_notice_changes_nothing(self):
        self.event("Notification", 1, notification_type="elicitation_dialog")
        self.assertEqual((self.record()["state"], self.record()["detail"]), ("waiting", "question"))
        self.event("Stop", 2)
        self.event("Notification", 3, notification_type="idle_prompt")
        self.assertEqual(self.record()["state"], "done")

    def test_session_end_leaves_an_ended_record_that_late_hooks_cannot_bring_back(self):
        self.event("SessionStart", 1)
        hook.statusline({"session_id": "abc", "context_window": {"used_percentage": 10}}, "")
        self.event("Stop", 35)  # its Python started later than SessionEnd's
        self.event("SessionEnd", 30, reason="exit")
        self.event("Stop", 20)  # ran in the background, came last
        self.event("Stop", 40)
        hook.statusline({"session_id": "abc", "context_window": {"used_percentage": 11}}, "")
        self.assertEqual(self.record()["state"], "ended")
        self.assertEqual(sorted(os.listdir(self.sessions)), ["abc.json", "abc.lock"])

    def test_a_question_shown_as_a_prompt_keeps_its_tool(self):
        self.event("PreToolUse", 1, tool_name="AskUserQuestion")
        self.event("Notification", 2, notification_type="permission_prompt")
        self.assertEqual((self.record()["state"], self.record()["detail"]), ("waiting", "AskUserQuestion"))

    def test_a_session_resumed_after_it_ended_comes_back(self):
        self.event("SessionEnd", 1)
        self.event("SessionStart", 2, source="resume")
        self.assertEqual(self.record()["state"], "idle")

    def test_inside_tmux_the_iterm_session_is_not_trusted(self):
        found = hook.terminal({"TERM_PROGRAM": "tmux", "TMUX": "/tmp/tmux-501/default,1,0",
                               "ITERM_SESSION_ID": "w0t0p0:UUID-1", "__CFBundleIdentifier": "com.googlecode.iterm2"})
        self.assertEqual((found["iterm"], found["multiplexed"]), ("", True))
        self.assertEqual(hook.terminal({"TERM_PROGRAM": "vscode", "ITERM_SESSION_ID": "w0t0p0:UUID-1"})["iterm"], "")

    def test_the_hooks_write_to_the_app_data_folder_wherever_the_script_is(self):
        with mock.patch.dict(os.environ, {"OSH_CLAUDE_DATA": ""}):
            self.assertEqual(hook.data_folder(), os.path.expanduser(
                "~/Library/Application Support/dev.openstreamhub/plugin-data/com.github.goniszewski.claudecode"))

    def test_a_bad_session_id_writes_nothing(self):
        self.assertIsNone(hook.handle({"session_id": "../x", "hook_event_name": "Stop"}, environ={}))
        self.assertIsNone(hook.handle({"hook_event_name": "Stop"}, environ={}))

    def test_the_claude_process_is_found_past_shells_with_the_ttys_above_it(self):
        table = {10: (9, "??", "sh"), 9: (8, "ttys007", "zsh"), 8: (7, "ttys007", "claude"),
                 7: (6, "ttys007", "zsh"), 6: (5, "ttys002", "zsh (wrapper)"), 5: (1, "??", "iTermServer")}
        self.assertEqual(hook.claude_process(10, table.get), (8, ["/dev/ttys007", "/dev/ttys002"], [8, 7, 6, 5]))
        self.assertEqual(hook.claude_process(10, {10: (1, "??", "sh")}.get), (None, [], []))
        self.assertEqual(hook.claude_process(1, table.get), (None, [], []), "a hook whose Claude Code has exited")

    def test_the_status_line_writes_the_meter_and_prints_a_line(self):
        line = hook.statusline({"session_id": "abc", "model": {"display_name": "Opus"},
                                "context_window": {"used_percentage": 41.6}, "cost": {"total_cost_usd": 1.234}}, "")
        self.assertEqual(line, "Opus · context 42% · $1.23")
        with open(os.path.join(self.sessions, "abc.meter.json")) as file:
            meter = json.load(file)
        self.assertEqual((meter["used"], meter["cost"]), (41.6, 1.234))

    def test_the_status_line_runs_your_own_and_prints_what_it_prints(self):
        line = hook.statusline({"session_id": "abc"}, "/usr/bin/python3 -c 'import sys,json;"
                               "print(\"mine\", json.load(sys.stdin)[\"session_id\"])'")
        self.assertEqual(line, "mine abc")

    def test_the_hook_prints_nothing_and_exits_0_on_bad_input(self):
        done = subprocess.run([sys.executable, "-B", hook.__file__], input="not json", capture_output=True,
                              text=True, env=dict(os.environ, OSH_CLAUDE_DATA=self.folder.name))
        self.assertEqual((done.returncode, done.stdout), (0, ""))


class PromptTests(unittest.TestCase):
    def test_a_prompt_with_yes_chosen(self):
        self.assertEqual(plugin.prompt_on_screen(PROMPT), "yes")

    def test_a_prompt_with_another_option_chosen(self):
        text = PROMPT.replace("❯ 1. Yes", "  1. Yes").replace("   3. No", " ❯ 3. No")
        self.assertEqual(plugin.prompt_on_screen(text), "other")

    def test_a_prompt_in_a_box(self):
        boxed = "\n".join("│ %s │" % line for line in PROMPT.splitlines())
        self.assertEqual(plugin.prompt_on_screen(boxed), "yes")

    def test_a_prompt_followed_by_output_is_gone(self):
        later = PROMPT + "\n⏺ Bash(npm test)\n  ⎿ 12 passing\n\n⏺ All tests pass.\n\n> \n  ? for shortcuts\n"
        self.assertIsNone(plugin.prompt_on_screen(later))

    def test_a_long_option_that_wraps(self):
        text = PROMPT.replace("in /repo", "in\n     /a/very/long/path")
        self.assertEqual(plugin.prompt_on_screen(text), "yes")

    def test_a_quoted_or_prose_list_is_not_a_prompt(self):
        self.assertIsNone(plugin.prompt_on_screen("> 1. Yes\n> 2. No"))
        self.assertIsNone(plugin.prompt_on_screen("Pick one:\n 1. Yes\n 2. No\n\n> \n  ? for shortcuts"))
        self.assertIsNone(plugin.prompt_on_screen("Ok?\n ❯ 1. Yes\n   3. No"), "numbered with a gap")

    def test_a_question_claude_asks_is_not_a_permission_prompt(self):
        asked = "Should I add tests?\n❯ 1. Yes\n  2. No\nEnter to select · Esc to cancel"
        self.assertIsNone(plugin.prompt_on_screen(asked))

    def test_a_last_option_that_wraps_and_an_earlier_list_above(self):
        text = "Earlier:\nDo you want to x?\n 1. Yes\n 2. No\n" + PROMPT.replace(
            "differently (esc)", "differently\n     (esc)")
        self.assertEqual(plugin.prompt_on_screen(text), "yes")

    def test_a_real_prompt_whose_option_wraps_over_several_lines(self):
        # Claude Code 2.x in an 80-column iTerm2 window, 17 November 2026 (the folder name changed)
        with open(os.path.join(os.path.dirname(os.path.abspath(__file__)), "screens", "bash-prompt-wrapped.txt"),
                  encoding="utf-8") as file:
            screen = file.read()
        self.assertEqual(plugin.prompt_on_screen(screen), "yes")
        self.assertEqual(plugin.prompt_on_screen(screen.replace("❯ 1. Yes", "  1. Yes").replace("   4. No", " ❯ 4. No")),
                         "other")

    def test_text_without_a_prompt(self):
        self.assertIsNone(plugin.prompt_on_screen("1. Yes\nsome list\n2. Maybe"))
        self.assertIsNone(plugin.prompt_on_screen(""))


def session(id="a", state="idle", project="repo", at=1, detail="", terminal=None, used=None, cost=None,
            ttys=("/dev/ttys001",), pids=(500, 400)):
    return plugin.Session(id, state, detail, project, at, 1, ttys, pids,
                          terminal if terminal is not None else {"iterm": "U-" + id}, used, cost)


class StoreTests(DataFolder):
    def test_sessions_are_read_with_their_meter_and_dead_ones_are_dropped(self):
        self.event("Stop", 5)
        hook.statusline({"session_id": "abc", "context_window": {"used_percentage": 50},
                         "cost": {"total_cost_usd": 2.0}}, "")
        store = plugin.Store(self.sessions, is_alive=lambda pid: True, clock=lambda: 10)
        [found] = store.sessions()
        self.assertEqual((found.id, found.state, found.project, found.used, found.cost),
                         ("abc", "done", "OpenStreamHub", 50, 2.0))
        self.assertEqual(plugin.Store(self.sessions, is_alive=lambda pid: False).sessions(), [])
        self.assertEqual(os.listdir(self.sessions), ["abc.lock"], "a session whose Claude Code is gone is removed")

    def test_ended_records_and_orphan_meters_are_cleaned_up_later(self):
        with open(os.path.join(self.folder.name, "zzz.meter.json"), "w") as file:
            file.write("{}")
        now = os.path.getmtime(os.path.join(self.folder.name, "zzz.meter.json"))
        os.makedirs(self.sessions, exist_ok=True)
        os.replace(os.path.join(self.folder.name, "zzz.meter.json"), os.path.join(self.sessions, "zzz.meter.json"))
        self.event("SessionEnd", int(now * 1e9))
        self.assertEqual(plugin.Store(self.sessions, clock=lambda: now).sessions(), [])
        self.assertEqual(len(os.listdir(self.sessions)), 3, "kept for a while")
        plugin.Store(self.sessions, clock=lambda: now + 2 * plugin.ENDED_SECONDS).sessions()
        self.assertEqual(os.listdir(self.sessions), ["abc.lock"])

    def test_a_session_of_unknown_process_is_dropped_after_a_day(self):
        hook.handle({"session_id": "abc", "hook_event_name": "Stop"}, environ={}, now=0,
                    find_claude=lambda: (None, [], []))
        self.assertEqual(len(plugin.Store(self.sessions, clock=lambda: 3600).sessions()), 1)
        self.assertEqual(plugin.Store(self.sessions, clock=lambda: 2 * 86400).sessions(), [])

    def test_the_one_that_most_needs_you_comes_first(self):
        listed = plugin.ordered([session("i", "idle", at=9), session("w", "working", at=8),
                                 session("d", "done", at=1), session("q", "waiting", at=2, project="other")])
        self.assertEqual([s.id for s in listed], ["q", "d", "w", "i"])
        self.assertEqual([s.id for s in plugin.ordered(listed, "other")], ["q"])


class PressTests(unittest.TestCase):
    def setUp(self):
        self.clock = Clock()
        self.done = []
        self.presses = plugin.Presses(lambda kind, control, params: self.done.append(kind), self.clock.schedule)

    def test_short_and_long(self):
        self.presses.press(KEY, {})
        self.presses.release(KEY)
        self.assertEqual(self.done, ["short"])
        self.presses.press(KEY, {})
        self.clock.fire()
        self.presses.release(KEY)
        self.assertEqual(self.done, ["short", "long"])

    def test_a_key_whose_release_never_comes_acts_at_each_press(self):
        self.presses.press(KEY, {})
        self.presses.press(KEY, {})  # no release in between: its Release runs something else
        self.assertEqual(self.done, ["short"])
        self.presses.press(KEY, {})
        self.assertEqual(self.done, ["short", "short"])
        self.clock.fire()
        self.assertEqual(self.done, ["short", "short"])

    def test_a_key_that_leaves_the_screen_while_held_does_nothing(self):
        self.presses.press(KEY, {})
        self.presses.forget(KEY)
        self.clock.fire()
        self.presses.release(KEY)
        self.assertEqual(self.done, [])


def look(front=True, prompt="yes", owned=True):
    return plugin.Look(front, prompt, "TEXT", owned, "PROMPT" if prompt else None)


class FakeStore:
    def __init__(self, sessions):
        self.list = sessions

    def sessions(self):
        return list(self.list)


class FakeTerminals:
    def __init__(self, look=None):
        self.calls = []
        self.looked = look

    def focus(self, session):
        self.calls.append(("focus", session.id))

    def look(self, session):
        self.calls.append(("look", session.id))
        return self.looked

    def key(self, session, verb, text):
        self.calls.append((verb, session.id, text))


class FakeArtist:
    def __init__(self):
        self.faces = []

    def image(self, face):
        self.faces.append(face)
        return face, "IMG"


class PluginTests(unittest.TestCase):
    def setUp(self):
        self.sent = []
        self.clock = Clock()
        self.store = FakeStore([session("a", "waiting", at=5, detail="Bash"), session("b", "done", at=4)])
        self.terminals = FakeTerminals()
        self.artist = FakeArtist()
        self.plugin = plugin.Plugin(self.sent.append, self.store, self.terminals, self.artist,
                                    schedule=self.clock.schedule, start_threads=False, clock=self.clock)
        self.plugin.handle({"type": "appear", "key": "k1", "action": "session", "params": {"answer": True},
                            "control": {"module": "m4315", "index": 3}})

    def press(self, long=False, params=None):
        params = {"answer": True} if params is None else params
        self.plugin.handle({"type": "invoke", "request": 1, "action": "session", "params": params,
                            "input": {"gesture": "pressed", "control": {"module": "m4315", "index": 3}}})
        if long:
            self.clock.fire()
        self.plugin.handle({"type": "event", "action": "session", "params": params, "gesture": "released",
                            "control": {"module": "m4315", "index": 3}})
        while self.plugin.work:
            self.plugin.run_job(*self.plugin.take_job())

    def in_front_for_a_while(self, prompt="yes", owned=True):
        """The key showed the session waiting, and the terminal its prompt in front, a second ago."""
        self.terminals.looked = look(True, prompt, owned)
        self.plugin.poll_once()
        self.clock.now += 1
        self.terminals.calls.clear()

    def logs(self):
        return [m["message"] for m in self.sent if m["type"] == "log"]

    def test_a_press_is_answered_at_once(self):
        self.press()
        self.assertEqual(self.sent[0], {"type": "result", "request": 1, "ok": True})

    def test_a_press_answers_yes_only_with_the_terminal_in_front_for_a_while_and_yes_chosen(self):
        self.in_front_for_a_while()
        self.press()
        self.assertEqual(self.terminals.calls, [("look", "a"), ("return", "a", "TEXT")])
        self.assertIn("answered yes (once) to Bash in repo", self.logs())
        self.assertEqual(self.plugin.sessions()[0].state, "working")

    def test_a_terminal_just_brought_to_the_front_is_not_answered_yet(self):
        self.terminals.looked = look(False)
        self.plugin.poll_once()
        self.clock.now += 1
        self.terminals.looked = look(True)
        self.press()
        self.assertEqual(self.terminals.calls[-1], ("look", "a"))
        self.assertTrue(any("only just come up or come to the front" in line for line in self.logs()))
        self.clock.now += 1
        self.press()
        self.assertEqual(self.terminals.calls[-1], ("return", "a", "TEXT"))

    def test_a_prompt_the_key_has_not_shown_yet_is_not_answered(self):
        self.store.list = [session("a", "working", at=4)]
        self.terminals.looked = look(True)
        self.plugin.poll_once()  # the key shows "working"
        self.clock.now += 5
        self.store.list = [session("a", "waiting", at=5, detail="Bash")]  # a prompt, not drawn yet
        self.press()
        self.assertEqual(self.terminals.calls[-1], ("focus", "a"), "the press meant what the key showed")
        self.plugin.poll_once()
        self.press()
        self.assertNotIn("return", [call[0] for call in self.terminals.calls], "drawn only just now")

    def test_a_new_prompt_in_the_same_session_waits_again(self):
        self.in_front_for_a_while()
        self.terminals.looked = plugin.Look(True, "yes", "TEXT2", True, "NEXT PROMPT")
        self.press()
        self.assertEqual(self.terminals.calls, [("look", "a")])

    def test_a_terminal_another_program_owns_is_not_answered(self):
        self.in_front_for_a_while(owned=False)
        self.press()
        self.assertEqual(self.terminals.calls, [("look", "a")])
        self.assertTrue(any("another program" in line for line in self.logs()))

    def test_with_the_terminal_behind_a_press_brings_it_to_the_front(self):
        self.terminals.looked = look(False)
        self.plugin.poll_once()
        self.clock.now += 1
        self.terminals.calls.clear()
        self.press()
        self.assertEqual(self.terminals.calls, [("look", "a"), ("focus", "a")])

    def test_with_another_option_chosen_nothing_is_typed(self):
        self.in_front_for_a_while("other")
        self.press()
        self.assertEqual(self.terminals.calls, [("look", "a")])
        self.assertTrue(any("another option" in line for line in self.logs()))

    def test_a_long_press_declines(self):
        self.in_front_for_a_while("other")
        self.press(long=True)
        self.assertEqual(self.terminals.calls, [("look", "a"), ("escape", "a", "TEXT")])
        self.assertEqual(self.plugin.sessions()[0].state, "idle", "Claude Code stops its turn when declined")

    def test_the_keys_keep_updating_after_a_prompt_is_answered_from_a_key(self):
        for long, state in ((False, "working"), (True, "idle")):
            with self.subTest(long=long):
                self.setUp()
                self.in_front_for_a_while("yes" if not long else "other")
                self.press(long=long)
                self.terminals.looked = look(True, None)  # the prompt has left the screen
                for _ in range(3):
                    self.plugin.poll_once()
                    self.assertEqual(self.plugin.sessions()[0].state, state)
                self.assertFalse(any("could not" in line for line in self.logs()))

    def test_a_question_is_never_answered_from_a_key(self):
        for detail in ("question", "AskUserQuestion", "ExitPlanMode"):
            self.store.list = [session("a", "waiting", at=5, detail=detail)]
            self.in_front_for_a_while()
            self.press()
            self.assertEqual(self.terminals.calls, [("focus", "a")], detail)

    def test_a_press_that_waited_too_long_is_dropped(self):
        self.plugin.queue("short", KEY, {"_action": "session"})
        self.clock.now += plugin.JOB_SECONDS + 0.5
        self.plugin.queue("long", KEY, {"_action": "session"})
        self.assertEqual(self.plugin.take_job()[0], "long")
        self.assertIn("dropped a press that waited too long for the terminal", self.logs())

    def test_a_press_acts_on_the_session_the_key_shows(self):
        self.terminals.looked = look(False, None)
        self.plugin.poll_once()  # the key shows "a"
        self.store.list = [session("c", "waiting", at=9, detail="Edit")] + self.store.list  # not drawn yet
        self.press(params={})
        self.assertEqual(self.terminals.calls[-1], ("focus", "a"))

    def test_without_answering_on_a_press_only_brings_the_terminal_to_the_front(self):
        self.in_front_for_a_while()
        self.press(params={})
        self.assertEqual(self.terminals.calls, [("focus", "a")])

    def test_a_long_press_away_from_a_prompt_shows_the_next_session_for_a_while(self):
        self.press(long=True, params={})
        self.assertEqual(self.terminals.calls, [])
        shown, others = self.plugin.chosen(self.plugin.sessions(), KEY, {})
        self.assertEqual((shown.id, others), ("b", 1))
        self.clock.now += plugin.PIN_SECONDS
        self.assertEqual(self.plugin.chosen(self.plugin.sessions(), KEY, {})[0].id, "a")

    def test_a_done_session_brought_to_the_front_shows_idle(self):
        self.store.list = [session("b", "done", at=4)]
        self.press(params={})
        self.assertEqual(self.terminals.calls, [("focus", "b")])
        self.assertEqual(self.plugin.sessions()[0].state, "idle")
        self.store.list = [session("b", "done", at=9)]  # done again later
        self.assertEqual(self.plugin.sessions()[0].state, "done")

    def test_a_prompt_answered_in_the_terminal_shows_working_after_two_looks(self):
        self.terminals.looked = look(True, None)
        self.plugin.poll_once()
        self.assertEqual(self.artist.faces[-1][0], "waiting")
        self.plugin.poll_once()
        self.assertEqual({s.id: s.state for s in self.plugin.sessions()}, {"a": "working", "b": "done"})
        self.assertEqual(self.artist.faces[-1][0], "done", "the key moves on to the session that needs you")
        self.store.list = [session("a", "waiting", at=6, detail="Edit")]  # a new prompt
        self.terminals.looked = look(False)
        self.plugin.poll_once()
        self.assertEqual(self.artist.faces[-1][:3], ("waiting", "repo", "Edit"))

    def test_what_is_known_of_ended_sessions_is_forgotten(self):
        self.in_front_for_a_while()
        self.assertIn("a", self.plugin.front_since)
        self.store.list = []
        self.plugin.poll_once()
        self.assertEqual((self.plugin.front_since, self.plugin.gone), ({}, {}))

    def test_keys_are_drawn_when_they_change(self):
        self.terminals.looked = look(False)
        self.plugin.poll_once()
        images = [m for m in self.sent if m["type"] == "set_image"]
        self.assertEqual([(m["key"], m["image"]) for m in images], [("k1", "IMG")])
        self.assertEqual(self.artist.faces[-1], ("waiting", "repo", "Bash", "1"))
        self.plugin.poll_once()
        self.assertEqual(len([m for m in self.sent if m["type"] == "set_image"]), 1, "unchanged: not sent again")

    def test_the_context_key_shows_the_meter(self):
        self.store.list = [session("a", "working", used=63.4, cost=0.5)]
        self.plugin.handle({"type": "appear", "key": "k2", "action": "context", "params": {},
                            "control": {"module": "m4315", "index": 4}})
        self.plugin.poll_once()
        self.assertIn(("meter", "63", "$0.50", "repo"), self.artist.faces)

    def test_on_the_dial_the_context_key_shows_the_round_meter(self):
        self.store.list = [session("a", "working", used=63.4, cost=0.5)]
        self.plugin.handle({"type": "appear", "key": "k3", "action": "context", "params": {},
                            "control": {"module": "m131e", "index": 0}})
        self.plugin.poll_once()
        self.assertIn(("dial", "meter", "63", "$0.50", "repo"), self.artist.faces)

    def test_the_condition(self):
        self.plugin.handle({"type": "check", "request": 7, "condition": "state", "params": {"state": "done"}})
        self.plugin.handle({"type": "check", "request": 8, "condition": "state",
                            "params": {"state": "working", "project": "repo"}})
        results = {m["request"]: m["value"] for m in self.sent if m["type"] == "result"}
        self.assertEqual(results, {7: True, 8: False})

    def test_an_unknown_action_fails(self):
        self.plugin.handle({"type": "invoke", "request": 3, "action": "nope", "params": {}})
        self.assertEqual(self.sent[-1]["ok"], False)

    def test_no_session_is_logged(self):
        self.store.list = []
        self.press(params={"project": "elsewhere"})
        self.assertTrue(any("no Claude Code session of elsewhere" in line for line in self.logs()))


class TerminalTests(unittest.TestCase):
    def run_fake(self, out, code=0, foreground="  400 S+\n  401 S\n"):
        calls = []

        def run(args, **kwargs):
            calls.append(args)
            if args[0] == "/bin/ps":
                return subprocess.CompletedProcess(args, 0, foreground, "")
            return subprocess.CompletedProcess(args, code, out, "")
        return calls, plugin.Terminals(run)

    def test_iterm_is_read_by_session_id_and_ttys_and_its_owner_checked(self):
        calls, terminals = self.run_fake("true\t/dev/ttys002\n" + PROMPT + "\n\n")
        found = terminals.look(session("a", ttys=("/dev/ttys003", "/dev/ttys002")))
        self.assertEqual((found.front, found.prompt, found.owned), (True, "yes", True))
        self.assertEqual(found.text, PROMPT[PROMPT.index(" Do you want"):].rstrip("\n"),
                         "only the prompt is checked again: a line above it may blink")
        self.assertEqual(calls[0][1:], [os.path.join(plugin.HERE, "iterm.applescript"), "look", "U-a",
                                        "/dev/ttys003,/dev/ttys002"])
        self.assertEqual(calls[1], ["/bin/ps", "-o", "pid=,stat=", "-t", "ttys002"])

    def test_a_terminal_whose_foreground_is_another_program_is_not_owned(self):
        calls, terminals = self.run_fake("true\t/dev/ttys002\n" + PROMPT, foreground="  400 S\n  999 S+\n")
        self.assertFalse(terminals.look(session("a")).owned)

    def test_terminal_app_is_read_by_tty(self):
        calls, terminals = self.run_fake("false\t/dev/ttys001\n")
        found = session("a", terminal={"program": "Apple_Terminal", "bundle": "com.apple.Terminal"})
        self.assertEqual(terminals.look(found), plugin.Look(False, None, "", False, None))
        self.assertEqual(calls[0][2:], ["look", "/dev/ttys001"])
        self.assertIsNone(plugin.Terminals.where(session("a", terminal={"bundle": "com.apple.Terminal"})),
                          "an app started from Terminal inherits its bundle id, not TERM_PROGRAM")

    def test_under_tmux_or_without_ttys_it_is_only_brought_to_the_front(self):
        for other in (session("a", terminal={"iterm": "U-a", "bundle": "com.googlecode.iterm2", "multiplexed": True}),
                      session("a", ttys=(), terminal={"iterm": "U-a", "bundle": "com.googlecode.iterm2"})):
            calls, terminals = self.run_fake("")
            self.assertIsNone(terminals.look(other))
            terminals.focus(other)
            self.assertEqual(calls, [[plugin.OPEN, "-b", "com.googlecode.iterm2"]])

    def test_another_terminal_is_only_brought_to_the_front(self):
        calls, terminals = self.run_fake("")
        other = session("a", terminal={"bundle": "com.mitchellh.ghostty"})
        self.assertIsNone(terminals.look(other))
        terminals.focus(other)
        self.assertEqual(calls, [[plugin.OPEN, "-b", "com.mitchellh.ghostty"]])
        with self.assertRaises(plugin.TerminalError):
            terminals.key(other, "return", "")

    def test_a_key_is_sent_with_the_text_it_was_checked_against(self):
        calls, terminals = self.run_fake("ok\n")
        terminals.key(session("a"), "return", "Do you want to proceed?\n❯ 1. Yes\n  2. No")
        self.assertEqual(calls[0][2:], ["return", "U-a", "/dev/ttys001", "3", "Do you want to proceed?\n❯ 1. Yes\n  2. No"])
        for out, reason in (("not_front", "no longer in front"), ("changed", "changed while answering")):
            calls, terminals = self.run_fake(out + "\n")
            with self.assertRaisesRegex(plugin.TerminalError, reason):
                terminals.key(session("a"), "return", "SCREEN")


class InstallTests(unittest.TestCase):
    def test_install_twice_then_remove_gives_back_the_settings(self):
        mine = {"statusLine": {"type": "command", "command": "~/bin/line.sh", "padding": 0},
                "hooks": {"Stop": [{"hooks": [{"type": "command", "command": "say done"}]}]}, "model": "opus"}
        settings = json.loads(json.dumps(mine))
        install_hooks.install(settings)
        install_hooks.install(settings)
        self.assertEqual(len(settings["hooks"]["Stop"]), 2, "the plugin's hook is added once, beside yours")
        self.assertEqual(sorted(settings["hooks"]), sorted(set(install_hooks.EVENTS)))
        self.assertTrue(all(entry["async"] for event in install_hooks.EVENTS
                            for group in settings["hooks"][event] for entry in group["hooks"]
                            if install_hooks.ours(entry)))
        self.assertIn("statusline '~/bin/line.sh'", settings["statusLine"]["command"])
        install_hooks.uninstall(settings)
        self.assertEqual(settings, mine)

    def test_without_a_status_line_of_your_own_remove_takes_it_out(self):
        settings = install_hooks.install({})
        self.assertTrue(settings["statusLine"]["command"].endswith("statusline"))
        self.assertEqual(install_hooks.uninstall(settings), {})

    def test_your_own_empty_hooks_and_a_status_line_that_is_not_a_command_are_kept(self):
        mine = {"hooks": {"Stop": [{"hooks": []}]}, "statusLine": {"type": "static", "text": "hi"}}
        settings = json.loads(json.dumps(mine))
        install_hooks.install(settings)
        self.assertEqual(settings["statusLine"], {"type": "static", "text": "hi"})
        self.assertEqual(install_hooks.uninstall(settings), mine)
        self.assertEqual(install_hooks.uninstall({"hooks": {}}), {"hooks": {}})

    def test_hooks_of_an_unexpected_shape_are_refused(self):
        with self.assertRaises(install_hooks.SettingsError):
            install_hooks.install({"hooks": {"Stop": {"oops": 1}}})

    def test_the_default_settings_file_follows_claude_config_dir(self):
        home = os.path.expanduser("~")
        self.assertEqual(install_hooks.default_settings({}), os.path.join(home, ".claude", "settings.json"))
        self.assertEqual(install_hooks.default_settings({"CLAUDE_CONFIG_DIR": ""}),
                         os.path.join(home, ".claude", "settings.json"), "an empty one is not set")
        self.assertEqual(install_hooks.default_settings({"CLAUDE_CONFIG_DIR": "~/.claude-work"}),
                         os.path.join(home, ".claude-work", "settings.json"))
        self.assertEqual(install_hooks.default_settings({"CLAUDE_CONFIG_DIR": "/tmp/cc"}), "/tmp/cc/settings.json")

    def test_a_linked_settings_file_is_changed_where_it_points_and_backed_up_once(self):
        with tempfile.TemporaryDirectory() as folder:
            real = os.path.join(folder, "dotfiles-settings.json")
            with open(real, "w") as file:
                file.write('{"model": "opus"}')
            os.chmod(real, 0o640)
            link = os.path.join(folder, "settings.json")
            os.symlink(real, link)
            with mock.patch("sys.stdout"), mock.patch("sys.stderr"):
                self.assertEqual(install_hooks.main(["--settings", link]), 0)
                self.assertEqual(install_hooks.main(["--settings", link]), 0)
            self.assertTrue(os.path.islink(link))
            self.assertEqual(os.stat(real).st_mode & 0o777, 0o640)
            with open(real) as file:
                self.assertIn("hooks", json.load(file))
            with open(real + ".osh-backup") as file:
                self.assertEqual(json.load(file), {"model": "opus"}, "the backup is the settings from before")
            with mock.patch("sys.stdout"), mock.patch("sys.stderr"):
                install_hooks.main(["--settings", link, "--remove"])
                with open(real, "w") as file:
                    file.write('{"model": "sonnet"}')  # months later
                install_hooks.main(["--settings", link])
            with open(real + ".osh-backup") as file:
                self.assertEqual(json.load(file), {"model": "sonnet"}, "a new install backs up the settings of now")

    def test_a_bad_settings_file_is_left_alone(self):
        with tempfile.TemporaryDirectory() as folder:
            path = os.path.join(folder, "settings.json")
            with open(path, "w") as file:
                file.write("{oops")
            with mock.patch("sys.stderr"):
                self.assertEqual(install_hooks.main(["--settings", path]), 1)
            with open(path) as file:
                self.assertEqual(file.read(), "{oops")


class RenderTests(unittest.TestCase):
    def test_render_draws_every_face(self):
        artist = plugin.Artist(tempfile.mkdtemp())
        for face in [("waiting", "OpenStreamHub", "Bash", "2"), ("done", "repo", "", ""),
                     ("working", "repo", "Edit", ""), ("idle", "", "", ""), ("none",),
                     ("meter", "72", "$3.41", "repo"), ("meter", "", "", "")]:
            _, image = artist.image(face)
            data = base64.b64decode(image)
            self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n", face)
            self.assertEqual(int.from_bytes(data[16:20], "big"), 72)
            self.assertLessEqual(len(data), plugin.MAX_IMAGE)
        for face in [("dial", "meter", "72", "$3.41", "OpenStreamHub"), ("dial", "meter", "", "", ""),
                     ("dial", "waiting", "repo", "Bash", "2"), ("dial", "none")]:
            _, image = artist.image(face)
            data = base64.b64decode(image)
            self.assertEqual(int.from_bytes(data[16:20], "big"), 240, face)
            self.assertLessEqual(len(data), plugin.MAX_DIAL_IMAGE, face)


if __name__ == "__main__":
    unittest.main()
