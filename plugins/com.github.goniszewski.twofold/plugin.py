"""Twofold: a sliding, merging number game on the M4315's 15 keys, an OpenStreamHub plugin using only the standard
library.

Action "tile" (bind it to each M4315 key's Press, with nothing on its Release): the key shows one cell of a board of
3 rows of 5, the cell where the key sits (wire index = row x 5 + column).
- Hold any key for a second: the middle row shows the score and the best score while you hold it.
- Press any key twice quickly: the middle row asks "NEW GAME?"; press YES to start again, anything else (or a move) to
  go on.
A move puts the board back from any message but "GAME OVER".
- When the game is over the middle row says so; a press starts a new game.

Action "move" with a direction (bind it to each roller's turns, for example: the first roller's up and down to up and
down, the second's to left and right): one move each time it runs.

The game is kept in the plugin's data folder, so it survives restarts. Pictures are drawn by picture.py; only keys
whose picture changed are sent, at most MAX_IMAGES_PER_SECOND a second (what the limit holds back goes out a moment
later). Everything that decides or sends a picture runs under one lock, so a key never ends up with an older one.

Fields it does not know, in any message, are ignored: newer apps may add some.
"""

import base64
import collections
import json
import os
import random
import socket
import sys
import threading
import time

import game
import picture

TILE, MOVE = "tile", "move"
KEYPAD = "m4315"
DOUBLE_SECONDS = 0.4
HOLD_SECONDS = 1.0
AFTER_RELEASE_SECONDS = 1.5  # the score stays this long after the key that showed it is let go
CONFIRM_SECONDS = 5.0
WON_SECONDS = 3.0
OVER_GRACE_SECONDS = 1.0  # a press this soon after the game ended does not start a new one
MAX_IMAGES_PER_SECOND = 15  # well under the app's 20: it counts when it reads them, which may be later
RATE_WINDOW_SECONDS = 1.0
CONFIRM_GRACE_SECONDS = 0.6  # a press this soon after "NEW GAME?" came up does not answer it
SCORE_MAX_SECONDS = 10.0  # the score goes away by itself, even if a key's release never comes
IMAGE_RETRIES = 3
LOG_REPEAT_SECONDS = 60.0
# The middle row, where messages go.
MIDDLE = [game.COLUMNS + column for column in range(game.COLUMNS)]
YES_CELL = MIDDLE[2]


def timer(seconds, callback):
    timing = threading.Timer(seconds, callback)
    timing.daemon = True
    timing.start()
    return timing


def message_faces(kind, score, best):
    """The middle row's words for a message, by cell: the whole row, so the message reads as one band over the board,
    which stays as it is (only five keys change)."""
    words = {
        "score": ["SCORE", str(score), None, "BEST", str(best)],
        "confirm": ["NEW", "GAME?", "YES", "NO", None],
        "over": [None, "GAME", "OVER", str(score), None],
        "won": [None, "YOU", "MADE", str(game.GOAL), None],
    }[kind]
    faces = {}
    for cell, word in zip(MIDDLE, words):
        word = word or ""
        tone = "yes" if kind == "confirm" and cell == YES_CELL else "number" if word.isdigit() else "plain"
        faces[cell] = ("word", word, tone)
    return faces


class Store:
    """The saved game, in one JSON file (none without a data folder)."""

    def __init__(self, path):
        self.path = path

    def load(self):
        if not self.path:
            return None
        try:
            with open(self.path, encoding="utf-8") as file:
                return json.load(file)
        except (OSError, ValueError):
            return None

    def save(self, value):
        if not self.path:
            return
        temporary = "%s.%d.tmp" % (self.path, os.getpid())
        with open(temporary, "w", encoding="utf-8") as file:
            json.dump(value, file)
            file.flush()
            os.fsync(file.fileno())  # so a power loss leaves the old file or the new one, never an empty one
        os.replace(temporary, self.path)


