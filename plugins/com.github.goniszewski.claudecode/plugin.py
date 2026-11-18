"""Claude Code: an OpenStreamHub plugin that puts Claude Code sessions on M4315 keys, using only the standard library.

Claude Code tells the plugin what its sessions do through hooks: `hook.py`, which `install_hooks.py` adds to
~/.claude/settings.json, writes each session's state to the plugin's data folder, and the plugin reads it.

Action "session" (bind it to a key's Press, with nothing on its Release): the key shows one session, the one that
most needs you (waiting for you, then done, then working, then idle) or, with "Project" set, one of that project's.
- A short press brings the session's terminal to the front.
- A long press (held 600 ms) shows the next session; after 30 seconds the key goes back to the one that most needs you.
- With "Answer permission prompts" on, while the session waits for a permission: once its terminal is in front and
  the prompt is on screen with "1. Yes" chosen, a short press answers it (Return: yes, once) and a long press
  declines it (Escape). Only in iTerm2 and Terminal, whose text the plugin can read; anywhere else a press only brings
  the terminal to the front.

Action "context": the key shows how full the session's context window is, as a ring, and what it has cost, from the
status line (`hook.py statusline`). Presses work as on "session", without answering prompts.

Condition "state": whether a session (of the project, when set) is waiting for you, working, done or idle.

Fields it does not know, in any message, are ignored: newer apps may add some.
"""

import atexit
import base64
import collections
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time

HERE = os.path.dirname(os.path.abspath(__file__))
OSASCRIPT = "/usr/bin/osascript"
OPEN = "/usr/bin/open"
ACTIONS = ("session", "context")
STATES = ("waiting", "done", "working", "idle")
RANK = {state: rank for rank, state in enumerate(STATES)}
MAX_IMAGE = 32 * 1024
MAX_DIAL_IMAGE = 40 * 1024
DIAL_MODULE = "m131e"
POLL_SECONDS = 1.0
LONG_SECONDS = 0.6
PIN_SECONDS = 30.0
SCRIPT_SECONDS = 5.0
STALE_SECONDS = 7 * 24 * 3600  # a record this old is dropped even when its process is alive
UNKNOWN_STALE_SECONDS = 24 * 3600  # and this old when its process is not known
ENDED_SECONDS = 600  # an ended session's record, kept so a late hook cannot bring it back
FRONT_SECONDS = 0.7  # how long a terminal must have been seen in front before a press answers in it
JOB_SECONDS = 1.0  # a press that waited longer than this behind a slow terminal is dropped
PROMPT_GONE_LOOKS = 2  # a waiting session whose prompt is not on screen this many looks in a row is shown working
IMAGE_RETRIES = 3
LOG_REPEAT_SECONDS = 60.0

# A permission prompt as Claude Code draws it, last on screen:
#    Do you want to proceed?
#    ❯ 1. Yes
#      2. Yes, and don't ask again for …
#      3. No, and tell Claude what to do differently (esc)
#    Esc to cancel · Tab to amend
# Box borders are taken off first. Only "❯" marks the chosen option, so a quoted "> 1. Yes" is not one.
BORDER = "│┃|╭╮╰╯─ \t"
PROMPT_LINES = 16  # how far up from the end of the screen a prompt is looked for
NOT_PERMISSIONS = ("question", "AskUserQuestion", "ExitPlanMode")  # waits that are never answered from a key
OPTION = re.compile(r"^(❯)?\s*(\d)\.\s+(.*)$")


def prompt_lines(text):
    lines = [line.strip(BORDER) for line in (text or "").splitlines()]
    return [line for line in lines if line][-PROMPT_LINES:]


