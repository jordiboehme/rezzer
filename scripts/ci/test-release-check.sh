#!/usr/bin/env bash
# Tests for release-check.sh. No network, no gh login. Each case builds a
# small git repository whose remote is a local bare repository, and puts a
# stub `gh` first on PATH that answers the release lookup from a file.
# Usage: bash scripts/ci/test-release-check.sh
set -u

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
script="$here/release-check.sh"

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

# The stub gh: `gh api --silent repos/<repo>/releases/tags/<tag>` passes when
# <tag> is a line of $work/releases, prints an HTTP 404 when it is not, and
# fails with another error when $work/gh-broken exists.
mkdir -p "$work/bin"
cat >"$work/bin/gh" <<'EOF'
#!/usr/bin/env bash
[ ! -e "$STUB_DIR/gh-broken" ] || { echo "HTTP 502: Bad Gateway" >&2; exit 1; }
tag="${3##*/}"
if grep -qxF -- "$tag" "$STUB_DIR/releases" 2>/dev/null; then
  exit 0
fi
echo "gh: Not Found (HTTP 404)" >&2
exit 1
EOF
chmod +x "$work/bin/gh"
export STUB_DIR="$work"
export PATH="$work/bin:$PATH"
export RELEASE_CHECK_TRIES=2 RELEASE_CHECK_WAIT=0 RELEASE_CHECK_REPO=owner/rezzer
unset GITHUB_REPOSITORY GITHUB_OUTPUT

g() {
  git -c user.name=test -c user.email=test@example.invalid -c commit.gpgsign=false \
    -c tag.gpgsign=false "$@"
}

repo() {
  # repo <version>: $work/r with one commit at <version> and its notes, and
  # the bare remote $work/remote.git with that commit on main
  rm -rf "$work/r" "$work/remote.git" "$work/releases" "$work/gh-broken" "$work/output"
  : >"$work/releases"
  g init -q --bare "$work/remote.git"
  g init -q -b main "$work/r"
  mkdir -p "$work/r/release-notes"
  cat >"$work/r/Cargo.toml" <<EOF
[workspace]
members = ["crates/rezzer"]

[workspace.package]
version = "$1"
edition = "2024"

[workspace.dependencies]
zip = { version = "=9.1.3", default-features = false }
EOF
  printf '# Greetings, programs\n\nThe release.\n' >"$work/r/release-notes/v$1.md"
  g -C "$work/r" add -A
  g -C "$work/r" commit -q -m init
  g -C "$work/r" remote add origin "$work/remote.git"
  g -C "$work/r" push -q origin main
}

run() {
  # run [args]: the script's exit status, its output in $work/out
  RELEASE_CHECK_ROOT="$work/r" bash "$script" "$@" >"$work/out" 2>&1
}

release_check_releases_a_new_version_with_notes() {
  repo 1.2.3
  : >"$work/output"
  GITHUB_OUTPUT="$work/output" run || return 1
  grep -qx 'tag=v1.2.3' "$work/output" && grep -qx 'release=true' "$work/output"
}

release_check_releases_a_pre_release() {
  repo 1.2.3-rc.1
  : >"$work/output"
  GITHUB_OUTPUT="$work/output" run || return 1
  grep -qx 'tag=v1.2.3-rc.1' "$work/output" && grep -qx 'release=true' "$work/output"
}

release_check_skips_a_released_version_tagged_on_the_remote() {
  repo 1.2.3
  g -C "$work/r" tag -a v1.2.3 -m v1.2.3
  g -C "$work/r" push -q origin v1.2.3
  g -C "$work/r" tag -d v1.2.3 >/dev/null
  echo v1.2.3 >"$work/releases"
  : >"$work/output"
  GITHUB_OUTPUT="$work/output" run || return 1
  grep -qx 'release=false' "$work/output" && grep -q 'on origin' "$work/out"
}

release_check_skips_a_released_version_tagged_here() {
  repo 1.2.3
  g -C "$work/r" tag v1.2.3
  echo v1.2.3 >"$work/releases"
  run && grep -q 'already tagged here and released' "$work/out"
}

release_check_skips_without_notes_when_released() {
  # The notes check is for a new version only.
  repo 1.2.3
  rm "$work/r/release-notes/v1.2.3.md"
  g -C "$work/r" tag v1.2.3
  echo v1.2.3 >"$work/releases"
  run
}

release_check_fails_on_a_stray_tag() {
  repo 1.2.3
  g -C "$work/r" tag v1.2.3
  g -C "$work/r" push -q origin v1.2.3
  ! run && grep -q 'exists but there is no GitHub release' "$work/out" \
    && grep -q 'try 1 of 2' "$work/out"
}

