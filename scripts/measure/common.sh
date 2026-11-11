# Shared by the measure scripts: the repo, the work folder, the lock and the worktree.
# Sourced, not run. Everything is set by arguments or by the environment:
#   MEASURE_HOME   the work folder (worktree, build output, results); default
#                  ${XDG_CACHE_HOME:-$HOME/.cache}/rezzer-measure
#   MEASURE_FORMS  a folder of public PDF forms, as a second set beside the corpus; unset
#                  or missing: the set is left out
#   MEASURE_LOCK   a file; while it exists no cargo run starts

REPO=$(git -C "$(dirname "${BASH_SOURCE[0]}")" rev-parse --show-toplevel) || exit 2
SCRIPTS=$REPO/scripts/measure
MEASURE_HOME=${MEASURE_HOME:-${XDG_CACHE_HOME:-$HOME/.cache}/rezzer-measure}
WT=$MEASURE_HOME/worktree

# Waits while the lock file is there.
waitlock() {
  while [ -n "${MEASURE_LOCK:-}" ] && [ -e "$MEASURE_LOCK" ]; do
    echo "waiting for $MEASURE_LOCK" >&2
    sleep 20
  done
}

# Puts the worktree at commit $1. A worktree with changes is not touched.
worktree() {
  mkdir -p "$MEASURE_HOME"
  if [ ! -d "$WT" ]; then
    git -C "$REPO" worktree add --detach "$WT" "$1" || exit 1
  else
    if [ -n "$(git -C "$WT" status --porcelain --untracked-files=no)" ]; then
      echo "worktree $WT has changes; not touching it" >&2
      exit 1
    fi
    git -C "$WT" checkout --quiet --detach "$1" || exit 1
  fi
  [ "$(git -C "$WT" rev-parse HEAD)" = "$(git -C "$REPO" rev-parse "$1^{commit}")" ] || {
    echo "worktree not at $1" >&2
    exit 1
  }
}

# Runs cargo in the worktree, with its build output in the work folder.
wt_cargo() {
  waitlock
  (cd "$WT" && CARGO_TARGET_DIR="$MEASURE_HOME/target" cargo "$@")
}

# Runs cargo in the repo as it is checked out.
repo_cargo() {
  waitlock
  (cd "$REPO" && cargo "$@")
}

# The scorecard tool, built from the repo as it is checked out.
scorecard_tool() {
  repo_cargo build --release -p scorecard >&2 || exit 1
  echo "${CARGO_TARGET_DIR:-$REPO/target}/release/scorecard"
}
