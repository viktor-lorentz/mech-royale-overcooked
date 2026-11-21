#!/bin/bash
# Usage: tools/eval.sh [val|train] [squad_matches] [ffa_matches]
# val   = held-out validation seeds (777000+): ONLY for accept/reject decisions, never for tuning
# train = tuning seeds (1000+)
set -e
cd "$(dirname "$0")/../sim"
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cargo build --release -q 2>&1 | grep -E "^error" && exit 1
SET=${1:-val}; NS=${2:-384}; NF=${3:-192}
if [ "$SET" = val ]; then S=777000; else S=1000; fi
BIN=${CARGO_TARGET_DIR:-./target}/release/mech-sim
sq=$($BIN --mode squad --matches $NS --seed $S --threads 4 2>/dev/null)
ff=$($BIN --mode ffa --matches $NF --seed $((S+50000)) --threads 4 2>/dev/null)
s=$(echo "$sq" | awk '/^score/{print $3}'); se=$(echo "$sq" | awk '/^score/{print $5}')
f=$(echo "$ff" | awk '/^score/{print $3}'); fe=$(echo "$ff" | awk '/^score/{print $5}')
lat=$(printf "%s\n%s\n" "$sq" "$ff" | awk '/decision time/{for(i=1;i<=NF;i++) if($i ~ /^max=/){sub("max=","",$i); sub("us","",$i); if($i+0>m)m=$i+0}} END{print m}')
comp=$(awk -v s=$s -v f=$f 'BEGIN{printf "%.3f", s + f/2}')
echo "set=$SET squad=$s+-$se ffa=$f+-$fe composite=$comp maxlat_us=$lat"
