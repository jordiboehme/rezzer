#!/usr/bin/env bash
# Usage: version-pins.sh [root]
#
# Checks that the files which name the release version agree with it, so a
# release PR fails here and not at the tag:
#
# - the version is `version` in [workspace.package] of the root Cargo.toml,
#   and crates/rezzer/Cargo.toml takes it with `version.workspace = true`
#   (the binary's --version and the tag come from there)
# - release-notes/v<version>.md exists, is not empty and opens with a
#   level-one heading, the release motto. The release workflow's `notes` job
#   checks the same on the tag and publishes the file as the release page
# - every archive name `rezzer-v<x.y.z>-` in README.md names this version. A
#   README that names no archive passes
#
# Reads the files only: no cargo, no network. root defaults to the repository
# this script is in. Exit 1 with an ::error:: line for each problem.
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"

# `version = "x.y.z"` inside [workspace.package], up to the next section.
version="$(awk '
  /^\[/ { inside = ($0 ~ /^\[workspace\.package\][[:space:]]*$/) ; next }
  inside && /^version[[:space:]]*=/ {
    sub(/^version[[:space:]]*=[[:space:]]*"/, ""); sub(/".*$/, ""); print; exit
  }' "$root/Cargo.toml")"
if [ -z "$version" ]; then
  echo "::error file=Cargo.toml::Cargo.toml has no version in [workspace.package]." >&2
  exit 1
fi

bad=0

if ! grep -qE '^version\.workspace[[:space:]]*=[[:space:]]*true' "$root/crates/rezzer/Cargo.toml"; then
  echo "::error file=crates/rezzer/Cargo.toml::crates/rezzer/Cargo.toml must take its version from the workspace (version.workspace = true), so the binary, the tag and the release notes name the same version $version." >&2
  bad=1
fi

notes="release-notes/v$version.md"
if [ ! -s "$root/$notes" ]; then
  echo "::error file=$notes::$notes is missing or empty. The workspace version is $version, and the release workflow publishes this file as the release page. Add it before the tag." >&2
  bad=1
elif ! head -n1 "$root/$notes" | grep -qE '^# .*[^[:space:]]'; then
  echo "::error file=$notes::$notes must open with a level-one heading ('# ' and the release motto)." >&2
  bad=1
fi

archives="$(grep -oE 'rezzer-v[0-9]+\.[0-9]+\.[0-9]+-' "$root/README.md" | sort -u || true)"
while read -r name; do
  [ -n "$name" ] || continue
  if [ "$name" != "rezzer-v$version-" ]; then
    echo "::error file=README.md::README.md names the archive ${name}... but the workspace version is $version. Update the README with the version bump." >&2
    bad=1
  fi
done <<<"$archives"

[ "$bad" = "0" ] || exit 1
count="$(printf '%s' "$archives" | grep -c . || true)"
echo "version pins: $version (Cargo.toml, crates/rezzer, $notes, $count archive name(s) in README.md)"