def prompt_on_screen(text):
    """Whether Claude Code's permission prompt is the last thing in a terminal's text: None when it is not, else
    "yes" when its chosen option is "1. Yes" (yes, once) and "other" when another option is chosen.

    It must ask "Do you want to …?" just above its options (a long question may wrap), have options numbered from 1
    without a gap ("1. Yes" first, one starting "No"), and nothing after them but lines a long option wrapped onto and
    the "Esc to cancel" footer, last. A question Claude asks you reads otherwise, so it is never taken for one."""
    lines = prompt_lines(text)
    starts = [i for i, line in enumerate(lines) if re.match(r"^❯?\s*1\.\s+Yes$", line)]
    if not starts:
        return None
    first = starts[-1]
    asked = [i for i in range(max(0, first - 3), first) if lines[i].startswith("Do you want to")]
    if not asked or not lines[first - 1].endswith("?"):
        return None
    labels, chosen, last = [], None, first
    for place in range(first, len(lines)):
        match = OPTION.match(lines[place])
        if match is None:
            following = next((OPTION.match(line) for line in lines[place + 1:] if OPTION.match(line)), None)
            if following and int(following.group(2)) == len(labels) + 1:
                continue  # a long option wrapped onto this line (and maybe more), before the next option
            break
        if int(match.group(2)) != len(labels) + 1:
            return None
        labels.append(match.group(3).strip())
        last = place
        if match.group(1):
            chosen = len(labels)
    if len(labels) < 2 or not any(label.startswith("No") for label in labels):
        return None
    after = lines[last + 1:]
    if after:
        # only the footer, last, and before it at most four lines the last option wrapped onto
        if "Esc to cancel" not in after[-1] or len(after) > 5:
            return None
    return "yes" if chosen == 1 else "other"


def short(text, limit=200):
    return " ".join(str(text or "").split())[:limit]


def control_of(control):
    control = control if isinstance(control, dict) else {}
    return control.get("module"), control.get("index")


def project_of(params):
    return short((params or {}).get("project", ""), 64).lower()


def answers(params):
    value = (params or {}).get("answer", False)
    return value is True


# --- Sessions, as the hooks wrote them ---

Session = collections.namedtuple("Session", "id state detail project at pid ttys pids terminal used cost")


def alive(pid):
    if not isinstance(pid, int) or isinstance(pid, bool) or pid <= 1:
        return None  # cannot tell
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def number(value):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


class Store:
    """Reads the sessions folder the hooks write. `alive` and `clock` (time.time) are replaced in tests. Lock files are
    left to the hooks, apart from those of sessions long gone."""

    def __init__(self, folder, is_alive=alive, clock=time.time):
        self.folder = folder
        self.is_alive = is_alive
        self.clock = clock

    def _read(self, name):
        try:
            with open(os.path.join(self.folder, name), encoding="utf-8") as file:
                value = json.load(file)
            return value if isinstance(value, dict) else None
        except (OSError, ValueError):
            return None

    def _remove(self, name):
        try:
            os.remove(os.path.join(self.folder, name))
        except OSError:
            pass

    def _older(self, name, seconds):
        try:
            return self.clock() - os.path.getmtime(os.path.join(self.folder, name)) > seconds
        except OSError:
            return False

    def sessions(self):
        try:
            names = set(os.listdir(self.folder))
        except OSError:
            return []
        found = []
        for name in sorted(names):
            if name.endswith(".meter.json"):
                if name[:-len(".meter.json")] + ".json" not in names and self._older(name, ENDED_SECONDS):
                    self._remove(name)  # written by a status line after the session ended
                continue
            if name.endswith(".lock"):
                if name[:-len(".lock")] + ".json" not in names and self._older(name, UNKNOWN_STALE_SECONDS):
                    self._remove(name)
                continue
            if not name.endswith(".json"):
                continue
            record = self._read(name)
            if record is None:
                continue
            session = name[:-len(".json")]
            at = record.get("at") if isinstance(record.get("at"), int) else 0
            age = self.clock() - at / 1e9
            if record.get("state") == "ended":
                if age > ENDED_SECONDS:
                    self._remove(name)
                continue
            if record.get("state") not in STATES:
                continue
            pid = record.get("pid")
            living = self.is_alive(pid)
            if living is False or age > STALE_SECONDS or (living is None and age > UNKNOWN_STALE_SECONDS):
                self._remove(name)  # Claude Code ended without SessionEnd (closed terminal, crash)
                self._remove(session + ".meter.json")
                continue
            meter = self._read(session + ".meter.json") or {}
            terminal = record.get("terminal") if isinstance(record.get("terminal"), dict) else {}
            ttys = record.get("ttys") if isinstance(record.get("ttys"), list) else []
            pids = record.get("pids") if isinstance(record.get("pids"), list) else []
            found.append(Session(
                session, record["state"], short(record.get("detail"), 64), short(record.get("project"), 64), at,
                pid, tuple(t for t in ttys if isinstance(t, str) and re.match(r"^/dev/tty[a-z0-9]+$", t)),
                tuple(p for p in pids if isinstance(p, int) and not isinstance(p, bool) and p > 1), terminal,
                number(meter.get("used")), number(meter.get("cost"))))
        return found


