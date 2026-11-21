//! v1 brain: nav-aware looter/fighter that leads shots, checks line of sight and faces attackers.
use crate::geom::*;
use crate::nav::Nav;
use crate::protocol::*;
use crate::world::World;
use serde_json::Value;
use std::f64::consts::PI;

const SEEN_CELL: f64 = 24.0;

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
    cover: Option<[f64; 2]>,
    cover_t: f64,
    seen: Vec<f64>,
    sgw: usize,
    sgh: usize,
    sx0: f64,
    sy0: f64,
    explore: Option<[f64; 2]>,
    explore_t: f64,
    last_alive: u32,
    others_dead: u32,
    pub royale: bool,
}

fn hp_frac_early(me: &SelfObs) -> f64 {
    me.hp / me.max_hp.max(1.0)
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
        let (sx0, sy0, sgw, sgh) = match &world {
            Some(w) => (w.bounds[0], w.bounds[1], ((w.bounds[2] - w.bounds[0]) / SEEN_CELL).ceil() as usize + 1, ((w.bounds[3] - w.bounds[1]) / SEEN_CELL).ceil() as usize + 1),
            None => (0.0, 0.0, 0, 0),
        };
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
            cover: None,
            cover_t: -10.0,
            seen: vec![-1.0; sgw * sgh],
            sgw,
            sgh,
            sx0,
            sy0,
            explore: None,
            explore_t: -10.0,
            last_alive: 0,
            others_dead: 0,
            royale: false,
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

    /// Nearest nav cell around `pos` that no known threat can see (feet-to-head) and that is inside the zone.
    fn find_cover(&self, pos: V3, threats: &[V3], zone: &Zone) -> Option<[f64; 2]> {
        let (w, nav) = (self.world.as_ref()?, self.nav.as_ref()?);
        for r in [8.0f64, 14.0, 20.0, 28.0, 36.0] {
            let mut best: Option<([f64; 2], f64)> = None;
            for k in 0..16 {
                let ang = k as f64 * PI / 8.0;
                let (x, y) = (pos[0] + r * ang.cos(), pos[1] + r * ang.sin());
                if nav.is_blocked(x, y) {
                    continue;
                }
                if (x - zone.center[0]).hypot(y - zone.center[1]) > zone.radius - 6.0 {
                    continue;
                }
                let g = w.ground(x, y);
                let hidden = threats.iter().all(|t| {
                    let eye = [t[0], t[1], t[2] + 8.0];
                    w.blocked(eye, [x, y, g + 4.5]) && w.blocked(eye, [x, y, g + 8.5])
                });
                if hidden {
                    let far = threats.iter().map(|t| (t[0] - x).hypot(t[1] - y)).fold(f64::MAX, f64::min);
                    if best.map(|b| far > b.1).unwrap_or(true) {
                        best = Some(([x, y], far));
                    }
                }
            }
            if let Some((c, _)) = best {
                return Some(c);
            }
        }
        None
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
            if let Some(p) = nav.path(pos, goal, 12000) {
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

        // ---- FFA detector: house squads never kill each other, so deaths we did not cause mean a free-for-all
        let my_kills_now = o.events.iter().filter(|e| e.t == "kill").count() as u32;
        if self.last_alive > 0 && o.alive < self.last_alive {
            let died = self.last_alive - o.alive;
            self.others_dead += died.saturating_sub(my_kills_now);
            if self.others_dead >= 3 && !self.royale {
                self.royale = true;
                eprintln!("ROYALE mode at t={:.1} alive={}", o.t, o.alive);
            }
        }
        self.last_alive = o.alive;

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
            const PREF_G: [&str; 16] = [
                "laser_charge", "laser_damage", "plating", "armor", "gun_damage", "gun_rate", "lightning_efficiency",
                "nanites", "lightning_range", "gyros", "gun_velocity", "gun_mag", "mortar_reload", "servos", "lightning_chain", "reactor",
            ];
            const PREF_R: [&str; 16] = [
                "plating", "armor", "laser_charge", "nanites", "laser_damage", "gun_damage", "lightning_efficiency", "gun_rate",
                "sensors", "gyros", "lightning_range", "servos", "gun_velocity", "gun_mag", "mortar_reload", "reactor",
            ];
            let pref: &[&str] = if self.royale { &PREF_R } else { &PREF_G };
            let mut best = 0usize;
            let mut best_rank = usize::MAX;
            for (i, c) in of.choices.iter().enumerate() {
                let r = pref.iter().position(|p| *p == c.id).unwrap_or(99);
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
        let held_kind: Option<String> = me.weapons.get(1).and_then(|w| w.as_ref()).map(|w| w.kind.clone());
        let held_pref = held_kind.as_deref().map(Self::weapon_pref).unwrap_or(-1);
        let mut pick_target: Option<&Pickup> = None;
        if enemies.is_empty() {
            let mut best = f64::MAX;
            for p in &o.pickups {
                if p.kind == "gun" || Self::weapon_pref(&p.kind) <= held_pref {
                    continue;
                }
                if me.weapons.iter().flatten().any(|w| w.kind == p.kind) {
                    continue;
                }
                let d = dist2d(pos, p.pos);
                let reach = if held_pref < 0 { 90.0 } else { 45.0 };
                let sc = d - 20.0 * Self::weapon_pref(&p.kind) as f64;
                if sc < best && d < reach {
                    best = sc;
                    pick_target = Some(p);
                }
            }
        }
        if let Some(p) = pick_target {
            want = Some([p.pos[0], p.pos[1]]);
            if dist2d(pos, p.pos) < self.pickup_radius - 0.5 && (p.pos[2] - pos[2]).abs() < 3.5 {
                a.take = Some(p.id);
                if held_pref >= 0 {
                    a.drop = Some(1);
                }
            }
        } else if enemies.is_empty() {
            let mut best = f64::MAX;
            let mut got_cube = false;
            for c in &o.cubes {
                let d = dist2d(pos, c.pos);
                if d < best && d < 50.0 {
                    best = d;
                    want = Some([c.pos[0], c.pos[1]]);
                    got_cube = true;
                }
            }
            if !got_cube && hp_frac_early(me) > 0.5 {
                if let Some(dr) = drones.iter().filter(|d| dist(d.pos, pos) < 80.0).min_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap()) {
                    want = if dist2d(pos, dr.pos) > 30.0 { Some([dr.pos[0], dr.pos[1]]) } else { None };
                } else if !o.crates.is_empty() {
                    // crates handled in combat section
                } else if o.t < 170.0 {
                    // fall through to exploration below
                }
            }
        }

        // ---- supply drops: grab the free cubes + full repair when it is safe
        let mut drop_goal: Option<[f64; 2]> = None;
        if enemies.is_empty() && hp_frac_early(me) > 0.4 {
            for d in &o.drops {
                let dd = dist2d(pos, d.pos);
                let mine = d.holder.map(|h| h == me.id).unwrap_or(true);
                let eta = dd / 14.0;
                if mine && dd < 150.0 && d.lands_in < eta + 14.0 {
                    if drop_goal.map(|g| dd < dist2d(pos, [g[0], g[1], 0.0])).unwrap_or(true) {
                        drop_goal = Some([d.pos[0], d.pos[1]]);
                    }
                }
            }
        }
        if let Some(g) = drop_goal {
            // inside the ring: stand still to capture
            let in_ring = o.drops.iter().any(|d| dist2d(pos, d.pos) < d.radius - 2.0 && d.lands_in <= 0.0);
            want = if in_ring { None } else { Some(g) };
            if !in_ring && dist2d(pos, [g[0], g[1], 0.0]) > 20.0 {
                boost = boost || me.energy > 60.0;
            }
        }
        // ---- incoming mortar shells: step out of the predicted blast
        let mut mortar_evade: Option<[f64; 2]> = None;
        for pr in o.projectiles.iter().filter(|p| p.kind == "mortar") {
            let (mut x, mut y, mut zz) = (pr.pos[0], pr.pos[1], pr.pos[2]);
            let (vx, vy, mut vz) = (pr.vel[0], pr.vel[1], pr.vel[2]);
            let mut t = 0.0;
            let mut hit = None;
            while t < 6.0 {
                x += vx * 0.05;
                y += vy * 0.05;
                vz -= self.mortar_g * 0.05;
                zz += vz * 0.05;
                t += 0.05;
                let gnd = self.world.as_ref().map(|w| w.ground(x, y)).unwrap_or(0.0);
                if zz <= gnd {
                    hit = Some([x, y, t]);
                    break;
                }
            }
            if let Some([ix, iy, tt]) = hit {
                let dd = (ix - pos[0]).hypot(iy - pos[1]);
                if dd < 11.0 && tt > 0.2 {
                    let away = (pos[1] - iy).atan2(pos[0] - ix);
                    mortar_evade = Some([away.cos(), away.sin()]);
                }
            }
        }
        if mortar_evade.is_some() {
            boost = me.energy > 15.0;
        }

        // ---- exploration bookkeeping: mark nearby cells as seen, pick the nearest unseen safe cell
        if self.sgw > 0 {
            let r = 44.0;
            let (i0, i1) = (((pos[0] - r - self.sx0) / SEEN_CELL).floor().max(0.0) as usize, (((pos[0] + r - self.sx0) / SEEN_CELL).floor().max(0.0) as usize).min(self.sgw - 1));
            let (j0, j1) = (((pos[1] - r - self.sy0) / SEEN_CELL).floor().max(0.0) as usize, (((pos[1] + r - self.sy0) / SEEN_CELL).floor().max(0.0) as usize).min(self.sgh - 1));
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let (cx, cy) = (self.sx0 + (i as f64 + 0.5) * SEEN_CELL, self.sy0 + (j as f64 + 0.5) * SEEN_CELL);
                    if (cx - pos[0]).hypot(cy - pos[1]) < r {
                        self.seen[j * self.sgw + i] = o.t;
                    }
                }
            }
        }
        let calm = enemies.is_empty() && hp_frac_early(me) > 0.6;
        if calm && o.t < 170.0 && self.sgw > 0 && (self.explore.is_none() || o.t - self.explore_t > 1.5) {
            self.explore_t = o.t;
            let (sc, sr) = if o.t + 10.0 >= z.shrink_start { (z.next_center, z.next_radius) } else { (z.center, z.radius) };
            let mut best: Option<([f64; 2], f64)> = None;
            for j in 0..self.sgh {
                for i in 0..self.sgw {
                    if self.seen[j * self.sgw + i] >= 0.0 {
                        continue;
                    }
                    let (cx, cy) = (self.sx0 + (i as f64 + 0.5) * SEEN_CELL, self.sy0 + (j as f64 + 0.5) * SEEN_CELL);
                    if (cx - sc[0]).hypot(cy - sc[1]) > sr * 0.9 {
                        continue;
                    }
                    if self.nav.as_ref().map(|n| n.is_blocked(cx, cy)).unwrap_or(false) {
                        continue;
                    }
                    let d = (cx - pos[0]).hypot(cy - pos[1]);
                    if d < 170.0 && best.map(|b| d < b.1).unwrap_or(true) {
                        best = Some(([cx, cy], d));
                    }
                }
            }
            self.explore = best.map(|b| b.0);
        }
        if !calm {
            self.explore = None;
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
        let royale = self.royale;
        let target = (if royale {
            enemies.iter().copied().min_by(|a, b| {
                let sc = |m: &Actor| dist(m.pos, pos) + 160.0 * (m.hp.unwrap_or(1400.0) / m.max_hp.unwrap_or(1400.0).max(1.0));
                sc(a).partial_cmp(&sc(b)).unwrap()
            })
        } else {
            enemies.first().copied()
        })
        .or_else(|| {
            drones.iter().copied().filter(|d| dist(d.pos, pos) < 35.0).min_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap())
        });
        let hp_frac = me.hp / me.max_hp.max(1.0);
        // ---- heal-cycle: when hurt with threats around, break line of sight and regenerate
        let threats: Vec<V3> = enemies.iter().filter(|m| dist(m.pos, pos) < 160.0).take(4).map(|m| m.pos).collect();
        let hurt = hp_frac < 0.5 && !threats.is_empty();
        let mut hiding = false;
        if hurt || (self.cover.is_some() && hp_frac < 0.8 && o.t - self.cover_t < 8.0) {
            if o.t - self.cover_t > 0.6 || self.cover.is_none() {
                if !threats.is_empty() {
                    self.cover = self.find_cover(pos, &threats, &o.zone);
                    self.cover_t = o.t;
                }
            }
            if let Some(c) = self.cover {
                let at = (c[0] - pos[0]).hypot(c[1] - pos[1]) < 4.0;
                if !at {
                    want = Some(c);
                    hiding = true;
                } else {
                    hiding = true;
                    want = None;
                }
            }
        } else {
            self.cover = None;
        }
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
                        // hitscan: aim at where the target will be when the next tick resolves, not at the gun lead
                        let full = w.stats.get("charge").copied().unwrap_or(self.laser_charge);
                        let range = w.stats.get("range").copied().unwrap_or(280.0) - 10.0;
                        let ch = w.charge.unwrap_or(0.0);
                        let hp_pt = [tpos[0] + t.vel[0] * 0.1, tpos[1] + t.vel[1] * 0.1, tpos[2] + t.vel[2] * 0.1];
                        let la = [yaw_to(muzzle, hp_pt), pitch_to(muzzle, hp_pt)];
                        let l_tol = (2.0 / d.max(5.0)).atan() * 0.8;
                        let l_aligned = wrap(la[0] - me.aim[0]).abs() < l_tol && (la[1] - me.aim[1]).abs() < l_tol * 1.5;
                        if ch > 0.0 {
                            // charging: once full, keep holding until the beam would land, then let go
                            aim = la;
                            if ch >= full - 0.01 {
                                fire[1] = !(l_aligned && los);
                            } else {
                                fire[1] = true;
                            }
                        } else if los && d < range && w.cooldown <= 0.05 && yaw_err < tol * 4.0 {
                            fire[1] = true; // start charging
                        }
                    }
                    "lightning" => {
                        let range = w.stats.get("range").copied().unwrap_or(self.lightning_range);
                        let cone = w.stats.get("cone_deg").copied().unwrap_or(30.0).to_radians();
                        let ly = yaw_to(muzzle, tpos);
                        if d < range && los {
                            // lightning ignores pitch and lead: keep the yaw inside the cone
                            if wrap(ly - me.aim[0]).abs() < cone * 0.8 && me.energy > 6.0 {
                                fire[1] = true;
                            }
                        }
                    }
                    "mortar" => {
                        let ms = w.stats.get("speed").copied().unwrap_or(self.mortar_speed);
                        if (!los || d > 110.0) && w.cooldown <= 0.1 && d > 18.0 && d < 220.0 {
                            let (mp, _) = intercept(muzzle, tpos, t.vel, ms * 0.8);
                            if let Some(p) = mortar_pitch(dist2d(muzzle, mp), mp[2] - (pos[2] + 9.5 * size), ms, self.mortar_g) {
                                aim = [yaw_to(muzzle, mp), p];
                                fire[0] = false;
                                if wrap(aim[0] - me.aim[0]).abs() < 0.04 && (aim[1] - me.aim[1]).abs() < 0.04 {
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
                // preferred engagement band depends on what we carry (laser duels at range, lightning brawls)
                let (near, far) = match w1.map(|w| w.kind.as_str()) {
                    Some("laser") => (80.0, 150.0),
                    Some("lightning") => (20.0, 38.0),
                    Some("mortar") => (60.0, 140.0),
                    _ => (40.0, 90.0),
                };
                let t_frac = t.hp.unwrap_or(1.0) / t.max_hp.unwrap_or(1400.0).max(1.0);
                let finishing = t_frac < 0.35 && hp_frac > 0.5;
                let radial = if low {
                    -1.0
                } else if finishing && d > 25.0 {
                    0.7
                } else if d > far && !(royale && t_frac > 0.5) {
                    0.6
                } else if d < near {
                    -0.8
                } else {
                    0.0
                };
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
                // dodge charging lasers aimed at us
                for e in enemies.iter().filter(|e| e.charging && dist(e.pos, pos) < 280.0) {
                    let to_me = yaw_to(e.pos, pos);
                    let off = wrap(e.aim[0] - to_me).abs();
                    let dd = dist(e.pos, pos);
                    if off < (3.5 / dd.max(8.0)).atan() + 0.12 {
                        let side = if wrap(e.aim[0] - to_me) > 0.0 { -1.0 } else { 1.0 };
                        let pd = e.aim[0] + side * PI / 2.0;
                        mv_override = Some([pd.cos(), pd.sin()]);
                        boost = true;
                        break;
                    }
                }
                if hiding && self.cover.is_some() {
                    mv_override = None; // let steering take us to cover; keep shooting if LOS allows
                }
            }
        } else if let Some(cr) = o.crates.iter().filter(|c| dist(c.pos, pos) < 60.0).min_by(|a, b| dist(a.pos, pos).partial_cmp(&dist(b.pos, pos)).unwrap()) {
            let tpos = [cr.pos[0], cr.pos[1], cr.pos[2] + 1.2];
            aim = [yaw_to(muzzle, tpos), pitch_to(muzzle, tpos)];
            if self.clear(muzzle, tpos) && wrap(aim[0] - me.aim[0]).abs() < 0.05 && (aim[1] - me.aim[1]).abs() < 0.06 {
                fire[0] = true;
            }
            want = if dist2d(pos, cr.pos) > 25.0 { Some([cr.pos[0], cr.pos[1]]) } else { None };
        } else if let Some((ht, hb)) = self.hit_bearing.filter(|(ht, _)| o.t - *ht < 2.0) {
            // someone we can't see just hurt us: turn to face them (rear armor is x1.5)
            let _ = ht;
            aim = [hb, 0.0];
        } else if let Some(sd) = o
            .sounds
            .iter()
            .filter(|s| matches!(s.kind.as_str(), "gun" | "laser" | "laser_charge" | "lightning" | "mortar") && s.dist != "far" && s.age < 0.5)
            .min_by(|a, b| a.age.partial_cmp(&b.age).unwrap())
        {
            // heard a weapon nearby: look that way so the threat enters our vision wedge
            self.hit_bearing = Some((o.t - 1.2, sd.bearing));
            aim = [sd.bearing, 0.0];
        }

        // ---- movement
        let loot_free = pick_target.is_none() && drop_goal.is_none()
            && o.cubes.iter().all(|c| dist2d(pos, c.pos) >= 50.0)
            && drones.iter().all(|d| dist(d.pos, pos) >= 80.0)
            && o.crates.iter().all(|c| dist(c.pos, pos) >= 60.0);
        if loot_free && target.is_none() && !hiding {
            if let Some(e) = self.explore {
                want = Some(e);
            }
        }
        let mut mv = mv_override.unwrap_or([0.0, 0.0]);
        if mv_override.is_none() {
            if let Some(w) = want {
                mv = self.steer(o.t, pos, w);
                let facing_hit = self.hit_bearing.map(|(ht, _)| o.t - ht < 2.0).unwrap_or(false);
                if target.is_none() && !facing_hit && o.crates.iter().all(|c| dist(c.pos, pos) >= 60.0) {
                    // sweep the torso while travelling: widens the 120-degree vision wedge
                    let heading = mv[1].atan2(mv[0]);
                    aim = [heading + 0.9 * (o.t * 1.6).sin(), 0.0];
                }
            }
        }

        if let Some(m) = mortar_evade {
            mv = m;
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
            let mag = g.stats.get("mag").copied().unwrap_or(12.0);
            let ammo = g.ammo.unwrap_or(mag);
            if g.reload.unwrap_or(0.0) <= 0.0 && (ammo < 1.0 || (target.is_none() && ammo < mag * 0.6)) {
                a.reload = Some(true);
            }
        }
        a
    }
}
