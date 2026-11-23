#!/usr/bin/env bash
# Usage: release-check.sh
#        release-check.sh tag-commit <tag>
#
# Decides whether a push to main releases, and looks up where a tag points.
#
# A release is the last pull request of a version: it bumps `version` in
# [workspace.package] of the root Cargo.toml and adds
# release-notes/v<version>.md, and merging it releases. CI's release-check job
# runs this on every push to main once ci-passed is green:
#
#   bash scripts/ci/release-check.sh
#
# reads the workspace version and asks whether the tag v<version> exists on
# the remote (only there: a tag in a local clone proves nothing):
#
# - the tag exists and so does its GitHub release: the version did not
#   change since the last release, so there is nothing to release. One line,
#   release=false, exit 0.
# - the tag exists at another commit and has no GitHub release: this push
#   did not bring the version, so it releases nothing (release=false, exit
#   0), with a ::warning:: that names the tag. Either the release of that
#   commit is publishing right now (it makes the tag a moment before the
#   release), or it stopped after the tag, or the tag was pushed by hand.
#   Failing here would turn every merge in a release's publish window red.
# - the tag exists at this commit and has no GitHub release: a release of
#   this commit stopped after it made the tag, so this run releases it again
#   (as a new tag would). release.yml accepts a tag at the tested commit.
# - the tag is new and release-notes/v<version>.md is there, not empty and
#   opens with a level-one heading (the motto): release=true, exit 0.
# - the tag is new and the notes file is missing or has no heading: exit 1.
#   The version moved without its notes, so the release pull request forgot
#   them. This fails loudly instead of skipping quietly, because a quiet skip
#   would leave a version on main that never ships.
#
# A tag lookup that fails (network, a remote that does not answer) exits 1. It
# never reads as "no tag", which would start a release. A release lookup that
# fails (anything but a 404) is tried again, then exits 1, and never reads as
# "no release". $RELEASE_CHECK_TRIES (default 6) and $RELEASE_CHECK_WAIT
# (seconds between tries, default 30) set how long it waits.
#
# The release lookup goes through `gh api`, so it needs gh and a token
# (GH_TOKEN on GitHub Actions, `gh auth login` locally). The repository is
# $GITHUB_REPOSITORY on GitHub Actions, else $RELEASE_CHECK_REPO, else the
# owner/name of the remote's GitHub URL.
#
# On GitHub Actions the answer goes to $GITHUB_OUTPUT as `tag=v<version>` and
# `release=true|false`. Locally it is only printed.
#
#   bash scripts/ci/release-check.sh tag-commit v1.2.3
#
# prints the commit the tag points to on the remote (peeled, so an annotated
# tag gives its commit), or nothing when the tag does not exist; exit 1 on a
# lookup error. release.yml uses it to find a tag another run already made.
#
# The remote is `origin`, or $RELEASE_CHECK_REMOTE when set. The tree is the
# repository this script is in, or $RELEASE_CHECK_ROOT when set (the tests).
set -euo pipefail

root="${RELEASE_CHECK_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
remote="${RELEASE_CHECK_REMOTE:-origin}"

# The commit a tag points to, from the remote, peeled. Empty when the remote
# has no such tag. Returns 1 on a lookup error.
tag_commit() {
  local tag="$1" out rc=0
  out="$(git -C "$root" ls-remote --exit-code --tags "$remote" \
    "refs/tags/$tag" "refs/tags/$tag^{}")" || rc=$?
  case "$rc" in
    0) ;;
    2) return 0 ;;
    *)
      echo "::error::could not look up the tag $tag on the remote $remote (git ls-remote exit $rc)" >&2
      return 1
      ;;
  esac
  # An annotated tag lists the tag object and then the peeled commit (^{});
  # a lightweight tag lists the commit alone.
  local peeled plain
  peeled="$(awk -v r="refs/tags/$tag^{}" '$2 == r { print $1 }' <<<"$out")"
  plain="$(awk -v r="refs/tags/$tag" '$2 == r { print $1 }' <<<"$out")"
  printf '%s\n' "${peeled:-$plain}"
}

# The owner/name of the GitHub repository whose releases are looked up.
repo_slug() {
  if [ -n "${GITHUB_REPOSITORY:-}" ]; then
    printf '%s\n' "$GITHUB_REPOSITORY"
  elif [ -n "${RELEASE_CHECK_REPO:-}" ]; then
    printf '%s\n' "$RELEASE_CHECK_REPO"
  else
    local url
    url="$(git -C "$root" remote get-url "$remote")" || return 1
    url="${url%.git}"
    if [[ "$url" =~ github\.com[:/]([^/]+/[^/]+)$ ]]; then
      printf '%s\n' "${BASH_REMATCH[1]}"
    else
      echo "::error::the remote $remote ($url) is no GitHub repository; set RELEASE_CHECK_REPO to owner/name" >&2
      return 1
    fi
  fi
}

