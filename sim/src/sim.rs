//! Match state and construction. Behaviour lives in phys.rs / combat.rs / ents.rs / obs.rs.
use crate::cfg::*;
use crate::map::World;
use crate::rng::Rng;
use mech_bot::protocol::Event;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone, Default)]
pub struct Cmd {
    pub mv: [f64; 2],
    pub boost: bool,
    pub aim: Option<[f64; 2]>,
    pub fire: [bool; 2],
    pub jump: bool,
}

#[derive(Clone, Debug)]
pub struct Weapon {
    pub kind: usize,
    pub cooldown: f64,
    pub ammo: f64,
    pub reload: f64,
    pub charge: f64,
    pub charging: bool,
    pub overhold: f64,
}

impl Weapon {
    pub fn new(kind: usize, cfg: &Cfg) -> Weapon {
        let mag = cfg.weapons[kind].get("mag").copied().unwrap_or(0.0);
        Weapon { kind, cooldown: 0.0, ammo: mag, reload: 0.0, charge: 0.0, charging: false, overhold: 0.0 }
    }
}

pub struct OfferState {
    pub choices: Vec<usize>, // indexes into cfg.upgrades
    pub expires: f64,
}

pub struct Mech {
    pub id: usize,
    pub alive: bool,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub cyaw: f64,
    pub cpitch: f64,
    pub hp: f64,
    pub max_hp: f64,
    pub energy: f64,
    pub max_energy: f64,
    pub en_idle: f64,
    pub grounded: bool,
    pub cubes: u32,
    pub upg: Vec<u32>, // counts per upgrade def
    pub upg_list: Vec<String>,
    pub stats: HashMap<String, f64>,
    pub wstats: [HashMap<String, f64>; 4],
    pub ms: Ms,
    pub slots: [Option<Weapon>; 2],
    pub cmd: Cmd,
    pub jump_pending: bool,
    pub last_dmg_t: f64,
    pub last_attacker: Option<usize>,
    pub offer: Option<OfferState>,
    pub offer_queue: u32,
    pub short_until: f64,
    pub step_timer: f64,
    pub kills: u32,
    pub events: Vec<Event>,
    pub place: u32,
    pub death_t: f64,
    pub in_zone: bool,
    pub knocked: bool,
}

pub struct Npc {
    pub id: usize,
    pub alive: bool,
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub yaw: f64,
    pub pitch: f64,
    pub hp: f64,
    pub home: [f64; 2],
    pub target: Option<usize>,
    pub cooldown: f64,
    pub goal: [f64; 2],
    pub wait: f64,
    pub scan: f64,
    pub provoked: bool,
}

pub struct Crate {
    pub id: usize,
    pub pos: [f64; 3],
    pub hp: f64,
    pub alive: bool,
}

pub struct CubeItem {
    pub id: usize,
    pub pos: [f64; 3],
}

pub struct PickupItem {
    pub id: usize,
    pub w: Weapon,
    pub pos: [f64; 3],
}

#[derive(Clone, Copy, PartialEq)]
pub enum ProjKind {
    Gun,
    Mortar,
    Npc,
}

#[derive(Clone)]
pub struct MortarParams {
    pub damage: f64,
    pub splash: f64,
    pub radius: f64,
    pub edge: f64,
    pub knock: f64,
    pub knock_up: f64,
    pub drain: f64,
}

pub struct Proj {
    pub id: usize,
    pub kind: ProjKind,
    pub owner: i64, // mech id or npc id
    pub pos: [f64; 3],
    pub vel: [f64; 3],
    pub dmg: f64,
    pub age: f64,
    pub left: f64, // remaining travel distance (gun/npc) or time (mortar)
    pub mp: Option<MortarParams>,
    pub mult: f64,
}

pub struct SoundEv {
    pub tick: u64,
    pub pos: [f64; 3],
    pub kind: String,
    pub src: i64,
}

pub struct BeamEv {
    pub tick: u64,
    pub by: usize,
    pub kind: &'static str,
    pub path: Vec<[f64; 3]>,
}

pub struct DropItem {
    pub id: usize,
    pub pos: [f64; 3],
    pub called: f64,
    pub lands_at: f64,
    pub holder: Option<usize>,
    pub progress: f64,
}

