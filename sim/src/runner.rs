//! Running whole matches: compositions, controllers, lockstep loop, results, replay dump.
use crate::cfg::Cfg;
use crate::ext::{docker_cmd, ExtBot};
use crate::house::*;
use crate::map::World;
use crate::rng::{mix, Rng};
use crate::sim::Sim;
use mech_bot::brain::Brain;
use mech_bot::protocol::*;
use std::fmt::Write as _;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::Instant;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Squad,
    Ffa,
    Field,
}

#[derive(Clone)]
pub struct Opts {
    pub mode: Mode,
    pub seed: u64,
    pub matches: usize,
    pub full: bool,
    pub replay_dir: Option<String>,
    pub replay_matches: usize,
    pub map_dir: String,
    pub hunt_fuzz: f64,
    pub player: String,
    pub ffa_pool: Vec<(String, f64)>,
    pub field_images: Vec<String>,
    pub field_brains: usize,
    pub arena_host: String,
    pub ext_budget_ms: u64,
    pub field_n: usize,
    pub params: mech_bot::params::Params,
}

pub enum Ctl {
    Brain(Box<Brain>),
    Fighter(Box<Fighter>, usize), // squad idx (usize::MAX = none)
    Ext(Box<ExtBot>),
}

pub struct LatHist {
    pub counts: Vec<u64>,
    pub over: u64,
    pub max_us: f64,
    pub n: u64,
    pub panics: u64,
}
const BUCKET_US: f64 = 20.0;
const NBUCKETS: usize = 5000;

impl LatHist {
    pub fn new() -> LatHist {
        LatHist { counts: vec![0; NBUCKETS], over: 0, max_us: 0.0, n: 0, panics: 0 }
    }
    pub fn add(&mut self, us: f64) {
        self.n += 1;
        if us > self.max_us {
            self.max_us = us;
        }
        let b = (us / BUCKET_US) as usize;
        if b < NBUCKETS {
            self.counts[b] += 1;
        } else {
            self.over += 1;
        }
    }
    pub fn merge(&mut self, o: &LatHist) {
        for i in 0..NBUCKETS {
            self.counts[i] += o.counts[i];
        }
        self.over += o.over;
        self.n += o.n;
        self.panics += o.panics;
        self.max_us = self.max_us.max(o.max_us);
    }
    pub fn quantile(&self, q: f64) -> f64 {
        let target = (self.n as f64 * q).ceil() as u64;
        let mut acc = 0;
        for i in 0..NBUCKETS {
            acc += self.counts[i];
            if acc >= target {
                return (i as f64 + 1.0) * BUCKET_US;
            }
        }
        self.max_us
    }
}

pub struct Tracked {
    pub slot: usize,
    pub place: u32,
    pub kills: u32,
    pub survive_s: f64,
}

pub struct MatchResult {
    pub idx: usize,
    pub tracked: Vec<Tracked>,
    pub duration: f64,
    pub lat: LatHist,
    pub replay: Option<String>,
    pub ticks: u64,
    pub names: Vec<String>,
    pub deaths: Vec<(u32, f64, usize, Option<usize>)>,
}

fn weighted_pick(rng: &mut Rng, pool: &[(String, f64)]) -> String {
    let tot: f64 = pool.iter().map(|p| p.1).sum();
    let mut r = rng.f() * tot;
    for (n, w) in pool {
        if r < *w {
            return n.clone();
        }
        r -= *w;
    }
    pool[0].0.clone()
}

