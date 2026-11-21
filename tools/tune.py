#!/usr/bin/env python3
"""Self-play parameter tuning on TRAIN seeds only (validation seeds 777000+ are never touched here).

Coordinate search: perturb one tunable at a time, screen on seed block A, confirm on a fresh block B,
accept when the mean improvement of the composite (squad + ffa/2) clears MARGIN.
Writes tools/params_best.json and appends every trial to tools/tune_log.tsv.
Usage: tools/tune.py [hours]
"""
import json, os, random, shutil, subprocess, sys, time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SIM_SRC = os.path.join(ROOT, "sim", "target", "release", "mech-sim")
SIM = os.path.join(ROOT, "sim", "mech-sim-tuner")  # private copy: rebuilding the sim mid-run must not change the tuner
BEST = os.path.join(ROOT, "tools", "params_best.json")
LOG = os.path.join(ROOT, "tools", "tune_log.tsv")
NS, NF, THREADS, MARGIN = 384, 192, int(os.environ.get("THREADS", 2)), 0.08

# name -> (step, lo, hi) ; band params are [near, far] pairs handled per index
SPACE = {
    "hurt_hp": (0.08, 0.2, 0.8), "cover_exit_hp": (0.08, 0.5, 1.0), "threat_r": (30, 60, 280),
    "outnumber_n": (1, 0, 4), "low_hp": (0.07, 0.0, 0.6), "finish_tfrac": (0.08, 0.0, 0.7),
    "finish_hp": (0.1, 0.2, 0.9), "push_radial": (0.2, 0.0, 1.2), "retreat_radial": (0.2, 0.0, 1.2),
    "royale_hp_w": (60, 0, 500), "royale_hold_tfrac": (0.15, 0.1, 1.01), "drone_r": (25, 0, 160),
    "drone_hp": (0.12, 0.2, 0.95), "cube_r": (15, 10, 100), "explore_until": (40, 0, 400),
    "strafe_period": (0.4, 0.4, 3.0), "fire_tol": (0.15, 0.4, 1.6),
    "laser_band.0": (20, 20, 200), "laser_band.1": (25, 60, 270), "light_band.0": (6, 0, 40),
    "light_band.1": (6, 20, 60), "gun_band.0": (15, 10, 120), "gun_band.1": (20, 40, 200),
    "mortar_band.0": (20, 20, 150), "mortar_band.1": (25, 60, 220),
}


def run(params, mode, n, seed):
    out = subprocess.run([SIM, "--mode", mode, "--matches", str(n), "--seed", str(seed), "--threads", str(THREADS),
                          "--params", json.dumps(params)], capture_output=True, text=True).stdout
    for l in out.splitlines():
        if l.startswith("score"):
            return float(l.split()[2])
    raise RuntimeError(out[-500:])


def score(params, block):
    s = run(params, "squad", NS, 100000 + block * 1000)
    f = run(params, "ffa", NF, 400000 + block * 1000)
    return s + f / 2, s, f


def get(p, k):
    if "." in k:
        a, i = k.split(".")
        return p[a][int(i)]
    return p[k]


def put(p, k, v):
    q = json.loads(json.dumps(p))
    if "." in k:
        a, i = k.split(".")
        q[a][int(i)] = v
    else:
        q[k] = v
    return q


def main():
    hours = float(sys.argv[1]) if len(sys.argv) > 1 else 6
    shutil.copy(SIM_SRC, SIM)
    deadline = time.time() + hours * 3600
    if os.path.exists(BEST):
        cur = json.load(open(BEST))
    else:
        cur = json.loads(subprocess.run([SIM, "--dump-params"], capture_output=True, text=True).stdout)
    block = int(time.time()) % 997  # fresh train blocks every run
    if not os.path.exists(LOG):
        open(LOG, "w").write("time\tparam\tvalue\tbase\tcand\tconfirm_base\tconfirm_cand\taccepted\n")
    keys = list(SPACE)
    while time.time() < deadline:
        random.shuffle(keys)
        for k in keys:
            if time.time() > deadline:
                break
            step, lo, hi = SPACE[k]
            v0 = get(cur, k)
            sign = random.choice([-1, 1])
            v = min(hi, max(lo, v0 + sign * step))
            if isinstance(v0, int) and not isinstance(v0, bool):
                v = int(round(v))
            if v == v0:
                continue
            cand = put(cur, k, v)
            block += 1
            b = score(cur, block)[0]
            c = score(cand, block)[0]
            cb = cc = float("nan")
            acc = False
            if c - b > MARGIN:
                block += 1
                cb = score(cur, block)[0]
                cc = score(cand, block)[0]
                if ((c - b) + (cc - cb)) / 2 > MARGIN and cc > cb:
                    acc = True
                    cur = cand
                    json.dump(cur, open(BEST, "w"), indent=1)
            with open(LOG, "a") as f:
                f.write(f"{time.strftime('%H:%M:%S')}\t{k}\t{v}\t{b:.3f}\t{c:.3f}\t{cb:.3f}\t{cc:.3f}\t{acc}\n")
            print(k, v0, "->", v, f"{b:.3f} {c:.3f} {cb:.3f} {cc:.3f}", "ACCEPT" if acc else "", flush=True)


if __name__ == "__main__":
    main()