def ordered(sessions, project=""):
    """The sessions of `project` (all when empty), the one that most needs you first, then the latest."""
    chosen = [s for s in sessions if not project or s.project.lower() == project]
    return sorted(chosen, key=lambda s: (RANK[s.state], -s.at, s.id))


# --- Terminals ---

class TerminalError(Exception):
    pass


# prompt: None, "yes" or "other"; text: the prompt's lines as read, from "Do you want to" to the end, for the answer to
# check again (only those: a line above it, such as a blinking ⏺, may change while it waits);
# owned: whether a process of the session (Claude Code or one above it) is in the foreground of the terminal found;
# seen: the prompt's last lines, to tell one prompt from the next.
Look = collections.namedtuple("Look", "front prompt text owned seen")


class Terminals:
    """Brings a session's terminal to the front, reads it, and types Return or Escape in it. `run` is subprocess.run,
    replaced in tests. iTerm2 sessions are found by their id and Terminal tabs by their tty, and either only when its
    tty is the Claude Code process's or one above it (not another tmux pane). Any other terminal app is only brought
    to the front, and cannot be read."""

    def __init__(self, run=subprocess.run):
        self.run = run

    def owned(self, session, tty):
        """Whether one of the session's processes is in the foreground of `tty`: false when another program runs
        there (Claude Code ended and something else started), or the tty belongs to another terminal app's parent."""
        if not session.pids or not re.match(r"^/dev/tty[a-z0-9]+$", tty or ""):
            return False
        try:
            done = self.run(["/bin/ps", "-o", "pid=,stat=", "-t", tty[len("/dev/"):]], capture_output=True,
                            text=True, timeout=SCRIPT_SECONDS)
        except subprocess.TimeoutExpired:
            return False
        foreground = set()
        for row in done.stdout.splitlines():
            parts = row.split()
            if len(parts) == 2 and "+" in parts[1] and parts[0].isdigit():
                foreground.add(int(parts[0]))
        return bool(foreground & set(session.pids))

    @staticmethod
    def where(session):
        """The script and its first arguments for a terminal the plugin can read, or None."""
        terminal = session.terminal or {}
        if not session.ttys or terminal.get("multiplexed"):
            return None
        ttys = ",".join(session.ttys)
        if terminal.get("iterm"):
            return ["iterm.applescript", terminal["iterm"], ttys]
        if terminal.get("program") == "Apple_Terminal":
            return ["terminal.applescript", ttys]
        return None

    def _script(self, where, verb, *more):
        try:
            done = self.run([OSASCRIPT, os.path.join(HERE, where[0]), verb] + where[1:] + list(more),
                            capture_output=True, text=True, timeout=SCRIPT_SECONDS)
        except subprocess.TimeoutExpired:
            raise TerminalError("the terminal did not answer in time")
        if done.returncode != 0:
            raise TerminalError(explain(done.stderr))
        out = done.stdout
        return out[:-1] if out.endswith("\n") else out  # osascript adds one line break

    def focus(self, session):
        where = self.where(session)
        if where is not None and self._script(where, "focus") == "ok":
            return
        bundle = (session.terminal or {}).get("bundle")
        if not bundle:
            raise TerminalError("the session's terminal is not known; restart Claude Code with the hooks installed")
        try:
            done = self.run([OPEN, "-b", bundle], capture_output=True, text=True, timeout=SCRIPT_SECONDS)
        except subprocess.TimeoutExpired:
            raise TerminalError("the terminal did not open in time")
        if done.returncode != 0:
            raise TerminalError("could not open %s: %s" % (bundle, short(done.stderr, 200)))

    def look(self, session):
        """Look(front, prompt, text), or None when the terminal cannot be read."""
        where = self.where(session)
        if where is None:
            return None
        out = self._script(where, "look")
        if out == "missing":
            return None
        first, _, text = out.partition("\n")
        front, _, tty = first.partition("\t")
        text = text.rstrip("\r\n")
        prompt = prompt_on_screen(text)
        if prompt is not None:
            raw = re.split(r"\r\n|\r|\n", text)
            start = max(i for i, line in enumerate(raw) if line.strip(BORDER).startswith("Do you want to"))
            text = "\n".join(raw[start:])
        owned = self.owned(session, tty) if prompt is not None else False
        return Look(front == "true", prompt, text, owned, "\n".join(prompt_lines(text)) if prompt else None)

    def key(self, session, verb, text):
        """Types Return or Escape in the session's terminal, in the same run that checks it is in front and its text
        is still `text`, what was read and checked."""
        where = self.where(session)
        if where is None:
            raise TerminalError("this terminal cannot be answered from a key")
        out = self._script(where, verb, str(len(re.split(r"\r\n|\r|\n", text))), text)
        if out == "ok":
            return
        raise TerminalError({"not_front": "the terminal is no longer in front: nothing was typed",
                             "changed": "the terminal changed while answering: nothing was typed; press again",
                             "missing": "the session's terminal is gone"}.get(out, out))