class Painter:
    """Sends each key the picture it should show, only when that changed, at most MAX_IMAGES_PER_SECOND a second.
    A picture the limit holds back is sent once a slot frees; a newer one replaces it. Runs under the plugin's lock,
    sends included, so pictures for one key go out in the order they were decided."""

    def __init__(self, lock, send_image, clock, schedule, draw=picture.draw):
        self.lock = lock
        self.send_image = send_image  # (key, image) -> the request id
        self.clock = clock
        self.schedule = schedule
        self.draw = draw
        self.wanted = {}  # key -> face
        self.sent = {}  # key -> (face, request id) sent last
        self.retries = {}  # key -> refusals of the face sent
        self.order = []  # keys, in the order they came
        self.times = collections.deque()  # when the last pictures were sent
        self.cache = collections.OrderedDict()
        self.waiting = None  # the timer that sends what the limit held back

    def want(self, key, face):
        with self.lock:
            if key not in self.wanted:
                self.order.append(key)
            self.wanted[key] = face
            self.flush()

    def forget(self, key):
        with self.lock:
            self.wanted.pop(key, None)
            self.sent.pop(key, None)
            self.retries.pop(key, None)
            if key in self.order:
                self.order.remove(key)

    def image(self, face):
        if face not in self.cache:
            self.cache[face] = base64.b64encode(self.draw(face)).decode("ascii")
            while len(self.cache) > 96:
                self.cache.popitem(last=False)
        self.cache.move_to_end(face)
        return self.cache[face]

    def answered(self, key, number, ok, error=""):
        """The app's reply to request `number` for `key`. Only the reply to what was sent last counts. A picture held
        back by the app's rate limit is sent again later, however often; any other refusal a few times."""
        with self.lock:
            if self.sent.get(key, (None, None))[1] != number:
                return
            if ok:
                self.retries.pop(key, None)
                return
            if "a second" not in error:
                count = self.retries.get(key, 0)
                if count >= IMAGE_RETRIES:
                    return
                self.retries[key] = count + 1
            del self.sent[key]
            self.defer(RATE_WINDOW_SECONDS)

    def defer(self, seconds):
        if self.waiting is None:
            self.waiting = self.schedule(max(0.01, seconds), self._later)

    def flush(self):
        with self.lock:
            now = self.clock()
            while self.times and now - self.times[0] >= RATE_WINDOW_SECONDS:
                self.times.popleft()
            for key in list(self.order):
                face = self.wanted[key]
                if self.sent.get(key, (None,))[0] == face:
                    continue
                if len(self.times) >= MAX_IMAGES_PER_SECOND:
                    self.defer(RATE_WINDOW_SECONDS - (now - self.times[0]))
                    break
                image = self.image(face)
                self.times.append(now)
                if self.sent.get(key, (None,))[0] is not None:
                    self.retries.pop(key, None)  # a new picture, not a retry
                self.sent[key] = (face, self.send_image(key, image))

    def _later(self):
        with self.lock:
            self.waiting = None
            self.flush()


