//! Scripted opponents: house-bot difficulty levels, squad coordination, FFA archetypes.
use crate::ents::lead;
use crate::map::World;
use crate::phys::{wrap, Body};
use crate::rng::Rng;
use mech_bot::geom::mortar_pitch;
use mech_bot::protocol::*;
use std::f64::consts::PI;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Params {
    pub name: &'static str,
    pub react: f64,      // seconds before firing on a newly seen target
    pub aim_sigma: f64,  // rad of aim noise per decision
    pub lead: f64,       // 0..1 lead factor
    pub secondary: bool, // uses slot-1 weapons
    pub strafe: f64,     // 0..1
    pub boosty: bool,
    pub jumpy: bool,
    pub cover: bool,
    pub pref_range: f64,
    pub loot: bool,
    pub camp: bool,
    pub turn_mult: f64,
}

pub fn params_for(name: &str) -> Params {
    let base = Params {
        name: "medium",
        react: 0.5,
        aim_sigma: 0.035,
        lead: 0.6,
        secondary: true,
        strafe: 0.5,
        boosty: false,
        jumpy: false,
        cover: false,
        pref_range: 55.0,
        loot: true,
        camp: false,
        turn_mult: 1.0,
    };
    match name {
        "easy" => Params { name: "easy", react: 0.9, aim_sigma: 0.07, lead: 0.0, secondary: false, strafe: 0.0, pref_range: 40.0, loot: false, ..base },
        "medium" => base,
        "hard" => Params { name: "hard", react: 0.3, aim_sigma: 0.02, lead: 1.0, strafe: 0.8, boosty: true, cover: true, ..base },
        "expert" => Params { name: "expert", react: 0.15, aim_sigma: 0.009, lead: 1.0, strafe: 1.0, boosty: true, jumpy: true, cover: true, pref_range: 60.0, ..base },
        "hunter" => Params { name: "hunter", react: 0.3, aim_sigma: 0.025, lead: 1.0, strafe: 0.7, boosty: true, pref_range: 45.0, ..base },
        "camper" => Params { name: "camper", react: 0.25, aim_sigma: 0.02, lead: 1.0, strafe: 0.2, cover: true, pref_range: 70.0, camp: true, ..base },
        _ => base,
    }
}

#[derive(Default, Clone)]
pub struct Ctx {
    /// Squad focus (hunting): player id and estimated position.
    pub focus: Option<(i64, [f64; 3])>,
    pub squad_ids: Vec<i64>,
    /// Passive squad member (not hunting): only defends.
    pub passive: bool,
}

pub struct Fighter {
    pub p: Params,
    rng: Rng,
    world: Arc<World>,
    acq: Option<(i64, f64)>,
    last_target_t: f64,
    strafe_sign: f64,
    strafe_until: f64,
    hist: Vec<(f64, [f64; 3])>,
    unstick_until: f64,
    unstick_dir: f64,
    cover: Option<([f64; 2], f64)>,
    camp_spot: Option<([f64; 2], f64)>,
    sound_goal: Option<([f64; 2], f64)>,
    next_jump: f64,
    boost_until: f64,
}

fn pref_rank(id: &str) -> usize {
    const PREF: [&str; 12] = ["plating", "armor", "gun_damage", "laser_damage", "nanites", "gyros", "servos", "gun_rate", "mortar_reload", "lightning_chain", "reactor", "capacitor"];
    PREF.iter().position(|p| *p == id).unwrap_or(99)
}