def explain(stderr):
    text = " ".join((stderr or "").split())
    if "-1743" in text:
        return "macOS does not allow OpenStreamHub to control the terminal: allow it in System Settings > " \
               "Privacy & Security > Automation"
    if "-1719" in text or "-25211" in text or "assistive" in text:
        return "macOS does not allow OpenStreamHub to press keys: allow it in System Settings > Privacy & " \
               "Security > Accessibility"
    return text[-300:] or "the script failed"


# --- Drawing ---

class Artist:
    """Draws keys with render.js, keeping the last few pictures by what they show."""

    def __init__(self, folder, run=subprocess.run):
        self.folder = folder
        self.run = run
        self.cache = collections.OrderedDict()

    def image(self, face):
        """(signature, base64 PNG) for a face: a tuple of strings that render.js takes as its arguments (first "dial"
        for the dial screen)."""
        if face in self.cache:
            self.cache.move_to_end(face)
            return face, self.cache[face]
        out = os.path.join(self.folder, "key.png")
        done = self.run([OSASCRIPT, "-l", "JavaScript", os.path.join(HERE, "render.js"), out] + list(face),
                        capture_output=True, text=True, timeout=SCRIPT_SECONDS * 2)
        if done.returncode != 0:
            raise TerminalError("drawing the key failed: " + " ".join(done.stderr.split())[-300:])
        with open(out, "rb") as file:
            data = file.read()
        limit = MAX_DIAL_IMAGE if face[:1] == ("dial",) else MAX_IMAGE
        if len(data) > limit:
            raise TerminalError("the key image is %d bytes, more than %d" % (len(data), limit))
        image = base64.b64encode(data).decode("ascii")
        self.cache[face] = image
        while len(self.cache) > 32:
            self.cache.popitem(last=False)
        return face, image


def face_of(action, session, others, dial=False):
    """What a key shows, as render.js's arguments; on the dial screen (`dial`), the same after "dial"."""
    if dial:
        return ("dial",) + face_of(action, session, others)
    if session is None:
        return ("none",) if action == "session" else ("meter", "", "", "")
    if action == "context":
        used = "" if session.used is None else str(int(round(min(100, max(0, session.used)))))
        cost = "" if session.cost is None else "$%.2f" % session.cost
        return ("meter", used, cost, session.project)
    return (session.state, session.project, session.detail, str(others) if others else "")


# --- Presses ---

