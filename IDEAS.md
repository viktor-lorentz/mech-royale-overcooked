# Experiment backlog (ranked). Re-read after any context compaction.
Eval: `tools/eval.sh val` (held-out seeds 777000+, accept/reject only) / `tools/eval.sh train` (tuning).
Composite = squad + ffa/2. Accept when composite improves by > ~2 s.e. without squad dropping.
Submit only when local beats best-submitted clearly AND docker sandbox check passes.

## Status (Fri 22:30 ET)
Server best 3.04 (90e4970). Rank ~60/108; top-32 cut ~4.1 -> squad score is the gate. Field (real bots) mean place ~8.
Sim diag (SIM_STATS=1): ~24% squad deaths <35s (2 mechs or drones); half alive time unarmed.

## NEXT ACTION (on resume)
1. Read score of A (BB-38F51286, 96a986a). Submit B 9a0a40b then C 0bdf825 (one at a time; server queues one per team). Log in results.tsv.
2. Keep server-best variant; iterate on it, 4 submits/hour, one hypothesis each. main HEAD = heal variant commit; set VARIANT to winner.
Sim presets (val squad): base 2.536 surv82s | survivor 2.484 surv96s | controlled 2.490 surv95s | heal 2.422. Sim deaths: 65% have >=2 attackers in last 10s.

## Next
0. Unarmed-time: first weapon at ~35s. Path to nearest weapon at spawn? sweep torso more when unarmed.
0b. Early 2v1 deaths: retreat to break LOS from 2nd mech.
1. External-process opponents in sim: real teams' public bots (opponents/, gitignored) as FFA field. Tune for the final field.
2. Survival: bot dies at ~82 s in squad sim. Study replays: what kills us (laser at range? swarmed?).
3. Peek-shoot laser from cover (charge behind cover, step out, release, step back).
4. Params struct + per-mech param overrides in sim -> automatic self-play tuning (ranges, hp thresholds, cover radius).
5. Endgame HP race: hold final-circle center with max HP; capture last supply drop.
6. Weapon choice vs field (lightning vs laser) measured in FFA vs real bots.
7. Upgrade order A/B (damage-first vs survival-first) per mode.
8. Latency guard: per-decision time budget (skip A*/cover when > 15 ms elapsed).

## Done
- 90e4970 drone awareness + face attackers (3.04). 391d45d pickup memory (+0.17 squad sim).
- No effect: gauntlet low-hp targeting, always-Royale in field, outnumbered->cover, hurt_hp/cover_exit tweaks.
- v0..v6 (see results.tsv); A* capped at 12k expansions; Royale threshold 3 deaths.
