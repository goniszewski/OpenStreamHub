#!/usr/bin/env bash
# record-session.sh [options] <module> <seconds>
#   module   base|knobs|dial|keys|faders|rollers (or m0064|m003k|m131e|m4315|m005s|m032le)
#   seconds  finite capture deadline per interface, 1..86400
# Options:
#   --label TEXT            free-text step label stored in meta.json (e.g. "knob1-cw-one-detent")
#   --interface N           record only USB interface number N (default: every interface of the module,
#                           concurrently, because interface selection is an open question)
#   --max-reports N         stop an interface early after N nonempty reports
#   --out-root DIR          parent directory (default: <repo>/captures/hardware)
#   --software-mode         WRITES TO HARDWARE (dial/keys only): sends the software-mode feature
#                           report, then attempts to restore hardware mode. Requires --yes-write-hardware.
#   --yes-write-hardware    acknowledge the write above
# Records via `masterhub-diag record` (reads only unless --software-mode). Creates a NEW
# timestamped directory; nothing is overwritten. Never touches firmware.
# Exit: 0 ok (also when no matching device is attached), 2 usage, 3 missing tool, 4 failure.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

usage() { sed -n '2,19p' "$0" | sed 's/^# \{0,1\}//'; }
LABEL="" ; IFACE="" ; MAXR="" ; OUT_ROOT="$HW_OUT_ROOT_DEFAULT" ; SWMODE=0 ; ACK=0
POS=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --label) [[ $# -ge 2 ]] || hw_die 2 "--label needs a value"; LABEL="$2"; shift 2 ;;
    --interface) [[ $# -ge 2 ]] || hw_die 2 "--interface needs a value"; IFACE="$2"; shift 2 ;;
    --max-reports) [[ $# -ge 2 ]] || hw_die 2 "--max-reports needs a value"; MAXR="$2"; shift 2 ;;
    --out-root) [[ $# -ge 2 ]] || hw_die 2 "--out-root needs a value"; OUT_ROOT="$2"; shift 2 ;;
    --software-mode) SWMODE=1; shift ;;
    --yes-write-hardware) ACK=1; shift ;;
    --*) hw_die 2 "unknown option: $1" ;;
    *) POS+=("$1"); shift ;;
  esac
done
[[ ${#POS[@]} -eq 2 ]] || { usage >&2; exit 2; }
MODULE="$(printf '%s' "${POS[0]}" | tr '[:upper:]' '[:lower:]')"; SECS="${POS[1]}"
[[ "$SECS" =~ ^[0-9]+$ ]] && (( SECS >= 1 && SECS <= 86400 )) || hw_die 2 "seconds must be an integer 1..86400"
[[ -z "$IFACE" || "$IFACE" =~ ^[0-9]+$ ]] || hw_die 2 "--interface must be a number"
[[ -z "$MAXR" || "$MAXR" =~ ^[0-9]+$ ]] || hw_die 2 "--max-reports must be a number"
read -r KIND VID PID_A PID_B < <(hw_module_pids "$MODULE") || hw_die 2 "unknown module: $MODULE"
if (( SWMODE )); then
  [[ "$KIND" == dial || "$KIND" == keys ]] || hw_die 2 "--software-mode is accepted only for dial/keys"
  (( ACK )) || hw_die 2 "--software-mode writes to hardware; add --yes-write-hardware to confirm"
fi

hw_need jq "brew install jq"
hw_need shasum "ships with macOS"
hw_find_diag

LIST="$("$DIAG" list)" || hw_die 4 "masterhub-diag list failed"
MATCHES="$(printf '%s' "$LIST" | jq -c --argjson v "$VID" --argjson a "$PID_A" --argjson b "$PID_B" --arg i "$IFACE" \
  '[.devices[] | select(.vendor_id==$v and (.product_id==$a or .product_id==$b) and ($i=="" or (.interface_number|tostring)==$i))]')"
COUNT="$(printf '%s' "$MATCHES" | jq 'length')"
if [[ "$COUNT" -eq 0 ]]; then
  echo "no devices found for module '$KIND' (VID 0x2516 PID alt). Nothing recorded."
  exit 0
fi

OUT="$OUT_ROOT/$KIND-$(hw_timestamp)${LABEL:+-$(printf '%s' "$LABEL" | tr -c 'A-Za-z0-9._-' '_')}"
hw_new_dir "$OUT"
printf '%s' "$LIST" > "$OUT/list-before.json"
hw_system_info > "$OUT/system.txt"

echo "recording $COUNT interface(s) of '$KIND' for ${SECS}s into $OUT"
echo ">>> perform the physical action(s) now; log what you did and when (see doc)."
(( SWMODE )) && echo ">>> WRITE: software mode requested; hardware mode will be restored afterwards."

PIDS=() ; FILES=()
for ((n = 0; n < COUNT; n++)); do
  ENTRY="$(printf '%s' "$MATCHES" | jq -c ".[$n]")"
  HEX="$(printf '%s' "$ENTRY" | jq -r '.path_hex')"
  IF="$(printf '%s' "$ENTRY" | jq -r '.interface_number')"
  PIDH="$(printf '%04x' "$(printf '%s' "$ENTRY" | jq -r '.product_id')")"
  F="$OUT/iface${IF}-pid${PIDH}-${n}.jsonl"
  ARGS=(record --path-hex "$HEX" --output "$F" --duration-secs "$SECS")
  [[ -n "$MAXR" ]] && ARGS+=(--max-reports "$MAXR")
  (( SWMODE )) && ARGS+=(--software-mode)
  "$DIAG" "${ARGS[@]}" > "$F.stdout" 2> "$F.stderr" &
  PIDS+=("$!") ; FILES+=("$F")
done

STATUS=0
for i in "${!PIDS[@]}"; do
  if wait "${PIDS[$i]}"; then RC=0; else RC=$?; STATUS=4; fi
  echo "${FILES[$i]##*/}: exit $RC" | tee -a "$OUT/exit-codes.txt"
done

# Post-processing (offline): replay, descriptor extraction, hashes.
for F in "${FILES[@]}"; do
  [[ -s "$F" ]] || continue
  "$DIAG" replay "$F" > "$F.replay.txt" 2> "$F.replay.err" || echo "replay failed: ${F##*/} (see .replay.err)" >&2
  if command -v xxd >/dev/null 2>&1; then
    HEXD="$(head -n1 "$F" | jq -r 'select(.report_descriptor.status=="available") | .report_descriptor.raw_hex' 2>/dev/null || true)"
    [[ -n "$HEXD" ]] && printf '%s' "$HEXD" | xxd -r -p > "${F%.jsonl}.hid"
  fi
done

jq -n --arg module "$KIND" --arg label "$LABEL" --argjson secs "$SECS" --arg iface "$IFACE" \
  --argjson sw "$SWMODE" --arg started "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --arg exit_status "$STATUS" \
  --argjson matches "$MATCHES" \
  '{module:$module,label:$label,duration_secs:$secs,interface_filter:$iface,software_mode_requested:($sw==1),
    finished_utc:$started,aggregate_status:($exit_status|tonumber),devices:$matches,
    note:"capture JSONL carries its own session provenance; this file ties the run together"}' > "$OUT/meta.json"
( cd "$OUT" && for f in *; do [[ "$f" == SHA256SUMS ]] || printf '%s  %s\n' "$(hw_sha256 "$f")" "$f"; done > SHA256SUMS )
echo "done: $OUT (status $STATUS)"
exit "$STATUS"
