#!/usr/bin/env bash
# Leak check for the private pagevision test set.
# Reads every truth line of 20 characters or more from the files the private
# manifest names (text, edges, nodes), searches the worktree for it with
# git grep -F (tracked and untracked files), and exits 1 on any hit. A region
# truth file (regions) holds numbers only, so the worktree is searched for its
# page id (the "page" value) instead, counted as one truth line. It never
# prints a matched line or id, only counts.
# Usage: scripts/pagevision-leak-check.sh <private-manifest.toml>
set -u

if [ $# -ne 1 ] || [ ! -f "$1" ]; then
    echo "usage: $0 <private-manifest.toml>" >&2
    exit 2
fi

manifest="$1"
base="$(cd "$(dirname "$manifest")" && pwd)"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root" || exit 2

checked=0
hits=0
entries=0

# search <text>: count the text as checked, and as a hit when the worktree holds it.
search() {
    checked=$((checked + 1))
    git grep --untracked -qF -e "$1" -- . >/dev/null 2>&1
    rc=$?
    if [ "$rc" -eq 0 ]; then
        hits=$((hits + 1))
    elif [ "$rc" -ne 1 ]; then
        echo "git grep failed (exit $rc)" >&2
        exit 2
    fi
}

while read -r key rel; do
    entries=$((entries + 1))
    case "$rel" in
        /*) file="$rel" ;;
        *) file="$base/$rel" ;;
    esac
    [ -f "$file" ] && [ -r "$file" ] || { echo "missing or unreadable truth file: $rel" >&2; exit 2; }
    if [ "$key" = "regions" ]; then
        page="$(sed -nE 's/.*"page"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/p' "$file" | head -n 1)"
        [ -n "$page" ] || { echo "region truth without a page id: $rel" >&2; exit 2; }
        search "$page"
        continue
    fi
    while IFS= read -r line || [ -n "$line" ]; do
        line="${line%$'\r'}"
        [ "${#line}" -ge 20 ] || continue
        search "$line"
    done < "$file"
done < <(sed -nE \
    -e 's/^[[:space:]]*(text|edges|nodes|regions)[[:space:]]*=[[:space:]]*"([^"]*)".*/\1 \2/p' \
    -e "s/^[[:space:]]*(text|edges|nodes|regions)[[:space:]]*=[[:space:]]*'([^']*)'.*/\\1 \\2/p" \
    "$manifest")

# Count the truth keys with a looser pattern (also inline tables and quoted keys). Every one
# must have been parsed above, or part of the truth went unchecked.
keys=$(grep -cE "[\"']?(text|edges|nodes|regions)[\"']?[[:space:]]*=" "$manifest")
if [ "$keys" -ne "$entries" ]; then
    echo "leak check: the manifest has $keys truth keys but $entries were parsed; refusing to pass" >&2
    exit 2
fi

if [ "$entries" -eq 0 ] || [ "$checked" -eq 0 ]; then
    echo "leak check: no truth lines to check ($entries entries, $checked lines); refusing to pass" >&2
    exit 2
fi

if [ "$hits" -gt 0 ]; then
    echo "leak check FAILED: $hits of $checked private truth lines found in the worktree" >&2
    exit 1
fi
echo "leak check ok: $checked private truth lines checked, none found"