pub fn run_match(cfg: &Arc<Cfg>, world: &Arc<World>, o: &Opts, idx: usize) -> MatchResult {
    let mseed = mix(o.seed, idx as u64 + 1);
    let mut crng = Rng::new(mseed ^ 0xC0FFEE);
    let n = cfg.n_mechs;
    // composition: name per slot
    let mut names: Vec<String> = vec![String::new(); n];
    let mut squad: Option<Squad> = None;
    let mut player = usize::MAX;
    match o.mode {
        Mode::Squad => {
            player = crng.below(n);
            names[player] = o.player.clone();
            let mut levels: Vec<&str> = vec![];
            for (lv, c) in [("easy", 4), ("medium", 4), ("hard", 4), ("expert", 3)] {
                for _ in 0..c {
                    levels.push(lv);
                }
            }
            // shuffle slot assignment
            let mut slots: Vec<usize> = (0..n).filter(|&s| s != player).collect();
            for i in (1..slots.len()).rev() {
                let j = crng.below(i + 1);
                slots.swap(i, j);
            }
            let mut members = vec![];
            for (k, lv) in levels.iter().enumerate() {
                if k < slots.len() {
                    names[slots[k]] = lv.to_string();
                    members.push(slots[k]); // already weakest-first
                }
            }
            squad = Some(Squad::new(members, player, o.hunt_fuzz, mseed ^ 0xABC));
        }
        Mode::Field => {
            // our brain(s) + distinct real opponents (docker images), filled with expert fighters if short
            let mut imgs = o.field_images.clone();
            for i in (1..imgs.len()).rev() {
                let j = crng.below(i + 1);
                imgs.swap(i, j);
            }
            let mut k = 0;
            for s in 0..n {
                names[s] = if s < o.field_brains.max(1) {
                    "brain".into()
                } else if k < imgs.len() && k < o.field_n {
                    k += 1;
                    format!("img:{}", imgs[k - 1])
                } else {
                    "expert".into()
                };
            }
            for i in (1..n).rev() {
                let j = crng.below(i + 1);
                names.swap(i, j);
            }
        }
        Mode::Ffa => {
            for s in 0..n {
                names[s] = if s == 0 { "brain".into() } else { weighted_pick(&mut crng, &o.ffa_pool) };
            }
            // shuffle so the guaranteed brain isn't always slot 0
            for i in (1..n).rev() {
                let j = crng.below(i + 1);
                names.swap(i, j);
            }
        }
    }
    let is_brain = |s: usize| names[s] == "brain";
    let tracked_slots: Vec<usize> = (0..n).filter(|&s| is_brain(s)).collect();
    let tracked_slots = if o.mode == Mode::Squad && !is_brain(player) { vec![player] } else { tracked_slots };

    let mut sim = Sim::new(cfg.clone(), world.clone(), mseed);
    let mut ctls: Vec<Ctl> = vec![];
    for s in 0..n {
        let seed = mix(mseed, 1000 + s as u64);
        if is_brain(s) {
            let h = Hello {
                slot: s as u32,
                seed,
                map: HelloMap { dir: o.map_dir.clone() },
                config: cfg.raw["config"].clone(),
            };
            ctls.push(Ctl::Brain(Box::new(Brain::with_params(&h, o.params.clone()))));
        } else if let Some(img) = names[s].strip_prefix("img:") {
            match ExtBot::spawn(img, docker_cmd(img, &o.arena_host)) {
                Some(b) => ctls.push(Ctl::Ext(Box::new(b))),
                None => ctls.push(Ctl::Fighter(Box::new(Fighter::new(params_for("expert"), seed, world.clone())), usize::MAX)),
            }
        } else {
            let sq_idx = squad.as_ref().and_then(|q| q.members.iter().position(|&m| m == s)).unwrap_or(usize::MAX);
            ctls.push(Ctl::Fighter(Box::new(Fighter::new(params_for(&names[s]), seed, world.clone())), sq_idx));
        }
    }

    for s in 0..n {
        let seed = mix(mseed, 1000 + s as u64);
        if let Ctl::Ext(b) = &mut ctls[s] {
            if !b.hello(s, seed, &cfg.raw["config"], std::time::Duration::from_secs(25)) {
                eprintln!("ext bot {} failed hello; it will sit idle", b.name);
            }
        }
    }
    let mut held_t: std::collections::BTreeMap<String, f64> = Default::default();
    let mut lat = LatHist::new();
    let mut replay = if idx < o.replay_matches && o.replay_dir.is_some() { Some(String::new()) } else { None };
    if let Some(r) = replay.as_mut() {
        let _ = writeln!(
            r,
            "{{\"k\":\"hdr\",\"seed\":{},\"match\":{},\"mode\":\"{:?}\",\"names\":{},\"tracked\":{:?},\"zone\":{}}}",
            o.seed,
            idx,
            o.mode,
            serde_json::to_string(&names).unwrap(),
            tracked_slots,
            serde_json::to_string(&(0..=sim.zone.starts.len()).map(|k| (sim.zone.centers[k], sim.zone.radii[k])).collect::<Vec<_>>()).unwrap()
        );
    }
    let squad_ids: Vec<i64> = squad.as_ref().map(|q| q.members.iter().map(|&m| m as i64).collect()).unwrap_or_default();
    let de = cfg.decide_every;
    let mut actions: Vec<Option<Action>> = vec![None; n];
    let max_ticks = (cfg.duration / cfg.dt).round() as u64 + 1;
    while !sim.over && sim.tick < max_ticks {
        if !o.full && tracked_slots.iter().all(|&s| !sim.mechs[s].alive) {
            break;
        }
        let obs = sim.build_all_obs();
        for &s in &tracked_slots {
            if sim.mechs[s].alive {
                let k = sim.mechs[s].slots[1].as_ref().map(|w| crate::cfg::KIND_NAMES[w.kind]).unwrap_or("none");
                *held_t.entry(k.to_string()).or_default() += cfg.dt * de as f64;
            }
        }
        let t = sim.t();
        // squad knowledge
        let mut focus = None;
        let mut active: Vec<usize> = vec![];
        if let Some(q) = squad.as_mut() {
            let truth = if sim.mechs[q.player].alive { Some(sim.mechs[q.player].pos) } else { None };
            focus = q.update(t, truth, &obs);
            let alive_members: Vec<usize> = q.members.iter().copied().filter(|&m| sim.mechs[m].alive).collect();
            let k = q.n_active(t, alive_members.len());
            active = alive_members.into_iter().take(k).collect();
        }
        let ext_deadline = Instant::now() + std::time::Duration::from_millis(o.ext_budget_ms);
        for s in 0..n {
            if let (Some(ob), Ctl::Ext(b)) = (&obs[s], &mut ctls[s]) {
                b.send_obs(ob);
            }
        }
        for s in 0..n {
            actions[s] = None;
            let Some(ob) = &obs[s] else { continue };
            match &mut ctls[s] {
                Ctl::Brain(b) => {
                    let t0 = Instant::now();
                    let r = catch_unwind(AssertUnwindSafe(|| b.act(ob)));
                    lat.add(t0.elapsed().as_secs_f64() * 1e6);
                    match r {
                        Ok(a) => actions[s] = Some(a),
                        Err(_) => lat.panics += 1,
                    }
                }
                Ctl::Fighter(f, sq_idx) => {
                    let ctx = if *sq_idx != usize::MAX {
                        let hunting = active.contains(&s);
                        Ctx { focus: if hunting { focus } else { None }, squad_ids: squad_ids.clone(), passive: !hunting }
                    } else {
                        Ctx::default()
                    };
                    let mut a = f.act(ob, &ctx);
                    a.tick = ob.tick;
                    actions[s] = Some(a);
                }
                Ctl::Ext(_) => {}
            }
        }
        for s in 0..n {
            if let (Some(_), Ctl::Ext(b)) = (&obs[s], &mut ctls[s]) {
                actions[s] = b.recv_action(ext_deadline);
            }
        }
        let mut takes = vec![];
        for s in 0..n {
            if let Some(a) = &actions[s] {
                sim.apply_action(s, a);
                if a.take.is_some() || a.drop.is_some() {
                    takes.push((s, a.take, a.drop));
                }
            } else if sim.mechs[s].alive {
                // missed answer: keep continuous parts, drop one-shots
                sim.mechs[s].cmd.fire = [false, false];
            }
        }
        if !takes.is_empty() {
            sim.resolve_takes(&takes);
        }
        if let Some(r) = replay.as_mut() {
            frame(r, &sim);
        }
        for _ in 0..de {
            sim.step();
            if sim.over {
                break;
            }
        }
    }
    for s in 0..n {
        if let Ctl::Ext(b) = &mut ctls[s] {
            eprintln!("EXT\t{}\t{}\t{}\t{}", b.name, if sim.mechs[s].place == 0 { 1 } else { sim.mechs[s].place }, b.answered, b.late);
            b.end(1, 0);
        }
    }
    if std::env::var("SIM_STATS").is_ok() {
        for &s in &tracked_slots {
            let mut line = String::from("STAT");
            for (tag, mine) in [("dealt", true), ("taken", false)] {
                let mut by_w: std::collections::BTreeMap<String, f64> = Default::default();
                let mut rear = 0.0;
                for (t, by, w, d, r) in &sim.dmg_log {
                    let hit = if mine { *by == Some(s) && *t != s } else { *t == s };
                    if hit {
                        *by_w.entry(w.clone()).or_default() += d;
                        if *r {
                            rear += d;
                        }
                    }
                }
                for (w, d) in by_w {
                    let _ = write!(line, "\t{tag}_{w}={d:.0}");
                }
                let _ = write!(line, "\t{tag}_rear={rear:.0}");
            }
            let kt: Vec<String> = sim.deaths_log.iter().filter(|d| d.3 == Some(s)).map(|d| format!("{:.0}", d.1)).collect();
            let dt = sim.deaths_log.iter().find(|d| d.0 as usize == s).map(|d| d.1).unwrap_or(-1.0);
            for (k, v) in &held_t {
                let _ = write!(line, "\theld_{k}={v:.0}");
            }
            let _ = write!(line, "\tdeath={dt:.0}\tkills_at={}", kt.join(","));
            if dt > 0.0 && dt < 35.0 {
                let mut g: std::collections::BTreeMap<String, f64> = Default::default();
                for (t, by, w, d, _) in &sim.dmg_log {
                    if *t == s {
                        let who = by.map(|b| if b < names.len() { names[b].clone() } else { "npc".into() }).unwrap_or("npc".into());
                        *g.entry(format!("{who}/{w}")).or_default() += d;
                    }
                }
                let _ = write!(line, "\tEARLY {:?}", g.iter().map(|(k, v)| format!("{k}:{v:.0}")).collect::<Vec<_>>());
            }
            eprintln!("{line}");
        }
    }
    let t_end = sim.t();
    let mut tracked = vec![];
    for &s in &tracked_slots {
        let m = &sim.mechs[s];
        let place = if m.place == 0 { 1 } else { m.place };
        tracked.push(Tracked { slot: s, place, kills: m.kills, survive_s: if m.alive { t_end } else { m.death_t } });
    }
    if let Some(r) = replay.as_mut() {
        frame(r, &sim);
        for (id, t, place, killer) in &sim.deaths_log {
            let _ = writeln!(r, "{{\"k\":\"death\",\"id\":{},\"t\":{:.2},\"place\":{},\"killer\":{}}}", id, t, place, killer.map_or(-1, |k| k as i64));
        }
        let _ = writeln!(r, "{{\"k\":\"end\",\"t\":{:.2},\"places\":{:?}}}", t_end, sim.mechs.iter().map(|m| m.place).collect::<Vec<_>>());
    }
    MatchResult { idx, tracked, duration: t_end, lat, replay, ticks: sim.tick, names, deaths: sim.deaths_log.clone() }
}

