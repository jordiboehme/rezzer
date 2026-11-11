#!/bin/bash
# Writes the assertion files of a set of PDF files: the truth the files state themselves.
# usage: truth.sh <pdf-dir> <facts-dir>
# Every .pdf under <pdf-dir> is read with the reader of the repo as it is checked out, and
# `scorecard gen all` writes <facts-dir>/<relative path>.facts from its tags, its invoice XML
# and its filled form fields. A file that gives no fact gets no assertion file.
#
# Run it once per set and keep the result: the truth must not move with the reader it scores.
# For the corpus the result is in the repo (tests/scorecard/corpus). A set whose files have no
# license record that allows it is kept outside the repo.
# The files are data: they are only read.
set -u
. "$(dirname "$0")/common.sh"
[ $# -eq 2 ] || { echo "usage: truth.sh <pdf-dir> <facts-dir>" >&2; exit 2; }
PDFS=$(cd "$1" && pwd) || exit 2
mkdir -p "$2" || exit 2
FACTS=$(cd "$2" && pwd)
SC=$(scorecard_tool)
[ -x "$SC" ] || { echo "the scorecard tool is not built" >&2; exit 1; }
mkdir -p "$MEASURE_HOME" || exit 1
READS=$(mktemp -d "$MEASURE_HOME/truth-reads.XXXXXX") || exit 1
repo_cargo run --release -p rezzer-read-pdf --example region-report -- dump "$READS" "$PDFS" || exit 1
n=0
failed=0
while IFS= read -r pdf; do
  rel=${pdf#"$PDFS"/}
  [ -f "$READS/$rel.json" ] || { echo "no read of $rel" >&2; continue; }
  mkdir -p "$(dirname "$FACTS/$rel")"
  if ! "$SC" gen all "$pdf" "$READS/$rel.json" > "$FACTS/$rel.facts"; then
    echo "the facts of $rel are not written" >&2
    failed=$((failed + 1))
    rm -f "$FACTS/$rel.facts"
  elif [ -s "$FACTS/$rel.facts" ]; then
    n=$((n + 1))
  else
    rm -f "$FACTS/$rel.facts"
  fi
done < <(find "$PDFS" -type f -iname '*.pdf' | LC_ALL=C sort)
rm -rf "$READS"
find "$FACTS" -mindepth 1 -type d -empty -delete
echo "$n assertion files in $FACTS"
[ "$failed" -eq 0 ] || { echo "$failed files failed" >&2; exit 1; }