class Plugin:
    def __init__(self, send, store, rng=None, schedule=timer, clock=time.monotonic, draw=picture.draw):
        self.send = send
        self.store = store
        self.schedule = schedule
        self.clock = clock
        self.lock = threading.RLock()
        self.next_id = 1
        self.requests = {}  # set_image id -> key
        self.logged = {}
        self.cells = {}  # key id -> cell index
        self.game = game.Game(rng or random.Random())
        if not self.game.restore(store.load()):
            self.save()
        self.message = None  # "score", "confirm", "won" or None
        self.message_since = None
        self.message_timer = None
        self.over_since = self.clock() if self.game.over else None
        self.last_press = None
        self.holding = None  # (control, timer) of a press that may become a hold
        self.held = None  # the control whose hold shows the score
        self.painter = Painter(self.lock, self.send_image, clock, schedule, draw)

    # --- Sending ---

    def log(self, text, once=False):
        moment = self.clock()
        with self.lock:
            if once:
                last = self.logged.get(text)
                if last is not None and moment - last < LOG_REPEAT_SECONDS:
                    return
                self.logged[text] = moment
        self.send({"type": "log", "message": text[:1000]})

    def send_image(self, key, image):
        with self.lock:
            number = self.next_id
            self.next_id += 1
            self.requests[number] = key
            self.send({"type": "set_image", "id": number, "key": key, "image": image})
            return number

    def save(self):
        try:
            self.store.save(self.game.to_dict())
        except OSError as error:
            self.log("could not save the game: %s" % error, once=True)

    # --- What the keys show ---

    def faces(self):
        """The face of every cell now. "2048" is shown even when that move also ended the game."""
        board = self.game.board
        kind = self.message if self.message == "won" else "over" if self.game.over else self.message
        overlay = message_faces(kind, self.game.score, self.game.best) if kind else {}
        return [overlay.get(cell, ("tile", board[cell])) for cell in range(game.CELLS)]

    def redraw(self):
        with self.lock:
            faces = self.faces()
            for key, cell in self.cells.items():
                self.painter.want(key, faces[cell])

    def show(self, kind, seconds=None):
        """Shows a message, for `seconds` or until something else replaces it."""
        with self.lock:
            if self.message_timer is not None:
                self.message_timer.cancel()
                self.message_timer = None
            self.message = kind
            self.message_since = self.clock()
            if kind is not None and seconds is not None:
                timing = []
                timing.append(self.schedule(seconds, lambda: self._expire(kind, timing)))
                self.message_timer = timing[0]
            self.redraw()

    def _expire(self, kind, timing):
        with self.lock:
            if self.message != kind or self.message_timer is not timing[0]:
                return
            self.message_timer = None
            if kind != "score" and self.held is not None:
                self.show("score", SCORE_MAX_SECONDS)  # back to the score while its key is still held
            else:
                self.held = None
                self.message = None
                self.redraw()

    # --- Messages from the app ---

    def handle(self, message):
        kind = message.get("type")
        if kind == "ping":
            self.send({"type": "pong"})
        elif kind == "invoke":
            self.invoke(message)
        elif kind == "event":
            if message.get("action") == TILE and message.get("gesture") == "released":
                self.release(control_of(message.get("control")))
        elif kind == "check":
            self.send({"type": "result", "request": message.get("request"), "ok": False,
                       "error": "Twofold has no conditions"})
        elif kind == "appear":
            module, index = control_of(message.get("control"))
            key = message.get("key")
            if message.get("action") == TILE and key and module == KEYPAD and \
                    isinstance(index, int) and not isinstance(index, bool) and 0 <= index < game.CELLS:
                with self.lock:
                    self.cells[key] = index
                    self.redraw()
        elif kind == "disappear":
            key = message.get("key")
            with self.lock:
                cell = self.cells.pop(key, None)
                self.painter.forget(key)
                if cell is not None and (KEYPAD, cell) in (self.held, (self.holding or (None,))[0]):
                    self.let_go()  # its release will not come
        elif kind == "reply":
            with self.lock:
                key = self.requests.pop(message.get("id"), None)
                if key is None:
                    return
                ok = bool(message.get("ok"))
                self.painter.answered(key, message.get("id"), ok, str(message.get("error", "")))
            if not ok:
                self.log("a key picture was refused: %s" % message.get("error", ""), once=True)

    def invoke(self, message):
        request, action = message.get("request"), message.get("action")
        if action not in (TILE, MOVE):
            self.send({"type": "result", "request": request, "ok": False, "error": "unknown action %s" % action})
            return
        # Answered first: the app sends a key's release only once its press is answered.
        self.send({"type": "result", "request": request, "ok": True})
        params = message.get("params") if isinstance(message.get("params"), dict) else {}
        if action == MOVE:
            self.move(params.get("direction"))
            return
        source = message.get("input") if isinstance(message.get("input"), dict) else {}
        if source.get("gesture") == "pressed":
            self.press(control_of(source.get("control")))

    def on_screen(self, control):
        module, index = control
        return module == KEYPAD and index in self.cells.values()

    # --- Playing ---

    def let_go(self):
        """Forgets a hold, and the score it shows."""
        if self.holding is not None:
            self.holding[1].cancel()
            self.holding = None
        self.held = None
        if self.message == "score":
            self.show(None)

    def move(self, direction):
        if direction not in game.DIRECTIONS:
            self.log("a move needs a direction: up, down, left or right", once=True)
            return
        with self.lock:
            if self.message == "confirm":
                self.show(None)  # a move means: go on
                return
            if self.game.over:
                return
            outcome = self.game.play(direction)
            if not outcome:
                return
            self.save()
            if self.game.over:
                self.over_since = self.clock()
            # The score would hide the cells this move changed.
            if self.holding is not None:
                self.holding[1].cancel()
                self.holding = None
            self.held = None
            if outcome == "won":
                self.show("won", WON_SECONDS)
            else:
                self.show(None)

    def new_game(self):
        with self.lock:
            self.game.new()
            self.over_since = None
            self.save()
            self.show(None)

    def press(self, control):
        now = self.clock()
        with self.lock:
            if self.message == "confirm":
                if now - self.message_since < CONFIRM_GRACE_SECONDS:
                    return  # a quick third tap, before the question could be read
                self.last_press = None
                if control == (KEYPAD, YES_CELL):
                    self.new_game()
                else:
                    self.show(None)
                return
            if self.game.over and self.message != "won":
                if self.over_since is not None and now - self.over_since >= OVER_GRACE_SECONDS:
                    self.new_game()
                return
            if self.held is not None and self.held != control:
                self.let_go()  # another key: the hold that showed the score is over
            double = self.last_press is not None and now - self.last_press <= DOUBLE_SECONDS
            self.last_press = None if double else now
            if self.holding is not None:
                self.holding[1].cancel()
                self.holding = None
            if double:
                self.show("confirm", CONFIRM_SECONDS)
                return
            if self.on_screen(control):
                self.holding = (control, self.schedule(HOLD_SECONDS, lambda: self._hold(control)))
            if self.message == "won":
                self.show(None)

    def _hold(self, control):
        with self.lock:
            if self.holding is None or self.holding[0] != control:
                return
            self.holding = None
            self.held = control
            self.last_press = None
            self.show("score", SCORE_MAX_SECONDS)

    def release(self, control):
        with self.lock:
            if self.holding is not None and self.holding[0] == control:
                self.holding[1].cancel()
                self.holding = None
            if self.held == control:
                self.held = None
                if self.message == "score":
                    self.show("score", AFTER_RELEASE_SECONDS)


def control_of(control):
    control = control if isinstance(control, dict) else {}
    return control.get("module"), control.get("index")


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
    path = os.path.join(data, "game.json") if data and os.path.isdir(data) else None
    plugin = Plugin(send, Store(path))
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
