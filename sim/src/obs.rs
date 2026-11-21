//! Observation building: vision (with memory), sound, beams, events.
use crate::cfg::*;
use crate::combat::dist3;
use crate::phys::wrap;
use crate::sim::*;
use mech_bot::protocol::*;

pub struct Viewer {
    pub eye: [f64; 3],
    pub yaw: f64,
    pub near: f64,
    pub range: f64,
    pub half_fov: f64,
}

impl Sim {
    pub fn viewer(&self, i: usize) -> Viewer {
        let m = &self.mechs[i];
        Viewer {
            eye: [m.pos[0], m.pos[1], m.pos[2] + m.ms.eye],
            yaw: m.yaw,
            near: self.cfg.vis_near + m.ms.near_bonus,
            range: self.cfg.vis_range + m.ms.vision_bonus,
            half_fov: self.cfg.vis_fov * 0.5,
        }
    }

    pub fn sees_point(&self, v: &Viewer, p: [f64; 3]) -> bool {
        let d = dist3(v.eye, p);
        if d > v.range {
            return false;
        }
        // pull the target point slightly toward the eye so surfaces it rests on do not occlude it
        let k = if d > 0.5 { 0.25 / d } else { 0.0 };
        let q = [p[0] + (v.eye[0] - p[0]) * k, p[1] + (v.eye[1] - p[1]) * k, p[2] + (v.eye[2] - p[2]) * k];
        if d <= v.near {
            return self.world.los(v.eye, q, false);
        }
        let ang = (p[1] - v.eye[1]).atan2(p[0] - v.eye[0]);
        if wrap(ang - v.yaw).abs() > v.half_fov {
            return false;
        }
        self.world.los(v.eye, q, true)
    }

    pub fn sees_body(&self, v: &Viewer, pos: [f64; 3], h: f64) -> bool {
        self.sees_point(v, [pos[0], pos[1], pos[2] + h * 0.5]) || self.sees_point(v, [pos[0], pos[1], pos[2] + h])
    }

    fn weapon_obs(&self, i: usize, w: &Weapon) -> WeaponObs {
        let m = &self.mechs[i];
        let st = m.wstats[w.kind].clone();
        let ct = st.get("charge").copied().unwrap_or(0.75);
        WeaponObs {
            kind: KIND_NAMES[w.kind].to_string(),
            cooldown: w.cooldown.max(0.0),
            ammo: if w.kind == GUN { Some(w.ammo) } else { None },
            reload: if w.kind == GUN { Some(w.reload.max(0.0)) } else { None },
            charge: if w.kind == LASER { Some(if w.charging { w.charge.min(ct) } else { 0.0 }) } else { None },
            stats: st,
        }
    }

    fn actor_of_mech(&self, j: usize) -> Actor {
        let m = &self.mechs[j];
        Actor {
            id: j as i64,
            kind: "mech".into(),
            pos: m.pos,
            vel: m.vel,
            aim: [m.yaw, m.pitch],
            hp: Some(m.hp),
            max_hp: Some(m.max_hp),
            cubes: Some(m.cubes as f64),
            weapons: m.slots.iter().flatten().map(|w| KIND_NAMES[w.kind].to_string()).collect(),
            charging: m.slots.iter().flatten().any(|w| w.kind == LASER && w.charging),
        }
    }

