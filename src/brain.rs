//! v1 brain: nav-aware looter/fighter that leads shots, checks line of sight and faces attackers.
use crate::geom::*;
use crate::nav::Nav;
use crate::protocol::*;
use crate::world::World;
use serde_json::Value;
use std::f64::consts::PI;

pub struct Brain {
    pub slot: u32,
    pickup_radius: f64,
    mortar_speed: f64,
    mortar_g: f64,
    laser_charge: f64,
    lightning_range: f64,
    world: Option<World>,
    nav: Option<Nav>,
    path: Vec<[f64; 2]>,
    path_goal: [f64; 2],
    path_t: f64,
    hist: Vec<(f64, V3)>,
    unstick_until: f64,
    unstick_dir: f64,
    strafe_sign: f64,
    strafe_flip_at: f64,
    hit_bearing: Option<(f64, f64)>, // (time, bearing)
}

fn num(v: &Value, path: &[&str], d: f64) -> f64 {
    let mut c = v;
    for p in path {
        c = &c[*p];
    }
    c.as_f64().unwrap_or(d)
}

impl Brain {
    pub fn new(h: &Hello) -> Self {
        let c = &h.config;
        let world = if h.map.dir.is_empty() { None } else { World::load(&h.map.dir) };
        let nav = world.as_ref().map(Nav::build);
        Brain {
            slot: h.slot,
            pickup_radius: num(c, &["loot", "pickup_radius"], 4.0),
            mortar_speed: num(c, &["weapons", "mortar", "speed"], 72.0),
            mortar_g: num(c, &["weapons", "mortar", "gravity"], 22.0),
            laser_charge: num(c, &["weapons", "laser", "charge"], 0.75),
            lightning_range: num(c, &["weapons", "lightning", "range"], 45.0),
            world,
            nav,
            path: Vec::new(),
            path_goal: [1e9, 1e9],
            path_t: -10.0,
            hist: Vec::new(),
            unstick_until: -1.0,
            unstick_dir: 1.0,
            strafe_sign: 1.0,
            strafe_flip_at: 0.0,
            hit_bearing: None,
        }
    }

    fn weapon_pref(kind: &str) -> i32 {
        match kind {
            "laser" => 3,
            "lightning" => 2,
            "mortar" => 1,
            _ => 0,
        }
    }

    fn clear(&self, a: V3, b: V3) -> bool {
        match &self.world {
            Some(w) => !w.blocked(a, b),
            None => true,
        }
    }

    /// Steering direction toward `goal`, routing around obstacles. Falls back to a straight line.
    fn steer(&mut self, t: f64, pos: V3, goal: [f64; 2]) -> [f64; 2] {
        let straight = {
            let yaw = (goal[1] - pos[1]).atan2(goal[0] - pos[0]);
            [yaw.cos(), yaw.sin()]
        };
        let Some(nav) = &self.nav else { return straight };
        let moved = (goal[0] - self.path_goal[0]).hypot(goal[1] - self.path_goal[1]);
        if self.path.is_empty() || moved > 12.0 || t - self.path_t > 2.5 {
            if let Some(p) = nav.path(pos, goal, 40000) {
                self.path = p;
                self.path_goal = goal;
                self.path_t = t;
            } else {
                self.path.clear();
                return straight;
            }
        }
        while self.path.len() > 1 && (self.path[0][0] - pos[0]).hypot(self.path[0][1] - pos[1]) < 4.5 {
            self.path.remove(0);
        }
        // look ahead: aim at the farthest of the next few waypoints that is still straight-walkable
        let mut tgt = self.path[0];
        for k in 1..self.path.len().min(5) {
            let wp = self.path[k];
            let steps = ((wp[0] - pos[0]).hypot(wp[1] - pos[1]) / 3.0).ceil().max(1.0) as usize;
            let mut ok = true;
            for s in 1..steps {
                let f = s as f64 / steps as f64;
                if nav.is_blocked(pos[0] + (wp[0] - pos[0]) * f, pos[1] + (wp[1] - pos[1]) * f) {
                    ok = false;
                    break;
                }
            }
            if ok {
                tgt = wp;
            } else {
                break;
            }
        }
        let yaw = (tgt[1] - pos[1]).atan2(tgt[0] - pos[0]);
        [yaw.cos(), yaw.sin()]
    }

