"""Now Playing: an OpenStreamHub plugin for Apple Music and Spotify, using only the Python standard library.

Action "control" (bind it to an M4315 key's Press, with nothing on its Release):
- a short press toggles play/pause,
- a long press (held for `long_ms`, 500 ms by default) goes to the previous track,
- a double press (the second within `double_ms`, 300 ms by default) goes to the next track,
and the key shows the current track's cover, dimmed with a pause sign while paused. Short, long and double presses
are told apart only on a key the app says is on screen (`appear`), because only such a key is sure to send its
release. Anywhere else (a step of a multi-action or if / else, Built-in animation mode, a knob's press) a press
toggles at once. Turning a knob or the dial right (a roller down) goes to the next track and left (a roller up) to
the previous; without dial functions, a dial click toggles, a double click goes next and a long click goes back.

Condition "is_playing": whether the chosen player (or either) is playing.

The manifest sets "events": true, so the app also sends each key's release, and "image": true, so it says which
keys are on screen. A press is answered at once: the app sends the release only after that, and a single press is
known only once the double-press window has passed. What the player did, or why it could not, goes to the log in
Settings > Plugins.

Players are driven with AppleScript (music.applescript, spotify.applescript), so macOS asks once whether
OpenStreamHub may control Music or Spotify. A player is asked for its state only while it is running, so the plugin
never starts one, except that play/pause starts the chosen player (Apple Music for "Whichever is playing") when none
is running. Keys are drawn by render.js with AppKit, through osascript. Spotify's covers come from i.scdn.co.

Fields it does not know, in any message, are ignored: newer apps may add some.
"""

import atexit
import base64
import collections
import json
import os
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import threading
import time
import urllib.request

HERE = os.path.dirname(os.path.abspath(__file__))
OSASCRIPT = "/usr/bin/osascript"
PGREP = "/usr/bin/pgrep"
# The process name pgrep looks for and the script that drives the player.
PLAYERS = {"music": ("Music", "music.applescript"), "spotify": ("Spotify", "spotify.applescript")}
CHOICES = ("auto", "music", "spotify")
PLAYING = ("playing", "fast forwarding", "rewinding")  # Music reports seeking as its own states
MAX_IMAGE = 32 * 1024  # the app refuses a larger set_image
MAX_COVER = 4 * 1024 * 1024
COVER_HOST = "https://i.scdn.co/"  # the only host the manifest declares
POLL_SECONDS = 2.0
AFTER_COMMAND_SECONDS = 0.4  # players take a moment to report a new track
READ_SECONDS = 10.0
COMMAND_SECONDS = 5.0
TRANSPORT_MIN_SECONDS = 2.0  # what play/pause, next or previous keeps for itself after a slow read
COMMAND_EXPIRY_SECONDS = 3.0  # a press that waited longer than this behind a slow player is dropped
CHECK_SECONDS = 3.0  # what a check has at most, less a margin for the answer
CHECK_MARGIN_SECONDS = 0.3
COVER_RETRY_SECONDS = 2.0  # a track without a cover yet is asked again, for a while
COVER_GIVE_UP_SECONDS = 20.0
IMAGE_RETRIES = 3  # a refused set_image is sent again at most this often until the picture changes
LOG_REPEAT_SECONDS = 60.0

# One player's state. `player` is None when nothing is running; `track` tells tracks apart for the key image.
Now = collections.namedtuple("Now", "player state track title artist cover_url")
NOTHING = Now(None, "stopped", None, None, None, None)


def choice_of(params):
    choice = (params or {}).get("player", "auto")
    return choice if choice in CHOICES else "auto"


def milliseconds(params, name, default, low, high):
    value = (params or {}).get(name, default)
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        value = default
    return min(high, max(low, value)) / 1000.0


class PlayerError(Exception):
    pass


def time_left(deadline):
    left = deadline - time.monotonic()
    if left <= 0:
        raise PlayerError("the players did not answer in time")
    return left