class Presses:
    """Tells a short press from a long one, per control. `act(kind, control, params)` gets "short" or "long".
    `schedule(seconds, callback)` returns an object with cancel(): threading.Timer here, a fake clock in tests."""

    def __init__(self, act, schedule):
        self.act = act
        self.schedule = schedule
        self.lock = threading.Lock()
        self.controls = {}

    def press(self, control, params):
        with self.lock:
            state = self.controls.setdefault(control, {"down": False, "timer": None, "params": {}, "no_release": False})
            if state["timer"] is not None:
                state["timer"].cancel()
                state["timer"] = None
            state["params"] = params or {}
            if state["down"] or state["no_release"]:
                # The last press never sent its release (the key's Release runs something else): no long press here.
                state["down"], state["no_release"] = True, True
                self.act("short", control, state["params"])
                return
            state["down"] = True
            state["timer"] = self.schedule(LONG_SECONDS, lambda: self._long(control, state))

    def _long(self, control, state):
        with self.lock:
            if self.controls.get(control) is not state or not state["down"] or state["timer"] is None:
                return
            state["timer"] = None
            self.act("long", control, state["params"])

    def release(self, control):
        with self.lock:
            state = self.controls.get(control)
            if state is None or not state["down"]:
                return
            state["down"], state["no_release"] = False, False
            if state["timer"] is not None:
                state["timer"].cancel()
                state["timer"] = None
                self.act("short", control, state["params"])

    def forget(self, control):
        with self.lock:
            state = self.controls.get(control)
            if state is not None:
                if state["timer"] is not None:
                    state["timer"].cancel()
                    state["timer"] = None
                state["down"], state["no_release"] = False, False


def timer(seconds, callback):
    handle = threading.Timer(seconds, callback)
    handle.daemon = True
    handle.start()
    return handle


# --- The plugin ---