    pub fn build_obs(&mut self, i: usize) -> Obs {
        let t = self.t();
        let v = self.viewer(i);
        let nm = self.mechs.len();
        let cfg = self.cfg.clone();
        let mut o = Obs { tick: self.tick, t, alive: self.alive as u32, ..Default::default() };
        // self
        {
            let m = &self.mechs[i];
            let mut stats = m.stats.clone();
            stats.insert("radius".into(), m.ms.radius);
            stats.insert("height".into(), m.ms.height);
            stats.insert("eye_height".into(), m.ms.eye);
            stats.insert("muzzle_height".into(), m.ms.muzzle);
            o.me = SelfObs {
                id: i as i64,
                pos: m.pos,
                vel: m.vel,
                aim: [m.yaw, m.pitch],
                hp: m.hp,
                max_hp: m.max_hp,
                energy: m.energy,
                max_energy: m.max_energy,
                grounded: m.grounded,
                cubes: m.cubes as f64,
                upgrades: m.upg_list.clone(),
                in_zone: m.in_zone,
                weapons: vec![],
                stats,
            };
        }
        let wo: Vec<Option<WeaponObs>> = (0..2).map(|s| self.mechs[i].slots[s].as_ref().map(|w| self.weapon_obs(i, w))).collect();
        o.me.weapons = wo;
        // mechs
        for j in 0..nm {
            if j == i || !self.mechs[j].alive {
                continue;
            }
            let m = &self.mechs[j];
            if self.sees_body(&v, m.pos, m.ms.height) {
                self.last_seen[i][j] = t;
            }
            if t - self.last_seen[i][j] <= cfg.memory_s + 1e-9 {
                o.mechs.push(self.actor_of_mech(j));
            }
        }
        for j in 0..self.npcs.len() {
            let n = &self.npcs[j];
            if !n.alive {
                continue;
            }
            if self.sees_body(&v, n.pos, cfg.npc_height) {
                self.last_seen[i][nm + j] = t;
            }
            if t - self.last_seen[i][nm + j] <= cfg.memory_s + 1e-9 {
                let n = &self.npcs[j];
                o.mechs.push(Actor {
                    id: n.id as i64,
                    kind: "npc".into(),
                    pos: n.pos,
                    vel: n.vel,
                    aim: [n.yaw, n.pitch],
                    hp: Some(n.hp),
                    max_hp: Some(cfg.npc_hp),
                    cubes: None,
                    weapons: vec![],
                    charging: false,
                });
            }
        }
        // projectiles
        for p in &self.projs {
            if p.owner == i as i64 {
                continue;
            }
            if self.sees_point(&v, p.pos) {
                o.projectiles.push(Projectile {
                    id: p.id as i64,
                    kind: match p.kind {
                        ProjKind::Gun => "gun",
                        ProjKind::Mortar => "mortar",
                        ProjKind::Npc => "npc",
                    }
                    .into(),
                    pos: p.pos,
                    vel: p.vel,
                });
            }
        }
        for c in &self.crates {
            if c.alive && self.sees_body(&v, c.pos, cfg.crate_h) {
                o.crates.push(Item { id: c.id as i64, pos: c.pos, hp: Some(c.hp) });
            }
        }
        for c in &self.cubes {
            if self.sees_point(&v, [c.pos[0], c.pos[1], c.pos[2] + 0.5]) {
                o.cubes.push(Item { id: c.id as i64, pos: c.pos, hp: None });
            }
        }
        for p in &self.pickups {
            if self.sees_point(&v, [p.pos[0], p.pos[1], p.pos[2] + 0.5]) {
                o.pickups.push(Pickup { id: p.id as i64, kind: KIND_NAMES[p.w.kind].to_string(), pos: p.pos });
            }
        }
        // sounds
        let me_pos = self.mechs[i].pos;
        let hearing = self.mechs[i].ms.hearing;
        let mut snds = vec![];
        for s in &self.sounds {
            if s.src == i as i64 {
                continue;
            }
            let range = cfg.snd_range.get(&s.kind).copied().unwrap_or(100.0);
            let d = dist3(me_pos, s.pos);
            if d > range {
                continue;
            }
            snds.push((s.kind.clone(), s.pos, d, s.tick));
        }
        for (kind, pos, d, tick) in snds {
            let bearing = (pos[1] - me_pos[1]).atan2(pos[0] - me_pos[0]) + self.rng.gauss() * cfg.bearing_noise * hearing;
            o.sounds.push(Sound {
                kind,
                bearing: wrap(bearing),
                dist: if d < cfg.snd_near { "near" } else if d < cfg.snd_mid { "mid" } else { "far" }.to_string(),
                age: (self.tick - tick) as f64 * cfg.dt,
            });
        }
        // beams
        for b in &self.beams {
            if b.by == i {
                continue;
            }
            let mut pts: Vec<[f64; 3]> = vec![];
            for (k, p) in b.path.iter().enumerate() {
                pts.push(*p);
                if k + 1 < b.path.len() {
                    let q = b.path[k + 1];
                    pts.push([(p[0] + q[0]) * 0.5, (p[1] + q[1]) * 0.5, (p[2] + q[2]) * 0.5]);
                }
            }
            if pts.iter().any(|p| self.sees_point(&v, *p)) {
                o.beams.push(Beam { by: b.by as i64, kind: b.kind.to_string(), path: b.path.clone(), age: (self.tick - b.tick) as f64 * cfg.dt });
            }
        }
        o.events = std::mem::take(&mut self.mechs[i].events);
        // zone
        let (c, r, nc, nr, s, e, dps) = self.zone_at(t);
        o.zone = Zone { center: c, radius: r, next_center: nc, next_radius: nr, shrink_start: s, shrink_end: e, dps };
        o.offer = self.mechs[i].offer.as_ref().map(|of| Offer {
            choices: of.choices.iter().map(|&k| OfferChoice { id: cfg.upgrades[k].id.clone() }).collect(),
        });
        for d in &self.drops {
            o.drops.push(SupplyDrop {
                id: d.id as i64,
                pos: d.pos,
                radius: cfg.drop_radius,
                lands_in: (d.lands_at - t).max(0.0),
                capture_s: cfg.drop_capture,
                progress: d.progress,
                holder: d.holder.map(|h| h as i64),
            });
        }
        o
    }

    /// Build observations for all alive mechs, then clear transient lists.
    pub fn build_all_obs(&mut self) -> Vec<Option<Obs>> {
        let n = self.mechs.len();
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            if self.mechs[i].alive {
                out.push(Some(self.build_obs(i)));
            } else {
                out.push(None);
            }
        }
        self.sounds.clear();
        self.beams.clear();
        out
    }
}
