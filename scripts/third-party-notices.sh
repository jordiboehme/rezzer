#!/usr/bin/env bash
# Writes THIRD-PARTY-NOTICES.txt for the release archives: the licenses of the
# crates built into the rezzer binary (cargo-about with about.toml and
# about.hbs), then the notices of data bundled inside crates
# (licenses/bundled-data.txt).
#
# Usage: scripts/third-party-notices.sh <out-file>
#
# Needs cargo-about on PATH and the crates fetched (cargo fetch --locked).
# Exit 2 on a usage error, 1 when cargo-about fails or its output lists no
# crate.
set -euo pipefail

usage() {
  echo "usage: $(basename "$0") <out-file>" >&2
  exit 2
}

[ "$#" -eq 1 ] || usage
[ -n "$1" ] || usage
out="$1"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
bundled="$root/licenses/bundled-data.txt"
[ -s "$bundled" ] || { echo "error: $bundled is missing or empty" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
raw="$work/raw.txt"
listed="$work/listed.txt"
count="$work/count.txt"

# --manifest-path on the binary crate: the graph is what rezzer links, not every
# workspace member. --locked: a Cargo.lock that would change fails. --fail: a crate
# whose license cannot be read or clarified fails. -o, not a redirect: cargo-about
# refuses to write to stdout under PowerShell.
cargo about generate --locked --fail \
  --manifest-path "$root/crates/rezzer/Cargo.toml" \
  --config "$root/about.toml" \
  --output-file "$raw" \
  "$root/about.hbs"

[ -s "$raw" ] || { echo "error: cargo-about wrote no output" >&2; exit 1; }

# Keep a license block only when it names at least one crate, and turn the
# crate markers into list items. A block whose crates are all Rezzer's own path
# crates comes out with no crate line and is left out.
awk -v count="$count" '
  $0 == "@@rezzer-block" { inblock = 1; buf = ""; crates = 0; next }
  $0 == "@@rezzer-end" {
    if (crates > 0) printf "%s", buf
    inblock = 0
    next
  }
  inblock && /^@@rezzer-crate / {
    sub(/^@@rezzer-crate /, "  - ")
    crates++
    total++
    buf = buf $0 "\n"
    next
  }
  inblock { buf = buf $0 "\n"; next }
  { print }
  END {
    if (inblock) { print "error: unterminated license block" > "/dev/stderr"; exit 1 }
    print total + 0 > count
  }
' "$raw" > "$listed"

if grep -q '^@@rezzer' "$listed"; then
  echo "error: a template marker is left in the output" >&2
  exit 1
fi
crates="$(cat "$count")"
if [ "$crates" -eq 0 ]; then
  echo "error: cargo-about output lists no third-party crate" >&2
  exit 1
fi

{
  cat "$listed"
  echo
  cat "$bundled"
} > "$out"

echo "wrote $out ($crates crate entries)"