    pub fn act(&mut self, o: &Obs) -> Action {
        let me = &o.me;
        let pos = me.pos;
        let size = 1.0 + 0.05 * me.cubes;
        let muzzle = [pos[0], pos[1], pos[2] + 7.0 * size];
        let mut a = Action { tick: o.tick, ..Default::default() };

        // ---- events
        for e in &o.events {
            if e.t == "damaged" {
                if let (Some(b), Some(w)) = (e.bearing, e.weapon.as_deref()) {
                    if w != "zone" {
                        self.hit_bearing = Some((o.t, b));
                    }
                }
            }
        }

        // ---- upgrade offers
        if let Some(of) = &o.offer {
            const PREF: [&str; 12] = [
                "plating", "armor", "laser_charge", "laser_damage", "nanites", "gyros", "servos", "gun_damage",
                "gun_rate", "mortar_reload", "lightning_chain", "reactor",
            ];
            let mut best = 0usize;
            let mut best_rank = usize::MAX;
            for (i, c) in of.choices.iter().enumerate() {
                let r = PREF.iter().position(|p| *p == c.id).unwrap_or(99);
                if r < best_rank {
                    best_rank = r;
                    best = i;
                }
            }
            a.pick = Some(best);
        }

        self.hist.push((o.t, pos));
        self.hist.retain(|(t, _)| o.t - *t <= 2.0);

        // ---- perception
        let mut enemies: Vec<&Actor> = o.mechs.iter().filter(|m| m.kind == "mech").collect();
        enemies.sort_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap());
        let drones: Vec<&Actor> = o.mechs.iter().filter(|m| m.kind == "npc").collect();

        // ---- movement goal (zone)
        let z = &o.zone;
        let shrinking_soon = o.t + 15.0 >= z.shrink_start;
        let (gc, gr) = if shrinking_soon { (z.next_center, z.next_radius) } else { (z.center, z.radius) };
        let goal = [gc[0], gc[1], pos[2]];
        let d_goal = dist2d(pos, goal);
        let mut boost = !me.in_zone;
        let mut want: Option<[f64; 2]> = if d_goal < gr * 0.5 { None } else { Some([goal[0], goal[1]]) };

        // loot
        let slot1_empty = me.weapons.get(1).map(|w| w.is_none()).unwrap_or(true);
        let mut pick_target: Option<&Pickup> = None;
        if slot1_empty && enemies.is_empty() {
            let mut best = f64::MAX;
            for p in &o.pickups {
                let d = dist2d(pos, p.pos);
                let sc = d - 20.0 * Self::weapon_pref(&p.kind) as f64;
                if sc < best && d < 90.0 && p.kind != "gun" {
                    best = sc;
                    pick_target = Some(p);
                }
            }
        }
        if let Some(p) = pick_target {
            want = Some([p.pos[0], p.pos[1]]);
            if dist2d(pos, p.pos) < self.pickup_radius - 0.5 && (p.pos[2] - pos[2]).abs() < 3.5 {
                a.take = Some(p.id);
            }
        } else if enemies.is_empty() {
            let mut best = f64::MAX;
            for c in &o.cubes {
                let d = dist2d(pos, c.pos);
                if d < best && d < 50.0 {
                    best = d;
                    want = Some([c.pos[0], c.pos[1]]);
                }
            }
        }

