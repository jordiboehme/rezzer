#!/usr/bin/env bash
# Tests for third-party-notices.sh that need no cargo-about: the usage errors
# exit 2 before cargo-about runs. The real run is the notices job in release.yml.
# Usage: bash scripts/test-third-party-notices.sh
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/third-party-notices.sh"

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

failures=0

check() {
  # check <name> <condition result: 0 = ok>
  if [ "$2" -eq 0 ]; then
    echo "ok   $1"
  else
    echo "FAIL $1"
    failures=$((failures + 1))
  fi
}

usage_error() {
  # usage_error <args...>: exit 2, a usage line on stderr, no output file
  bash "$script" "$@" >"$work/out" 2>"$work/err"
  status=$?
  [ "$status" -eq 2 ] && grep -q '^usage: ' "$work/err" && [ ! -s "$work/out" ]
}

notices_rejects_no_argument() {
  usage_error
}

notices_rejects_two_arguments() {
  usage_error "$work/a.txt" "$work/b.txt" && [ ! -e "$work/a.txt" ] && [ ! -e "$work/b.txt" ]
}

notices_rejects_empty_argument() {
  usage_error ""
}

for t in notices_rejects_no_argument notices_rejects_two_arguments \
  notices_rejects_empty_argument; do
  "$t"
  check "$t" $?
done

if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) failed"
  exit 1
fi
echo "all tests passed"