# Whether the GitHub release for a tag exists: prints yes or no (a 404). A
# failed lookup is tried again, up to $RELEASE_CHECK_TRIES times, so a
# passing API error passes. Returns 1 when the last lookup failed, which
# never reads as "no release".
release_exists() {
  local tag="$1" repo err="" rc try
  local tries="${RELEASE_CHECK_TRIES:-6}" wait="${RELEASE_CHECK_WAIT:-30}"
  case "$tries" in '' | *[!0-9]* | 0) echo "::error::RELEASE_CHECK_TRIES must be a whole number above 0, not '$tries'" >&2; return 1 ;; esac
  case "$wait" in '' | *[!0-9]*) echo "::error::RELEASE_CHECK_WAIT must be a whole number, not '$wait'" >&2; return 1 ;; esac
  repo="$(repo_slug)" || return 1
  if ! command -v gh >/dev/null 2>&1; then
    echo "::error::gh is not installed, so the release $tag cannot be looked up" >&2
    return 1
  fi
  for ((try = 1; ; try++)); do
    rc=0
    err="$(gh api --silent "repos/$repo/releases/tags/$tag" 2>&1 >/dev/null)" || rc=$?
    if [ "$rc" -eq 0 ]; then
      echo yes
      return 0
    fi
    if grep -q 'HTTP 404' <<<"$err"; then
      echo no
      return 0
    fi
    if [ "$try" -ge "$tries" ]; then
      break
    fi
    echo "release: the lookup of the release $tag in $repo failed (try $try of $tries), trying again in ${wait}s" >&2
    sleep "$wait"
  done
  echo "::error::could not look up the release $tag in $repo (gh api exit $rc: $err)" >&2
  return 1
}

output() {
  if [ -n "${GITHUB_OUTPUT:-}" ]; then
    printf '%s=%s\n' "$1" "$2" >>"$GITHUB_OUTPUT"
  fi
}

if [ "${1:-}" = "tag-commit" ]; then
  [ "$#" -eq 2 ] && [ -n "$2" ] || { echo "usage: $0 tag-commit <tag>" >&2; exit 2; }
  tag_commit "$2"
  exit 0
fi
[ "$#" -eq 0 ] || { echo "usage: $0 [tag-commit <tag>]" >&2; exit 2; }

# `version = "x.y.z"` inside [workspace.package], up to the next section: the
# same read as version-pins.sh.
version="$(awk '
  /^\[/ { inside = ($0 ~ /^\[workspace\.package\][[:space:]]*$/) ; next }
  inside && /^version[[:space:]]*=/ {
    sub(/^version[[:space:]]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit
  }' "$root/Cargo.toml")"
# A pre-release such as 0.2.0-rc.1 releases too; release.yml marks it as one.
if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "::error file=Cargo.toml::no version of the form X.Y.Z or X.Y.Z-pre in [workspace.package] of Cargo.toml (found '$version')" >&2
  exit 1
fi
tag="v$version"
output tag "$tag"

commit="$(tag_commit "$tag")"
if [ -n "$commit" ]; then
  released="$(release_exists "$tag")"
  if [ "$released" = yes ]; then
    echo "release: $tag is already tagged on $remote (at $commit) and released, nothing to release"
    output release false
    exit 0
  fi
  head="$(git -C "$root" rev-parse HEAD)"
  if [ "$commit" != "$head" ]; then
    echo "::warning::the tag $tag is at $commit, not at this commit, and has no GitHub release yet. This push does not change the version, so it releases nothing. If the release run of $commit is still publishing, nothing is to do. If it failed, re-run its failed jobs. If the tag was pushed by hand, an admin deletes it with 'git push origin :refs/tags/$tag' and re-runs the release-check job of the commit that should release $tag."
    output release false
    exit 0
  fi
  echo "release: $tag is at this commit but has no GitHub release, so this run releases it again"
fi

notes="release-notes/$tag.md"
if [ ! -s "$root/$notes" ]; then
  echo "::error file=$notes::the version moved to $version but $notes is missing or empty. Add the notes on main to release $tag." >&2
  exit 1
fi
if ! head -n1 "$root/$notes" | grep -qE '^# .*[^[:space:]]'; then
  echo "::error file=$notes::$notes must open with a level-one heading ('# ' and the release motto)." >&2
  exit 1
fi

echo "release: $tag is new and $notes is in place, so this commit releases $tag"
output release true
