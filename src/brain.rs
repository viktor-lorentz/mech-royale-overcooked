//! v0 brain: zone-seeking looter that leads shots. Replaced step by step by the full plan.
use crate::geom::*;
use crate::protocol::*;
use serde_json::Value;
use std::f64::consts::PI;

pub struct Brain {
    pub slot: u32,
    pickup_radius: f64,
    mortar_speed: f64,
    mortar_g: f64,
    laser_charge: f64,
    lightning_range: f64,
    hist: Vec<(f64, V3)>,
    unstick_until: f64,
    unstick_dir: f64,
    strafe_sign: f64,
    strafe_flip_at: f64,
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
        Brain {
            slot: h.slot,
            pickup_radius: num(c, &["loot", "pickup_radius"], 4.0),
            mortar_speed: num(c, &["weapons", "mortar", "speed"], 72.0),
            mortar_g: num(c, &["weapons", "mortar", "gravity"], 22.0),
            laser_charge: num(c, &["weapons", "laser", "charge"], 0.75),
            lightning_range: num(c, &["weapons", "lightning", "range"], 45.0),
            hist: Vec::new(),
            unstick_until: -1.0,
            unstick_dir: 1.0,
            strafe_sign: 1.0,
            strafe_flip_at: 0.0,
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

    pub fn act(&mut self, o: &Obs) -> Action {
        let me = &o.me;
        let pos = me.pos;
        let size = 1.0 + 0.05 * me.cubes;
        let muzzle = [pos[0], pos[1], pos[2] + 7.0 * size];
        let mut a = Action { tick: o.tick, ..Default::default() };

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

        // ---- track position history for unstick
        self.hist.push((o.t, pos));
        self.hist.retain(|(t, _)| o.t - *t <= 2.0);

        // ---- pick targets
        let mut enemies: Vec<&Actor> = o.mechs.iter().filter(|m| m.kind == "mech").collect();
        enemies.sort_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap());
        let drones: Vec<&Actor> = o.mechs.iter().filter(|m| m.kind == "npc").collect();

        // ---- movement goal
        let z = &o.zone;
        let shrinking_soon = o.t + 12.0 >= z.shrink_start;
        let (gc, gr) = if shrinking_soon { (z.next_center, z.next_radius) } else { (z.center, z.radius) };
        let goal = [gc[0], gc[1], pos[2]];
        let d_goal = dist2d(pos, goal);
        let mut mv = [0.0, 0.0];
        let mut boost = !me.in_zone;
        let mut want = Some(goal);
        if d_goal < gr * 0.55 {
            want = None;
        }

        // loot: nearest visible weapon pickup when slot 1 empty
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
            want = Some(p.pos);
            if dist2d(pos, p.pos) < self.pickup_radius - 0.5 && (p.pos[2] - pos[2]).abs() < 3.5 {
                a.take = Some(p.id);
            }
        } else if enemies.is_empty() {
            // cubes
            let mut best = f64::MAX;
            for c in &o.cubes {
                let d = dist2d(pos, c.pos);
                if d < best && d < 50.0 {
                    best = d;
                    want = Some(c.pos);
                }
            }
        }

        // ---- combat
        let mut aim = me.aim;
        let mut fire = [false, false];
        let gun = me.weapons.get(0).and_then(|w| w.as_ref());
        let w1 = me.weapons.get(1).and_then(|w| w.as_ref());
        let gun_speed = gun.and_then(|g| g.stats.get("speed")).copied().unwrap_or(252.0);
        let gun_range = gun.and_then(|g| g.stats.get("range")).copied().unwrap_or(230.0);
        let mut engaged = false;
        let target = enemies.first().copied().or_else(|| {
            // shoot drones only when close (they give cubes)
            drones.iter().copied().filter(|d| dist(d.pos, pos) < 35.0).min_by(|a, b| {
                dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap()
            })
        });
        if let Some(t) = target {
            let tpos = [t.pos[0], t.pos[1], t.pos[2] + 4.5];
            let d = dist(muzzle, tpos);
            let (ip, _tof) = intercept(muzzle, tpos, t.vel, gun_speed);
            let desired = [yaw_to(muzzle, ip), pitch_to(muzzle, ip)];
            aim = desired;
            let yaw_err = wrap(desired[0] - me.aim[0]).abs();
            let pit_err = (desired[1] - me.aim[1]).abs();
            let tol = (2.0 / d.max(5.0)).atan() * 0.9;
            let aligned = yaw_err < tol && pit_err < tol * 1.5;
            engaged = t.kind == "mech" || d < 35.0;
            if aligned && d < gun_range {
                fire[0] = true;
            }
            if let Some(w) = w1 {
                match w.kind.as_str() {
                    "laser" => {
                        let ch = w.charge.unwrap_or(0.0);
                        if w.cooldown <= 0.05 && d < 270.0 {
                            if ch >= self.laser_charge - 0.01 {
                                fire[1] = false; // release
                            } else if yaw_err < tol * 3.0 {
                                fire[1] = true; // charge
                            }
                        }
                        if ch > 0.0 && ch < self.laser_charge && yaw_err < tol * 3.0 {
                            fire[1] = true;
                        }
                    }
                    "lightning" => {
                        if d < self.lightning_range && yaw_err < 0.3 && me.energy > 8.0 {
                            fire[1] = true;
                        }
                    }
                    "mortar" => {
                        if d > 15.0 && d < 200.0 {
                            if let Some(p) = mortar_pitch(dist2d(muzzle, tpos), tpos[2] - (pos[2] + 9.5), self.mortar_speed, self.mortar_g) {
                                aim = [yaw_to(muzzle, tpos), p];
                                let ye = wrap(aim[0] - me.aim[0]).abs();
                                let pe = (aim[1] - me.aim[1]).abs();
                                if ye < 0.05 && pe < 0.05 {
                                    fire[1] = true;
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
            // movement while fighting: hold ~50 m and strafe
            if t.kind == "mech" {
                if o.t > self.strafe_flip_at {
                    self.strafe_sign = -self.strafe_sign;
                    self.strafe_flip_at = o.t + 1.2 + (o.tick % 7) as f64 * 0.15;
                }
                let toward = yaw_to(pos, t.pos);
                let perp = toward + self.strafe_sign * PI / 2.0;
                let radial = if d > 60.0 { 0.6 } else if d < 35.0 { -0.8 } else { 0.0 };
                mv = [perp.cos() + radial * toward.cos(), perp.sin() + radial * toward.sin()];
                want = None;
            }
        } else if let Some(cr) = o.crates.iter().filter(|c| dist(c.pos, pos) < 60.0).min_by(|a, b| {
            dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap()
        }) {
            // chew crates for cubes and weapons
            let tpos = [cr.pos[0], cr.pos[1], cr.pos[2] + 1.2];
            aim = [yaw_to(muzzle, tpos), pitch_to(muzzle, tpos)];
            if wrap(aim[0] - me.aim[0]).abs() < 0.05 && (aim[1] - me.aim[1]).abs() < 0.06 {
                fire[0] = true;
            }
            if dist2d(pos, cr.pos) > 25.0 {
                want = Some(cr.pos);
            } else {
                want = None;
            }
        }

        if mv == [0.0, 0.0] {
            if let Some(w) = want {
                let yaw = yaw_to(pos, w);
                mv = [yaw.cos(), yaw.sin()];
                if target.is_none() {
                    aim = [yaw, 0.0];
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
            }
        }

        // low energy: don't boost
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
