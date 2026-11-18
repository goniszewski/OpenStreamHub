"""Claude Code hooks and status line for the OpenStreamHub Claude Code plugin, using only the standard library.

Claude Code runs this script; the plugin never does. Two ways:

- As a hook, `hook.py` with the hook's JSON on stdin: it writes the session's state (working, waiting for you, done,
  idle) to `sessions/<session id>.json` in the plugin's data folder, which the plugin reads. SessionEnd leaves a short
  "ended" record, so a hook that runs late cannot bring the session back.
- As the status line, `hook.py statusline [command]`: it writes the context window's use and the session's cost to
  `sessions/<session id>.meter.json`, then prints a short status line, or runs `command` (your own status line, through
  the shell, with the same stdin) and prints what it prints.

It never blocks or changes what Claude Code does: it prints nothing as a hook, decides nothing, and always exits 0.
`install_hooks.py` adds it to ~/.claude/settings.json.
"""

import fcntl
import json
import os
import re
import subprocess
import sys
import time

PLUGIN_ID = "com.github.goniszewski.claudecode"
APP_DATA = os.path.expanduser("~/Library/Application Support/dev.openstreamhub")
SESSION_ID = re.compile(r"^[A-Za-z0-9_-]{1,128}$")
SHELLS = {"sh", "bash", "zsh", "dash", "fish", "env"}
MAX_TEXT = 200
ANCESTORS = 12
QUESTIONS = ("AskUserQuestion", "ExitPlanMode")  # tools that ask you something rather than for a permission


def data_folder():
    """The plugin's data folder, `<app data>/plugin-data/<id>`, which the app gives the plugin as OSH_PLUGIN_DATA,
    wherever this copy of the script is."""
    explicit = os.environ.get("OSH_CLAUDE_DATA")  # tests
    if explicit:
        return explicit
    return os.path.join(APP_DATA, "plugin-data", PLUGIN_ID)


def sessions_folder():
    folder = os.path.join(data_folder(), "sessions")
    os.makedirs(folder, mode=0o700, exist_ok=True)
    return folder


def short(text):
    text = " ".join(str(text or "").split())
    return text[:MAX_TEXT]


def number(value):
    return value if isinstance(value, (int, float)) and not isinstance(value, bool) else None


def write_json(path, value):
    temporary = "%s.%d.tmp" % (path, os.getpid())
    with open(temporary, "w", encoding="utf-8") as file:
        json.dump(value, file)
    os.replace(temporary, path)


def read_json(path):
    try:
        with open(path, encoding="utf-8") as file:
            value = json.load(file)
        return value if isinstance(value, dict) else None
    except (OSError, ValueError):
        return None


def ps(pid):
    """(parent pid, tty, command name) of a process, or None."""
    try:
        done = subprocess.run(["/bin/ps", "-o", "ppid=,tty=,comm=", "-p", str(pid)], capture_output=True, text=True,
                              timeout=2)
    except (OSError, subprocess.TimeoutExpired):
        return None
    parts = done.stdout.split(None, 2)
    if done.returncode != 0 or len(parts) < 3:
        return None
    try:
        return int(parts[0]), parts[1], os.path.basename(parts[2].strip())
    except ValueError:
        return None


def claude_process(start=None, look=ps):
    """The Claude Code process that ran this hook: (pid, ttys, pids), or (None, [], []).

    Hooks may run through a shell, so it walks up past shells to the first other process. `ttys` are the terminals of
    that process and of the processes above it, nearest first: a wrapper that gives Claude Code a terminal of its own
    sits between it and the terminal app's. Under tmux or screen, whose server is not above Claude Code, the terminal
    app's tty is not among them, so the plugin cannot mistake another pane for this session. `pids` are that process
    and those above it: before answering, the plugin checks that one of them is in the foreground of the terminal it
    found, so a terminal app started from it, or a program run there since, is not taken for this session."""
    pid = start or os.getppid()
    found = None
    for _ in range(ANCESTORS):
        if pid <= 1:
            return None, [], []
        row = look(pid)
        if row is None:
            return None, [], []
        if row[2] not in SHELLS:
            found = pid
            break
        pid = row[0]
    if found is None:
        return None, [], []
    ttys, pids = [], []
    for _ in range(ANCESTORS):
        if pid <= 1:
            break
        row = look(pid)
        if row is None:
            break
        pids.append(pid)
        if row[1] not in ("?", "??", "") and "/dev/" + row[1] not in ttys:
            ttys.append("/dev/" + row[1])
        pid = row[0]
    return found, ttys, pids


def terminal(environ):
    """The session's terminal app: its bundle id and name and, in iTerm2, the session's id. A multiplexer (tmux,
    screen), or a terminal app started from another, inherits these from the terminal that started it, so they are
    not trusted to name a session there; the plugin also checks who owns the terminal before answering."""
    multiplexed = bool(environ.get("TMUX") or environ.get("STY"))
    program = short(environ.get("TERM_PROGRAM", ""))
    iterm = environ.get("ITERM_SESSION_ID", "")
    return {
        "program": program,
        "bundle": short(environ.get("__CFBundleIdentifier", "")),
        "iterm": iterm.split(":", 1)[1] if ":" in iterm and program == "iTerm.app" and not multiplexed else "",
        "multiplexed": multiplexed,
    }


