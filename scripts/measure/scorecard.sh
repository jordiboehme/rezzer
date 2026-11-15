#!/bin/bash
# The scorecard of one commit: how many facts about the public sets its reads hold.
# usage: scorecard.sh <commit> [out-dir]
#
# Builds a release of <commit> in a worktree, reads every public set to model JSON, checks the
# assertion files against it and writes, in <out-dir> (default $MEASURE_HOME/scorecard-<commit>):
#   scorecard-<commit>.tsv   set  half  class  passed  total  unread_files
#   regions-<commit>.tsv     set  half  kind  regions  facts_passed  facts_total
#   scorecard-<commit>.md    a short summary of both
#   check-<set>.txt          per file and class, with the failing facts
#
# The sets:
#   corpus            tests/corpus/pdf, against tests/scorecard/corpus
#   corpus-untagged   the tagged files of the corpus with their tags taken out, against the
#                     assertion files of those files: a lower bound for the layout engine
#   forms, forms-untagged   the same for the folder in MEASURE_FORMS, when it is set. Its
#                     assertion files are made once (truth.sh) and kept in
#                     $MEASURE_HOME/truth/forms, outside the repo
#   generated         the generated variants, read at <commit>, against what the variants of
#                     the repo as it is checked out must read as
# Every set is split in a `dev` and a `held` half by a hash of the file name.
#
# The files of the corpus and the tools are taken from the repo as it is checked out, so two
# commits are scored on the same files with the same truth. See common.sh for the environment.
#
# An assertion file with no read is counted in the tables (unread_files): its facts are in no
# count. The script then still writes everything and exits with 4, so that a read that went
# missing (a reader that stopped, tags that were not taken out) is never a better score.
set -u
. "$(dirname "$0")/common.sh"
[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: scorecard.sh <commit> [out-dir]" >&2; exit 2; }
C=$(git -C "$REPO" rev-parse --short=8 "$1^{commit}") || exit 2
OUT=${2:-$MEASURE_HOME/scorecard-$C}
mkdir -p "$OUT" || exit 2
OUT=$(cd "$OUT" && pwd)
FORMS=${MEASURE_FORMS:-}
[ -n "$FORMS" ] && [ ! -d "$FORMS" ] && { echo "no folder $FORMS: the forms set is left out" >&2; FORMS=; }

SC=$(scorecard_tool)
[ -x "$SC" ] || { echo "the scorecard tool is not built" >&2; exit 1; }
worktree "$1"
rm -rf "$OUT/dump" "$OUT/stripped" "$OUT/truth"
mkdir -p "$OUT/dump" "$OUT/truth"

# Reads the PDF files under $2 to $OUT/dump/$1.
dump() {
  mkdir -p "$OUT/dump/$1"
  wt_cargo run --release -p rezzer-read-pdf --example region-report -- dump "$OUT/dump/$1" "$2" \
    > "$OUT/dump-$1.log" 2>&1 || { echo "the dump of $1 failed, see $OUT/dump-$1.log" >&2; exit 1; }
}

# Writes the tagged PDF files under $2 without their tags to $OUT/stripped/$1, and the
# assertion files of those files (from $3) to $OUT/truth/$1-untagged: the truth of the
# untagged set. A tagged file whose tags are not taken out keeps its assertion file there, so
# it shows as an assertion file with no read.
strip() {
  mkdir -p "$OUT/stripped/$1" "$OUT/truth/$1-untagged"
  while IFS= read -r pdf; do
    rel=${pdf#"$2"/}
    mkdir -p "$(dirname "$OUT/stripped/$1/$rel")"
    "$SC" strip-tags "$pdf" "$OUT/stripped/$1/$rel" 2>> "$OUT/strip-$1.log"
    case $? in
      3) continue ;;
      0) ;;
      *) echo "tags not taken out of $rel, see $OUT/strip-$1.log" >&2 ;;
    esac
    if [ -f "$3/$rel.facts" ]; then
      mkdir -p "$(dirname "$OUT/truth/$1-untagged/$rel")"
      cp "$3/$rel.facts" "$OUT/truth/$1-untagged/$rel.facts"
    fi
  done < <(find "$2" -type f -iname '*.pdf' | LC_ALL=C sort)
}

SETS=
CORPUS=$REPO/tests/corpus/pdf
TRUTH_corpus=$REPO/tests/scorecard/corpus
dump corpus "$CORPUS"
strip corpus "$CORPUS" "$TRUTH_corpus"
dump corpus-untagged "$OUT/stripped/corpus"
SETS="corpus corpus-untagged"
if [ -n "$FORMS" ]; then
  FORMS=$(cd "$FORMS" && pwd)
  TRUTH_forms=$MEASURE_HOME/truth/forms
  [ -d "$TRUTH_forms" ] || "$SCRIPTS/truth.sh" "$FORMS" "$TRUTH_forms" || exit 1
  dump forms "$FORMS"
  strip forms "$FORMS" "$TRUTH_forms"
  dump forms-untagged "$OUT/stripped/forms"
  SETS="$SETS forms forms-untagged"
fi
REZZER_DUMP_DIR=$OUT/dump/generated wt_cargo nextest run --release -p rezzer-read-pdf \
  --test variants --run-ignored only dump_generated_reads > "$OUT/dump-generated.log" 2>&1 \
  || { echo "the dump of generated failed, see $OUT/dump-generated.log" >&2; exit 1; }
REZZER_WANTS_DIR=$OUT/truth/generated repo_cargo nextest run -p rezzer-read-pdf \
  --test variants --run-ignored only dump_generated_wants > "$OUT/truth-generated.log" 2>&1 \
  || { echo "the truth of generated failed, see $OUT/truth-generated.log" >&2; exit 1; }
SETS="$SETS generated"

TSV=$OUT/scorecard-$C.tsv
REGIONS=$OUT/regions-$C.tsv
rm -f "$TSV" "$REGIONS"
for set in $SETS; do
  case $set in
    corpus) truth=$TRUTH_corpus ;;
    forms) truth=$TRUTH_forms ;;
    *) truth=$OUT/truth/$set ;;
  esac
  "$SC" check --names --set "$set" --tsv "$TSV" --regions-tsv "$REGIONS" \
    "$truth" "$OUT/dump/$set" > "$OUT/check-$set.txt" || { echo "the check of $set failed" >&2; exit 1; }
done
"$SC" summary "$C" "$TSV" "$REGIONS" > "$OUT/scorecard-$C.md" || exit 1
cat "$OUT/scorecard-$C.md"
echo
echo "written: $TSV, $REGIONS, $OUT/scorecard-$C.md"
# The sum of the last field over one class per set and half: the assertion files with no read.
UNREAD=$(awk -F'\t' 'NR > 1 && $3 == "table" { n += $6 } END { print n + 0 }' "$TSV")
if [ "$UNREAD" -ne 0 ]; then
  echo "$UNREAD assertion files have no read: their facts are in no count" >&2
  exit 4
fi