fn frame(r: &mut String, sim: &Sim) {
    let _ = write!(r, "{{\"k\":\"f\",\"tick\":{},\"t\":{:.2},\"m\":[", sim.tick, sim.t());
    let mut first = true;
    for m in &sim.mechs {
        if !first {
            r.push(',');
        }
        first = false;
        let _ = write!(
            r,
            "[{},{:.1},{:.1},{:.1},{:.2},{:.0},{:.0},{},{}]",
            m.id,
            m.pos[0],
            m.pos[1],
            m.pos[2],
            m.yaw,
            m.hp,
            m.energy,
            m.cubes,
            if m.alive { 1 } else { 0 }
        );
    }
    r.push_str("],\"n\":[");
    let mut first = true;
    for n in sim.npcs.iter().filter(|n| n.alive) {
        if !first {
            r.push(',');
        }
        first = false;
        let _ = write!(r, "[{},{:.1},{:.1},{:.1},{:.0}]", n.id, n.pos[0], n.pos[1], n.pos[2], n.hp);
    }
    r.push_str("],\"p\":[");
    let mut first = true;
    for p in sim.projs.iter().take(60) {
        if !first {
            r.push(',');
        }
        first = false;
        let _ = write!(r, "[{:.1},{:.1},{:.1}]", p.pos[0], p.pos[1], p.pos[2]);
    }
    r.push_str("]}\n");
}