#[derive(Clone)]
pub struct ZoneState {
    pub centers: Vec<[f64; 2]>, // circle k end center (k=0 initial)
    pub radii: Vec<f64>,
    pub starts: Vec<f64>, // shrink_start for phase k (transition k -> k+1)
    pub ends: Vec<f64>,
    pub dps: Vec<f64>,
}

pub struct Sim {
    pub cfg: Arc<Cfg>,
    pub world: Arc<World>,
    pub rng: Rng,
    pub tick: u64,
    pub mechs: Vec<Mech>,
    pub npcs: Vec<Npc>,
    pub crates: Vec<Crate>,
    pub cubes: Vec<CubeItem>,
    pub pickups: Vec<PickupItem>,
    pub projs: Vec<Proj>,
    pub sounds: Vec<SoundEv>,
    pub beams: Vec<BeamEv>,
    pub drops: Vec<DropItem>,
    pub zone: ZoneState,
    pub next_cube: usize,
    pub next_pickup: usize,
    pub next_proj: usize,
    pub next_drop: usize,
    pub next_offer: usize,
    pub drops_called: usize,
    pub last_seen: Vec<Vec<f64>>, // [observer][target idx: mechs 0..n, npcs n..]
    pub alive: usize,
    pub over: bool,
    pub dying: Vec<(usize, Option<usize>)>,
    pub deaths_log: Vec<(u32, f64, usize, Option<usize>)>, // (id, t, place, killer)
    pub dmg_log: Vec<(usize, Option<usize>, String, f64, bool)>, // (target, by, weapon, dmg, rear)
}

pub const EVENT_NONE: fn() -> Event = Event::default;

impl Sim {
    pub fn t(&self) -> f64 {
        self.tick as f64 * self.cfg.dt
    }

    pub fn recompute_ms(&mut self, i: usize) {
        let c = self.cfg.clone();
        let m = &mut self.mechs[i];
        m.ms = Ms::from_map(&m.stats, m.cubes, c.cube_size);
    }