class Players:
    """Reads and drives Apple Music and Spotify. `run` is subprocess.run, replaced in tests.

    Every call takes a `deadline` (time.monotonic()) that bounds all the programs it runs together.
    """

    def __init__(self, run=subprocess.run):
        self.run = run

    def running(self, player, deadline):
        try:
            done = self.run([PGREP, "-x", PLAYERS[player][0]], stdout=subprocess.DEVNULL,
                            stderr=subprocess.DEVNULL, timeout=time_left(deadline))
        except subprocess.TimeoutExpired:
            raise PlayerError("pgrep did not answer in time")
        return done.returncode == 0

    def script(self, player, deadline, *args):
        name = PLAYERS[player][0]
        try:
            done = self.run(
                [OSASCRIPT, os.path.join(HERE, PLAYERS[player][1])] + list(args),
                capture_output=True,
                text=True,
                timeout=time_left(deadline),
            )
        except subprocess.TimeoutExpired:
            raise PlayerError("%s did not answer in time" % name)
        if done.returncode != 0:
            raise PlayerError(explain(name, done.stderr))
        return done.stdout.rstrip("\r\n")  # not strip(): an empty last field (no artist) is a field

    def read(self, player, deadline):
        """The player's state, or None when it is not running."""
        if not self.running(player, deadline):
            return None
        fields = self.script(player, deadline, "state").split("\t")
        if fields[0] == "not_running":
            return None
        if fields[0] not in PLAYING + ("paused",) or len(fields) < 4:
            return Now(player, "stopped", None, None, None, None)
        state = "paused" if fields[0] == "paused" else "playing"
        # Spotify puts the cover's address last; a tab in a name cannot move it.
        last = fields[-1]
        if last.startswith("http://"):
            last = "https://" + last[len("http://"):]
        cover_url = last if len(fields) > 4 and last.startswith("https://") else None
        return Now(player, state, fields[1], fields[2], fields[3], cover_url)

    def resolve(self, choice, deadline=None, skipped=None):
        """The state of the chosen player; for "auto", the one playing, else one paused, else one running.

        For "auto", a player that cannot be read (not allowed, not answering) is left out and its error kept in
        `skipped` (a list, when given); when no player could be read, the error stops the read.
        """
        if deadline is None:
            deadline = time.monotonic() + READ_SECONDS
        if choice != "auto":
            return self.read(choice, deadline) or Now(None, "stopped", None, None, None, None)
        found, errors = [], []
        for player in ("music", "spotify"):
            try:
                now = self.read(player, deadline)
            except PlayerError as error:
                errors.append(error)
                continue
            if now is not None:
                found.append(now)
        if errors and not found:
            raise errors[0]
        if skipped is not None:
            skipped.extend(str(error) for error in errors)
        for state in ("playing", "paused"):
            for now in found:
                if now.state == state:
                    return now
        return found[0] if found else NOTHING

    def command(self, choice, verb):
        """Runs play/pause, next or previous on the chosen player. Returns what to log."""
        deadline = time.monotonic() + COMMAND_SECONDS
        now = self.resolve(choice, deadline)
        player = now.player
        if player is None:
            if verb != "playpause":
                return "%s: nothing is playing" % verb
            player = "music" if choice == "auto" else choice
        self.script(player, max(deadline, time.monotonic() + TRANSPORT_MIN_SECONDS), verb)
        return None

    def cover(self, now, path):
        """Writes the current track's cover to `path`; False when it has none."""
        if now.player == "music":
            return self.script("music", time.monotonic() + READ_SECONDS, "artwork", path) == "ok"
        if now.player == "spotify" and now.cover_url and now.cover_url.startswith(COVER_HOST):
            with COVER_OPENER.open(now.cover_url, timeout=5) as response:
                data = response.read(MAX_COVER + 1)
            if len(data) > MAX_COVER:
                return False
            with open(path, "wb") as file:
                file.write(data)
            return True
        return False


