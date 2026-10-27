#!/usr/bin/env bash
# Tests for homebrew-formula.sh. No network. Needs ruby for the syntax check
# (`ruby -c`); without ruby those tests fail, they are not skipped.
# Usage: bash scripts/ci/test-homebrew-formula.sh
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/homebrew-formula.sh"

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

# One distinct sum per file, 64 lowercase hex characters each.
sum() {
  # sum <digit or letter a-f>: that character 64 times
  printf "%064d" 0 | tr 0 "$1"
}
SUM_MACOS_ARM="$(sum 1)"
SUM_MACOS_INTEL="$(sum 2)"
SUM_LINUX_AMD="$(sum 3)"
SUM_LINUX_ARM="$(sum 4)"

sums_file() {
  # sums_file <tag> <file>: SHA256SUMS in the shape collect writes (all
  # archives and SBOMs, `<sum>  <name>`), each line with its own sum
  tag="$1"
  {
    printf '%s  rezzer-%s-linux-amd64.cdx.json\n' "$(sum 5)" "$tag"
    printf '%s  rezzer-%s-linux-amd64.tar.gz\n' "$SUM_LINUX_AMD" "$tag"
    printf '%s  rezzer-%s-linux-arm64.cdx.json\n' "$(sum 6)" "$tag"
    printf '%s  rezzer-%s-linux-arm64.tar.gz\n' "$SUM_LINUX_ARM" "$tag"
    printf '%s  rezzer-%s-macos-arm64.cdx.json\n' "$(sum 7)" "$tag"
    printf '%s  rezzer-%s-macos-arm64.tar.gz\n' "$SUM_MACOS_ARM" "$tag"
    printf '%s  rezzer-%s-macos-intel.cdx.json\n' "$(sum 8)" "$tag"
    printf '%s  rezzer-%s-macos-intel.tar.gz\n' "$SUM_MACOS_INTEL" "$tag"
    printf '%s  rezzer-%s-windows-amd64.cdx.json\n' "$(sum 9)" "$tag"
    printf '%s  rezzer-%s-windows-amd64.zip\n' "$(sum a)" "$tag"
    printf '%s  rezzer-%s-windows-arm64.cdx.json\n' "$(sum b)" "$tag"
    printf '%s  rezzer-%s-windows-arm64.zip\n' "$(sum c)" "$tag"
  } >"$2"
}

blocks() {
  # blocks <formula>: one line "<os> <arch> <platform> <sum>" for each url
  # line, with the on_* blocks it sits in and the sha256 on the next line
  awk '
    /^ *on_(macos|linux) do$/ { os = $1; sub(/^on_/, "", os); next }
    /^ *on_(arm|intel) do$/ { arch = $1; sub(/^on_/, "", arch); next }
    /^ *end$/ { if (arch != "") arch = ""; else os = ""; next }
    /^ *url "/ {
      platform = $2
      sub(/^"https:\/\/github\.com\/jordiboehme\/rezzer\/releases\/download\/v#\{version\}\/rezzer-v#\{version\}-/, "", platform)
      sub(/\.tar\.gz"$/, "", platform)
      pending = os " " arch " " platform
      next
    }
    /^ *sha256 "/ && pending != "" {
      s = $2; gsub(/"/, "", s)
      print pending " " s
      pending = ""
    }
  ' "$1"
}

render_ok() {
  # render_ok <tag> <sums>: exit 0, formula on stdout, nothing on stderr
  bash "$script" "$1" "$2" >"$work/out" 2>"$work/err" && [ -s "$work/out" ] && [ ! -s "$work/err" ]
}

render_fails() {
  # render_fails <status> <message part> <tag> <sums>: that exit status, the
  # message on stderr and nothing on stdout (never a half formula)
  want="$1"
  msg="$2"
  shift 2
  bash "$script" "$@" >"$work/out" 2>"$work/err"
  status=$?
  [ "$status" -eq "$want" ] && [ ! -s "$work/out" ] && grep -qF -- "$msg" "$work/err"
}

formula_pairs_each_block_with_its_archive_and_sum() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  render_ok v0.1.0 "$work/SHA256SUMS" || return 1
  blocks "$work/out" >"$work/blocks"
  printf '%s\n' \
    "macos arm macos-arm64 $SUM_MACOS_ARM" \
    "macos intel macos-intel $SUM_MACOS_INTEL" \
    "linux intel linux-amd64 $SUM_LINUX_AMD" \
    "linux arm linux-arm64 $SUM_LINUX_ARM" >"$work/want"
  diff -u "$work/want" "$work/blocks"
}

formula_has_the_fixed_fields() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  render_ok v0.1.0 "$work/SHA256SUMS" || return 1
  f="$work/out"
  grep -qx 'class Rezzer < Formula' "$f" \
    && grep -qx '  desc "Turns files into knowledge for AI agents"' "$f" \
    && grep -qx '  homepage "https://github.com/jordiboehme/rezzer"' "$f" \
    && grep -qx '  license "MPL-2.0"' "$f" \
    && grep -qx '  version "0.1.0"' "$f" \
    && grep -qx '    bin.install "rezzer"' "$f" \
    && grep -qxF '    assert_match version.to_s, shell_output("#{bin}/rezzer --version")' "$f" \
    && [ "$(grep -c '^ *url "' "$f")" -eq 4 ] \
    && [ "$(grep -c '^ *sha256 "' "$f")" -eq 4 ] \
    && ! grep -q 'windows\|cdx\.json' "$f"
}