    pub fn new(cfg: Arc<Cfg>, world: Arc<World>, seed: u64) -> Sim {
        let mut rng = Rng::new(seed);
        let n = cfg.n_mechs;
        let mut sim = Sim {
            zone: Self::build_zone(&cfg, &mut rng),
            cfg: cfg.clone(),
            world,
            rng: Rng::new(seed ^ 0x5151),
            tick: 0,
            mechs: vec![],
            npcs: vec![],
            crates: vec![],
            cubes: vec![],
            pickups: vec![],
            projs: vec![],
            sounds: vec![],
            beams: vec![],
            drops: vec![],
            next_cube: 5000,
            next_pickup: 8000,
            next_proj: 1,
            next_drop: 20000,
            next_offer: 1,
            drops_called: 0,
            last_seen: vec![vec![-1e9; n + cfg.npc_count]; n],
            alive: n,
            over: false,
            dying: vec![],
            deaths_log: vec![],
            dmg_log: vec![],
        };
        // mechs
        let mut spawns: Vec<[f64; 2]> = vec![];
        let sr = cfg.zone_radius * cfg.spawn_frac;
        let mut sep = cfg.spawn_sep;
        let mut tries = 0;
        while spawns.len() < n {
            tries += 1;
            if tries % 4000 == 0 {
                sep *= 0.9;
            }
            let p = sim.random_open_point(&mut rng, sr, 4.0);
            if let Some(p) = p {
                if !sim.world.in_bounds(p[0], p[1], cfg.margin) {
                    continue;
                }
                if spawns.iter().all(|q| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt() >= sep) {
                    spawns.push(p);
                }
            }
        }
        for i in 0..n {
            let p = spawns[i];
            let z = sim.world.ground(p[0], p[1]);
            let yaw = rng.range(-std::f64::consts::PI, std::f64::consts::PI);
            let mut stats = cfg.mech.clone();
            stats.entry("hp".into()).or_insert(1400.0);
            let ms = Ms::from_map(&stats, 0, cfg.cube_size);
            let hp = stats["hp"];
            let en = stats.get("energy").copied().unwrap_or(100.0);
            sim.mechs.push(Mech {
                id: i,
                alive: true,
                pos: [p[0], p[1], z],
                vel: [0.0; 3],
                yaw,
                pitch: 0.0,
                cyaw: yaw,
                cpitch: 0.0,
                hp,
                max_hp: hp,
                energy: en,
                max_energy: en,
                en_idle: 10.0,
                grounded: true,
                cubes: 0,
                upg: vec![0; cfg.upgrades.len()],
                upg_list: vec![],
                stats,
                wstats: [cfg.weapons[0].clone(), cfg.weapons[1].clone(), cfg.weapons[2].clone(), cfg.weapons[3].clone()],
                ms,
                slots: [Some(Weapon::new(GUN, &cfg)), None],
                cmd: Cmd { aim: None, ..Default::default() },
                jump_pending: false,
                last_dmg_t: -100.0,
                last_attacker: None,
                offer: None,
                offer_queue: 0,
                short_until: -1.0,
                step_timer: 0.0,
                kills: 0,
                events: vec![],
                place: 0,
                death_t: -1.0,
                in_zone: true,
                knocked: false,
            });
        }
        // crates
        let nroof = (cfg.crate_count as f64 * cfg.crate_roof).round() as usize;
        for k in 0..cfg.crate_count {
            let pos = if k < nroof { sim.random_roof_point(&mut rng, sr) } else { None };
            let pos = pos.or_else(|| {
                let mut r2 = rng.clone();
                let p = sim.random_open_point(&mut r2, sr, 3.0);
                rng = r2;
                p.map(|p| [p[0], p[1], sim.world.ground(p[0], p[1])])
            });
            if let Some(p) = pos {
                sim.crates.push(Crate { id: 1000 + k, pos: p, hp: cfg.crate_hp, alive: true });
            }
        }
        // floor weapons
        let nroof = (cfg.floor_count as f64 * cfg.floor_roof).round() as usize;
        for k in 0..cfg.floor_count {
            let pos = if k < nroof { sim.random_roof_point(&mut rng, sr) } else { None };
            let pos = pos.or_else(|| {
                let mut r2 = rng.clone();
                let p = sim.random_open_point(&mut r2, sr, 3.0);
                rng = r2;
                p.map(|p| [p[0], p[1], sim.world.ground(p[0], p[1])])
            });
            if let Some(p) = pos {
                let kind = sim.random_weapon_kind(&mut rng);
                sim.add_pickup(Weapon::new(kind, &cfg), p);
            }
        }
        // npcs
        for k in 0..cfg.npc_count {
            if let Some(p) = sim.random_open_point(&mut rng, sr, 4.0) {
                let z = sim.world.ground(p[0], p[1]);
                sim.npcs.push(Npc {
                    id: 100 + k,
                    alive: true,
                    pos: [p[0], p[1], z],
                    vel: [0.0; 3],
                    yaw: rng.range(-3.1, 3.1),
                    pitch: 0.0,
                    hp: cfg.npc_hp,
                    home: p,
                    target: None,
                    cooldown: 0.0,
                    goal: p,
                    wait: rng.range(0.0, 3.0),
                    scan: 0.0,
                    provoked: false,
                });
            }
        }
        sim.rng = rng;
        sim.last_seen = vec![vec![-1e9; n + sim.npcs.len()]; n];
        sim
    }

    pub fn random_weapon_kind(&self, rng: &mut Rng) -> usize {
        let w = &self.cfg.loot_weights;
        let tot: f64 = w.iter().sum();
        let mut r = rng.f() * tot;
        for k in 0..4 {
            if r < w[k] {
                return k;
            }
            r -= w[k];
        }
        MORTAR
    }

    pub fn add_pickup(&mut self, w: Weapon, pos: [f64; 3]) -> usize {
        let id = self.next_pickup;
        self.next_pickup += 1;
        self.pickups.push(PickupItem { id, w, pos });
        id
    }