        // ---- combat
        let mut aim = me.aim;
        let mut fire = [false, false];
        let mut mv_override: Option<[f64; 2]> = None;
        let gun = me.weapons.get(0).and_then(|w| w.as_ref());
        let w1 = me.weapons.get(1).and_then(|w| w.as_ref());
        let gun_speed = gun.and_then(|g| g.stats.get("speed")).copied().unwrap_or(252.0);
        let gun_range = gun.and_then(|g| g.stats.get("range")).copied().unwrap_or(230.0);
        let mut engaged = false;
        let target = enemies.first().copied().or_else(|| {
            drones.iter().copied().filter(|d| dist(d.pos, pos) < 35.0).min_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap())
        });
        let hp_frac = me.hp / me.max_hp.max(1.0);
        if let Some(t) = target {
            let tpos = [t.pos[0], t.pos[1], t.pos[2] + 4.5];
            let d = dist(muzzle, tpos);
            let los = self.clear(muzzle, tpos);
            let (ip, _tof) = intercept(muzzle, tpos, t.vel, gun_speed);
            let desired = [yaw_to(muzzle, ip), pitch_to(muzzle, ip)];
            aim = desired;
            let yaw_err = wrap(desired[0] - me.aim[0]).abs();
            let pit_err = (desired[1] - me.aim[1]).abs();
            let tol = (2.0 / d.max(5.0)).atan() * 0.9;
            let aligned = yaw_err < tol && pit_err < tol * 1.5;
            engaged = t.kind == "mech" || d < 35.0;
            if aligned && d < gun_range && los {
                fire[0] = true;
            }
            if let Some(w) = w1 {
                match w.kind.as_str() {
                    "laser" => {
                        let ch = w.charge.unwrap_or(0.0);
                        if los && d < 270.0 {
                            if ch >= self.laser_charge - 0.01 {
                                fire[1] = false;
                            } else if w.cooldown <= 0.05 && yaw_err < tol * 3.0 {
                                fire[1] = true;
                            } else if ch > 0.0 && yaw_err < tol * 3.0 {
                                fire[1] = true;
                            }
                        }
                    }
                    "lightning" => {
                        if los && d < self.lightning_range && yaw_err < 0.3 && me.energy > 8.0 {
                            fire[1] = true;
                        }
                    }
                    "mortar" => {
                        if d > 15.0 && d < 200.0 {
                            if let Some(p) = mortar_pitch(dist2d(muzzle, tpos), tpos[2] - (pos[2] + 9.5), self.mortar_speed, self.mortar_g) {
                                aim = [yaw_to(muzzle, tpos), p];
                                if wrap(aim[0] - me.aim[0]).abs() < 0.05 && (aim[1] - me.aim[1]).abs() < 0.05 {
                                    fire[1] = true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            if t.kind == "mech" {
                if o.t > self.strafe_flip_at {
                    self.strafe_sign = -self.strafe_sign;
                    self.strafe_flip_at = o.t + 1.2 + (o.tick % 7) as f64 * 0.15;
                }
                let toward = yaw_to(pos, t.pos);
                let low = hp_frac < 0.3 && enemies.len() >= 1;
                let radial = if low { -1.0 } else if d > 60.0 { 0.6 } else if d < 35.0 { -0.8 } else { 0.0 };
                let mut perp = toward + self.strafe_sign * PI / 2.0;
                let mut m = [perp.cos() + radial * toward.cos(), perp.sin() + radial * toward.sin()];
                if let Some(nav) = &self.nav {
                    let probe = [pos[0] + m[0] * 7.0, pos[1] + m[1] * 7.0];
                    if nav.is_blocked(probe[0], probe[1]) {
                        self.strafe_sign = -self.strafe_sign;
                        perp = toward + self.strafe_sign * PI / 2.0;
                        m = [perp.cos() + radial * toward.cos(), perp.sin() + radial * toward.sin()];
                    }
                }
                mv_override = Some(m);
                if low {
                    boost = true;
                }
            }
        } else if let Some(cr) = o.crates.iter().filter(|c| dist(c.pos, pos) < 60.0).min_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap()) {
            let tpos = [cr.pos[0], cr.pos[1], cr.pos[2] + 1.2];
            aim = [yaw_to(muzzle, tpos), pitch_to(muzzle, tpos)];
            if self.clear(muzzle, tpos) && wrap(aim[0] - me.aim[0]).abs() < 0.05 && (aim[1] - me.aim[1]).abs() < 0.06 {
                fire[0] = true;
            }
            want = if dist2d(pos, cr.pos) > 25.0 { Some([cr.pos[0], cr.pos[1]]) } else { None };
        } else if let Some((ht, hb)) = self.hit_bearing {
            // someone we can't see just hurt us: turn to face them (rear armor is x1.5)
            if o.t - ht < 2.0 {
                aim = [hb, 0.0];
            }
        }

        // ---- movement
        let mut mv = mv_override.unwrap_or([0.0, 0.0]);
        if mv_override.is_none() {
            if let Some(w) = want {
                mv = self.steer(o.t, pos, w);
                if target.is_none() && !(self.hit_bearing.map(|(ht, _)| o.t - ht < 2.0).unwrap_or(false)) && o.crates.iter().all(|c| dist(c.pos, pos) >= 60.0) {
                    aim = [mv[1].atan2(mv[0]), 0.0];
                }
            }
        }

        // ---- unstick
        if o.t < self.unstick_until {
            let yaw = mv[1].atan2(mv[0]) + self.unstick_dir * 1.2;
            mv = [yaw.cos(), yaw.sin()];
            a.jump = Some(true);
            boost = true;
        } else if self.hist.len() > 15 && (mv[0] != 0.0 || mv[1] != 0.0) {
            let first = self.hist[0].1;
            if dist2d(first, pos) < 2.5 && !engaged {
                self.unstick_until = o.t + 1.0;
                self.unstick_dir = if (o.tick / 20) % 2 == 0 { 1.0 } else { -1.0 };
                self.hist.clear();
                self.path.clear();
            }
        }

        if me.energy < 12.0 && me.in_zone {
            boost = false;
        }
        a.mv = Some(mv);
        a.boost = Some(boost);
        a.aim = Some(aim);
        a.fire = Some(fire);
        if let Some(g) = gun {
            if g.ammo.unwrap_or(1.0) < 1.0 && g.reload.unwrap_or(0.0) <= 0.0 {
                a.reload = Some(true);
            }
        }
        a
    }
}
