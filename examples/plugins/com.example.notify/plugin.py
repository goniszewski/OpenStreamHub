"""An example OpenStreamHub plugin, using only the Python standard library.

The app starts it with OSH_PLUGIN_SOCKET, OSH_PLUGIN_ID and OSH_PLUGIN_TOKEN in the environment. It connects,
says hello, then answers each invoke and each check with one result. See docs/plugins.md for the protocol.

Actions:
- notify: shows a macOS notification with `title` and `message` (the message may use {{variables}}).
- count: adds one to a counter and writes it to the profile variable `plugin_presses`, which the profile must
  declare as a number.
- hold: bind it to a key's press. Its manifest sets "events": true, so the app also sends the key's release as an
  `event`, and the plugin logs how long the key was held (Settings > Plugins shows the log).
- counter: counts its presses and draws the count on its keys. Its manifest sets "image": true, so the app says
  which of its keys are on screen (`appear`, `disappear`), and the plugin answers each with a `set_image`: a 72x72
  PNG it makes itself with zlib and struct, from a tiny built-in digit font.

Conditions (an if / else asks them as a `check` when its key is pressed, and the answer carries `value`):
- pressed_at_least: true once the `count` action's counter has reached the number `count`. It only reads the
  counter and answers at once, as a check should.

Fields it does not know, in any message, are ignored: newer apps may add some.
"""

import base64
import json
import os
import socket
import struct
import subprocess
import sys
import time
import zlib

# Text arrives in argv, never inside the script, so a message cannot inject AppleScript.
NOTIFY = 'on run argv\ndisplay notification (item 2 of argv) with title (item 1 of argv)\nend run'


# Digits 3 cells wide and 5 high, "1" for a lit cell.
DIGITS = {
    "0": ("111", "101", "101", "101", "111"),
    "1": ("010", "110", "010", "010", "111"),
    "2": ("111", "001", "111", "100", "111"),
    "3": ("111", "001", "111", "001", "111"),
    "4": ("101", "101", "111", "001", "001"),
    "5": ("111", "100", "111", "001", "111"),
    "6": ("111", "100", "111", "101", "111"),
    "7": ("111", "001", "001", "001", "001"),
    "8": ("111", "101", "111", "101", "111"),
    "9": ("111", "101", "111", "001", "111"),
}
SIDE = 72
BACKGROUND, INK = (20, 40, 90), (255, 255, 255)


def png(rows):
    """A PNG of RGB rows (each a bytes of 3 bytes per pixel): signature, header, pixels, end."""
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    header = struct.pack(">IIBBBBB", len(rows[0]) // 3, len(rows), 8, 2, 0, 0, 0)  # 8-bit RGB
    pixels = zlib.compress(b"".join(b"\x00" + row for row in rows))  # filter 0 on every row
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", pixels) + chunk(b"IEND", b"")


def number_image(number):
    """A 72x72 PNG of `number` in white on blue, as large as fits."""
    text = str(number)
    cells = 4 * len(text) - 1  # each digit 3 cells wide, 1 cell between digits
    scale = max(1, min(8, (SIDE - 8) // cells))
    left, top = (SIDE - cells * scale) // 2, (SIDE - 5 * scale) // 2
    rows = []
    for y in range(SIDE):
        row = bytearray(bytes(BACKGROUND) * SIDE)
        cell_y = (y - top) // scale
        if 0 <= y - top < 5 * scale:
            for x in range(SIDE):
                cell_x = (x - left) // scale
                digit, column = divmod(cell_x, 4)
                if 0 <= x - left and digit < len(text) and column < 3 and DIGITS[text[digit]][cell_y][column] == "1":
                    row[3 * x:3 * x + 3] = bytes(INK)
        rows.append(bytes(row))
    return base64.b64encode(png(rows)).decode("ascii")


def main():
    connection = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    connection.connect(os.environ["OSH_PLUGIN_SOCKET"])
    lines = connection.makefile("r", encoding="utf-8")

    def send(message):
        connection.sendall((json.dumps(message) + "\n").encode("utf-8"))

    send({"type": "hello", "id": os.environ["OSH_PLUGIN_ID"], "token": os.environ["OSH_PLUGIN_TOKEN"]})
    welcome = json.loads(lines.readline() or "null")
    if not welcome or welcome.get("type") != "welcome":
        print("the app refused the connection:", welcome, file=sys.stderr)
        return 1
    send({"type": "log", "message": "connected"})

    presses = 0
    counted = 0
    # The keys bound to "counter" that are on screen, by the id the app gave them.
    counter_keys = set()
    next_id = 1

    def request(message):
        nonlocal next_id
        message["id"] = next_id
        next_id += 1
        send(message)

    def draw(key):
        request({"type": "set_image", "key": key, "image": number_image(counted)})
    # When each held control was pressed, by (module, index).
    held = {}

    def control_of(input):
        control = (input or {}).get("control") or {}
        return control.get("module"), control.get("index")
    for line in lines:
        message = json.loads(line)
        kind = message.get("type")
        if kind == "ping":
            send({"type": "pong"})
        elif kind == "invoke":
            action, params = message["action"], message.get("params", {})
            result = {"type": "result", "request": message["request"], "ok": True}
            try:
                if action == "notify":
                    subprocess.run(
                        ["/usr/bin/osascript", "-e", NOTIFY, params.get("title", "OpenStreamHub"), params["message"]],
                        check=True,
                        timeout=5,
                        capture_output=True,
                    )
                elif action == "count":
                    presses += 1
                    request({"type": "set_variable", "name": "plugin_presses", "value": presses})
                elif action == "counter":
                    counted += 1
                    for key in counter_keys:
                        draw(key)
                elif action == "hold":
                    held[control_of(message.get("input"))] = time.monotonic()
                else:
                    raise ValueError("unknown action " + action)
            except Exception as error:  # Any failure fails the press, with a short reason.
                result = {"type": "result", "request": message["request"], "ok": False, "error": str(error)[:200]}
            send(result)
        elif kind == "check":
            condition, params = message["condition"], message.get("params", {})
            try:
                if condition != "pressed_at_least":
                    raise ValueError("unknown condition " + condition)
                result = {"type": "result", "request": message["request"], "ok": True,
                          "value": presses >= params["count"]}
            except Exception as error:  # A failed check fails the press, and neither branch runs.
                result = {"type": "result", "request": message["request"], "ok": False, "error": str(error)[:200]}
            send(result)
        elif kind == "event":
            # No answer: an event is only told. Here, the release of a key whose press ran "hold".
            if message.get("action") == "hold" and message.get("gesture") == "released":
                pressed = held.pop(control_of(message), None)
                if pressed is not None:
                    send({"type": "log", "message": "held for %.1f s" % (time.monotonic() - pressed)})
        elif kind == "appear":
            # A key of "counter" came on screen (or the plugin just connected): draw it at once.
            # Only keys on screen are redrawn on a press; one that comes back shows its old count until this
            # redraw lands, a moment later.
            key = message.get("key")
            if message.get("action") == "counter" and key:
                counter_keys.add(key)
                draw(key)
        elif kind == "disappear":
            counter_keys.discard(message.get("key"))
        elif kind == "reply" and not message.get("ok"):
            send({"type": "log", "message": "request %s was refused: %s" % (message.get("id"), message.get("error", ""))})
    return 0


if __name__ == "__main__":
    sys.exit(main())