    /// Random open-ground point inside the circle of radius `rad` around the zone start.
    pub fn random_open_point(&self, rng: &mut Rng, rad: f64, clearance: f64) -> Option<[f64; 2]> {
        let w = &self.world;
        let c = self.cfg.zone_center;
        for _ in 0..500 {
            let x = rng.range(w.bounds[0] + self.cfg.margin, w.bounds[2] - self.cfg.margin);
            let y = rng.range(w.bounds[1] + self.cfg.margin, w.bounds[3] - self.cfg.margin);
            if ((x - c[0]).powi(2) + (y - c[1]).powi(2)).sqrt() > rad {
                continue;
            }
            if w.slope_at(x, y) > 0.35 {
                continue;
            }
            if !w.open_ground(x, y, clearance + 2.5) {
                continue;
            }
            return Some([x, y]);
        }
        None
    }

    pub fn random_roof_point(&self, rng: &mut Rng, rad: f64) -> Option<[f64; 3]> {
        let w = &self.world;
        let c = self.cfg.zone_center;
        for _ in 0..800 {
            let pi = rng.below(w.prisms.len());
            let p = &w.prisms[pi];
            if p.kind != 0 || p.top - p.base < 3.0 || p.base > w.ground((p.bb[0] + p.bb[2]) * 0.5, (p.bb[1] + p.bb[3]) * 0.5) + 2.0 {
                continue;
            }
            let x = rng.range(p.bb[0], p.bb[2]);
            let y = rng.range(p.bb[1], p.bb[3]);
            if !p.contains_xy(x, y) {
                continue;
            }
            if ((x - c[0]).powi(2) + (y - c[1]).powi(2)).sqrt() > rad {
                continue;
            }
            // must be the highest surface there, and not at the wall edge
            if let Some((top, idx)) = w.roof_top(x, y) {
                if idx == pi && (top - w.ground(x, y)) > 3.0 && p.circle_overlaps(x, y, 0.0) {
                    // keep away from the edge: a point 2 m in every direction must also be in prism
                    let ok = [(2.0, 0.0), (-2.0, 0.0), (0.0, 2.0), (0.0, -2.0)].iter().all(|d| p.contains_xy(x + d.0, y + d.1));
                    if ok {
                        return Some([x, y, top]);
                    }
                }
            }
        }
        None
    }

    fn build_zone(cfg: &Cfg, rng: &mut Rng) -> ZoneState {
        let mut z = ZoneState { centers: vec![cfg.zone_center], radii: vec![cfg.zone_radius], starts: vec![], ends: vec![], dps: vec![] };
        let mut t = 0.0;
        for ph in &cfg.phases {
            let (c, r) = (*z.centers.last().unwrap(), *z.radii.last().unwrap());
            let room = (r - ph.radius).max(0.0);
            let ang = rng.range(0.0, std::f64::consts::TAU);
            let d = room * rng.f().sqrt() * 0.9;
            let nc = if ph.radius <= 0.0 { c } else { [c[0] + ang.cos() * d, c[1] + ang.sin() * d] };
            let start = t + ph.hold_s;
            let end = start + ph.shrink_s;
            z.centers.push(nc);
            z.radii.push(ph.radius);
            z.starts.push(start);
            z.ends.push(end);
            z.dps.push(ph.dps);
            t = end;
        }
        z
    }

    /// (phase index k, current circle (c, r), next circle, shrink_start, shrink_end, dps) at time t.
    pub fn zone_at(&self, t: f64) -> ([f64; 2], f64, [f64; 2], f64, f64, f64, f64) {
        let z = &self.zone;
        let n = z.starts.len();
        let mut k = 0;
        while k < n && t >= z.ends[k] {
            k += 1;
        }
        if k >= n {
            let c = *z.centers.last().unwrap();
            return (c, *z.radii.last().unwrap(), c, 0.0, t, t, *z.dps.last().unwrap_or(&20.0));
        }
        let (c0, r0) = (z.centers[k], z.radii[k]);
        let (c1, r1) = (z.centers[k + 1], z.radii[k + 1]);
        let (s, e) = (z.starts[k], z.ends[k]);
        let f = ((t - s) / (e - s)).clamp(0.0, 1.0);
        let c = [c0[0] + (c1[0] - c0[0]) * f, c0[1] + (c1[1] - c0[1]) * f];
        let r = r0 + (r1 - r0) * f;
        (c, r, c1, r1, s, e, z.dps[k])
    }
}
