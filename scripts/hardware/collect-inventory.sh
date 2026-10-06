#!/usr/bin/env bash
# collect-inventory.sh [OUT_DIR]
# Step 0 of docs/masterhub-first-hardware-session.md.
# Runs `masterhub-diag list` (enumeration only: opens nothing, writes nothing to
# hardware) and saves JSON + host facts into a NEW directory.
# Exit: 0 ok (including "no devices found"), 2 usage, 3 missing tool, 4 failure.
set -euo pipefail
# shellcheck source=lib.sh
source "$(dirname "${BASH_SOURCE[0]}")/lib.sh"

case "${1:-}" in -h|--help) sed -n '2,7p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;; esac
[[ $# -le 1 ]] || hw_die 2 "usage: collect-inventory.sh [OUT_DIR]"

hw_need jq "brew install jq"
hw_need shasum "ships with macOS"
hw_find_diag

OUT="${1:-$HW_OUT_ROOT_DEFAULT/inventory-$(hw_timestamp)}"
hw_new_dir "$OUT"

"$DIAG" list > "$OUT/list.json" || hw_die 4 "masterhub-diag list failed"
jq -e '.devices | type == "array"' "$OUT/list.json" >/dev/null || hw_die 4 "unexpected list output"
hw_system_info > "$OUT/system.txt"

# Read-only host USB view for cross-checking topology (optional, local).
if command -v system_profiler >/dev/null 2>&1; then
  system_profiler SPUSBDataType > "$OUT/system_profiler_usb.txt" 2>/dev/null || true
fi
if command -v ioreg >/dev/null 2>&1; then
  ioreg -p IOUSB -l -w0 > "$OUT/ioreg_iousb.txt" 2>/dev/null || true
fi

jq -r '.devices[] | [.module // "unknown", (.vendor_id|tostring), (.product_id|tostring), (.interface_number|tostring), (.usage_page|tostring), (.usage|tostring), (.serial_number // "-"), .path_hex] | @tsv' \
  "$OUT/list.json" > "$OUT/summary.tsv"

N="$(jq '.devices | length' "$OUT/list.json")"
if [[ "$N" -eq 0 ]]; then
  echo "no devices found (VID 0x2516). Check cabling/power, then re-run. Output: $OUT"
else
  echo "found $N HID interface(s). Columns: module vid pid iface usage_page usage serial path_hex"
  cat "$OUT/summary.tsv"
  echo "output: $OUT"
fi
( cd "$OUT" && for f in *; do printf '%s  %s\n' "$(hw_sha256 "$f")" "$f"; done > SHA256SUMS )
