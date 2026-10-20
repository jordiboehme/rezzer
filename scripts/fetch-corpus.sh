#!/bin/sh
# Fetches the large corpus files listed in tests/corpus/remote.tsv
# (lines: path<TAB>url<TAB>sha256, paths relative to tests/corpus).
# A file that exists with the right hash is skipped. Downloads are data: they are
# hash-checked and never executed. Limits per file: 50 MB and 300 seconds (curl --max-filesize,
# --max-time); a file that needs more fails with a hint.
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
list="$root/tests/corpus/remote.tsv"
corpus="$root/tests/corpus"

if command -v shasum >/dev/null 2>&1; then
    hash_of() { shasum -a 256 "$1" | cut -d ' ' -f 1; }
elif command -v sha256sum >/dev/null 2>&1; then
    hash_of() { sha256sum "$1" | cut -d ' ' -f 1; }
else
    echo "fetch-corpus: need shasum or sha256sum" >&2
    exit 1
fi

tab=$(printf '\t')
failed=0
while IFS="$tab" read -r path url sha || [ -n "${path:-}" ]; do
    case "$path" in ''|'#'*) continue ;; esac
    case "$path" in /*|*..*) echo "fetch-corpus: bad path $path" >&2; failed=1; continue ;; esac
    case "$url" in https://*) ;; *) echo "fetch-corpus: $path: url must be https" >&2; failed=1; continue ;; esac
    target="$corpus/$path"
    if [ -f "$target" ] && [ "$(hash_of "$target")" = "$sha" ]; then
        echo "ok      $path"
        continue
    fi
    mkdir -p "$(dirname "$target")"
    tmp="$target.part"
    if ! curl --fail --location --proto =https --max-filesize 50M --max-time 300 --silent --show-error -o "$tmp" "$url"; then
        rm -f "$tmp"
        echo "failed  $path: download failed (limits: 50 MB and 300 s per file; a bigger or slower file fails here)" >&2
        failed=1
        continue
    fi
    if [ "$(hash_of "$tmp")" != "$sha" ]; then
        rm -f "$tmp"
        echo "failed  $path: hash mismatch" >&2
        failed=1
        continue
    fi
    mv "$tmp" "$target"
    echo "fetched $path"
done < "$list"
exit "$failed"