formula_passes_ruby_syntax_check() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  render_ok v0.1.0 "$work/SHA256SUMS" || return 1
  cp "$work/out" "$work/rezzer.rb"
  ruby -c "$work/rezzer.rb" >/dev/null
}

formula_renders_the_dry_run_version() {
  # The dispatch dry run renders with v0.0.0-dev.
  sums_file v0.0.0-dev "$work/SHA256SUMS"
  render_ok v0.0.0-dev "$work/SHA256SUMS" || return 1
  cp "$work/out" "$work/rezzer.rb"
  grep -qx '  version "0.0.0-dev"' "$work/rezzer.rb" && ruby -c "$work/rezzer.rb" >/dev/null
}

formula_accepts_binary_mode_lines() {
  # sha256sum -b writes `<sum> *<name>`.
  sums_file v0.1.0 "$work/SHA256SUMS"
  sed 's/  rezzer-/ *rezzer-/' "$work/SHA256SUMS" >"$work/SHA256SUMS.b"
  render_ok v0.1.0 "$work/SHA256SUMS.b" || return 1
  [ "$(blocks "$work/out" | wc -l | tr -d ' ')" -eq 4 ]
}

formula_rejects_a_missing_sum() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  grep -v 'linux-arm64\.tar\.gz' "$work/SHA256SUMS" >"$work/SHA256SUMS.m"
  render_fails 1 'rezzer-v0.1.0-linux-arm64.tar.gz' v0.1.0 "$work/SHA256SUMS.m"
}

formula_rejects_sums_of_another_version() {
  sums_file v0.0.9 "$work/SHA256SUMS"
  render_fails 1 'rezzer-v0.1.0-macos-arm64.tar.gz' v0.1.0 "$work/SHA256SUMS"
}

formula_rejects_a_name_that_only_contains_the_file() {
  # A line for a longer name must not count as the archive's line.
  sums_file v0.1.0 "$work/SHA256SUMS"
  grep -v 'macos-intel\.tar\.gz' "$work/SHA256SUMS" >"$work/SHA256SUMS.x"
  printf '%s  rezzer-v0.1.0-macos-intel.tar.gz.old\n' "$(sum d)" >>"$work/SHA256SUMS.x"
  render_fails 1 'rezzer-v0.1.0-macos-intel.tar.gz' v0.1.0 "$work/SHA256SUMS.x"
}

formula_rejects_a_duplicate_line() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  printf '%s  rezzer-v0.1.0-macos-intel.tar.gz\n' "$(sum e)" >>"$work/SHA256SUMS"
  render_fails 1 'rezzer-v0.1.0-macos-intel.tar.gz' v0.1.0 "$work/SHA256SUMS"
}

formula_rejects_a_bad_sum() {
  # bad_sum_case <replacement sum>: the linux-amd64 sum replaced
  bad_sum_case() {
    sums_file v0.1.0 "$work/SHA256SUMS"
    sed "s/^$SUM_LINUX_AMD /$1 /" "$work/SHA256SUMS" >"$work/SHA256SUMS.s"
    render_fails 1 'rezzer-v0.1.0-linux-amd64.tar.gz' v0.1.0 "$work/SHA256SUMS.s"
  }
  short="$(printf "%063d" 0 | tr 0 3)"
  upper="$(printf "%064d" 0 | tr 0 A)"
  nothex="$(printf "%064d" 0 | tr 0 g)"
  bad_sum_case "$short" && bad_sum_case "$upper" && bad_sum_case "$nothex"
}

formula_rejects_a_bad_tag() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  render_fails 2 'tag' 0.1.0 "$work/SHA256SUMS" \
    && render_fails 2 'tag' v0.1 "$work/SHA256SUMS" \
    && render_fails 2 'tag' vfoo "$work/SHA256SUMS" \
    && render_fails 2 'tag' 'v0.1.0"; system("x' "$work/SHA256SUMS" \
    && render_fails 2 'tag' 'v0.1.0-' "$work/SHA256SUMS" \
    && render_fails 2 'tag' '' "$work/SHA256SUMS"
}

formula_rejects_bad_usage() {
  sums_file v0.1.0 "$work/SHA256SUMS"
  render_fails 2 'usage: ' \
    && render_fails 2 'usage: ' v0.1.0 \
    && render_fails 2 'usage: ' v0.1.0 "$work/SHA256SUMS" extra \
    && render_fails 2 "$work/none" v0.1.0 "$work/none"
}

for t in formula_pairs_each_block_with_its_archive_and_sum \
  formula_has_the_fixed_fields formula_passes_ruby_syntax_check \
  formula_renders_the_dry_run_version formula_accepts_binary_mode_lines \
  formula_rejects_a_missing_sum formula_rejects_sums_of_another_version \
  formula_rejects_a_name_that_only_contains_the_file \
  formula_rejects_a_duplicate_line formula_rejects_a_bad_sum \
  formula_rejects_a_bad_tag formula_rejects_bad_usage; do
  "$t"
  check "$t" $?
done

if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) failed"
  exit 1
fi
echo "all tests passed"
