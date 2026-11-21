mod calib;
mod cfg;
mod combat;
mod ents;
mod ext;
mod house;
mod map;
mod obs;
mod phys;
mod rng;
mod runner;
mod sim;

use rayon::prelude::*;
use runner::*;
use std::sync::Arc;
use std::time::Instant;

const HELLO: &str = include_str!("../fixtures/hello.json");

fn usage() -> ! {
    eprintln!(
        "mech-sim: headless mechfight/1 simulator\n\n\
         USAGE: mech-sim [--mode squad|ffa] [--matches N] [--seed S] [--threads T]\n\
                         [--player brain|easy|medium|hard|expert|hunter|camper]   (squad mode: who is the tracked mech)\n\
                         [--full]                 keep simulating after the tracked mechs die\n\
                         [--replay-dir DIR] [--replay-matches K]   write JSON-lines replays for the first K matches\n\
                         [--map-dir DIR]          dir containing v2/world.json (default: ../kit/arena)\n\
                         [--hunt-fuzz M]          squad position-tip fuzz in metres (default 50)\n\
                         [--ffa-pool brain=3,hunter=8,camper=4]\n\
                         [--calibrate]            compare physics with kit/check/stream.jsonl.gz and print a report"
    );
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let default_map = concat!(env!("CARGO_MANIFEST_DIR"), "/../kit/arena").to_string();
    let mut o = Opts {
        mode: Mode::Squad,
        seed: 1,
        matches: 20,
        full: false,
        replay_dir: None,
        replay_matches: 0,
        map_dir: default_map,
        hunt_fuzz: 50.0,
        player: "brain".into(),
        ffa_pool: vec![("brain".into(), 3.0), ("hunter".into(), 8.0), ("camper".into(), 4.0)],
        field_images: vec![],
        field_brains: 1,
        arena_host: String::new(),
        ext_budget_ms: 60,
        field_n: usize::MAX,
        params: Default::default(),
    };
    let mut threads = 0usize;
    let mut do_cal = false;
    let mut verbose = false;
    let mut i = 0;
    let mut replay_set = false;
    while i < args.len() {
        let a = args[i].as_str();
        let mut val = || {
            i += 1;
            args.get(i).cloned().unwrap_or_else(|| usage())
        };
        match a {
            "--mode" => {
                o.mode = match val().as_str() {
                    "squad" => Mode::Squad,
                    "ffa" => Mode::Ffa,
                    "field" => Mode::Field,
                    _ => usage(),
                }
            }
            "--matches" => o.matches = val().parse().unwrap_or_else(|_| usage()),
            "--seed" => o.seed = val().parse().unwrap_or_else(|_| usage()),
            "--threads" => threads = val().parse().unwrap_or_else(|_| usage()),
            "--player" => o.player = val(),
            "--full" => o.full = true,
            "--verbose" | "-v" => verbose = true,
            "--replay-dir" => o.replay_dir = Some(val()),
            "--replay-matches" => {
                o.replay_matches = val().parse().unwrap_or_else(|_| usage());
                replay_set = true;
            }
            "--map-dir" => o.map_dir = val(),
            "--hunt-fuzz" => o.hunt_fuzz = val().parse().unwrap_or_else(|_| usage()),
            "--ffa-pool" => {
                o.ffa_pool = val()
                    .split(',')
                    .filter_map(|kv| {
                        let mut p = kv.split('=');
                        Some((p.next()?.to_string(), p.next()?.parse().ok()?))
                    })
                    .collect()
            }
            "--calibrate" => do_cal = true,
            "--field-images" => o.field_images = val().split(',').filter(|x| !x.is_empty()).map(String::from).collect(),
            "--field-file" => {
                o.field_images = std::fs::read_to_string(val()).unwrap_or_default().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty() && !l.starts_with('#')).collect()
            }
            "--field-brains" => o.field_brains = val().parse().unwrap_or_else(|_| usage()),
            "--arena-host" => o.arena_host = val(),
            "--field-n" => o.field_n = val().parse().unwrap_or_else(|_| usage()),
            "--preset" => o.params = mech_bot::params::Params::preset(&val()),
            "--dump-params" => {
                println!("{}", serde_json::to_string_pretty(&o.params).unwrap());
                return;
            }
            "--params" => {
                let v = val();
                let txt = if v.trim_start().starts_with('{') { v } else { std::fs::read_to_string(&v).unwrap_or_else(|_| usage()) };
                o.params = serde_json::from_str(&txt).unwrap_or_else(|e| { eprintln!("bad params: {e}"); std::process::exit(2) });
            }
            "--ext-budget-ms" => o.ext_budget_ms = val().parse().unwrap_or_else(|_| usage()),
            "-h" | "--help" => usage(),
            _ => usage(),
        }
        i += 1;
    }
    if o.replay_dir.is_some() && !replay_set {
        o.replay_matches = 1;
    }
    let hello: serde_json::Value = serde_json::from_str(HELLO).expect("hello fixture");
    let cfg = Arc::new(cfg::Cfg::from_hello(&hello));
    let world = match map::World::load(std::path::Path::new(&o.map_dir)) {
        Ok(w) => Arc::new(w),
        Err(e) => {
            eprintln!("cannot load map from {}: {e}", o.map_dir);
            std::process::exit(1);
        }
    };
    if do_cal {
        calib::run(&cfg, &world);
        return;
    }
    if threads > 0 {
        rayon::ThreadPoolBuilder::new().num_threads(threads).build_global().ok();
    }
    if let Some(d) = &o.replay_dir {
        std::fs::create_dir_all(d).ok();
    }
    let t0 = Instant::now();
    let results: Vec<MatchResult> = (0..o.matches).into_par_iter().map(|k| run_match(&cfg, &world, &o, k)).collect();
    let wall = t0.elapsed().as_secs_f64();

    if let Some(d) = &o.replay_dir {
        for r in &results {
            if let Some(txt) = &r.replay {
                let _ = std::fs::write(format!("{}/match_{:05}.jsonl", d, r.idx), txt);
            }
        }
    }

    if verbose {
        for r in &results {
            println!("match {} dur={:.0}s tracked={:?}", r.idx, r.duration, r.tracked.iter().map(|t| (t.slot, t.place, t.kills, t.survive_s as i64)).collect::<Vec<_>>());
            for (id, t, place, killer) in &r.deaths {
                println!("   t={:>6.1} #{:<2} {:<7} place {:<2} killed by {}", t, id, r.names[*id as usize], place, killer.map_or("zone/none".to_string(), |k| format!("#{} {}", k, r.names[k])));
            }
        }
    }
    // aggregate
    let mut scores = vec![];
    let mut kills = vec![];
    let mut surv = vec![];
    let mut hist = vec![0u64; cfg.n_mechs + 1];
    let mut lat = LatHist::new();
    let mut sim_s = 0.0;
    for r in &results {
        let sc: f64 = r.tracked.iter().map(|t| cfg.n_mechs as f64 - t.place as f64).sum::<f64>() / r.tracked.len().max(1) as f64;
        scores.push(sc);
        for t in &r.tracked {
            kills.push(t.kills as f64);
            surv.push(t.survive_s);
            hist[(t.place as usize).min(cfg.n_mechs)] += 1;
        }
        lat.merge(&r.lat);
        sim_s += r.duration;
    }
    let (m, se) = mean_se(&scores);
    let (mk, kse) = mean_se(&kills);
    let (ms, sse) = mean_se(&surv);
    println!("mode={:?} matches={} seed={} player={} threads={}", o.mode, o.matches, o.seed, o.player, rayon::current_num_threads());
    println!("score (16-place): {:.3} +/- {:.3} (s.e.)", m, se);
    println!("kills: {:.3} +/- {:.3}", mk, kse);
    println!("survival: {:.1} s +/- {:.1}", ms, sse);
    println!("place histogram (tracked bots, n={}):", kills.len());
    let tot: u64 = hist.iter().sum();
    let maxc = *hist.iter().max().unwrap_or(&1).max(&1);
    for p in 1..=cfg.n_mechs {
        let c = hist[p];
        println!("  {:>2}: {:>6} {:>5.1}% {}", p, c, 100.0 * c as f64 / tot.max(1) as f64, "#".repeat((40 * c / maxc) as usize));
    }
    if lat.n > 0 {
        println!(
            "bot decision time: n={} p50={:.0}us p99={:.0}us max={:.0}us panics={}",
            lat.n,
            lat.quantile(0.5),
            lat.quantile(0.99),
            lat.max_us,
            lat.panics
        );
    }
    println!("wall {:.1}s, {:.1} matches/s, {:.0}x realtime (sim {:.0}s)", wall, o.matches as f64 / wall, sim_s / wall, sim_s);
}

fn mean_se(v: &[f64]) -> (f64, f64) {
    let n = v.len() as f64;
    if n < 1.0 {
        return (0.0, 0.0);
    }
    let m = v.iter().sum::<f64>() / n;
    if n < 2.0 {
        return (m, 0.0);
    }
    let var = v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0);
    (m, (var / n).sqrt())
}
