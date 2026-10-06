# Shared helpers for scripts/hardware/*.sh. Source only; do not execute.
# Policy: no network, no hardware writes, no firmware access. The only
# filesystem writes are inside a NEW output directory (never overwritten).

HW_REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
HW_OUT_ROOT_DEFAULT="$HW_REPO_ROOT/captures/hardware"   # /captures is git-ignored

hw_die() { local code="$1"; shift; echo "error: $*" >&2; exit "$code"; }

hw_need() { command -v "$1" >/dev/null 2>&1 || hw_die 3 "required tool not found: $1 ($2)"; }

# Locate a prebuilt masterhub-diag. Never builds (cargo would write to target/).
hw_find_diag() {
  if [[ -n "${MASTERHUB_DIAG:-}" ]]; then
    [[ -x "$MASTERHUB_DIAG" ]] || hw_die 3 "MASTERHUB_DIAG is not executable: $MASTERHUB_DIAG"
    DIAG="$MASTERHUB_DIAG"; return
  fi
  local c
  for c in "$HW_REPO_ROOT/target/release/masterhub-diag" "$HW_REPO_ROOT/target/debug/masterhub-diag"; do
    if [[ -x "$c" ]]; then DIAG="$c"; return; fi
  done
  hw_die 3 "masterhub-diag not built. Run: cargo build --release -p cooltek-sdk-cli --locked (or set MASTERHUB_DIAG)"
}

hw_timestamp() { date -u +%Y%m%dT%H%M%SZ; }

# Create a brand-new directory; fails if it already exists (no overwrite).
hw_new_dir() { mkdir -p "$(dirname "$1")" && mkdir "$1" || hw_die 4 "cannot create new output directory (exists?): $1"; }

hw_sha256() { shasum -a 256 "$1" | awk '{print $1}'; }

# System facts, read-only, local only.
hw_system_info() {
  echo "collected_utc: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "uname: $(uname -a)"
  echo "arch: $(uname -m)"
  if command -v sw_vers >/dev/null 2>&1; then sw_vers; fi
  echo "rustc: $(rustc --version 2>/dev/null || echo unavailable)"
  echo "git_rev: $(git -C "$HW_REPO_ROOT" rev-parse HEAD 2>/dev/null || echo unavailable)"
  echo "git_dirty: $(git -C "$HW_REPO_ROOT" status --porcelain 2>/dev/null | wc -l | tr -d ' ') changed paths"
  echo "masterhub_diag: ${DIAG:-unset}"
  echo "masterhub_diag_version: $("${DIAG:-true}" --version 2>/dev/null || echo unavailable)"
}

# Module name -> "<kind> <vid-dec> <pid-dec> <pid-dec>" (both PID generations, VID 0x2516).
hw_module_pids() {
  case "$1" in
    base|m0064)    echo "base 9494 $((16#01f6)) $((16#9999))" ;;
    knobs|m003k)   echo "knobs 9494 $((16#0202)) $((16#9996))" ;;
    dial|m131e)    echo "dial 9494 $((16#0206)) $((16#9994))" ;;
    keys|m4315)    echo "keys 9494 $((16#01fe)) $((16#9997))" ;;
    faders|m005s)  echo "faders 9494 $((16#0200)) $((16#9991))" ;;
    rollers|m032le) echo "rollers 9494 $((16#0204)) $((16#9995))" ;;
    *) return 1 ;;
  esac
}
