//! NPC drones, zone, supply drops, regen, cube pickups and the master tick.
use crate::combat::{dir_of, dist3};
use crate::phys::Body;
use crate::sim::*;
use serde_json::json;

impl Sim {
    /// Advance the simulation by one tick (0.05 s).
    pub fn step(&mut self) {
        let n = self.mechs.len();
        for i in 0..n {
            if self.mechs[i].alive {
                self.phys_mech(i);
            }
        }
        self.separate_mechs();
        for i in 0..n {
            self.weapons_tick(i);
        }
        self.npcs_tick();
        self.step_projs();
        self.zone_tick();
        self.cube_pickups();
        self.drops_tick();
        for i in 0..n {
            self.offers_tick(i);
        }
        self.process_deaths();
        self.tick += 1;
        if !self.over && self.t() >= self.cfg.duration - 1e-9 {
            self.finish_timeout();
        }
    }

    fn finish_timeout(&mut self) {
        self.over = true;
        let hps: Vec<(usize, f64)> = self.mechs.iter().filter(|m| m.alive).map(|m| (m.id, m.hp)).collect();
        for &(id, hp) in &hps {
            let better = hps.iter().filter(|(_, h)| *h > hp + 1e-9).count();
            self.mechs[id].place = (better + 1) as u32;
        }
    }

    fn zone_tick(&mut self) {
        let t = self.t();
        let dt = self.cfg.dt;
        let (c, r, _, _, _, _, dps) = self.zone_at(t);
        for i in 0..self.mechs.len() {
            if !self.mechs[i].alive {
                continue;
            }
            let p = self.mechs[i].pos;
            let inside = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt() <= r;
            self.mechs[i].in_zone = inside;
            if !inside {
                self.zone_damage(i, dps * dt);
            } else {
                let m = &mut self.mechs[i];
                if t - m.last_dmg_t >= m.ms.regen_delay && m.hp < m.max_hp {
                    m.hp = (m.hp + m.ms.regen_per_s * dt).min(m.max_hp);
                }
            }
        }
    }

    fn cube_pickups(&mut self) {
        if self.cubes.is_empty() {
            return;
        }
        let r = self.cfg.cube_pickup_r;
        let mut taken: Vec<(usize, usize)> = vec![];
        for (ci, c) in self.cubes.iter().enumerate() {
            let mut best: Option<(f64, usize)> = None;
            for m in &self.mechs {
                if !m.alive {
                    continue;
                }
                let d = ((m.pos[0] - c.pos[0]).powi(2) + (m.pos[1] - c.pos[1]).powi(2)).sqrt();
                if d <= r && (m.pos[2] - c.pos[2]).abs() <= 4.0 && best.map_or(true, |b| d < b.0 - 1e-12) {
                    best = Some((d, m.id));
                }
            }
            if let Some((_, id)) = best {
                taken.push((ci, id));
            }
        }
        for &(ci, id) in taken.iter().rev() {
            self.cubes.swap_remove(ci);
            self.add_cubes(id, 1);
            let mut e = Self::ev("pickup");
            e.cubes = Some(self.mechs[id].cubes as f64);
            self.mechs[id].events.push(e);
        }
    }