class Plugin:
    def __init__(self, send, store, terminals, artist, schedule=timer, start_threads=True, clock=time.monotonic):
        self.send = send
        self.store = store
        self.terminals = terminals
        self.artist = artist
        self.clock = clock
        self.lock = threading.RLock()
        self.poll_ready = threading.Condition(self.lock)
        self.work_ready = threading.Condition(self.lock)
        self.next_id = 1
        self.keys = {}  # key id -> {"action", "params", "control", "shown", "retries"}
        self.waiting = {}  # set_image id -> (key id, signature)
        self.pins = {}  # control -> (session id, until)
        self.seen = set()  # (session id, at) of "done" sessions brought to the front from a key
        self.gone = {}  # session id -> (at, looks in a row without its prompt on screen[, "declined"])
        self.front_since = {}  # session id -> when its terminal was first seen in front, in this stretch
        self.on_key = {}  # control -> (session id, state) its picture shows, and since when
        self.prompt_seen = {}  # session id -> (its prompt's last lines, when first seen)
        self.wake = False
        self.logged = {}
        self.work = collections.deque()  # (when queued, kind, action, control, params)
        self.presses = Presses(self.queue, schedule)
        if start_threads:
            for target in (self.poll_loop, self.work_loop):
                threading.Thread(target=target, daemon=True).start()

    # --- Sending ---

    def log(self, message, once=False):
        moment = self.clock()
        with self.lock:
            if once:
                last = self.logged.get(message)
                if last is not None and moment - last < LOG_REPEAT_SECONDS:
                    return
                self.logged[message] = moment
                if len(self.logged) > 64:
                    self.logged.pop(next(iter(self.logged)))
        self.send({"type": "log", "message": message[:1000]})

    def request(self, message, signature=None):
        with self.lock:
            message["id"] = self.next_id
            self.next_id += 1
            if message["type"] == "set_image":
                self.waiting[message["id"]] = (message["key"], signature)
        self.send(message)

    # --- What each key shows ---

    def sessions(self):
        """Every live session, with a "done" one brought to the front from a key shown idle, and a waiting one whose
        prompt has been off its terminal for a while shown working."""
        result = []
        with self.lock:
            seen = set(self.seen)
        for session in self.store.sessions():
            if session.state == "done" and (session.id, session.at) in seen:
                session = session._replace(state="idle")
            elif session.state == "waiting":
                with self.lock:
                    gone = self.gone.get(session.id)
                if gone and gone[0] == session.at and gone[1] >= PROMPT_GONE_LOOKS:
                    # declined from a key: Claude Code stops its turn, and no hook says so
                    session = session._replace(state="idle" if gone[2:] == ("declined",) else "working", detail="")
            result.append(session)
        return result

    def chosen(self, sessions, control, params):
        """(the session a key shows, how many others of its project need you)."""
        listed = ordered(sessions, project_of(params))
        if not listed:
            return None, 0
        shown = listed[0]
        with self.lock:
            pin = self.pins.get(control)
            if pin is not None and self.clock() >= pin[1]:
                self.pins.pop(control, None)
                pin = None
        if pin is not None:
            shown = next((s for s in listed if s.id == pin[0]), shown)
        others = sum(1 for s in listed if s is not shown and s.state in ("waiting", "done"))
        return shown, others

    # --- Messages from the app ---

    def handle(self, message):
        kind = message.get("type")
        if kind == "ping":
            self.send({"type": "pong"})
        elif kind == "invoke":
            self.invoke(message)
        elif kind == "event":
            if message.get("action") in ACTIONS:
                self.gesture(message.get("action"), message.get("gesture"), message.get("control"),
                             message.get("params"))
        elif kind == "check":
            self.check(message)
        elif kind == "appear":
            key = message.get("key")
            if message.get("action") in ACTIONS and key:
                with self.lock:
                    self.keys[key] = {"action": message["action"], "params": message.get("params") or {},
                                      "control": control_of(message.get("control")), "shown": None, "retries": 0}
                self.refresh()
        elif kind == "disappear":
            with self.lock:
                entry = self.keys.pop(message.get("key"), None)
            if entry is not None:
                self.presses.forget(entry["control"])
        elif kind == "reply":
            self.reply(message)

    def reply(self, message):
        refused = not message.get("ok")
        with self.lock:
            key, signature = self.waiting.pop(message.get("id"), (None, None))
            entry = self.keys.get(key)
            if entry is not None and entry["shown"] == signature:
                if not refused:
                    entry["retries"] = 0
                elif entry["retries"] < IMAGE_RETRIES:
                    entry["retries"] += 1
                    entry["shown"] = None
        if refused:
            self.log("a key image was refused: %s" % message.get("error", ""), once=True)

    def on_screen(self, where):
        with self.lock:
            return any(entry["control"] == where for entry in self.keys.values())

    def invoke(self, message):
        request = message.get("request")
        action = message.get("action")
        if action not in ACTIONS:
            self.send({"type": "result", "request": request, "ok": False, "error": "unknown action %s" % action})
            return
        self.send({"type": "result", "request": request, "ok": True})  # first: the release comes only after
        input = message.get("input")
        if not isinstance(input, dict):
            self.queue("short", (None, None), message.get("params") or {}, action)
            return
        self.gesture(action, input.get("gesture"), input.get("control"), message.get("params"))

    def gesture(self, action, gesture, control, params):
        where = control_of(control)
        params = dict(params or {}, _action=action)
        if gesture in ("pressed", "released") and where[0] == "m4315" and self.on_screen(where):
            if gesture == "pressed":
                self.presses.press(where, params)
            else:
                self.presses.release(where)
        elif gesture in ("pressed", "short_press"):
            self.queue("short", where, params)  # a press whose release may never come acts at once
        elif gesture == "long_press":
            self.queue("long", where, params)
        elif gesture in ("clockwise", "right", "down"):
            self.cycle(where, params, 1)
        elif gesture in ("counter_clockwise", "left", "up"):
            self.cycle(where, params, -1)

    def check(self, message):
        request = message.get("request")
        if message.get("condition") != "state":
            self.send({"type": "result", "request": request, "ok": False,
                       "error": "unknown condition %s" % message.get("condition")})
            return
        params = message.get("params") or {}
        state = params.get("state", "waiting")
        listed = ordered(self.sessions(), project_of(params))
        self.send({"type": "result", "request": request, "ok": True, "value": any(s.state == state for s in listed)})

    # --- Presses, one at a time ---

    def queue(self, kind, control, params, action=None):
        with self.lock:
            self.work.append((self.clock(), kind, action or params.get("_action", "session"), control, params))
            self.work_ready.notify()

    def take_job(self):
        """The oldest press that has not waited too long, dropping those that have: run late, behind a slow terminal
        (an Automation prompt, say), a press meant to bring the terminal forward could answer its prompt."""
        with self.lock:
            while True:
                while not self.work:
                    self.work_ready.wait()
                job = self.work.popleft()
                if self.clock() - job[0] <= JOB_SECONDS:
                    return job[1:]
                self.log("dropped a press that waited too long for the terminal", once=True)

    def work_loop(self):
        while True:
            self.run_job(*self.take_job())

    def run_job(self, kind, action, control, params):
        try:
            sessions = self.sessions()
            session, _ = self.chosen(sessions, control, params)
            with self.lock:
                drawn = self.on_key.get(control)
            # the session whose picture the key shows, when it is still there
            if drawn is not None:
                session = next((s for s in ordered(sessions, project_of(params)) if s.id == drawn[0]), session)
            if session is None:
                self.log("no Claude Code session%s is running with the hooks installed" %
                         (" of %s" % params.get("project") if project_of(params) else ""), once=True)
                return
            if action == "session" and answers(params) and session.state == "waiting":
                if self.answer(kind, session, control):
                    return
            if kind == "long":
                self.cycle(control, params, 1)
                return
            self.terminals.focus(session)
            if session.state == "done":
                with self.lock:
                    self.seen.add((session.id, session.at))
        except Exception as error:
            self.log(str(error)[:500], once=True)
        finally:
            self.refresh()

    def answer(self, kind, session, control):
        """Answers the session's permission prompt; True when the press is used up (answered, or refused with a log
        line), False when it should bring the terminal forward (or, long, show the next session) instead.

        Only what you have seen: the key has shown this session waiting, the prompt has been on its screen, and the
        terminal in front, each for FRONT_SECONDS; one of the session's processes owns that terminal; and the prompt
        is a permission's. The key is typed in the same step that checks the screen did not change."""
        if session.detail in NOT_PERMISSIONS:
            return False  # a question for you, not a permission: only ever answered in the terminal
        now = self.clock()
        with self.lock:
            drawn = self.on_key.get(control)
        if drawn is None or drawn[:2] != (session.id, "waiting") or now - drawn[2] < FRONT_SECONDS:
            return False  # the key did not show this prompt (yet): the press meant what the key showed
        look = self.terminals.look(session)
        if look is None or not look.front:
            self.note_front(session, False)
            return False
        since = self.note_front(session, True)
        if look.prompt is None:
            return False
        first_seen = self.note_prompt(session, look.seen)
        if not look.owned:
            self.log("another program than this Claude Code session runs in that terminal: answer it there")
            return True
        if now - since < FRONT_SECONDS or now - first_seen < FRONT_SECONDS:
            self.log("the prompt has only just come up or come to the front: read it, then press again")
            return True
        if kind == "short":
            if look.prompt != "yes":
                self.log("the prompt has another option chosen than 1. Yes: answer it in the terminal")
                return True
            self.terminals.key(session, "return", look.text)
            self.log("answered yes (once) to %s in %s" % (session.detail or "a permission", session.project))
        else:
            self.terminals.key(session, "escape", look.text)
            self.log("declined %s in %s" % (session.detail or "a permission", session.project))
        with self.lock:
            self.gone[session.id] = (session.at, PROMPT_GONE_LOOKS) + (() if kind == "short" else ("declined",))
        return True

    def note_prompt(self, session, seen):
        """Notes the prompt on the session's screen; returns since when this same prompt has been there."""
        with self.lock:
            if seen is None:
                self.prompt_seen.pop(session.id, None)
                return None
            known = self.prompt_seen.get(session.id)
            if known is None or known[0] != seen:
                known = self.prompt_seen[session.id] = (seen, self.clock())
            return known[1]

    def note_front(self, session, front):
        """Notes whether the session's terminal is in front; returns since when it has been, when it is."""
        with self.lock:
            if not front:
                self.front_since.pop(session.id, None)
                return None
            return self.front_since.setdefault(session.id, self.clock())

    def cycle(self, control, params, step):
        listed = ordered(self.sessions(), project_of(params))
        if not listed:
            return
        shown, _ = self.chosen(listed, control, params)
        place = next((i for i, s in enumerate(listed) if s.id == shown.id), 0)
        with self.lock:
            self.pins[control] = (listed[(place + step) % len(listed)].id, self.clock() + PIN_SECONDS)
        self.refresh()

    # --- Key images ---

    def refresh(self):
        with self.lock:
            self.wake = True
            self.poll_ready.notify()

    def watch_prompts(self, sessions):
        """Looks at the terminal of each waiting session the keys show: once its prompt is gone (answered in the
        terminal), the session is shown working until the next hook says otherwise."""
        for session in sessions:
            if session.state != "waiting" or Terminals.where(session) is None:
                continue
            try:
                look = self.terminals.look(session)
            except Exception as error:
                self.log("could not read the terminal: %s" % error, once=True)
                continue
            if look is None:
                continue
            self.note_front(session, look.front)
            self.note_prompt(session, look.seen)
            with self.lock:
                at, count, *declined = self.gone.get(session.id, (session.at, 0))
                if at != session.at or look.prompt:
                    count, declined = 0, []  # a new turn, or the prompt is back on screen
                self.gone[session.id] = (session.at, count + (not look.prompt), *declined)

    def poll_once(self):
        with self.lock:
            keys = [(key, dict(entry)) for key, entry in self.keys.items()]
        if not keys:
            return
        raw = self.store.sessions()
        with self.lock:  # forget what is known of sessions that are gone
            live = {s.id for s in raw}
            self.seen = {pair for pair in self.seen if pair[0] in live}
            for known in (self.gone, self.front_since, self.prompt_seen):
                for gone in [id for id in known if id not in live]:
                    del known[gone]
        shown = {}
        for _, entry in keys:
            session = self.chosen(raw, entry["control"], entry["params"])[0]
            if session is not None:
                shown[session.id] = session
        self.watch_prompts(list(shown.values()))
        sessions = self.sessions()
        for key, entry in keys:
            session, others = self.chosen(sessions, entry["control"], entry["params"])
            try:
                signature, image = self.artist.image(face_of(entry["action"], session, others,
                                                             dial=entry["control"][0] == DIAL_MODULE))
            except Exception as error:
                self.log("could not draw a key: %s" % error, once=True)
                continue
            with self.lock:
                current = self.keys.get(key)
                if current is None:
                    continue
                # what the key shows, and since when (once it is sent; a refused picture is sent again)
                showing = (session.id, session.state) if session is not None else None
                known = self.on_key.get(entry["control"])
                if showing is None:
                    self.on_key.pop(entry["control"], None)
                elif known is None or known[:2] != showing:
                    self.on_key[entry["control"]] = showing + (self.clock(),)
                if current["shown"] == signature:
                    continue
                if current["shown"] is not None:
                    current["retries"] = 0
                current["shown"] = signature
            self.request({"type": "set_image", "key": key, "image": image}, signature)

    def poll_loop(self):
        while True:
            with self.lock:
                if not self.wake:
                    self.poll_ready.wait(POLL_SECONDS if self.keys else None)
                self.wake = False
            try:
                self.poll_once()
            except Exception as error:
                self.log("could not update the keys: %s" % error, once=True)


