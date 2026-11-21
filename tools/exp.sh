#!/bin/bash
# Create an experiment commit = current params defaults + JSON overrides applied to src/params.rs base().
# Usage: tools/exp.sh '<json overrides>' "description"   -> prints commit hash (pushed). Restores working tree after.
set -e
cd "$(dirname "$0")/.."
export PATH="/opt/homebrew/opt/rustup/bin:$PATH"
cp src/params.rs /tmp/params.rs.bak.$$
python3 - "$1" <<'PY'
import json,re,sys
ov=json.loads(sys.argv[1]); p='src/params.rs'; s=open(p).read()
b=s.index('pub fn base()')
for k,v in ov.items():
    val = ('true' if v else 'false') if isinstance(v,bool) else (json.dumps(v).replace(',',', ') if isinstance(v,list) else (repr(float(v)) if isinstance(v,float) else str(v)))
    if isinstance(v,list): val='['+', '.join(repr(float(x)) if isinstance(x,float) else str(x) for x in v)+']'
    pat=re.compile(r'(\n            %s: )([^\n]*),'%re.escape(k))
    m=pat.search(s,b); assert m, k
    s=s[:m.start(2)]+val+s[m.end(2):]
open(p,'w').write(s)
PY
cargo build --release >/dev/null 2>&1
python3 kit/check/check.py --map-dir "$PWD/kit/arena" -- ./target/release/mech-bot 2>&1 | tail -1 | grep -q PASS
git add src/params.rs
git -c user.name=viktor-lorentz -c user.email=viktor-lorentz@users.noreply.github.com commit -q -m "Experiment: $2 ($1)

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y9TS6wq1F5CGZT8sGQsHF2"
H=$(git rev-parse HEAD)
cp /tmp/params.rs.bak.$$ src/params.rs; rm /tmp/params.rs.bak.$$
git add src/params.rs
git -c user.name=viktor-lorentz -c user.email=viktor-lorentz@users.noreply.github.com commit -q -m "Restore best params after experiment

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>
Claude-Session: https://claude.ai/code/session_01Y9TS6wq1F5CGZT8sGQsHF2"
git push -q origin main
echo $H
