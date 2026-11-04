#!/bin/sh
# Shows the state of your work on deck keys: sets profile variables with `osh`, and key faces
# (Inspector > Faces) turn them into colours and words. See docs/status-keys.md.
#
#   deck-status.sh git [DIR] [--watch SECS]   git_state: clean | dirty | unknown
#   deck-status.sh ci [DIR] [--watch SECS]    ci_state:  passing | failing | running | unknown   (needs gh)
#   deck-status.sh run NAME -- CMD...         NAME: running | passing | failing, from CMD's exit code
#
# OSH overrides the osh to call (default: osh on PATH, else the OpenStreamHub app's own).

set -u

OSH=${OSH:-}
if [ -z "$OSH" ]; then
  if command -v osh >/dev/null 2>&1; then OSH=osh
  else OSH=/Applications/OpenStreamHub.app/Contents/MacOS/osh; fi
fi

usage() { sed -n '2,10p' "$0" | sed 's/^# \{0,1\}//' >&2; exit 2; }

# Sets a text variable; failing (app closed, variable not declared) is not an error for a status script.
put() { "$OSH" set --kind text "$1" "$2" >/dev/null 2>&1 || true; }

git_state() {
  if ! out=$(git -C "$1" status --porcelain 2>/dev/null); then put git_state unknown
  elif [ -n "$out" ]; then put git_state dirty
  else put git_state clean; fi
}

ci_state() {
  branch=$(git -C "$1" branch --show-current 2>/dev/null)
  run=$(cd "$1" 2>/dev/null && gh run list --branch "$branch" --limit 1 \
        --json status,conclusion --jq '.[0] | "\(.status) \(.conclusion)"' 2>/dev/null) || run=
  case $run in
    "completed success") put ci_state passing ;;
    "completed "*) put ci_state failing ;;
    "queued "*|"in_progress "*|"waiting "*|"requested "*|"pending "*) put ci_state running ;;
    *) put ci_state unknown ;;
  esac
}

mode=${1:-}
[ -n "$mode" ] || usage
shift

case $mode in
  git|ci)
    dir=.
    watch_secs=
    while [ $# -gt 0 ]; do
      case $1 in
        --watch)
          [ $# -ge 2 ] || usage
          watch_secs=$2
          case $watch_secs in ''|*[!0-9]*|0) usage ;; esac
          shift 2 ;;
        -*) usage ;;
        *) dir=$1; shift ;;
      esac
    done
    fn=${mode}_state
    while :; do
      "$fn" "$dir"
      [ -n "$watch_secs" ] || break
      sleep "$watch_secs"
    done
    ;;
  run)
    name=${1:-}; [ -n "$name" ] || usage
    shift
    [ "${1:-}" = -- ] || usage
    shift
    [ $# -gt 0 ] || usage
    put "$name" running
    "$@"
    status=$?
    if [ "$status" -eq 0 ]; then put "$name" passing; else put "$name" failing; fi
    exit "$status"
    ;;
  *) usage ;;
esac
