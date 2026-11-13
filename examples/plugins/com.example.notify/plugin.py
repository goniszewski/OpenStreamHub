"""An example OpenStreamHub plugin, using only the Python standard library.

The app starts it with OSH_PLUGIN_SOCKET, OSH_PLUGIN_ID and OSH_PLUGIN_TOKEN in the environment. It connects,
says hello, then answers each invoke and each check with one result. See docs/plugins.md for the protocol.

Actions:
- notify: shows a macOS notification with `title` and `message` (the message may use {{variables}}).
- count: adds one to a counter and writes it to the profile variable `plugin_presses`, which the profile must
  declare as a number.
- hold: bind it to a key's press. Its manifest sets "events": true, so the app also sends the key's release as an
  `event`, and the plugin logs how long the key was held (Settings > Plugins shows the log).

Conditions (an if / else asks them as a `check` when its key is pressed, and the answer carries `value`):
- pressed_at_least: true once the `count` action's counter has reached the number `count`. It only reads the
  counter and answers at once, as a check should.

Fields it does not know, in any message, are ignored: newer apps may add some.
"""

import json
import os
import socket
import subprocess
import sys
import time

# Text arrives in argv, never inside the script, so a message cannot inject AppleScript.
NOTIFY = 'on run argv\ndisplay notification (item 2 of argv) with title (item 1 of argv)\nend run'


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
    next_id = 1
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
                    send({"type": "set_variable", "id": next_id, "name": "plugin_presses", "value": presses})
                    next_id += 1
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
        elif kind == "reply" and not message.get("ok"):
            send({"type": "log", "message": "a variable write was refused: " + message.get("error", "")})
    return 0


if __name__ == "__main__":
    sys.exit(main())
