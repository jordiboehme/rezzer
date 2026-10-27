#!/usr/bin/env bash
# Usage: homebrew-formula.sh <tag> <SHA256SUMS>
#
# Prints the Homebrew formula Formula/rezzer.rb for the release <tag> (for
# example v0.1.0) to stdout. The sums come from <SHA256SUMS> as collect
# writes it (`<sum>  <name>`, or `<sum> *<name>`): one for each of the four
# archives rezzer-<tag>-{macos-arm64,macos-intel,linux-amd64,linux-arm64}.tar.gz.
#
# The tag must be v<major>.<minor>.<patch>, optionally with a pre-release
# suffix of letters, digits and dots (v0.0.0-dev in the dry run,
# v0.2.0-rc.1). The formula version is the tag without the v. A tag without
# the v, or in any other shape, is refused: the version is written into Ruby
# source, so only these characters may reach it.
#
# Each archive needs exactly one line, and its sum must be 64 lowercase hex
# characters, as sha256sum writes it. A missing line, a second line for the
# same archive or a bad sum is an error; nothing is printed then, so the
# caller never gets a formula with an empty or wrong sum.
#
# Exit 0 with the formula on stdout. Exit 1 when SHA256SUMS does not have the
# four sums, exit 2 on bad usage (arguments, tag, unreadable file). Errors go
# to stderr.
set -u
# Byte-wise ranges: [0-9a-f] must not match an upper-case letter.
export LC_ALL=C

usage() {
  echo "usage: homebrew-formula.sh <tag> <SHA256SUMS>" >&2
  exit 2
}

[ "$#" -eq 2 ] || usage
tag="$1"
sums="$2"

tag_re='^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$'
if ! [[ "$tag" =~ $tag_re ]]; then
  echo "homebrew-formula.sh: the tag must look like v1.2.3 or v1.2.3-rc.1, got '$tag'" >&2
  exit 2
fi
version="${tag#v}"

if [ ! -f "$sums" ] || [ ! -r "$sums" ]; then
  echo "homebrew-formula.sh: cannot read the checksum file '$sums'" >&2
  exit 2
fi

sum_for() {
  # sum_for <platform>: the one sum of rezzer-<tag>-<platform>.tar.gz, or an
  # error on stderr and status 1
  name="rezzer-${tag}-$1.tar.gz"
  # The name is compared as a whole string, never as a pattern. A line is
  # `<sum>` and then two characters: a space, and a space or `*`.
  found="$(awk -v name="$name" '
    {
      sep = substr($0, length($1) + 1, 2)
      if ((sep == "  " || sep == " *") && substr($0, length($1) + 3) == name) {
        n++
        s = $1
      }
    }
    END { printf "%d %s\n", n, s }
  ' "$sums")"
  count="${found%% *}"
  value="${found#* }"
  if [ "$count" -eq 0 ]; then
    echo "homebrew-formula.sh: $sums has no sum for $name" >&2
    return 1
  fi
  if [ "$count" -gt 1 ]; then
    echo "homebrew-formula.sh: $sums has $count lines for $name, expected one" >&2
    return 1
  fi
  if ! [[ "$value" =~ ^[0-9a-f]{64}$ ]]; then
    echo "homebrew-formula.sh: the sum for $name in $sums is not 64 lowercase hex characters: '$value'" >&2
    return 1
  fi
  printf '%s\n' "$value"
}

# All four first, so an error prints no formula at all.
bad=0
sha_macos_arm="$(sum_for macos-arm64)" || bad=1
sha_macos_intel="$(sum_for macos-intel)" || bad=1
sha_linux_amd64="$(sum_for linux-amd64)" || bad=1
sha_linux_arm64="$(sum_for linux-arm64)" || bad=1
[ "$bad" -eq 0 ] || exit 1

# #{version} is Ruby interpolation, written as is.
cat <<FORMULA
class Rezzer < Formula
  desc "Turns files into knowledge for AI agents"
  homepage "https://github.com/jordiboehme/rezzer"
  license "MPL-2.0"
  version "${version}"

  on_macos do
    on_arm do
      url "https://github.com/jordiboehme/rezzer/releases/download/v#{version}/rezzer-v#{version}-macos-arm64.tar.gz"
      sha256 "${sha_macos_arm}"
    end

    on_intel do
      url "https://github.com/jordiboehme/rezzer/releases/download/v#{version}/rezzer-v#{version}-macos-intel.tar.gz"
      sha256 "${sha_macos_intel}"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/jordiboehme/rezzer/releases/download/v#{version}/rezzer-v#{version}-linux-amd64.tar.gz"
      sha256 "${sha_linux_amd64}"
    end

    on_arm do
      url "https://github.com/jordiboehme/rezzer/releases/download/v#{version}/rezzer-v#{version}-linux-arm64.tar.gz"
      sha256 "${sha_linux_arm64}"
    end
  end

  def install
    bin.install "rezzer"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/rezzer --version")
  end
end
FORMULA