    fn drops_tick(&mut self) {
        let t = self.t();
        let dt = self.cfg.dt;
        // call new drops
        while self.drops_called < self.cfg.drop_times.len() && t >= self.cfg.drop_times[self.drops_called] - 1e-9 {
            self.drops_called += 1;
            let (_, _, nc, nr, ..) = self.zone_at(t);
            let mut pos = [nc[0], nc[1], 0.0];
            for _ in 0..200 {
                let a = self.rng.range(0.0, std::f64::consts::TAU);
                let d = self.rng.f().sqrt() * nr * self.cfg.drop_spread;
                let (x, y) = (nc[0] + a.cos() * d, nc[1] + a.sin() * d);
                if self.world.in_bounds(x, y, 12.0) && self.world.open_ground(x, y, self.cfg.drop_radius * 0.6) && self.world.slope_at(x, y) < 0.3 {
                    pos = [x, y, 0.0];
                    break;
                }
            }
            pos[2] = self.world.ground(pos[0], pos[1]);
            let id = self.next_drop;
            self.next_drop += 1;
            self.drops.push(DropItem { id, pos, called: t, lands_at: t + self.cfg.drop_fall, holder: None, progress: 0.0 });
        }
        let mut done: Vec<(usize, usize)> = vec![];
        let mut land_sounds = vec![];
        for (di, d) in self.drops.iter_mut().enumerate() {
            if t < d.lands_at {
                continue;
            }
            if t - d.lands_at < dt {
                land_sounds.push(d.pos);
            }
            let inside: Vec<usize> = self
                .mechs
                .iter()
                .filter(|m| m.alive && ((m.pos[0] - d.pos[0]).powi(2) + (m.pos[1] - d.pos[1]).powi(2)).sqrt() <= self.cfg.drop_radius && (m.pos[2] - d.pos[2]).abs() < 12.0)
                .map(|m| m.id)
                .collect();
            match inside.len() {
                0 => {
                    d.holder = None;
                    d.progress = 0.0;
                }
                1 => {
                    if d.holder == Some(inside[0]) {
                        d.progress += dt;
                    } else {
                        d.holder = Some(inside[0]);
                        d.progress = dt;
                    }
                    if d.progress >= self.cfg.drop_capture - 1e-9 {
                        done.push((di, inside[0]));
                    }
                }
                _ => {}
            }
        }
        for p in land_sounds {
            self.emit_sound("drop", p, -1);
        }
        for &(di, mid) in done.iter().rev() {
            let id = self.drops[di].id;
            self.drops.remove(di);
            self.add_cubes(mid, self.cfg.drop_cubes);
            let m = &mut self.mechs[mid];
            m.hp = m.max_hp;
            m.energy = m.max_energy;
            let mut e = Self::ev("drop");
            e.id = Some(json!(id));
            e.cubes = Some(self.mechs[mid].cubes as f64);
            self.mechs[mid].events.push(e);
        }
    }

