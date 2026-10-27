#!/usr/bin/env bash
# Checks a release tag before anything is built: the three app version files agree with each other and with the
# tag, the release notes file exists, and docs/release-process.md names the same current version.
#
#   scripts/check-release.sh v0.3.0
#
# A prerelease suffix is ignored for the comparison (v0.3.0-citest is checked as 0.3.0), so a throwaway tag can
# exercise the release workflow without a version bump.
set -euo pipefail

tag="${1:?usage: check-release.sh <tag, e.g. v0.3.0>}"
version="${tag#v}"
version="${version%%-*}"
app=apps/open-stream-hub
fail=0

report() { echo "::error::$1"; fail=1; }

package_json=$(jq -r .version "$app/package.json")
tauri_conf=$(jq -r .version "$app/src-tauri/tauri.conf.json")
cargo_toml=$(sed -n 's/^version = "\(.*\)"/\1/p' "$app/src-tauri/Cargo.toml" | head -n 1)

for pair in "package.json:$package_json" "tauri.conf.json:$tauri_conf" "src-tauri/Cargo.toml:$cargo_toml"; do
  if [ "${pair#*:}" != "$version" ]; then
    report "${pair%%:*} is ${pair#*:}, but the tag is $tag"
  fi
done

[ -f "docs/release-notes-$version.md" ] || report "docs/release-notes-$version.md is missing"

if ! grep -q "the app is currently \`$version\`" docs/release-process.md; then
  report "docs/release-process.md does not say the app is currently \`$version\`"
fi

[ "$fail" -eq 0 ] && echo "release checks passed for $tag ($version)"
exit "$fail"
