"""An example OpenStreamHub plugin, using only the Python standard library.

The app starts it with OSH_PLUGIN_SOCKET, OSH_PLUGIN_ID and OSH_PLUGIN_TOKEN in the environment. It connects,
says hello, then answers each invoke with one result. See docs/plugins.md for the protocol.

Actions:
- notify: shows a macOS notification with `title` and `message` (the message may use {{variables}}).
- count: adds one to a counter and writes it to the profile variable `plugin_presses`, which the profile must
  declare as a number.
"""

import json
import os
import socket
import subprocess
import sys

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
                else:
                    raise ValueError("unknown action " + action)
            except Exception as error:  # Any failure fails the press, with a short reason.
                result = {"type": "result", "request": message["request"], "ok": False, "error": str(error)[:200]}
            send(result)
        elif kind == "reply" and not message.get("ok"):
            send({"type": "log", "message": "a variable write was refused: " + message.get("error", "")})
    return 0


if __name__ == "__main__":
    sys.exit(main())