class SameHostRedirects(urllib.request.HTTPRedirectHandler):
    """Follows a redirect only within the cover host, the one the manifest declares."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        if not newurl.startswith(COVER_HOST):
            raise PlayerError("the cover moved off %s" % COVER_HOST)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


COVER_OPENER = urllib.request.build_opener(SameHostRedirects)


def explain(name, stderr):
    text = " ".join((stderr or "").split())
    if "-1743" in text:
        return "macOS does not allow OpenStreamHub to control %s: allow it in System Settings > Privacy & " \
               "Security > Automation" % name
    if "-1728" in text and "application id" in text:
        return "%s is not installed" % name
    if "-600" in text or "isn't running" in text or "not running" in text:
        return "%s is not running" % name
    return "%s: %s" % (name, text[-300:] or "the script failed")


class Artist:
    """Draws key images with render.js, keeping the last few by player, state, track and cover."""

    def __init__(self, players, folder, run=subprocess.run):
        self.players = players
        self.folder = folder
        self.run = run
        self.cache = collections.OrderedDict()
        # Per player, the track whose cover is in its file: {"track", "found", "first", "last"} (times).
        self.covered = {}
        self.log = lambda message: None  # the plugin sets it, to log why a cover could not be had

    def cover(self, now):
        """The path of the track's cover, or "" when it has none (yet: a new track is asked again for a while)."""
        path = os.path.join(self.folder, "cover-%s" % now.player)
        moment = time.monotonic()
        entry = self.covered.get(now.player)
        if entry is None or entry["track"] != now.track:
            entry = self.covered[now.player] = {"track": now.track, "found": False, "first": moment, "last": None}
        due = entry["last"] is None or (
            moment - entry["last"] >= COVER_RETRY_SECONDS and moment - entry["first"] < COVER_GIVE_UP_SECONDS)
        if not entry["found"] and due:
            entry["last"] = moment
            try:
                entry["found"] = bool(self.players.cover(now, path))
            except Exception as error:  # No cover is not a failure: the key shows a sign instead.
                entry["found"] = False
                self.log("could not get the %s cover: %s" % (now.player, str(error)[:300]))
        return path if entry["found"] else ""

    def image(self, now):
        """The key's (signature, base64 PNG). The signature changes whenever the picture would."""
        cover = self.cover(now) if now.state != "stopped" and now.track else ""
        signature = (now.player, now.state, now.track, bool(cover))
        if signature in self.cache:
            self.cache.move_to_end(signature)
            return signature, self.cache[signature]
        out = os.path.join(self.folder, "key.png")
        done = self.run(
            [OSASCRIPT, "-l", "JavaScript", os.path.join(HERE, "render.js"), out, now.state, cover],
            capture_output=True,
            text=True,
            timeout=READ_SECONDS,
        )
        if done.returncode != 0:
            raise PlayerError("drawing the key failed: " + " ".join(done.stderr.split())[-300:])
        with open(out, "rb") as file:
            data = file.read()
        if len(data) > MAX_IMAGE:
            raise PlayerError("the key image is %d bytes, more than %d" % (len(data), MAX_IMAGE))
        image = base64.b64encode(data).decode("ascii")
        self.cache[signature] = image
        while len(self.cache) > 8:
            self.cache.popitem(last=False)
        return signature, image


class Gestures:
    """Tells a short, long and double press apart, per control.

    `act(verb, choice)` runs "playpause", "next" or "previous". `schedule(seconds, callback)` returns an object with
    cancel(); it is threading.Timer in the plugin and a fake clock in tests. Every callback runs under the lock and
    checks that the control is still in the state that scheduled it.
    """

    def __init__(self, act, schedule):
        self.act = act
        self.schedule = schedule
        self.lock = threading.Lock()
        self.controls = {}

    def _state(self, control):
        return self.controls.setdefault(control, {"down": False, "used": False, "long": None, "tap": None,
                                                  "params": {}})

    def press(self, control, params):
        with self.lock:
            state = self._state(control)
            state["params"] = params or {}
            if state["tap"] is not None:  # the second press of a double press
                state["tap"].cancel()
                state["tap"] = None
                state["down"], state["used"] = True, True
                self.act("next", choice_of(params))
                return
            if state["long"] is not None:
                state["long"].cancel()
                state["long"] = None
            if state["down"] or state.get("no_release"):
                # The last press never sent its release (the key's Release runs something else): this key cannot
                # tell a long press, so it plays or pauses at once until a release comes.
                state["no_release"], state["down"], state["used"] = True, True, True
                self.act("playpause", choice_of(params))
                return
            state["down"], state["used"] = True, False
            seconds = milliseconds(params, "long_ms", 500, 200, 3000)
            state["long"] = self.schedule(seconds, lambda: self._long(control, state))

    def _long(self, control, state):
        with self.lock:
            if self.controls.get(control) is not state or not state["down"] or state["used"]:
                return
            state["long"], state["used"] = None, True
            self.act("previous", choice_of(state["params"]))

    def release(self, control):
        with self.lock:
            state = self.controls.get(control)
            if state is None or not state["down"]:
                return  # a release without its press, such as one pressed in a Test run
            state["down"], state["no_release"] = False, False
            if state["long"] is not None:
                state["long"].cancel()
                state["long"] = None
            if state["used"]:
                return
            self._tapped(control, state)

    def forget(self, control):
        """The key left the screen: a key held now may never send its release, so it cannot become a long press.
        A press already made and waiting for a second one still counts."""
        with self.lock:
            state = self.controls.get(control)
            if state is None:
                return
            if state["long"] is not None:
                state["long"].cancel()
                state["long"] = None
            state["down"], state["no_release"] = False, False

    def tap(self, control, params):
        """A whole click, as the dial reports it: a press and a release in one."""
        with self.lock:
            state = self._state(control)
            state["params"] = params or {}
            if state["tap"] is not None:
                state["tap"].cancel()
                state["tap"] = None
                self.act("next", choice_of(params))
                return
            self._tapped(control, state)

    def _tapped(self, control, state):
        seconds = milliseconds(state["params"], "double_ms", 300, 0, 1000)
        if seconds == 0:
            self.act("playpause", choice_of(state["params"]))
            return
        state["tap"] = self.schedule(seconds, lambda: self._single(control, state))

    def _single(self, control, state):
        with self.lock:
            if self.controls.get(control) is not state or state["tap"] is None:
                return
            state["tap"] = None
            self.act("playpause", choice_of(state["params"]))


