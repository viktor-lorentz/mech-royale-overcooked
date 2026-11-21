#!/bin/bash
# Field tournament: our brain vs real public opponent bots (docker images listed in a file, one per line).
# Usage: tools/field.sh <images_file> <matches> [field_n] [threads] [seed] [extra sim args...]
# Prints our mean place, and the mean place of every opponent image (lower = better).
set -e
cd "$(dirname "$0")/.."
IMGS=$1; N=$2; K=${3:-15}; T=${4:-1}; S=${5:-900000}; shift 5 2>/dev/null || shift $#
OUT=$(./sim/target/release/mech-sim --mode field --full --matches "$N" --seed "$S" --threads "$T" \
  --field-file "$IMGS" --field-n "$K" --arena-host "$PWD/kit/arena" "$@" 2>&1)
echo "$OUT" | grep -E "^score|^kills|^survival|decision time"
echo "$OUT" | awk -F'\t' '$1=="EXT"{n[$2]++; p[$2]+=$3; a[$2]+=$4; l[$2]+=$5}
  END{for(k in n) printf "%6.2f  n=%-3d late%%=%5.1f  %s\n", p[k]/n[k], n[k], 100*l[k]/(a[k]+l[k]+1e-9), k}' | sort -n
