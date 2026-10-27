#!/usr/bin/env bash
# Usage: retry.sh <tries> <command> [args...]
#
# Runs the command up to <tries> times. The wait between tries doubles,
# starting at 5 seconds (5, 10, 20, 40 for five tries). Exits 0 on the first
# success; after the last failure it prints an ::error:: line and exits with
# the command's own status. Exit 2 on bad usage.
#
# RETRY_SLEEP sets the first wait in seconds (default 5). The tests set it to
# 0 so they run fast.
set -u

if [ "$#" -lt 2 ]; then
  echo "usage: retry.sh <tries> <command> [args...]" >&2
  exit 2
fi

tries="$1"
shift

case "$tries" in
  '' | *[!0-9]* | 0)
    echo "retry.sh: <tries> must be a positive integer, got '$tries'" >&2
    exit 2
    ;;
esac

wait_s="${RETRY_SLEEP:-5}"
case "$wait_s" in
  '' | *[!0-9]*)
    echo "retry.sh: RETRY_SLEEP must be a whole number, got '$wait_s'" >&2
    exit 2
    ;;
esac
attempt=1
while true; do
  "$@" && exit 0
  status=$?
  if [ "$attempt" -ge "$tries" ]; then
    echo "::error::'$*' failed after $tries tries (last exit status $status)"
    exit "$status"
  fi
  echo "::warning::attempt $attempt of $tries failed: '$*'. Retrying in ${wait_s}s."
  sleep "$wait_s"
  wait_s=$((wait_s * 2))
  attempt=$((attempt + 1))
done
