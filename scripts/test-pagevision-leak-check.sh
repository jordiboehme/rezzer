#!/usr/bin/env bash
# Tests for pagevision-leak-check.sh. Works on a throwaway git repository with made-up truth.
# Usage: bash scripts/test-pagevision-leak-check.sh
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

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

# A repository with a copy of the script, and truth outside it.
repo="$work/repo"
mkdir -p "$repo/scripts" "$work/eval/truth"
cp "$here/pagevision-leak-check.sh" "$repo/scripts/"
git -C "$repo" init -q
secret="a made-up truth line that is long enough"
printf '%s\nshort\n' "$secret" > "$work/eval/truth/page.txt"
printf '[[pages]]\nid = "page"\ntext = "truth/page.txt"\n' > "$work/eval/manifest.toml"

run() {
  bash "$repo/scripts/pagevision-leak-check.sh" "$@" > "$work/out" 2>&1
}

clean_worktree_passes() {
  rm -f "$repo/leak.txt"
  run "$work/eval/manifest.toml" && grep -q "1 private truth lines checked" "$work/out"
}

untracked_leak_fails_without_printing_the_line() {
  printf 'x %s y\n' "$secret" > "$repo/leak.txt"
  run "$work/eval/manifest.toml"
  status=$?
  rm -f "$repo/leak.txt"
  [ "$status" -eq 1 ] && ! grep -qF "$secret" "$work/out"
}

short_lines_alone_refuse_to_pass() {
  printf 'short\n' > "$work/eval/truth/short.txt"
  printf '[[pages]]\nid = "s"\ntext = "truth/short.txt"\n' > "$work/eval/short.toml"
  run "$work/eval/short.toml"
  status=$?
  [ "$status" -eq 2 ]
}

missing_truth_file_fails() {
  printf '[[pages]]\nid = "m"\ntext = "truth/missing.txt"\n' > "$work/eval/missing.toml"
  run "$work/eval/missing.toml"
  status=$?
  [ "$status" -eq 2 ]
}

no_argument_prints_usage() {
  run
  status=$?
  [ "$status" -eq 2 ] && grep -q usage "$work/out"
}

# A region truth file holds numbers only: the worktree is searched for its page id.
regions_manifest() {
  printf '{\n "page": "made-up-page-id",\n "units": "page pixels x0,y0,x1,y1",\n "diagrams": []\n}\n' \
    > "$work/eval/truth/r.regions"
  printf '[[pages]]\nid = "page"\ntext = "truth/page.txt"\nregions = "truth/r.regions"\n' \
    > "$work/eval/regions.toml"
}

region_page_id_is_checked() {
  regions_manifest
  rm -f "$repo/leak.txt"
  run "$work/eval/regions.toml" && grep -q "2 private truth lines checked" "$work/out"
}

leaked_region_page_id_fails_without_printing_it() {
  regions_manifest
  printf 'see made-up-page-id here\n' > "$repo/leak.txt"
  run "$work/eval/regions.toml"
  status=$?
  rm -f "$repo/leak.txt"
  [ "$status" -eq 1 ] && ! grep -qF "made-up-page-id" "$work/out"
}

region_numbers_are_not_searched_as_lines() {
  regions_manifest
  # The units line is long enough to be a truth line; it must not count as a leak.
  printf '"units": "page pixels x0,y0,x1,y1",\n' > "$repo/leak.txt"
  run "$work/eval/regions.toml"
  status=$?
  rm -f "$repo/leak.txt"
  [ "$status" -eq 0 ]
}

region_truth_without_page_id_fails() {
  printf '{"diagrams": []}\n' > "$work/eval/truth/nopage.regions"
  printf '[[pages]]\nid = "n"\ntext = "truth/page.txt"\nregions = "truth/nopage.regions"\n' \
    > "$work/eval/nopage.toml"
  run "$work/eval/nopage.toml"
  status=$?
  [ "$status" -eq 2 ]
}

for t in clean_worktree_passes untracked_leak_fails_without_printing_the_line \
  short_lines_alone_refuse_to_pass missing_truth_file_fails no_argument_prints_usage \
  region_page_id_is_checked leaked_region_page_id_fails_without_printing_it \
  region_numbers_are_not_searched_as_lines region_truth_without_page_id_fails; do
  "$t"
  check "$t" $?
done

if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) failed"
  exit 1
fi
echo "all leak check tests passed"