def main():
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.connect(os.environ["OSH_PLUGIN_SOCKET"])
    lines = connection.makefile("r", encoding="utf-8")
    sending = threading.Lock()

    def send(message):
        with sending:
            connection.sendall((json.dumps(message) + "\n").encode("utf-8"))

    send({"type": "hello", "id": os.environ["OSH_PLUGIN_ID"], "token": os.environ["OSH_PLUGIN_TOKEN"]})
    welcome = json.loads(lines.readline() or "null")
    if not welcome or welcome.get("type") != "welcome":
        print("the app refused the connection:", welcome, file=sys.stderr)
        return 1

    data = os.environ.get("OSH_PLUGIN_DATA")
    if not data or not os.path.isdir(data):
        data = tempfile.mkdtemp(prefix="claudecode-")
        atexit.register(shutil.rmtree, data, True)
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))
    sessions = os.path.join(data, "sessions")
    drawn = os.path.join(data, "keys")
    for folder in (sessions, drawn):
        os.makedirs(folder, mode=0o700, exist_ok=True)
    plugin = Plugin(send, Store(sessions), Terminals(), Artist(drawn))
    for line in lines:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if isinstance(message, dict):
            try:
                plugin.handle(message)
            except Exception as error:
                plugin.log("could not handle a %s message: %s" % (message.get("type"), error), once=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