fn d2(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
fn d3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

impl Fighter {
    pub fn new(p: Params, seed: u64, world: Arc<World>) -> Fighter {
        let mut rng = Rng::new(seed);
        let s = if rng.chance(0.5) { 1.0 } else { -1.0 };
        Fighter {
            p,
            rng,
            world,
            acq: None,
            last_target_t: -10.0,
            strafe_sign: s,
            strafe_until: 0.0,
            hist: vec![],
            unstick_until: -1.0,
            unstick_dir: 1.0,
            cover: None,
            camp_spot: None,
            sound_goal: None,
            next_jump: 2.0,
            boost_until: 0.0,
        }
    }

    fn steer(&self, pos: [f64; 3], yaw: f64, body: &Body) -> (f64, bool) {
        let probe = 6.0;
        for off in [0.0, 0.5, -0.5, 1.0, -1.0, 1.6, -1.6, 2.3, -2.3] {
            let y = yaw + off;
            let (nx, ny, _) = self.world.walk(body, pos[0], pos[1], pos[2], true, y.cos() * probe, y.sin() * probe);
            if ((nx - pos[0]).powi(2) + (ny - pos[1]).powi(2)).sqrt() > probe * 0.7 {
                return (y, true);
            }
        }
        (yaw + PI * 0.5, false)
    }

    pub fn act(&mut self, o: &Obs, ctx: &Ctx) -> Action {
        let me = &o.me;
        let t = o.t;
        let pos = me.pos;
        let size = me.stats.get("radius").copied().unwrap_or(2.5) / 2.5;
        let height = me.stats.get("height").copied().unwrap_or(9.0);
        let body = Body {
            r: me.stats.get("radius").copied().unwrap_or(2.5),
            h: height,
            step_up: me.stats.get("step_up").copied().unwrap_or(1.2),
            max_slope: me.stats.get("max_slope_deg").copied().unwrap_or(50.0).to_radians(),
        };
        let muzzle = [pos[0], pos[1], pos[2] + me.stats.get("muzzle_height").copied().unwrap_or(7.0)];
        let mut a = Action { tick: o.tick, ..Default::default() };

        if let Some(of) = &o.offer {
            let mut best = 0;
            let mut br = usize::MAX;
            for (i, c) in of.choices.iter().enumerate() {
                let r = pref_rank(&c.id);
                if r < br {
                    br = r;
                    best = i;
                }
            }
            a.pick = Some(best);
        }

        self.hist.push((t, pos));
        self.hist.retain(|(tt, _)| t - *tt <= 2.0);

        // ---- targets
        let enemies: Vec<&Actor> = o.mechs.iter().filter(|m| m.kind == "mech" && !ctx.squad_ids.contains(&m.id)).collect();
        let drones: Vec<&Actor> = o.mechs.iter().filter(|m| m.kind == "npc").collect();
        let mut target: Option<&Actor> = None;
        if let Some((fid, _)) = ctx.focus {
            target = enemies.iter().copied().find(|m| m.id == fid);
            if target.is_none() {
                target = enemies.iter().copied().filter(|m| d3(m.pos, pos) < 35.0).min_by(|a, b| d3(a.pos, pos).partial_cmp(&d3(b.pos, pos)).unwrap());
            }
        } else {
            let lim = if ctx.passive { 80.0 } else { 1e9 };
            target = enemies.iter().copied().filter(|m| d3(m.pos, pos) < lim).min_by(|a, b| d3(a.pos, pos).partial_cmp(&d3(b.pos, pos)).unwrap());
        }
        let mut is_drone = false;
        if target.is_none() && self.p.secondary {
            if let Some(d) = drones.iter().copied().filter(|d| d3(d.pos, pos) < 30.0).min_by(|a, b| d3(a.pos, pos).partial_cmp(&d3(b.pos, pos)).unwrap()) {
                target = Some(d);
                is_drone = true;
            }
        }
        // reaction bookkeeping
        let mut can_fire = false;
        if let Some(tg) = target {
            if t - self.last_target_t > 1.0 || self.acq.map(|x| x.0) != Some(tg.id) {
                if self.acq.map(|x| x.0) != Some(tg.id) || t - self.last_target_t > 1.0 {
                    self.acq = Some((tg.id, t));
                }
            }
            self.last_target_t = t;
            can_fire = t - self.acq.unwrap().1 >= self.p.react;
        }

        // ---- weapons
        let gun = me.weapons.get(0).and_then(|w| w.as_ref());
        let w1 = me.weapons.get(1).and_then(|w| w.as_ref());
        let gun_speed = gun.and_then(|g| g.stats.get("speed")).copied().unwrap_or(252.0);
        let gun_range = gun.and_then(|g| g.stats.get("range")).copied().unwrap_or(230.0);
        let mut aim = me.aim;
        let mut fire = [false, false];
        let mut engaged = false;
        let mut tdist = 1e9;
        let mut toward = 0.0;
        if let Some(tg) = target {
            let h = if tg.kind == "npc" { 1.7 } else { 4.5 };
            let tcen = [tg.pos[0], tg.pos[1], tg.pos[2] + h];
            let tvel = [tg.vel[0] * self.p.lead, tg.vel[1] * self.p.lead, tg.vel[2] * self.p.lead];
            let d = d3(muzzle, tcen);
            tdist = d;
            let (ip, _) = lead(muzzle, tcen, tvel, gun_speed);
            let desired = [(ip[1] - muzzle[1]).atan2(ip[0] - muzzle[0]), ((ip[2] - muzzle[2]).atan2(d2(ip, muzzle).max(1e-6)))];
            let sig = self.p.aim_sigma;
            aim = [desired[0] + self.rng.gauss() * sig, desired[1] + self.rng.gauss() * sig];
            toward = (tg.pos[1] - pos[1]).atan2(tg.pos[0] - pos[0]);
            let yaw_err = wrap(desired[0] - me.aim[0]).abs();
            let pit_err = (desired[1] - me.aim[1]).abs();
            let tol = (1.8 / d.max(5.0)).atan() * if self.p.name == "easy" { 3.0 } else { 1.2 };
            engaged = !is_drone || d < 25.0;
            if can_fire && yaw_err < tol && pit_err < tol * 1.5 && d < gun_range {
                fire[0] = true;
            }
            if self.p.secondary && can_fire {
                if let Some(w) = w1 {
                    match w.kind.as_str() {
                        "laser" => {
                            let ct = w.stats.get("charge").copied().unwrap_or(0.75);
                            let ch = w.charge.unwrap_or(0.0);
                            if ch > 0.0 {
                                fire[1] = ch < ct - 0.03 && yaw_err < tol * 6.0 && d < 270.0;
                            } else if w.cooldown <= 0.05 && d < 250.0 && yaw_err < tol * 5.0 {
                                fire[1] = true;
                            }
                        }
                        "lightning" => {
                            let rng = w.stats.get("range").copied().unwrap_or(45.0);
                            if d < rng * 0.95 && yaw_err < 0.25 && me.energy > 6.0 {
                                fire[1] = true;
                            }
                        }
                        "mortar" => {
                            if d > 18.0 && d < 200.0 {
                                let spd = w.stats.get("speed").copied().unwrap_or(72.0);
                                let g = w.stats.get("gravity").copied().unwrap_or(22.0);
                                let tp = [tg.pos[0] + tg.vel[0] * (d / spd) * self.p.lead, tg.pos[1] + tg.vel[1] * (d / spd) * self.p.lead, tcen[2]];
                                if let Some(pt) = mortar_pitch(d2(muzzle, tp), tp[2] - (pos[2] + height + 0.5), spd, g) {
                                    aim = [(tp[1] - muzzle[1]).atan2(tp[0] - muzzle[0]) + self.rng.gauss() * sig, pt + self.rng.gauss() * sig * 0.5];
                                    if wrap(aim[0] - me.aim[0]).abs() < 0.04 && (aim[1] - me.aim[1]).abs() < 0.04 {
                                        fire[1] = true;
                                    }
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        } else if let Some(cr) = o.crates.iter().filter(|c| d3(c.pos, pos) < 40.0).min_by(|a, b| d3(a.pos, pos).partial_cmp(&d3(b.pos, pos)).unwrap()) {
            if self.p.loot {
                let tc = [cr.pos[0], cr.pos[1], cr.pos[2] + 1.2];
                aim = [(tc[1] - muzzle[1]).atan2(tc[0] - muzzle[0]), (tc[2] - muzzle[2]).atan2(d2(tc, muzzle).max(1e-6))];
                if wrap(aim[0] - me.aim[0]).abs() < 0.04 {
                    fire[0] = true;
                }
            }
        }

        // ---- movement goal
        let z = &o.zone;
        let zc = [z.center[0], z.center[1], pos[2]];
        let dzone = d2(pos, zc);
        let (nc, nr) = (z.next_center, z.next_radius);
        let shrinking_soon = t + 10.0 >= z.shrink_start;
        let (gc, gr) = if shrinking_soon { (nc, nr) } else { (z.center, z.radius) };
        let zone_goal = [gc[0], gc[1], pos[2]];
        let out_of_zone = !me.in_zone || dzone > z.radius - 12.0 || (shrinking_soon && d2(pos, zone_goal) > gr - 12.0);
        let mut goal: Option<[f64; 3]> = None;
        let mut mv_dir: Option<[f64; 2]> = None;
        let mut boost = false;

        if out_of_zone {
            goal = Some(zone_goal);
            boost = !me.in_zone;
        } else if let (Some(tg), true) = (target, engaged) {
            // combat movement
            let wk = w1.map(|w| w.kind.as_str()).unwrap_or("");
            let pref = if self.p.secondary {
                match wk {
                    "laser" => 90.0,
                    "mortar" => 100.0,
                    "lightning" => 25.0,
                    _ => self.p.pref_range,
                }
            } else {
                self.p.pref_range
            };
            if t > self.strafe_until {
                self.strafe_sign = -self.strafe_sign;
                self.strafe_until = t + self.rng.range(0.7, 2.0);
            }
            let radial = if tdist > pref + 8.0 { 1.0 } else if tdist < pref - 12.0 { -1.0 } else { 0.0 };
            let perp = toward + self.strafe_sign * PI / 2.0;
            let sw = self.p.strafe;
            let (vx, vy) = (perp.cos() * sw + toward.cos() * radial, perp.sin() * sw + toward.sin() * radial);
            if vx.abs() + vy.abs() > 0.05 {
                mv_dir = Some([vx, vy]);
            }
            // cover
            if self.p.cover && me.hp < me.max_hp * 0.45 {
                if self.cover.map_or(true, |c| t > c.1) {
                    // find nearest cover spot (blocked from target's eye)
                    let eye_t = [tg.pos[0], tg.pos[1], tg.pos[2] + 8.0];
                    let mut best: Option<([f64; 2], f64)> = None;
                    for k in 0..12 {
                        let ang = k as f64 / 12.0 * 2.0 * PI;
                        for rr in [8.0, 14.0, 22.0] {
                            let q = [pos[0] + ang.cos() * rr, pos[1] + ang.sin() * rr, pos[2]];
                            let qz = self.world.ground(q[0], q[1]);
                            if !self.world.open_ground(q[0], q[1], 2.6) {
                                continue;
                            }
                            let head = [q[0], q[1], qz + 6.0];
                            if self.world.raycast(eye_t, head).is_some() && best.map_or(true, |b| rr < b.1) {
                                best = Some(([q[0], q[1]], rr));
                            }
                        }
                    }
                    if let Some((q, _)) = best {
                        self.cover = Some((q, t + 3.0));
                    }
                }
                if let Some((q, until)) = self.cover {
                    if t < until {
                        goal = Some([q[0], q[1], pos[2]]);
                        mv_dir = None;
                        boost = true;
                    }
                }
            }
            if self.p.boosty && mv_dir.is_some() && me.energy > 45.0 && tdist > 30.0 && (o.tick / 6) % 3 == 0 {
                boost = true;
            }
            if self.p.jumpy && t > self.next_jump && me.grounded && me.energy > 30.0 {
                a.jump = Some(true);
                self.next_jump = t + self.rng.range(1.5, 3.5);
            }
        } else {
            // no combat: loot / hunt / camp / wander to zone
            let slot1_empty = me.weapons.get(1).map_or(true, |w| w.is_none());
            let mut looted = false;
            if self.p.loot && !ctx.focus.is_some() || (self.p.loot && o.pickups.iter().any(|p| d3(p.pos, pos) < 25.0)) {
                if slot1_empty {
                    if let Some(pk) = o.pickups.iter().filter(|p| d3(p.pos, pos) < 70.0).min_by(|a, b| d3(a.pos, pos).partial_cmp(&d3(b.pos, pos)).unwrap()) {
                        goal = Some(pk.pos);
                        looted = true;
                        if d2(pk.pos, pos) < 3.2 && (pk.pos[2] - pos[2]).abs() < 3.5 {
                            a.take = Some(pk.id);
                        }
                    }
                }
                if !looted {
                    if let Some(cb) = o.cubes.iter().filter(|c| d3(c.pos, pos) < 40.0).min_by(|a, b| d3(a.pos, pos).partial_cmp(&d3(b.pos, pos)).unwrap()) {
                        goal = Some(cb.pos);
                        looted = true;
                    }
                }
            }
            if !looted {
                if let Some((_, est)) = ctx.focus {
                    goal = Some(est);
                    boost = d2(est, pos) > 80.0 && me.energy > 50.0 && me.in_zone;
                } else if self.p.camp {
                    let need_new = self.camp_spot.map_or(true, |(s, _)| d2([s[0], s[1], 0.0], [gc[0], gc[1], 0.0]) > gr - 10.0);
                    if need_new {
                        let a = self.rng.range(0.0, 2.0 * PI);
                        let r = self.rng.f().sqrt() * gr * 0.7;
                        self.camp_spot = Some(([gc[0] + a.cos() * r, gc[1] + a.sin() * r], t));
                    }
                    let (s, _) = self.camp_spot.unwrap();
                    if d2(pos, [s[0], s[1], 0.0]) > 6.0 {
                        goal = Some([s[0], s[1], pos[2]]);
                    }
                } else if ctx.passive {
                    // lazy: sit near the zone centre
                    let need_new = self.camp_spot.map_or(true, |(s, _)| d2([s[0], s[1], 0.0], [gc[0], gc[1], 0.0]) > gr - 10.0);
                    if need_new {
                        let a = self.rng.range(0.0, 2.0 * PI);
                        let r = self.rng.f().sqrt() * gr * 0.6;
                        self.camp_spot = Some(([gc[0] + a.cos() * r, gc[1] + a.sin() * r], t));
                    }
                    let (s, _) = self.camp_spot.unwrap();
                    if d2(pos, [s[0], s[1], 0.0]) > 6.0 {
                        goal = Some([s[0], s[1], pos[2]]);
                    }
                } else {
                    // hunter: follow loud sounds, else zone centre
                    for s in &o.sounds {
                        if matches!(s.kind.as_str(), "gun" | "mortar" | "laser" | "lightning" | "explosion") {
                            let d = match s.dist.as_str() {
                                "near" => 40.0,
                                "mid" => 100.0,
                                _ => 180.0,
                            };
                            self.sound_goal = Some(([pos[0] + s.bearing.cos() * d, pos[1] + s.bearing.sin() * d], t + 8.0));
                        }
                    }
                    if let Some((g, until)) = self.sound_goal {
                        if t < until {
                            goal = Some([g[0], g[1], pos[2]]);
                        }
                    }
                    if goal.is_none() {
                        goal = Some(zone_goal);
                    }
                }
                if self.p.boosty && goal.map_or(false, |g| d2(g, pos) > 90.0) && me.energy > 60.0 && me.in_zone {
                    boost = true;
                }
            }
            if target.is_none() && goal.is_some() {
                let g = goal.unwrap();
                aim = [(g[1] - pos[1]).atan2(g[0] - pos[0]), 0.0];
            }
        }

        // ---- resolve direction with obstacle steering
        let mut mv = [0.0, 0.0];
        let desired_yaw = if let Some(m) = mv_dir {
            Some(m[1].atan2(m[0]))
        } else if let Some(g) = goal {
            if d2(g, pos) > 2.0 {
                Some((g[1] - pos[1]).atan2(g[0] - pos[0]))
            } else {
                None
            }
        } else {
            None
        };
        let mut jump = a.jump.unwrap_or(false);
        if let Some(y) = desired_yaw {
            let (sy, ok) = self.steer(pos, y, &body);
            mv = [sy.cos(), sy.sin()];
            if let Some(m) = mv_dir {
                let l = (m[0] * m[0] + m[1] * m[1]).sqrt().min(1.0);
                mv = [mv[0] * l, mv[1] * l];
            }
            if !ok {
                jump = true;
                boost = true;
            }
        }
        // unstick
        if t < self.unstick_until {
            if let Some(y) = desired_yaw {
                let yy = y + self.unstick_dir * 1.3;
                mv = [yy.cos(), yy.sin()];
            }
            jump = true;
            boost = true;
        } else if self.hist.len() > 16 && desired_yaw.is_some() && !engaged {
            let first = self.hist[0].1;
            if d2(first, pos) < 2.5 {
                self.unstick_until = t + 1.0;
                self.unstick_dir = if self.rng.chance(0.5) { 1.0 } else { -1.0 };
                self.hist.clear();
            }
        }
        if me.energy < 10.0 && me.in_zone {
            boost = false;
        }
        if me.in_zone && (me.energy < 25.0) && !jump {
            boost = false;
        }
        let _ = size;
        a.mv = Some(mv);
        a.boost = Some(boost);
        a.jump = Some(jump);
        a.aim = Some(aim);
        a.fire = Some(fire);
        if let Some(g) = gun {
            if g.ammo.unwrap_or(1.0) < 1.0 && g.reload.unwrap_or(0.0) <= 0.0 {
                a.reload = Some(true);
            } else if target.is_none() && g.ammo.unwrap_or(12.0) < 6.0 && g.reload.unwrap_or(0.0) <= 0.0 {
                a.reload = Some(true);
            }
        }
        a
    }
}

// --------------------------------------------------------------------- squad

pub struct Squad {
    pub members: Vec<usize>, // slots, weakest first
    pub init_count: usize,
    pub player: usize,
    pub est: [f64; 3],
    est_t: f64,
    fuzz: f64,
    rng: Rng,
}

impl Squad {
    pub fn new(members: Vec<usize>, player: usize, fuzz: f64, seed: u64) -> Squad {
        let n = members.len();
        Squad { members, init_count: n, player, est: [0.0; 3], est_t: -100.0, fuzz, rng: Rng::new(seed) }
    }

    /// Update shared knowledge of the player and return the focus for hunters.
    pub fn update(&mut self, t: f64, truth: Option<[f64; 3]>, obs: &[Option<Obs>]) -> Option<(i64, [f64; 3])> {
        let truth = truth?;
        let mut seen = false;
        for &m in &self.members {
            if let Some(Some(o)) = obs.get(m) {
                if let Some(a) = o.mechs.iter().find(|a| a.id == self.player as i64) {
                    self.est = a.pos;
                    self.est_t = t;
                    seen = true;
                }
            }
        }
        if !seen && t - self.est_t > 3.0 {
            // rough tip-off (sound / zone pressure): fuzzy position
            self.est = [truth[0] + self.rng.gauss() * self.fuzz, truth[1] + self.rng.gauss() * self.fuzz, truth[2]];
            self.est_t = t;
        }
        Some((self.player as i64, self.est))
    }

    /// Number of hunters at time t given how many members remain.
    pub fn n_active(&self, t: f64, alive: usize) -> usize {
        if t >= 300.0 {
            return alive;
        }
        let killed = self.init_count - alive;
        let by_time = 1 + (((t - 30.0).max(0.0)) / 45.0) as usize;
        by_time.max(1 + killed).min(alive)
    }
}