    fn npcs_tick(&mut self) {
        let dt = self.cfg.dt;
        let cfg = self.cfg.clone();
        let world = self.world.clone();
        let body = Body { r: cfg.npc_radius, h: cfg.npc_height, step_up: 1.2, max_slope: 0.87 };
        for j in 0..self.npcs.len() {
            if !self.npcs[j].alive {
                continue;
            }
            let (npos, home) = (self.npcs[j].pos, self.npcs[j].home);
            let dhome = ((npos[0] - home[0]).powi(2) + (npos[1] - home[1]).powi(2)).sqrt();
            self.npcs[j].cooldown = (self.npcs[j].cooldown - dt).max(0.0);
            let eye = [npos[0], npos[1], npos[2] + cfg.npc_height * 0.8];
            if let Some(tm) = self.npcs[j].target {
                let ok = self.mechs[tm].alive && {
                    let d = dist3(npos, self.mechs[tm].pos);
                    d <= cfg.npc_deaggro && dhome <= cfg.npc_chase_leash
                };
                if !ok {
                    self.npcs[j].target = None;
                    self.npcs[j].provoked = false;
                }
            }
            if self.npcs[j].target.is_none() {
                self.npcs[j].scan -= dt;
                if self.npcs[j].scan <= 0.0 && dhome <= cfg.npc_leash {
                    self.npcs[j].scan = 0.25;
                    let mut best: Option<(f64, usize)> = None;
                    for m in &self.mechs {
                        if !m.alive {
                            continue;
                        }
                        let cen = [m.pos[0], m.pos[1], m.pos[2] + m.ms.height * 0.5];
                        let d = dist3(npos, cen);
                        if d <= cfg.npc_aggro && best.map_or(true, |b| d < b.0) && world.los(eye, cen, false) {
                            best = Some((d, m.id));
                        }
                    }
                    if let Some((_, id)) = best {
                        self.npcs[j].target = Some(id);
                    }
                }
            }
            let mut goal: Option<[f64; 2]> = None;
            let mut speed = cfg.npc_speed;
            if let Some(tm) = self.npcs[j].target {
                let tp = self.mechs[tm].pos;
                let d = ((tp[0] - npos[0]).powi(2) + (tp[1] - npos[1]).powi(2)).sqrt();
                if d > 14.0 {
                    goal = Some([tp[0], tp[1]]);
                }
                // shoot
                let cen = [tp[0], tp[1], tp[2] + self.mechs[tm].ms.height * 0.5];
                if self.npcs[j].cooldown <= 0.0 && world.los(eye, cen, false) {
                    let tv = self.mechs[tm].vel;
                    let (ip, _) = lead(eye, cen, tv, cfg.npc_proj_speed);
                    let yaw = (ip[1] - eye[1]).atan2(ip[0] - eye[0]);
                    let hl = ((ip[0] - eye[0]).powi(2) + (ip[1] - eye[1]).powi(2)).sqrt();
                    let pitch = (ip[2] - eye[2]).atan2(hl.max(1e-6));
                    let (yy, pp) = (yaw + self.rng.gauss() * cfg.npc_spread, pitch + self.rng.gauss() * cfg.npc_spread);
                    let dir = dir_of(yy, pp);
                    let sp = [eye[0] + dir[0] * (cfg.npc_radius + 0.5), eye[1] + dir[1] * (cfg.npc_radius + 0.5), eye[2] + dir[2] * (cfg.npc_radius + 0.5)];
                    let id = self.next_proj;
                    self.next_proj += 1;
                    let oid = self.npcs[j].id as i64;
                    self.projs.push(Proj {
                        id,
                        kind: ProjKind::Npc,
                        owner: oid,
                        pos: sp,
                        vel: [dir[0] * cfg.npc_proj_speed, dir[1] * cfg.npc_proj_speed, dir[2] * cfg.npc_proj_speed],
                        dmg: cfg.npc_damage,
                        age: 0.0,
                        left: cfg.npc_deaggro + 40.0,
                        mp: None,
                        mult: 1.0,
                    });
                    self.npcs[j].cooldown = cfg.npc_cd;
                    self.npcs[j].yaw = yaw;
                    self.npcs[j].pitch = pitch;
                    self.emit_sound("npc_gun", npos, oid);
                }
            } else {
                speed *= 0.5;
                self.npcs[j].wait -= dt;
                let g = self.npcs[j].goal;
                let dg = ((g[0] - npos[0]).powi(2) + (g[1] - npos[1]).powi(2)).sqrt();
                if dhome > cfg.npc_leash {
                    goal = Some(home);
                    speed = cfg.npc_speed;
                } else if dg < 1.5 || self.npcs[j].wait < -8.0 {
                    if self.npcs[j].wait < 0.0 {
                        self.npcs[j].wait = self.rng.range(1.0, 4.0);
                        let a = self.rng.range(0.0, std::f64::consts::TAU);
                        let d = self.rng.f().sqrt() * cfg.npc_leash * 0.9;
                        self.npcs[j].goal = [home[0] + a.cos() * d, home[1] + a.sin() * d];
                    }
                } else if self.npcs[j].wait <= 0.0 {
                    goal = Some(g);
                }
            }
            let mut vel = [0.0; 3];
            if let Some(g) = goal {
                let (dx, dy) = (g[0] - npos[0], g[1] - npos[1]);
                let l = (dx * dx + dy * dy).sqrt().max(1e-9);
                let (vx, vy) = (dx / l * speed, dy / l * speed);
                let (nx, ny, blocked) = world.walk(&body, npos[0], npos[1], npos[2], true, vx * dt, vy * dt);
                if blocked && (nx - npos[0]).abs() + (ny - npos[1]).abs() < 1e-6 {
                    // stuck: pick a new goal
                    let a = self.rng.range(0.0, std::f64::consts::TAU);
                    self.npcs[j].goal = [home[0] + a.cos() * 10.0, home[1] + a.sin() * 10.0];
                }
                let z = world.support(nx, ny, npos[2], 1.2);
                let n = &mut self.npcs[j];
                n.pos = [nx, ny, z];
                vel = [(nx - npos[0]) / dt, (ny - npos[1]) / dt, 0.0];
                if self.npcs[j].target.is_none() {
                    self.npcs[j].yaw = vy.atan2(vx);
                }
            }
            self.npcs[j].vel = vel;
        }
    }
}

/// First-order lead point for a constant-speed projectile.
pub fn lead(from: [f64; 3], tpos: [f64; 3], tvel: [f64; 3], speed: f64) -> ([f64; 3], f64) {
    let r = [tpos[0] - from[0], tpos[1] - from[1], tpos[2] - from[2]];
    let a = tvel[0] * tvel[0] + tvel[1] * tvel[1] + tvel[2] * tvel[2] - speed * speed;
    let b = 2.0 * (r[0] * tvel[0] + r[1] * tvel[1] + r[2] * tvel[2]);
    let c = r[0] * r[0] + r[1] * r[1] + r[2] * r[2];
    let mut t = -1.0;
    if a.abs() < 1e-9 {
        if b.abs() > 1e-9 {
            t = -c / b;
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let s = disc.sqrt();
            let (t1, t2) = ((-b - s) / (2.0 * a), (-b + s) / (2.0 * a));
            let (lo, hi) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t = if lo > 0.0 { lo } else { hi };
        }
    }
    if !(t > 0.0) || t > 3.0 {
        return (tpos, c.sqrt() / speed);
    }
    ([tpos[0] + tvel[0] * t, tpos[1] + tvel[1] * t, tpos[2] + tvel[2] * t], t)
}