release_check_fails_when_the_release_lookup_fails() {
  repo 1.2.3
  g -C "$work/r" tag v1.2.3
  touch "$work/gh-broken"
  ! run && grep -q 'could not look up the release v1.2.3' "$work/out" \
    && ! grep -q 'no GitHub release' "$work/out"
}

release_check_fails_without_notes() {
  repo 1.2.3
  rm "$work/r/release-notes/v1.2.3.md"
  ! run && grep -q '::error file=release-notes/v1.2.3.md::' "$work/out"
}

release_check_fails_on_empty_notes() {
  repo 1.2.3
  : >"$work/r/release-notes/v1.2.3.md"
  ! run && grep -q '::error file=release-notes/v1.2.3.md::' "$work/out"
}

release_check_fails_when_the_notes_do_not_open_with_a_heading() {
  repo 1.2.3
  printf '## Greetings, programs\n' >"$work/r/release-notes/v1.2.3.md"
  ! run || return 1
  printf '#   \n\nText.\n' >"$work/r/release-notes/v1.2.3.md"
  ! run && grep -q 'level-one heading' "$work/out"
}

release_check_fails_when_the_remote_does_not_answer() {
  # A failed tag lookup must never read as "no tag".
  repo 1.2.3
  ! RELEASE_CHECK_REMOTE="$work/missing.git" run \
    && grep -q 'could not look up the tag v1.2.3' "$work/out"
}

release_check_reads_only_the_workspace_package_version() {
  repo 1.2.3
  printf '\n[workspace.metadata.x]\nversion = "9.9.9"\n' >>"$work/r/Cargo.toml"
  run && grep -q 'releases v1.2.3' "$work/out"
}

release_check_fails_without_a_version() {
  repo 1.2.3
  grep -v '^version = ' "$work/r/Cargo.toml" >"$work/c" && mv "$work/c" "$work/r/Cargo.toml"
  ! run && grep -q '::error file=Cargo.toml::' "$work/out"
}

release_check_refuses_a_bad_retry_count() {
  repo 1.2.3
  g -C "$work/r" tag v1.2.3
  ! RELEASE_CHECK_TRIES=x run && grep -q 'RELEASE_CHECK_TRIES' "$work/out" || return 1
  ! RELEASE_CHECK_TRIES=0 run && grep -q 'RELEASE_CHECK_TRIES' "$work/out" || return 1
  ! RELEASE_CHECK_WAIT=1.5 run && grep -q 'RELEASE_CHECK_WAIT' "$work/out"
}

tag_commit_peels_an_annotated_tag() {
  repo 1.2.3
  g -C "$work/r" tag -a v1.2.3 -m v1.2.3
  g -C "$work/r" push -q origin v1.2.3
  want="$(g -C "$work/r" rev-parse HEAD)"
  run tag-commit v1.2.3 && [ "$(cat "$work/out")" = "$want" ]
}

tag_commit_reads_a_lightweight_tag() {
  repo 1.2.3
  g -C "$work/r" tag v1.2.3
  g -C "$work/r" push -q origin v1.2.3
  want="$(g -C "$work/r" rev-parse HEAD)"
  run tag-commit v1.2.3 && [ "$(cat "$work/out")" = "$want" ]
}

tag_commit_prints_nothing_for_a_missing_tag() {
  repo 1.2.3
  run tag-commit v1.2.3 && [ ! -s "$work/out" ]
}

tag_commit_fails_when_the_remote_does_not_answer() {
  repo 1.2.3
  ! RELEASE_CHECK_REMOTE="$work/missing.git" run tag-commit v1.2.3
}

release_check_refuses_unknown_arguments() {
  repo 1.2.3
  ! run something || return 1
  ! run tag-commit
}

for t in \
  release_check_releases_a_new_version_with_notes \
  release_check_releases_a_pre_release \
  release_check_skips_a_released_version_tagged_on_the_remote \
  release_check_skips_a_released_version_tagged_here \
  release_check_skips_without_notes_when_released \
  release_check_fails_on_a_stray_tag \
  release_check_fails_when_the_release_lookup_fails \
  release_check_fails_without_notes \
  release_check_fails_on_empty_notes \
  release_check_fails_when_the_notes_do_not_open_with_a_heading \
  release_check_fails_when_the_remote_does_not_answer \
  release_check_reads_only_the_workspace_package_version \
  release_check_fails_without_a_version \
  release_check_refuses_a_bad_retry_count \
  tag_commit_peels_an_annotated_tag \
  tag_commit_reads_a_lightweight_tag \
  tag_commit_prints_nothing_for_a_missing_tag \
  tag_commit_fails_when_the_remote_does_not_answer \
  release_check_refuses_unknown_arguments; do
  "$t"
  check "$t" "$?"
done

if [ "$failures" -ne 0 ]; then
  echo "$failures test(s) failed"
  exit 1
fi
echo "all release-check.sh tests passed"
