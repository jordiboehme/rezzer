#!/usr/bin/env bash
# Tests for version-pins.sh. No network, no toolchain. Each case builds a
# small tree (Cargo.toml, crates/rezzer/Cargo.toml, README.md,
# release-notes/) in a temp folder and runs the script against it. The last
# case runs it against the real tree.
# Usage: bash scripts/ci/test-version-pins.sh
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/version-pins.sh"
repo="$(cd "$here/../.." && pwd)"

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

tree() {
  # tree <dir> <version>: a tree that passes, for version <version>
  dir="$1"
  rm -rf "$dir"
  mkdir -p "$dir/crates/rezzer" "$dir/release-notes"
  cat >"$dir/Cargo.toml" <<EOF
[workspace]
members = ["crates/rezzer"]

[workspace.package]
version = "$2"
edition = "2024"

[workspace.dependencies]
zip = { version = "=9.1.3", default-features = false }
EOF
  cat >"$dir/crates/rezzer/Cargo.toml" <<'EOF'
[package]
name = "rezzer"
version.workspace = true
edition.workspace = true
EOF
  printf '# Rezzer\n\nOutput carries generated: { by: rezzer/%s }.\n' "$2" >"$dir/README.md"
  printf '# Greetings, programs\n\nThe first release.\n' >"$dir/release-notes/v$2.md"
}

run() {
  # run <dir>: the script's exit status, its output in $work/out
  bash "$script" "$1" >"$work/out" 2>&1
}

version_pins_passes_on_a_good_tree() {
  tree "$work/t" 1.2.3
  run "$work/t" && grep -q '1.2.3' "$work/out"
}

version_pins_fails_without_the_notes_file() {
  tree "$work/t" 1.2.3
  rm "$work/t/release-notes/v1.2.3.md"
  ! run "$work/t" && grep -q '::error file=release-notes/v1.2.3.md::' "$work/out"
}

version_pins_fails_on_an_empty_notes_file() {
  tree "$work/t" 1.2.3
  : >"$work/t/release-notes/v1.2.3.md"
  ! run "$work/t" && grep -q '::error file=release-notes/v1.2.3.md::' "$work/out"
}

version_pins_fails_when_the_notes_do_not_open_with_a_heading() {
  tree "$work/t" 1.2.3
  printf 'Greetings, programs\n' >"$work/t/release-notes/v1.2.3.md"
  ! run "$work/t" || return 1
  printf '## Greetings, programs\n' >"$work/t/release-notes/v1.2.3.md"
  ! run "$work/t" || return 1
  printf '#   \n\nText.\n' >"$work/t/release-notes/v1.2.3.md"
  ! run "$work/t"
}

version_pins_ignores_notes_of_other_versions() {
  tree "$work/t" 1.2.3
  mv "$work/t/release-notes/v1.2.3.md" "$work/t/release-notes/v1.2.2.md"
  ! run "$work/t"
}

version_pins_passes_with_a_matching_readme_archive() {
  tree "$work/t" 1.2.3
  printf 'Download rezzer-v1.2.3-linux-arm64.tar.gz or rezzer-v1.2.3-windows-amd64.zip.\n' \
    >>"$work/t/README.md"
  run "$work/t"
}

version_pins_fails_on_a_stale_readme_archive() {
  tree "$work/t" 1.2.3
  printf 'Download rezzer-v1.2.3-linux-arm64.tar.gz.\nOr rezzer-v1.2.2-macos-intel.tar.gz.\n' \
    >>"$work/t/README.md"
  ! run "$work/t" && grep -q '::error file=README.md' "$work/out" \
    && grep -q 'rezzer-v1.2.2-' "$work/out"
}

version_pins_fails_without_a_workspace_version() {
  tree "$work/t" 1.2.3
  grep -v '^version = ' "$work/t/Cargo.toml" >"$work/c" && mv "$work/c" "$work/t/Cargo.toml"
  ! run "$work/t" && grep -q '::error file=Cargo.toml::' "$work/out"
}

version_pins_reads_only_the_workspace_package_version() {
  # A version line in another section must not count as the workspace one.
  tree "$work/t" 1.2.3
  printf '\n[workspace.metadata.x]\nversion = "9.9.9"\n' >>"$work/t/Cargo.toml"
  run "$work/t" && grep -q '1.2.3' "$work/out"
}

version_pins_fails_when_rezzer_has_its_own_version() {
  tree "$work/t" 1.2.3
  cat >"$work/t/crates/rezzer/Cargo.toml" <<'EOF'
[package]
name = "rezzer"
version = "1.2.4"
EOF
  ! run "$work/t" && grep -q '::error file=crates/rezzer/Cargo.toml::' "$work/out"
}

version_pins_passes_on_the_real_tree() {
  run "$repo"
}

for t in \
  version_pins_passes_on_a_good_tree \
  version_pins_fails_without_the_notes_file \
  version_pins_fails_on_an_empty_notes_file \
  version_pins_fails_when_the_notes_do_not_open_with_a_heading \
  version_pins_ignores_notes_of_other_versions \
  version_pins_passes_with_a_matching_readme_archive \
  version_pins_fails_on_a_stale_readme_archive \
  version_pins_fails_without_a_workspace_version \
  version_pins_reads_only_the_workspace_package_version \
  version_pins_fails_when_rezzer_has_its_own_version \
  version_pins_passes_on_the_real_tree; do
  "$t"
  check "$t" "$?"
done

if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) failed"
  exit 1
fi
echo "all version-pins.sh tests passed"