def next_state(event, record):
    """The (state, detail) after a hook event, or None to leave the record as it is. A wait for a permission has the
    tool's name (or "permission") as its detail, a wait for an answer "question"."""
    name = event.get("hook_event_name")
    if name == "SessionStart":
        return "idle", ""
    if name == "UserPromptSubmit":
        return "working", ""
    if name == "PreToolUse":
        return "working", short(event.get("tool_name"))
    if name == "PostToolUse":
        return "working", ""
    if name == "PermissionRequest":
        return "waiting", short(event.get("tool_name")) or "permission"
    if name == "Notification":
        kind = event.get("notification_type")
        if kind == "permission_prompt":
            # Comes about 6 seconds after the prompt; keeps the tool PermissionRequest named.
            if record.get("state") == "waiting":
                return None
            if record.get("detail") in QUESTIONS:
                return "waiting", record["detail"]  # a question shown as a prompt: the plugin never answers it
            return "waiting", "permission"
        if kind in ("elicitation_dialog", "elicitation_url_dialog", "agent_needs_input"):
            return "waiting", "question"
        return None
    if name == "Stop":
        return "done", ""
    return None


def handle(event, environ=None, now=None, find_claude=claude_process):
    """Updates the session's record for one hook event. Returns the record written, or None."""
    environ = os.environ if environ is None else environ
    at = now if now is not None else time.time_ns()
    session = event.get("session_id")
    if not isinstance(session, str) or not SESSION_ID.match(session):
        return None
    folder = sessions_folder()
    path = os.path.join(folder, session + ".json")
    # The lock file is never removed: a hook waiting on a removed one would hold a lock no other hook sees.
    with open(os.path.join(folder, session + ".lock"), "a") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        record = read_json(path) or {}
        if record.get("state") == "ended":
            if event.get("hook_event_name") != "SessionStart" or record.get("at", 0) > at:
                return None  # a hook that runs after SessionEnd
            record = {}  # the session resumed
        if event.get("hook_event_name") == "SessionEnd":  # whatever came before it, the session is over
            write_json(path, {"session": session, "state": "ended", "at": at})
            try:
                os.remove(os.path.join(folder, session + ".meter.json"))
            except FileNotFoundError:
                pass
            return None
        if record.get("at", 0) > at:
            return None  # hooks run in the background, so an older event can come last
        change = next_state(event, record)
        if change is None and record:
            return None
        if change is None:
            change = ("idle", "")
        project_dir = environ.get("CLAUDE_PROJECT_DIR") or event.get("cwd") or ""
        record.update({
            "session": session,
            "state": change[0],
            "detail": change[1],
            "project": short(os.path.basename(os.path.normpath(project_dir))) if project_dir else "",
            "at": at,
        })
        if record.get("pid") is None or event.get("hook_event_name") == "SessionStart":
            record["pid"], record["ttys"], record["pids"] = find_claude()
            record["terminal"] = terminal(environ)
        write_json(path, record)
        return record


def statusline(event, command, run=subprocess.run):
    """Writes the session's context use and cost, then returns the status line to print."""
    session = event.get("session_id")
    window = event.get("context_window") if isinstance(event.get("context_window"), dict) else {}
    cost = event.get("cost") if isinstance(event.get("cost"), dict) else {}
    model = event.get("model") if isinstance(event.get("model"), dict) else {}
    meter = {
        "used": number(window.get("used_percentage")),
        "cost": number(cost.get("total_cost_usd")),
        "model": short(model.get("display_name")),
        "at": time.time_ns(),
    }
    if isinstance(session, str) and SESSION_ID.match(session):
        folder = sessions_folder()
        record = read_json(os.path.join(folder, session + ".json"))
        if not record or record.get("state") != "ended":
            write_json(os.path.join(folder, session + ".meter.json"), meter)
    if command:
        done = run(command, shell=True, input=json.dumps(event), capture_output=True, text=True, timeout=10)
        return done.stdout.rstrip("\n")
    parts = [meter["model"]] if meter["model"] else []
    if meter["used"] is not None:
        parts.append("context %d%%" % round(meter["used"]))
    if meter["cost"] is not None:
        parts.append("$%.2f" % meter["cost"])
    return " · ".join(parts)


def main(argv):
    at = time.time_ns()  # first: the order hooks started in, not the order Python got going
    try:
        event = json.loads(sys.stdin.read() or "{}")
    except ValueError:
        event = {}
    if not isinstance(event, dict):
        event = {}
    if len(argv) > 1 and argv[1] == "statusline":
        try:
            print(statusline(event, " ".join(argv[2:])))
        except Exception as error:  # the status line must still show something
            print("OpenStreamHub status line: %s" % short(error))
        return 0
    try:
        handle(event, now=at)
    except Exception as error:  # never get in Claude Code's way
        print("OpenStreamHub hook: %s" % short(error), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
