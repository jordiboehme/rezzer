#!/bin/bash
# P6-X measure of one commit of the PDF reader (plan 7, Task 19a): a regression measure
# against the dumps of an earlier commit. The scorecard (scorecard.sh) is the measure against
# truth.
# usage: p6x.sh <commit> [stage...]   stages: tests dumps analyze (default: all three)
#  tests   : cargo nextest run -p rezzer-read-pdf --no-fail-fast in a worktree at <commit>
#  dumps   : release dumps of the corpus, the forms and the generated variants
#  analyze : triage of the failures by bucket, word check against the base dumps, summary
# Output: $MEASURE_HOME/p6x-<short>/ (tests.log, triage.tsv, dump/, engine/ (engine reports),
#         kind.md, cover.md, *-dump.log, cmp.log, words.md, facts.md, links.md, order.md,
#         summary.md) and the full report $MEASURE_HOME/p6x-<short>.md
# The triage reads the helper (the function a panic stands in) from the worktree's source, so
# the analyze stage needs the worktree at <commit> (the tests and dumps stages put it there).
#
# Besides the environment of common.sh:
#   P6X_BASE   the dumps of the commit to compare with (a folder with corpus/, forms/ and
#              generated/, as the dumps stage writes its dump/); needed by dumps and analyze
#   P6X_ACCEPTED  a folder with the accepted lists of the checks (words.json, links.json,
#              order.json: lists of {"key": [...], "why": "..."}). They name files and quote
#              text of the measured sets, so they are kept beside the sets and never in the
#              repo. Unset: nothing is accepted
set -u
. "$(dirname "$0")/common.sh"
[ $# -ge 1 ] || { echo "usage: p6x.sh <commit> [tests] [dumps] [analyze]" >&2; exit 2; }
COMMIT=$1
C=$(git -C "$REPO" rev-parse --short "$COMMIT") || exit 2
shift
STAGES="${*:-tests dumps analyze}"
OUT=$MEASURE_HOME/p6x-$C
mkdir -p "$OUT"
FORMS=${MEASURE_FORMS:-}

# The stages that compare need the base dumps.
need_base() {
  [ -n "${P6X_BASE:-}" ] && [ -d "$P6X_BASE" ] || {
    echo "set P6X_BASE to the dumps to compare with" >&2
    exit 2
  }
}

# The worktree at the commit, with the corpus files of the repo that git does not track (the
# large files fetched by scripts/fetch-corpus.sh), so that they are dumped and tested too.
measured_tree() {
  worktree "$COMMIT"
  git -C "$REPO" status --ignored --porcelain tests/corpus | sed -n 's/^!! //p' | while IFS= read -r f; do
    [ -f "$REPO/$f" ] && [ ! -e "$WT/$f" ] && cp "$REPO/$f" "$WT/$f"
  done
}

for st in $STAGES; do case $st in
tests)
  measured_tree
  wt_cargo nextest run -p rezzer-read-pdf --no-fail-fast > "$OUT/tests.log" 2>&1
  echo "tests done: $(tail -3 "$OUT/tests.log" | tr '\n' ' ')"
  ;;
dumps)
  need_base
  measured_tree
  O=$OUT/dump
  E=$OUT/engine
  rm -rf "$O" "$E"; mkdir -p "$O/corpus" "$O/forms" "$O/generated" "$E/corpus" "$E/forms" "$E/generated"
  REZZER_ENGINE_DIR=$E/corpus wt_cargo run --release -p rezzer-read-pdf --example region-report -- dump "$O/corpus" tests/corpus/pdf > "$OUT/corpus-dump.log" 2>&1
  if [ -n "$FORMS" ]; then
    REZZER_ENGINE_DIR=$E/forms wt_cargo run --release -p rezzer-read-pdf --example region-report -- dump "$O/forms" "$FORMS" > "$OUT/forms-dump.log" 2>&1
  fi
  REZZER_ENGINE_DIR=$E/generated REZZER_DUMP_DIR=$O/generated wt_cargo nextest run --release -p rezzer-read-pdf --test variants --run-ignored all dump_generated_reads > "$OUT/gen-dump.log" 2>&1
  for d in corpus forms generated; do
    echo "$d $(find "$O/$d" -type f | wc -l) diff:$(diff -rq "$P6X_BASE/$d" "$O/$d" | wc -l)"
  done > "$OUT/cmp.log"
  cat "$OUT/cmp.log"
  ;;
analyze)
  need_base
  measured_tree
  python3 -I "$SCRIPTS/p6x_triage.py" "$OUT/tests.log" "$WT" > "$OUT/triage.tsv"
  python3 -I "$SCRIPTS/p6x_words.py" "$P6X_BASE" "$OUT/dump" > "$OUT/words.md"
  python3 -I "$SCRIPTS/p6x_facts.py" "$P6X_BASE" "$OUT/dump" > "$OUT/facts.md"
  python3 -I "$SCRIPTS/p6x_links.py" "$P6X_BASE" "$OUT/dump" > "$OUT/links.md"
  python3 -I "$SCRIPTS/p6x_order.py" "$P6X_BASE" "$OUT/dump" > "$OUT/order.md"
  if [ -d "$OUT/engine" ]; then
    python3 -I "$SCRIPTS/p6x_kind.py" "$OUT/engine" > "$OUT/kind.md"
    python3 -I "$SCRIPTS/p6x_cover.py" "$OUT/engine" > "$OUT/cover.md"
  fi
  python3 -I "$SCRIPTS/p6x_summary.py" "$C" "$OUT" > "$OUT/summary.md"
  python3 -I "$SCRIPTS/p6x_summary.py" "$C" "$OUT" full > "$MEASURE_HOME/p6x-$C.md"
  cat "$OUT/summary.md"
  echo "full output: $MEASURE_HOME/p6x-$C.md"
  ;;
*) echo "unknown stage $st" >&2; exit 2;;
esac; done
