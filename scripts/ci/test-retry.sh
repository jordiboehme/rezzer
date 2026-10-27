#!/usr/bin/env bash
# Tests for retry.sh. No network. RETRY_SLEEP=0 makes the waits instant.
# Usage: bash scripts/ci/test-retry.sh
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
retry="$here/retry.sh"
export RETRY_SLEEP=0

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

count() {
  # count <pattern> <file>: number of matching lines
  grep -c -- "$1" "$2" || true
}

retry_sh_passes_first_try() {
  bash "$retry" 3 true >"$work/out" 2>&1
  status=$?
  [ "$status" -eq 0 ] && [ "$(count '::warning::' "$work/out")" -eq 0 ] \
    && [ "$(count '::error::' "$work/out")" -eq 0 ]
}

retry_sh_retries_then_passes() {
  rm -f "$work/counter"
  # Fails on the first two runs, passes on the third.
  # shellcheck disable=SC2016
  bash "$retry" 5 sh -c 'echo x >> "$1"; [ "$(wc -l < "$1")" -ge 3 ]' sh "$work/counter" \
    >"$work/out" 2>&1
  status=$?
  [ "$status" -eq 0 ] && [ "$(count '::warning::' "$work/out")" -eq 2 ] \
    && [ "$(count '::error::' "$work/out")" -eq 0 ] \
    && [ "$(wc -l < "$work/counter" | tr -d ' ')" -eq 3 ]
}

retry_sh_doubles_the_wait() {
  RETRY_SLEEP=1 bash "$retry" 3 false >"$work/out" 2>&1
  [ "$(count 'Retrying in 1s' "$work/out")" -eq 1 ] \
    && [ "$(count 'Retrying in 2s' "$work/out")" -eq 1 ]
}

retry_sh_rejects_bad_retry_sleep() {
  RETRY_SLEEP=x bash "$retry" 3 true >"$work/out" 2>&1
  letter=$?
  RETRY_SLEEP=-1 bash "$retry" 3 true >"$work/out" 2>&1
  minus=$?
  [ "$letter" -eq 2 ] && [ "$minus" -eq 2 ]
}

retry_sh_gives_up_with_the_status() {
  bash "$retry" 3 sh -c 'exit 7' >"$work/out" 2>&1
  status=$?
  [ "$status" -eq 7 ] && [ "$(count '::error::' "$work/out")" -eq 1 ] \
    && [ "$(count '::warning::' "$work/out")" -eq 2 ]
}

retry_sh_rejects_bad_tries() {
  bash "$retry" 0 true >"$work/out" 2>&1
  zero=$?
  bash "$retry" x true >"$work/out" 2>&1
  letter=$?
  [ "$zero" -eq 2 ] && [ "$letter" -eq 2 ]
}

retry_sh_rejects_missing_command() {
  bash "$retry" 3 >"$work/out" 2>&1
  [ "$?" -eq 2 ]
}

for t in retry_sh_passes_first_try retry_sh_retries_then_passes \
  retry_sh_gives_up_with_the_status retry_sh_rejects_bad_tries \
  retry_sh_doubles_the_wait retry_sh_rejects_bad_retry_sleep \
  retry_sh_rejects_missing_command; do
  "$t"
  check "$t" $?
done

if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) failed"
  exit 1
fi
echo "all tests passed"
