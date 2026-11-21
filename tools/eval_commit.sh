#!/bin/bash
# Evaluate the bot at an arbitrary commit with the CURRENT simulator. Usage: tools/eval_commit.sh <commit> [val|train] [NS] [NF]
set -e
REPO="$(cd "$(dirname "$0")/.." && pwd)"
C=$1; shift
WT=/private/tmp/claude-501/-Users-viktor/c5625454-9b56-476c-89b0-bc14d9e0f0ea/scratchpad/wt-$C
rm -rf "$WT"; git -C "$REPO" worktree add -q --detach "$WT" "$C"
rsync -a --exclude target "$REPO/sim/" "$WT/sim/"
mkdir -p "$WT/tools"; cp "$REPO/tools/eval.sh" "$WT/tools/"
CARGO_TARGET_DIR="$REPO/sim/target-eval" "$WT/tools/eval.sh" "$@" 2>&1 | sed "s/^/[$C] /" || true
git -C "$REPO" worktree remove --force "$WT"
