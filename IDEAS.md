# Experiment backlog (ranked). Re-read after any context compaction.
Eval: `tools/eval.sh val` (held-out seeds 777000+, accept/reject only) / `tools/eval.sh train` (tuning).
Composite = squad + ffa/2. Accept when composite improves by > ~2 s.e. without squad dropping.
Submit only when local beats best-submitted clearly AND docker sandbox check passes.

## Next
1. External-process opponents in sim: real teams' public bots (opponents/, gitignored) as FFA field. Tune for the final field.
2. Survival: bot dies at ~82 s in squad sim. Study replays: what kills us (laser at range? swarmed?).
3. Peek-shoot laser from cover (charge behind cover, step out, release, step back).
4. Params struct + per-mech param overrides in sim -> automatic self-play tuning (ranges, hp thresholds, cover radius).
5. Endgame HP race: hold final-circle center with max HP; capture last supply drop.
6. Weapon choice vs field (lightning vs laser) measured in FFA vs real bots.
7. Upgrade order A/B (damage-first vs survival-first) per mode.
8. Latency guard: per-decision time budget (skip A*/cover when > 15 ms elapsed).

## Done
- v0..v6 (see results.tsv); A* capped at 12k expansions; Royale threshold 3 deaths.