def timer(seconds, callback):
    handle = threading.Timer(seconds, callback)
    handle.daemon = True
    handle.start()
    return handle


TURNS = {"clockwise": "next", "right": "next", "down": "next",
         "counter_clockwise": "previous", "left": "previous", "up": "previous"}


def control_of(control):
    control = control if isinstance(control, dict) else {}
    return control.get("module"), control.get("index")


class Plugin:
    """The plugin's state: keys on screen, the images sent to them, and what to do with each message."""

    def __init__(self, send, players, artist, schedule=timer, start_threads=True, clock=time.monotonic):
        self.send = send
        self.players = players
        self.artist = artist
        if isinstance(artist, Artist):
            artist.log = lambda message: self.log(message, once=True)
        self.clock = clock
        self.lock = threading.RLock()
        self.poll_ready = threading.Condition(self.lock)
        self.command_ready = threading.Condition(self.lock)
        self.next_id = 1
        # key id -> {"choice", "control": (module, index), "shown": signature or None, "retries"}
        self.keys = {}
        self.waiting = {}  # set_image id -> (key id, signature)
        self.wake = False
        self.logged = {}  # message -> when it was last logged
        self.commands = collections.deque()  # (when queued, verb, choice)
        self.gestures = Gestures(self.queue_command, schedule)
        self.latest = {}  # choice -> (time, Now)
        if start_threads:
            for target in (self.poll_loop, self.command_loop):
                threading.Thread(target=target, daemon=True).start()

    # --- Sending ---

    def log(self, message, once=False):
        """Sends a log line. With `once`, the same line is sent again only after a minute."""
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

    # --- Messages from the app ---

    def handle(self, message):
        kind = message.get("type")
        if kind == "ping":
            self.send({"type": "pong"})
        elif kind == "invoke":
            self.invoke(message)
        elif kind == "event":
            if message.get("action") == "control":
                self.gesture(message.get("gesture"), message.get("control"), message.get("params"))
        elif kind == "check":
            threading.Thread(target=self.check, args=(message,), daemon=True).start()
        elif kind == "appear":
            key = message.get("key")
            if message.get("action") == "control" and key:
                with self.lock:
                    self.keys[key] = {"choice": choice_of(message.get("params")), "shown": None, "retries": 0,
                                      "control": control_of(message.get("control"))}
                self.refresh()
        elif kind == "disappear":
            with self.lock:
                entry = self.keys.pop(message.get("key"), None)
            if entry is not None:
                self.gestures.forget(entry["control"])
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
                    entry["shown"] = None  # try again at the next poll
        if refused:
            self.log("a key image was refused: %s" % message.get("error", ""), once=True)

    def on_screen(self, where):
        """Whether a key the app said is on screen is at this control, so its release will come."""
        with self.lock:
            return any(entry["control"] == where for entry in self.keys.values())

    def invoke(self, message):
        request = message.get("request")
        if message.get("action") != "control":
            self.send({"type": "result", "request": request, "ok": False,
                       "error": "unknown action %s" % message.get("action")})
            return
        # Answer first: the app sends the release only once the press is answered.
        self.send({"type": "result", "request": request, "ok": True})
        input = message.get("input")
        if not isinstance(input, dict):
            self.queue_command("playpause", choice_of(message.get("params")))
            return
        self.gesture(input.get("gesture"), input.get("control"), message.get("params"))

    def gesture(self, gesture, control, params):
        where = control_of(control)
        dial = where[0] == "m131e"
        if gesture in ("pressed", "released") and not dial:
            if not self.on_screen(where):
                # Not a key the app draws for this action (a step of a sequence, Built-in animation, a knob's
                # press): its release may never come, so a press acts at once and a release is ignored.
                if gesture == "pressed":
                    self.queue_command("playpause", choice_of(params))
            elif gesture == "pressed":
                self.gestures.press(where, params)
            else:
                self.gestures.release(where)
        elif gesture == "short_press":
            self.gestures.tap(where, params)
        elif gesture == "long_press":
            self.queue_command("previous", choice_of(params))
        elif gesture in TURNS:
            self.queue_command(TURNS[gesture], choice_of(params))

    def check(self, message):
        request = message.get("request")
        try:
            if message.get("condition") != "is_playing":
                raise PlayerError("unknown condition %s" % message.get("condition"))
            budget = CHECK_SECONDS
            timeout_ms = message.get("timeout_ms")
            if isinstance(timeout_ms, (int, float)) and not isinstance(timeout_ms, bool):
                budget = min(budget, timeout_ms / 1000.0)
            choice = choice_of(message.get("params"))
            with self.lock:
                fresh = self.latest.get(choice)
            if fresh and self.clock() - fresh[0] < 1.0:
                now = fresh[1]
            else:
                now = self.players.resolve(choice, time.monotonic() + budget - CHECK_MARGIN_SECONDS)
            self.send({"type": "result", "request": request, "ok": True, "value": now.state == "playing"})
        except Exception as error:  # A failed check fails the press, and neither branch runs.
            self.send({"type": "result", "request": request, "ok": False, "error": str(error)[:300]})

    # --- Commands, one at a time ---

    def queue_command(self, verb, choice):
        with self.lock:
            self.commands.append((self.clock(), verb, choice))
            self.command_ready.notify()

    def next_command(self, block=True):
        """The oldest command that has not waited too long, dropping (and logging) those that have."""
        dropped = 0
        with self.lock:
            while True:
                while not self.commands:
                    if not block:
                        return None
                    self.command_ready.wait()
                queued, verb, choice = self.commands.popleft()
                if self.clock() - queued <= COMMAND_EXPIRY_SECONDS:
                    break
                dropped += 1
        if dropped:
            self.log("dropped %d press(es) that waited too long for the player" % dropped)
        return verb, choice

    def run_command(self, verb, choice):
        try:
            with self.lock:
                self.latest.clear()  # a check must not answer from a reading made before this command
            note = self.players.command(choice, verb)
            if note:
                self.log(note)
        except Exception as error:
            self.log("%s failed: %s" % (verb, error))
        time.sleep(AFTER_COMMAND_SECONDS)
        self.refresh()

    def command_loop(self):
        while True:
            self.run_command(*self.next_command())

    # --- Key images ---

    def refresh(self):
        with self.lock:
            self.wake = True
            self.poll_ready.notify()

    def poll_once(self):
        with self.lock:
            choices = sorted({key["choice"] for key in self.keys.values()})
        for choice in choices:
            try:
                skipped = []
                now = self.players.resolve(choice, skipped=skipped)
                with self.lock:
                    self.latest[choice] = (self.clock(), now)
                signature, image = self.artist.image(now)
            except Exception as error:
                self.log("could not read the player: %s" % error, once=True)
                continue
            for reason in skipped:
                self.log("left out of Whichever is playing: %s" % reason, once=True)
            with self.lock:
                due = [key for key, entry in self.keys.items()
                       if entry["choice"] == choice and entry["shown"] != signature]
                for key in due:
                    if self.keys[key]["shown"] is not None:  # a new picture, not a retry of a refused one
                        self.keys[key]["retries"] = 0
                    self.keys[key]["shown"] = signature
            for key in due:
                self.request({"type": "set_image", "key": key, "image": image}, signature)

    def poll_loop(self):
        while True:
            with self.lock:
                if not self.wake:
                    self.poll_ready.wait(POLL_SECONDS if self.keys else None)
                self.wake = False
            self.poll_once()


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
    if data and os.path.isdir(data):
        folder = os.path.join(data, "covers")
        os.makedirs(folder, mode=0o700, exist_ok=True)
    else:
        folder = tempfile.mkdtemp(prefix="nowplaying-")
        atexit.register(shutil.rmtree, folder, True)
        signal.signal(signal.SIGTERM, lambda *_: sys.exit(0))  # the app stops plugins with SIGTERM
    players = Players()
    plugin = Plugin(send, players, Artist(players, folder))
    for line in lines:
        try:
            message = json.loads(line)
        except ValueError:
            continue
        if isinstance(message, dict):
            try:
                plugin.handle(message)
            except Exception as error:  # one odd message must not stop the plugin
                plugin.log("could not handle a %s message: %s" % (message.get("type"), error), once=True)
    return 0


if __name__ == "__main__":
    sys.exit(main())
