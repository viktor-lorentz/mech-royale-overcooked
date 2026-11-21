//! Actions, weapons, projectiles, damage, deaths, cubes and upgrades.
use crate::cfg::*;
use crate::map::seg_vcyl;
use crate::phys::wrap;
use crate::sim::*;
use mech_bot::protocol::{Action, Event};
use serde_json::json;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Tgt {
    Mech(usize),
    Npc(usize),
    Crate(usize),
}

pub fn dir_of(yaw: f64, pitch: f64) -> [f64; 3] {
    [pitch.cos() * yaw.cos(), pitch.cos() * yaw.sin(), pitch.sin()]
}

fn fin(x: f64) -> f64 {
    if x.is_finite() {
        x
    } else {
        0.0
    }
}

impl Sim {
    pub fn dmg_mult(&self, i: usize) -> f64 {
        1.0 + self.cfg.cube_dmg * (self.mechs[i].cubes as f64).min(self.cfg.cube_cap)
    }

    pub fn ev(t: &str) -> Event {
        Event { t: t.to_string(), ..Default::default() }
    }

    // ------------------------------------------------------------ actions
    pub fn apply_action(&mut self, i: usize, a: &Action) {
        if !self.mechs[i].alive {
            return;
        }
        {
            let m = &mut self.mechs[i];
            let mv = a.mv.unwrap_or([0.0, 0.0]);
            m.cmd.mv = [fin(mv[0]), fin(mv[1])];
            m.cmd.boost = a.boost.unwrap_or(false);
            if let Some(ai) = a.aim {
                if ai[0].is_finite() && ai[1].is_finite() {
                    m.cmd.aim = Some(ai);
                }
            }
            m.cmd.fire = a.fire.unwrap_or([false, false]);
            if a.jump.unwrap_or(false) {
                m.jump_pending = true;
            }
        }
        if let Some(p) = a.pick {
            self.apply_pick(i, p);
        }
        if a.reload.unwrap_or(false) {
            let m = &mut self.mechs[i];
            let rl = m.wstats[GUN].get("reload").copied().unwrap_or(2.4);
            let mag = m.wstats[GUN].get("mag").copied().unwrap_or(12.0);
            let mut snd = None;
            for w in m.slots.iter_mut().flatten() {
                if w.kind == GUN && w.reload <= 0.0 && w.ammo < mag {
                    w.reload = rl;
                    snd = Some(m.pos);
                }
            }
            if let Some(p) = snd {
                self.emit_sound("reload", p, i as i64);
            }
        }
    }

    /// Weapon pickup/drop requests are resolved together so that ties go to the closer mech.
    pub fn resolve_takes(&mut self, reqs: &[(usize, Option<i64>, Option<usize>)]) {
        // drops alone first
        for &(i, take, drop) in reqs {
            if take.is_none() {
                if let Some(s) = drop {
                    if s < 2 && self.mechs[i].alive {
                        self.drop_slot(i, s);
                    }
                }
            }
        }
        // group takes by pickup id
        let mut by_pickup: Vec<(i64, Vec<(usize, Option<usize>)>)> = vec![];
        for &(i, take, drop) in reqs {
            if let Some(id) = take {
                if !self.mechs[i].alive {
                    continue;
                }
                match by_pickup.iter_mut().find(|(p, _)| *p == id) {
                    Some(e) => e.1.push((i, drop)),
                    None => by_pickup.push((id, vec![(i, drop)])),
                }
            }
        }
        for (pid, mut takers) in by_pickup {
            let pidx = self.pickups.iter().position(|p| p.id as i64 == pid);
            let Some(pidx) = pidx else {
                for (i, _) in takers {
                    self.take_failed(i, pid, "gone");
                }
                continue;
            };
            let ppos = self.pickups[pidx].pos;
            let dist = |s: &Sim, i: usize| {
                let m = &s.mechs[i];
                ((m.pos[0] - ppos[0]).powi(2) + (m.pos[1] - ppos[1]).powi(2)).sqrt()
            };
            takers.sort_by(|a, b| dist(self, a.0).partial_cmp(&dist(self, b.0)).unwrap().then(a.0.cmp(&b.0)));
            let mut taken = false;
            for (i, drop) in takers {
                if taken {
                    self.take_failed(i, pid, "gone");
                    continue;
                }
                let m = &self.mechs[i];
                let dh = dist(self, i);
                let dz = (m.pos[2] - ppos[2]).abs();
                if dh > self.cfg.pickup_r || dz > 4.0 {
                    self.take_failed(i, pid, "range");
                    continue;
                }
                let kind = self.pickups[pidx].w.kind;
                if m.slots.iter().flatten().any(|w| w.kind == kind) {
                    self.take_failed(i, pid, "have");
                    continue;
                }
                let slot = match drop {
                    Some(s) if s < 2 => s,
                    _ => match m.slots.iter().position(|s| s.is_none()) {
                        Some(s) => s,
                        None => {
                            self.take_failed(i, pid, "full");
                            continue;
                        }
                    },
                };
                if let Some(w) = &m.slots[slot] {
                    if w.kind == LASER && w.charging {
                        self.take_failed(i, pid, "busy");
                        continue;
                    }
                }
                // success
                let mut w = self.pickups[pidx].w.clone();
                let cdstat = self.mechs[i].wstats[kind].get("cooldown").copied().unwrap_or(0.0);
                w.cooldown = w.cooldown.max(self.cfg.equip_s.max(cdstat));
                if self.mechs[i].slots[slot].is_some() {
                    self.drop_slot_inner(i, slot, Some(pid));
                }
                self.mechs[i].slots[slot] = Some(w);
                let mut e = Self::ev("took");
                e.id = Some(json!(pid));
                e.weapon = Some(KIND_NAMES[kind].to_string());
                self.mechs[i].events.push(e);
                taken = true;
            }
            if taken {
                self.pickups.retain(|p| p.id as i64 != pid);
            }
        }
    }

    fn take_failed(&mut self, i: usize, id: i64, reason: &str) {
        let mut e = Self::ev("take_failed");
        e.id = Some(json!(id));
        e.reason = Some(reason.to_string());
        self.mechs[i].events.push(e);
    }

    fn drop_slot(&mut self, i: usize, s: usize) {
        if let Some(w) = &self.mechs[i].slots[s] {
            if w.kind == LASER && w.charging {
                return;
            }
        } else {
            return;
        }
        self.drop_slot_inner(i, s, None);
    }

    fn drop_slot_inner(&mut self, i: usize, s: usize, _replacing: Option<i64>) {
        if let Some(w) = self.mechs[i].slots[s].take() {
            let pos = self.mechs[i].pos;
            let kind = w.kind;
            let id = self.add_pickup(w, pos);
            let mut e = Self::ev("dropped");
            e.id = Some(json!(id));
            e.weapon = Some(KIND_NAMES[kind].to_string());
            self.mechs[i].events.push(e);
        }
    }

    // ------------------------------------------------------------ cubes & upgrades
    pub fn recalc_vitals(&mut self, i: usize) {
        let cfg = self.cfg.clone();
        let m = &mut self.mechs[i];
        let base = m.stats.get("hp").copied().unwrap_or(1400.0);
        let new_max = base * (1.0 + cfg.cube_hp * (m.cubes as f64).min(cfg.cube_cap));
        if new_max > m.max_hp {
            m.hp += new_max - m.max_hp;
        }
        m.max_hp = new_max;
        m.hp = m.hp.min(m.max_hp);
        let en = m.stats.get("energy").copied().unwrap_or(100.0);
        if en > m.max_energy {
            m.energy += en - m.max_energy;
        }
        m.max_energy = en;
        m.energy = m.energy.min(m.max_energy);
        m.ms = Ms::from_map(&m.stats, m.cubes, cfg.cube_size);
    }

    pub fn add_cubes(&mut self, i: usize, n: u32) {
        let old = self.mechs[i].cubes;
        self.mechs[i].cubes += n;
        let new = self.mechs[i].cubes;
        let mut q = 0;
        for th in &self.cfg.thresholds {
            if (old as f64) < *th && (new as f64) >= *th {
                q += 1;
            }
        }
        self.mechs[i].offer_queue += q;
        self.recalc_vitals(i);
    }

    pub fn apply_pick(&mut self, i: usize, p: usize) {
        let Some(o) = &self.mechs[i].offer else { return };
        if p >= o.choices.len() {
            return;
        }
        let def = o.choices[p];
        self.mechs[i].offer = None;
        self.apply_upgrade(i, def);
    }

    pub fn apply_upgrade(&mut self, i: usize, def: usize) {
        let cfg = self.cfg.clone();
        let u = &cfg.upgrades[def];
        {
            let m = &mut self.mechs[i];
            m.upg[def] += 1;
            m.upg_list.push(u.id.clone());
            for e in &u.effects {
                let parts: Vec<&str> = e.path.split('.').collect();
                let map = if parts[0] == "mech" && parts.len() == 2 {
                    Some((&mut m.stats, parts[1]))
                } else if parts[0] == "weapons" && parts.len() == 3 {
                    kind_idx(parts[1]).map(|k| (&mut m.wstats[k], parts[2]))
                } else {
                    None
                };
                if let Some((map, key)) = map {
                    let v = map.entry(key.to_string()).or_insert(0.0);
                    if let Some(x) = e.mul {
                        *v *= x;
                    }
                    if let Some(x) = e.add {
                        *v += x;
                    }
                }
            }
        }
        self.recalc_vitals(i);
        let mut e = Self::ev("upgrade");
        e.id = Some(json!(u.id));
        self.mechs[i].events.push(e);
    }

    pub fn offers_tick(&mut self, i: usize) {
        let t = self.t();
        if !self.mechs[i].alive {
            return;
        }
        if let Some(o) = &self.mechs[i].offer {
            if t >= o.expires {
                let def = o.choices[0];
                self.mechs[i].offer = None;
                self.apply_upgrade(i, def);
            }
        }
        if self.mechs[i].offer.is_none() && self.mechs[i].offer_queue > 0 {
            self.mechs[i].offer_queue -= 1;
            let m = &self.mechs[i];
            let mut elig: Vec<usize> = vec![];
            for (k, u) in self.cfg.upgrades.iter().enumerate() {
                if m.upg[k] >= u.max {
                    continue;
                }
                if let Some(r) = &u.requires {
                    let ok = kind_idx(r).map_or(true, |kk| m.slots.iter().flatten().any(|w| w.kind == kk));
                    if !ok {
                        continue;
                    }
                }
                elig.push(k);
            }
            let mut ch = vec![];
            for _ in 0..3 {
                if elig.is_empty() {
                    break;
                }
                let j = self.rng.below(elig.len());
                ch.push(elig.swap_remove(j));
            }
            if !ch.is_empty() {
                self.next_offer += 1;
                self.mechs[i].offer = Some(OfferState { choices: ch, expires: t + self.cfg.offer_timeout });
            }
        }
    }

    // ------------------------------------------------------------ damage
    fn is_rear(&self, t: usize, src: [f64; 3]) -> bool {
        let m = &self.mechs[t];
        let ang = (src[1] - m.pos[1]).atan2(src[0] - m.pos[0]);
        wrap(ang - m.yaw).abs() > m.ms.front_arc * 0.5
    }

    /// Returns (damage applied, rear).
    pub fn dmg_mech(&mut self, t: usize, amount: f64, src: [f64; 3], by: Option<usize>, weapon: &str) -> (f64, bool) {
        if !self.mechs[t].alive || amount <= 0.0 {
            return (0.0, false);
        }
        let now = self.t();
        let rear = self.is_rear(t, src);
        let ms = self.mechs[t].ms.clone();
        let mut d = amount;
        if rear {
            d *= ms.rear_mult;
        }
        d *= (1.0 - ms.armor).max(0.05);
        if by == Some(t) {
            d *= ms.self_dmg;
        }
        let m = &mut self.mechs[t];
        let d = d.min(m.hp.max(0.0) + 0.0).max(0.0).max(0.0);
        let d = if d.is_nan() { 0.0 } else { d };
        let full = d;
        m.hp -= full;
        m.last_dmg_t = now;
        self.dmg_log.push((t, by, weapon.to_string(), full, rear, now));
        let m = &mut self.mechs[t];
        let bearing = (src[1] - m.pos[1]).atan2(src[0] - m.pos[0]);
        let mut e = Self::ev("damaged");
        e.amount = Some(full);
        e.weapon = Some(weapon.to_string());
        e.bearing = Some(bearing);
        e.by = by.map(|b| b as i64);
        if rear {
            e.rear = Some(true);
        }
        m.events.push(e);
        if let Some(b) = by {
            if b != t {
                m.last_attacker = Some(b);
                self.mechs[b].last_dmg_t = now;
            }
        }
        (full, rear)
    }

    pub fn zone_damage(&mut self, t: usize, amount: f64) {
        let now = self.t();
        self.dmg_log.push((t, None, "zone".into(), amount, false, now));
        let m = &mut self.mechs[t];
        m.hp -= amount;
        m.last_dmg_t = now;
        let mut e = Self::ev("damaged");
        e.amount = Some(amount);
        e.weapon = Some("zone".into());
        e.bearing = None;
        e.by = None;
        m.events.push(e);
    }

    fn hit_ev(&mut self, by: usize, target: i64, amount: f64, weapon: &str, effect: Option<&str>, rear: bool) {
        let mut e = Self::ev("hit");
        e.target = Some(target);
        e.amount = Some(amount);
        e.weapon = Some(weapon.to_string());
        e.effect = effect.map(|s| s.to_string());
        if rear {
            e.rear = Some(true);
        }
        self.mechs[by].events.push(e);
    }

    fn effect_ev(&mut self, victim: usize, effect: &str, by: usize) {
        let mut e = Self::ev("effect");
        e.effect = Some(effect.to_string());
        e.by = Some(by as i64);
        self.mechs[victim].events.push(e);
    }

    pub fn dmg_npc(&mut self, n: usize, amount: f64, by: Option<usize>) -> f64 {
        if !self.npcs[n].alive {
            return 0.0;
        }
        let d = amount.min(self.npcs[n].hp);
        self.npcs[n].hp -= d;
        if let Some(b) = by {
            self.npcs[n].target = Some(b);
            self.npcs[n].provoked = true;
        }
        if self.npcs[n].hp <= 0.0 {
            self.npcs[n].alive = false;
            let p = self.npcs[n].pos;
            for _ in 0..self.cfg.npc_cubes {
                self.spawn_cube(p, 2.0);
            }
        }
        d
    }

    pub fn dmg_crate(&mut self, c: usize, amount: f64) -> f64 {
        if !self.crates[c].alive {
            return 0.0;
        }
        let d = amount.min(self.crates[c].hp);
        self.crates[c].hp -= d;
        if self.crates[c].hp <= 0.0 {
            self.crates[c].alive = false;
            let p = self.crates[c].pos;
            for _ in 0..self.cfg.crate_cubes {
                self.spawn_cube(p, 1.0);
            }
            if self.rng.chance(self.cfg.crate_weapon_chance) {
                let mut r = self.rng.clone();
                let kind = self.random_weapon_kind(&mut r);
                self.rng = r;
                let w = Weapon::new(kind, &self.cfg);
                self.add_pickup(w, p);
            }
            self.emit_sound("crate", p, -1);
        }
        d
    }

    pub fn spawn_cube(&mut self, pos: [f64; 3], scatter: f64) {
        let a = self.rng.range(0.0, std::f64::consts::TAU);
        let d = self.rng.f().sqrt() * scatter;
        let (x, y) = (pos[0] + a.cos() * d, pos[1] + a.sin() * d);
        let z = self.world.support(x, y, pos[2], 1.2).max(pos[2].min(self.world.support(x, y, pos[2], 1.2)));
        let z = if (z - pos[2]).abs() > 3.0 { pos[2] } else { z };
        let id = self.next_cube;
        self.next_cube += 1;
        self.cubes.push(CubeItem { id, pos: [x, y, z] });
    }

    // ------------------------------------------------------------ deaths
    pub fn process_deaths(&mut self) {
        let mut died: Vec<usize> = vec![];
        for m in &self.mechs {
            if m.alive && m.hp <= 0.0 {
                died.push(m.id);
            }
        }
        if died.is_empty() {
            return;
        }
        let alive_after = self.alive - died.len();
        let place = (alive_after + 1) as u32;
        let t = self.t();
        for &i in &died {
            let killer = self.mechs[i].last_attacker.filter(|k| self.mechs[*k].alive || died.contains(k));
            // credit the kill only when the last damage was recent and by a mech
            let killer = killer.filter(|_| t - self.mechs[i].last_dmg_t < 1.0);
            {
                let m = &mut self.mechs[i];
                m.alive = false;
                m.hp = 0.0;
                m.place = place;
                m.death_t = t;
            }
            let pos = self.mechs[i].pos;
            for s in 0..2 {
                if let Some(w) = self.mechs[i].slots[s].take() {
                    if w.kind != GUN {
                        let mut ww = w;
                        ww.charging = false;
                        ww.charge = 0.0;
                        let a = self.rng.range(0.0, std::f64::consts::TAU);
                        let p = [pos[0] + a.cos() * 1.5, pos[1] + a.sin() * 1.5, pos[2]];
                        self.add_pickup(ww, p);
                    }
                }
            }
            let ncubes = self.mechs[i].cubes + self.cfg.cube_extra;
            for _ in 0..ncubes {
                self.spawn_cube(pos, 4.0);
            }
            self.emit_sound("death", pos, i as i64);
            if let Some(k) = killer {
                if k != i {
                    self.mechs[k].kills += 1;
                    let mut e = Self::ev("kill");
                    e.target = Some(i as i64);
                    self.mechs[k].events.push(e);
                }
            }
            self.deaths_log.push((i as u32, t, place as usize, killer));
        }
        self.alive = alive_after;
        if alive_after <= 1 {
            self.over = true;
            for m in self.mechs.iter_mut() {
                if m.alive {
                    m.place = 1;
                }
            }
        }
    }

    // ------------------------------------------------------------ weapons
    fn muzzle(&self, i: usize) -> [f64; 3] {
        let m = &self.mechs[i];
        [m.pos[0], m.pos[1], m.pos[2] + m.ms.muzzle]
    }

    /// Returns (spawn point, hit-wall point if barrel pokes in a wall)
    fn shot_origin(&self, i: usize, center: [f64; 3], dir: [f64; 3]) -> ([f64; 3], Option<[f64; 3]>) {
        let off = self.mechs[i].ms.radius + 0.5;
        let sp = [center[0] + dir[0] * off, center[1] + dir[1] * off, center[2] + dir[2] * off];
        if let Some(t) = self.world.raycast(center, sp) {
            let hp = [center[0] + dir[0] * off * t, center[1] + dir[1] * off * t, center[2] + dir[2] * off * t];
            return (sp, Some(hp));
        }
        (sp, None)
    }

    pub fn weapons_tick(&mut self, i: usize) {
        if !self.mechs[i].alive {
            return;
        }
        for s in 0..2 {
            if let Some(mut w) = self.mechs[i].slots[s].take() {
                self.fire_slot(i, s, &mut w);
                self.mechs[i].slots[s] = Some(w);
            }
        }
    }

    fn fire_slot(&mut self, i: usize, s: usize, w: &mut Weapon) {
        let dt = self.cfg.dt;
        let now = self.t();
        let fire = self.mechs[i].cmd.fire[s];
        let st = self.mechs[i].wstats[w.kind].clone();
        let g = |k: &str| st.get(k).copied().unwrap_or(0.0);
        let mult = self.dmg_mult(i);
        // timers run down first, then the trigger is evaluated (matches the recorded cooldown readings)
        if w.cooldown > 0.0 {
            w.cooldown -= dt; // may dip just below 0: the remainder carries into the next shot (exact fire rate)
        } else {
            w.cooldown = 0.0;
        }
        if w.reload > 0.0 {
            w.reload -= dt;
            if w.reload <= 1e-9 {
                w.reload = 0.0;
                w.ammo = st.get("mag").copied().unwrap_or(12.0);
            }
        }
        match w.kind {
            GUN => {
                if fire && w.cooldown <= 1e-9 && w.reload <= 0.0 && w.ammo >= 1.0 {
                    let (yaw, pitch) = (self.mechs[i].yaw, self.mechs[i].pitch);
                    let sp = g("spread_deg").to_radians();
                    let (yy, pp) = (yaw + self.rng.gauss() * sp, pitch + self.rng.gauss() * sp);
                    let dir = dir_of(yy, pp);
                    let c = self.muzzle(i);
                    let (spawn, wall) = self.shot_origin(i, c, dir);
                    if wall.is_none() {
                        let id = self.next_proj;
                        self.next_proj += 1;
                        let speed = g("speed");
                        self.projs.push(Proj {
                            id,
                            kind: ProjKind::Gun,
                            owner: i as i64,
                            pos: spawn,
                            vel: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
                            dmg: g("damage") * mult,
                            age: 0.0,
                            left: g("range"),
                            mp: None,
                            mult,
                        });
                    }
                    w.ammo -= 1.0;
                    w.cooldown += g("cooldown");
                    let p = self.mechs[i].pos;
                    self.emit_sound("gun", p, i as i64);
                    if w.ammo < 1.0 {
                        w.reload = g("reload");
                        self.emit_sound("reload", p, i as i64);
                    }
                }
            }
            MORTAR => {
                if fire && w.cooldown <= 1e-9 {
                    let (yaw, pitch) = (self.mechs[i].yaw, self.mechs[i].pitch);
                    let dir = dir_of(yaw, pitch);
                    let m = &self.mechs[i];
                    let c = [m.pos[0], m.pos[1], m.pos[2] + m.ms.height + 0.5 * m.ms.size];
                    let (spawn, wall) = self.shot_origin(i, c, dir);
                    let mp = MortarParams {
                        damage: g("damage"),
                        splash: g("splash"),
                        radius: g("splash_radius"),
                        edge: g("splash_edge"),
                        knock: g("knock"),
                        knock_up: g("knock_up"),
                        drain: g("drain"),
                    };
                    let speed = g("speed");
                    let p = self.mechs[i].pos;
                    self.emit_sound("mortar", p, i as i64);
                    if let Some(hp) = wall {
                        self.explode(hp, i as i64, &mp, mult, None);
                    } else {
                        let id = self.next_proj;
                        self.next_proj += 1;
                        self.projs.push(Proj {
                            id,
                            kind: ProjKind::Mortar,
                            owner: i as i64,
                            pos: spawn,
                            vel: [dir[0] * speed, dir[1] * speed, dir[2] * speed],
                            dmg: 0.0,
                            age: 0.0,
                            left: g("ttl").max(1.0),
                            mp: Some(mp),
                            mult,
                        });
                    }
                    w.cooldown += g("cooldown");
                }
            }
            LASER => {
                let ct = g("charge");
                let minc = g("min_charge");
                let hold = g("hold_s");
                if w.charging && self.mechs[i].short_until > now {
                    w.charging = false;
                    w.charge = 0.0;
                }
                if !w.charging && fire && w.cooldown <= 1e-9 && self.mechs[i].short_until <= now {
                    w.charging = true;
                    w.charge = 0.0;
                    w.overhold = 0.0;
                    let p = self.mechs[i].pos;
                    self.emit_sound("laser_charge", p, i as i64);
                }
                if w.charging {
                    w.charge += dt;
                    let release = (!fire && w.charge >= minc) || w.charge >= ct + hold;
                    if release {
                        let frac_c = w.charge.min(ct);
                        self.fire_laser(i, &st, frac_c / ct.max(1e-9), mult);
                        w.charging = false;
                        w.charge = 0.0;
                        w.cooldown = g("cooldown");
                    }
                }
            }
            LIGHTNING => {
                let cost = g("energy");
                if fire && w.cooldown <= 1e-9 && self.mechs[i].energy >= cost {
                    self.spend_energy(i, cost);
                    self.fire_lightning(i, &st, mult);
                    w.cooldown += g("cooldown");
                }
            }
            _ => {}
        }
    }

    fn fire_laser(&mut self, i: usize, st: &std::collections::HashMap<String, f64>, frac_c: f64, mult: f64) {
        let g = |k: &str| st.get(k).copied().unwrap_or(0.0);
        let (yaw, pitch) = (self.mechs[i].yaw, self.mechs[i].pitch);
        let dir = dir_of(yaw, pitch);
        let c = self.muzzle(i);
        let (spawn, wall) = self.shot_origin(i, c, dir);
        let range = g("range");
        let end_full = [c[0] + dir[0] * (range + 3.0), c[1] + dir[1] * (range + 3.0), c[2] + dir[2] * (range + 3.0)];
        let mut path = vec![spawn];
        let p = self.mechs[i].pos;
        self.emit_sound("laser", p, i as i64);
        if let Some(hp) = wall {
            path.push(hp);
            self.beams.push(BeamEv { tick: self.tick, by: i, kind: "laser", path });
            return;
        }
        let mut tmax = self.world.raycast(spawn, end_full).unwrap_or(1.0);
        let seglen = ((end_full[0] - spawn[0]).powi(2) + (end_full[1] - spawn[1]).powi(2) + (end_full[2] - spawn[2]).powi(2)).sqrt();
        // candidates
        let mut cands: Vec<(f64, Tgt)> = vec![];
        for (j, m) in self.mechs.iter().enumerate() {
            if j == i || !m.alive {
                continue;
            }
            if let Some(t) = seg_vcyl(spawn, end_full, m.pos[0], m.pos[1], m.ms.radius, m.pos[2], m.pos[2] + m.ms.height) {
                cands.push((t, Tgt::Mech(j)));
            }
        }
        for (j, n) in self.npcs.iter().enumerate() {
            if !n.alive {
                continue;
            }
            if let Some(t) = seg_vcyl(spawn, end_full, n.pos[0], n.pos[1], self.cfg.npc_radius, n.pos[2], n.pos[2] + self.cfg.npc_height) {
                cands.push((t, Tgt::Npc(j)));
            }
        }
        for (j, cr) in self.crates.iter().enumerate() {
            if !cr.alive {
                continue;
            }
            if let Some(t) = seg_vcyl(spawn, end_full, cr.pos[0], cr.pos[1], self.cfg.crate_r, cr.pos[2], cr.pos[2] + self.cfg.crate_h) {
                cands.push((t, Tgt::Crate(j)));
            }
        }
        cands.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        let mut pierce_left = g("pierce") as i64;
        let dmg_base = g("damage") * (g("min_frac") + (1.0 - g("min_frac")) * frac_c.clamp(0.0, 1.0)) * mult;
        for (t, tg) in cands {
            if t > tmax {
                break;
            }
            let hp = [spawn[0] + (end_full[0] - spawn[0]) * t, spawn[1] + (end_full[1] - spawn[1]) * t, spawn[2] + (end_full[2] - spawn[2]) * t];
            match tg {
                Tgt::Mech(j) => {
                    let mut d = dmg_base;
                    let mut eff = None;
                    if self.mechs[j].slots.iter().flatten().any(|w| w.kind == MORTAR) {
                        d *= 1.0 + g("cookoff");
                        eff = Some("cookoff");
                    }
                    let (dd, rear) = self.dmg_mech(j, d, c, Some(i), "laser");
                    self.hit_ev(i, j as i64, dd, "laser", eff, rear);
                    if eff.is_some() {
                        self.effect_ev(j, "cookoff", i);
                    }
                }
                Tgt::Npc(j) => {
                    let id = self.npcs[j].id as i64;
                    let dd = self.dmg_npc(j, dmg_base, Some(i));
                    self.hit_ev(i, id, dd, "laser", None, false);
                }
                Tgt::Crate(j) => {
                    let id = self.crates[j].id as i64;
                    let dd = self.dmg_crate(j, dmg_base);
                    self.hit_ev(i, id, dd, "laser", None, false);
                }
            }
            if pierce_left <= 0 {
                tmax = t;
                let _ = hp;
                break;
            }
            pierce_left -= 1;
        }
        let end = [spawn[0] + (end_full[0] - spawn[0]) * tmax.min(range / seglen.max(1e-9)), spawn[1] + (end_full[1] - spawn[1]) * tmax.min(range / seglen.max(1e-9)), spawn[2] + (end_full[2] - spawn[2]) * tmax.min(range / seglen.max(1e-9))];
        path.push(end);
        self.beams.push(BeamEv { tick: self.tick, by: i, kind: "laser", path });
    }

    fn fire_lightning(&mut self, i: usize, st: &std::collections::HashMap<String, f64>, mult: f64) {
        let g = |k: &str| st.get(k).copied().unwrap_or(0.0);
        let (yaw, pitch) = (self.mechs[i].yaw, self.mechs[i].pitch);
        let c = self.muzzle(i);
        let range = g("range");
        let cone = g("cone_deg").to_radians();
        let p = self.mechs[i].pos;
        self.emit_sound("lightning", p, i as i64);
        // find nearest
        let mut best: Option<(f64, Tgt, [f64; 3])> = None;
        let mut consider = |d: f64, tg: Tgt, cen: [f64; 3], s: &Sim| {
            if d > range {
                return;
            }
            let ang = (cen[1] - c[1]).atan2(cen[0] - c[0]);
            if wrap(ang - yaw).abs() > cone {
                return;
            }
            if !s.world.los(c, cen, false) {
                return;
            }
            if best.map_or(true, |b| d < b.0) {
                best = Some((d, tg, cen));
            }
        };
        for (j, m) in self.mechs.iter().enumerate() {
            if j == i || !m.alive {
                continue;
            }
            let cen = [m.pos[0], m.pos[1], m.pos[2] + m.ms.height * 0.5];
            consider(dist3(c, cen), Tgt::Mech(j), cen, self);
        }
        for (j, n) in self.npcs.iter().enumerate() {
            if !n.alive {
                continue;
            }
            let cen = [n.pos[0], n.pos[1], n.pos[2] + self.cfg.npc_height * 0.5];
            consider(dist3(c, cen), Tgt::Npc(j), cen, self);
        }
        for (j, cr) in self.crates.iter().enumerate() {
            if !cr.alive {
                continue;
            }
            let cen = [cr.pos[0], cr.pos[1], cr.pos[2] + self.cfg.crate_h * 0.5];
            consider(dist3(c, cen), Tgt::Crate(j), cen, self);
        }
        let mut path = vec![c];
        let Some((_, first, cen)) = best else {
            let dir = dir_of(yaw, pitch);
            let end = [c[0] + dir[0] * range, c[1] + dir[1] * range, c[2] + dir[2] * range];
            let t = self.world.raycast(c, end).unwrap_or(1.0);
            path.push([c[0] + (end[0] - c[0]) * t, c[1] + (end[1] - c[1]) * t, c[2] + (end[2] - c[2]) * t]);
            self.beams.push(BeamEv { tick: self.tick, by: i, kind: "lightning", path });
            return;
        };
        let mut hit: Vec<Tgt> = vec![first];
        let mut cur = (first, cen);
        let mut dmg = g("damage") * mult;
        let chain = g("chain") as usize;
        let falloff = g("chain_falloff");
        let chain_range = g("chain_range");
        let mut jumps = 0;
        loop {
            path.push(cur.1);
            let src = if jumps == 0 { c } else { path[path.len() - 2] };
            self.lightning_hit(i, cur.0, dmg, src, st);
            if jumps >= chain {
                break;
            }
            // next target
            let mut nb: Option<(f64, Tgt, [f64; 3])> = None;
            for (j, m) in self.mechs.iter().enumerate() {
                if j == i || !m.alive || hit.contains(&Tgt::Mech(j)) {
                    continue;
                }
                let ce = [m.pos[0], m.pos[1], m.pos[2] + m.ms.height * 0.5];
                let d = dist3(cur.1, ce);
                if d <= chain_range && nb.map_or(true, |b| d < b.0) && self.world.los(cur.1, ce, false) {
                    nb = Some((d, Tgt::Mech(j), ce));
                }
            }
            for (j, n) in self.npcs.iter().enumerate() {
                if !n.alive || hit.contains(&Tgt::Npc(j)) {
                    continue;
                }
                let ce = [n.pos[0], n.pos[1], n.pos[2] + self.cfg.npc_height * 0.5];
                let d = dist3(cur.1, ce);
                if d <= chain_range && nb.map_or(true, |b| d < b.0) && self.world.los(cur.1, ce, false) {
                    nb = Some((d, Tgt::Npc(j), ce));
                }
            }
            for (j, cr) in self.crates.iter().enumerate() {
                if !cr.alive || hit.contains(&Tgt::Crate(j)) {
                    continue;
                }
                let ce = [cr.pos[0], cr.pos[1], cr.pos[2] + self.cfg.crate_h * 0.5];
                let d = dist3(cur.1, ce);
                if d <= chain_range && nb.map_or(true, |b| d < b.0) && self.world.los(cur.1, ce, false) {
                    nb = Some((d, Tgt::Crate(j), ce));
                }
            }
            match nb {
                Some((_, tg, ce)) => {
                    hit.push(tg);
                    cur = (tg, ce);
                    dmg *= falloff;
                    jumps += 1;
                }
                None => break,
            }
        }
        self.beams.push(BeamEv { tick: self.tick, by: i, kind: "lightning", path });
    }

    fn lightning_hit(&mut self, i: usize, tg: Tgt, dmg: f64, src: [f64; 3], _st: &std::collections::HashMap<String, f64>) {
        match tg {
            Tgt::Mech(j) => {
                let short_s = self.mechs[i].wstats[LIGHTNING].get("short_s").copied().unwrap_or(2.0);
                let mut eff = None;
                let now = self.t();
                let mut shorted = false;
                for w in self.mechs[j].slots.iter_mut().flatten() {
                    if w.kind == LASER {
                        shorted = true;
                        w.charging = false;
                        w.charge = 0.0;
                    }
                }
                if shorted {
                    self.mechs[j].short_until = now + short_s;
                    eff = Some("short");
                }
                let (dd, rear) = self.dmg_mech(j, dmg, src, Some(i), "lightning");
                self.hit_ev(i, j as i64, dd, "lightning", eff, rear);
                if eff.is_some() {
                    self.effect_ev(j, "short", i);
                }
            }
            Tgt::Npc(j) => {
                let id = self.npcs[j].id as i64;
                let dd = self.dmg_npc(j, dmg, Some(i));
                self.hit_ev(i, id, dd, "lightning", None, false);
            }
            Tgt::Crate(j) => {
                let id = self.crates[j].id as i64;
                let dd = self.dmg_crate(j, dmg);
                self.hit_ev(i, id, dd, "lightning", None, false);
            }
        }
    }

    // ------------------------------------------------------------ projectiles
    pub fn step_projs(&mut self) {
        let projs = std::mem::take(&mut self.projs);
        let mut keep = Vec::with_capacity(projs.len());
        for mut p in projs {
            if self.step_one(&mut p) {
                keep.push(p);
            }
        }
        keep.append(&mut self.projs);
        self.projs = keep;
    }

    fn step_one(&mut self, p: &mut Proj) -> bool {
        let dt = self.cfg.dt;
        if p.kind == ProjKind::Mortar {
            let g = self.cfg.weapons[MORTAR].get("gravity").copied().unwrap_or(22.0);
            p.vel[2] -= g * dt;
        }
        let np = [p.pos[0] + p.vel[0] * dt, p.pos[1] + p.vel[1] * dt, p.pos[2] + p.vel[2] * dt];
        p.age += dt;
        let seg = dist3(p.pos, np);
        let rad = match p.kind {
            ProjKind::Mortar => 0.5,
            _ => self.cfg.gun_radius,
        };
        let mut best: Option<(f64, Option<Tgt>)> = self.world.raycast(p.pos, np).map(|t| (t, None));
        let mut consider = |t: f64, tg: Tgt, best: &mut Option<(f64, Option<Tgt>)>| {
            if best.map_or(true, |b| t < b.0) {
                *best = Some((t, Some(tg)));
            }
        };
        let reach = seg + 12.0;
        for (j, m) in self.mechs.iter().enumerate() {
            if !m.alive {
                continue;
            }
            if p.owner == j as i64 && (p.kind != ProjKind::Mortar || p.age < 0.3) {
                continue;
            }
            if (m.pos[0] - p.pos[0]).abs() > reach || (m.pos[1] - p.pos[1]).abs() > reach {
                continue;
            }
            if let Some(t) = seg_vcyl(p.pos, np, m.pos[0], m.pos[1], m.ms.radius + rad, m.pos[2] - rad, m.pos[2] + m.ms.height + rad) {
                consider(t, Tgt::Mech(j), &mut best);
            }
        }
        if p.kind != ProjKind::Npc {
            for (j, n) in self.npcs.iter().enumerate() {
                if !n.alive {
                    continue;
                }
                if let Some(t) = seg_vcyl(p.pos, np, n.pos[0], n.pos[1], self.cfg.npc_radius + rad, n.pos[2] - rad, n.pos[2] + self.cfg.npc_height + rad) {
                    consider(t, Tgt::Npc(j), &mut best);
                }
            }
            for (j, c) in self.crates.iter().enumerate() {
                if !c.alive {
                    continue;
                }
                if (c.pos[0] - p.pos[0]).abs() > reach || (c.pos[1] - p.pos[1]).abs() > reach {
                    continue;
                }
                if let Some(t) = seg_vcyl(p.pos, np, c.pos[0], c.pos[1], self.cfg.crate_r + rad, c.pos[2] - rad, c.pos[2] + self.cfg.crate_h + rad) {
                    consider(t, Tgt::Crate(j), &mut best);
                }
            }
        }
        match best {
            None => {
                p.pos = np;
                match p.kind {
                    ProjKind::Mortar => p.left -= dt,
                    _ => p.left -= seg,
                }
                p.left > 0.0
            }
            Some((t, tg)) => {
                let hp = [p.pos[0] + (np[0] - p.pos[0]) * t, p.pos[1] + (np[1] - p.pos[1]) * t, p.pos[2] + (np[2] - p.pos[2]) * t];
                match p.kind {
                    ProjKind::Mortar => {
                        let l = dist3([0.0; 3], p.vel).max(1e-9);
                        let blast = [hp[0] - p.vel[0] / l * 0.3, hp[1] - p.vel[1] / l * 0.3, hp[2] - p.vel[2] / l * 0.3 + if tg.is_none() { 0.2 } else { 0.0 }];
                        let mp = p.mp.clone().unwrap();
                        self.explode(blast, p.owner, &mp, p.mult, tg);
                    }
                    _ => {
                        if let Some(tg) = tg {
                            let l = dist3([0.0; 3], p.vel).max(1e-9);
                            let src = [hp[0] - p.vel[0] / l * 10.0, hp[1] - p.vel[1] / l * 10.0, hp[2] - p.vel[2] / l * 10.0];
                            let wname = if p.kind == ProjKind::Npc { "npc" } else { "gun" };
                            let by = if p.kind == ProjKind::Npc { None } else { Some(p.owner as usize) };
                            match tg {
                                Tgt::Mech(j) => {
                                    let (dd, rear) = self.dmg_mech(j, p.dmg, src, by, wname);
                                    if p.kind == ProjKind::Npc {
                                        // attribute by = npc id in the event
                                        if let Some(e) = self.mechs[j].events.last_mut() {
                                            e.by = Some(p.owner);
                                        }
                                    }
                                    if let Some(b) = by {
                                        self.hit_ev(b, j as i64, dd, "gun", None, rear);
                                    }
                                }
                                Tgt::Npc(j) => {
                                    let id = self.npcs[j].id as i64;
                                    let dd = self.dmg_npc(j, p.dmg, by);
                                    if let Some(b) = by {
                                        self.hit_ev(b, id, dd, "gun", None, false);
                                    }
                                }
                                Tgt::Crate(j) => {
                                    let id = self.crates[j].id as i64;
                                    let dd = self.dmg_crate(j, p.dmg);
                                    if let Some(b) = by {
                                        self.hit_ev(b, id, dd, "gun", None, false);
                                    }
                                }
                            }
                        }
                    }
                }
                false
            }
        }
    }

    pub fn explode(&mut self, pos: [f64; 3], owner: i64, mp: &MortarParams, mult: f64, direct: Option<Tgt>) {
        self.emit_sound("explosion", pos, owner);
        let by = if owner >= 0 { Some(owner as usize) } else { None };
        let probe = [pos[0], pos[1], pos[2] + 0.3];
        for j in 0..self.mechs.len() {
            if !self.mechs[j].alive {
                continue;
            }
            let (mpos, r, h) = (self.mechs[j].pos, self.mechs[j].ms.radius, self.mechs[j].ms.height);
            let hd = (((mpos[0] - pos[0]).powi(2) + (mpos[1] - pos[1]).powi(2)).sqrt() - r).max(0.0);
            let vd = if pos[2] < mpos[2] { mpos[2] - pos[2] } else if pos[2] > mpos[2] + h { pos[2] - (mpos[2] + h) } else { 0.0 };
            let d = (hd * hd + vd * vd).sqrt();
            let is_direct = direct == Some(Tgt::Mech(j));
            if d > mp.radius && !is_direct {
                continue;
            }
            let cen = [mpos[0], mpos[1], mpos[2] + h * 0.5];
            if !is_direct && self.world.raycast(probe, cen).is_some() {
                continue;
            }
            let frac = (1.0 - (1.0 - mp.edge) * (d / mp.radius).min(1.0)).max(0.0);
            let mut amount = mp.splash * frac * mult;
            if is_direct {
                amount += mp.damage * mult;
            }
            let (dd, rear) = self.dmg_mech(j, amount, pos, by, "mortar");
            // concussion
            let (dx, dy) = (cen[0] - pos[0], cen[1] - pos[1]);
            let l = (dx * dx + dy * dy).sqrt();
            let (ux, uy) = if l < 1e-6 { (0.0, 0.0) } else { (dx / l, dy / l) };
            {
                let m = &mut self.mechs[j];
                m.vel[0] += ux * mp.knock * frac;
                m.vel[1] += uy * mp.knock * frac;
                m.vel[2] = m.vel[2].max(0.0) + mp.knock_up * frac;
                if mp.knock_up * frac > 0.0 {
                    m.grounded = false;
                }
            }
            self.spend_energy(j, mp.drain * frac);
            if let Some(b) = by {
                self.effect_ev(j, "concuss", b);
                if b != j {
                    self.hit_ev(b, j as i64, dd, "mortar", Some("concuss"), rear);
                }
            }
        }
        for j in 0..self.npcs.len() {
            if !self.npcs[j].alive {
                continue;
            }
            let n = &self.npcs[j];
            let hd = (((n.pos[0] - pos[0]).powi(2) + (n.pos[1] - pos[1]).powi(2)).sqrt() - self.cfg.npc_radius).max(0.0);
            let vd = if pos[2] < n.pos[2] { n.pos[2] - pos[2] } else if pos[2] > n.pos[2] + self.cfg.npc_height { pos[2] - n.pos[2] - self.cfg.npc_height } else { 0.0 };
            let d = (hd * hd + vd * vd).sqrt();
            let is_direct = direct == Some(Tgt::Npc(j));
            if d > mp.radius && !is_direct {
                continue;
            }
            let frac = (1.0 - (1.0 - mp.edge) * (d / mp.radius).min(1.0)).max(0.0);
            let mut amount = mp.splash * frac * mult;
            if is_direct {
                amount += mp.damage * mult;
            }
            let id = n.id as i64;
            let dd = self.dmg_npc(j, amount, by);
            if let Some(b) = by {
                self.hit_ev(b, id, dd, "mortar", None, false);
            }
        }
        for j in 0..self.crates.len() {
            if !self.crates[j].alive {
                continue;
            }
            let c = &self.crates[j];
            let hd = (((c.pos[0] - pos[0]).powi(2) + (c.pos[1] - pos[1]).powi(2)).sqrt() - self.cfg.crate_r).max(0.0);
            let vd = if pos[2] < c.pos[2] { c.pos[2] - pos[2] } else if pos[2] > c.pos[2] + self.cfg.crate_h { pos[2] - c.pos[2] - self.cfg.crate_h } else { 0.0 };
            let d = (hd * hd + vd * vd).sqrt();
            let is_direct = direct == Some(Tgt::Crate(j));
            if d > mp.radius && !is_direct {
                continue;
            }
            let frac = (1.0 - (1.0 - mp.edge) * (d / mp.radius).min(1.0)).max(0.0);
            let mut amount = mp.splash * frac * mult;
            if is_direct {
                amount += mp.damage * mult;
            }
            let id = c.id as i64;
            let dd = self.dmg_crate(j, amount);
            if let Some(b) = by {
                self.hit_ev(b, id, dd, "mortar", None, false);
            }
        }
    }
}

pub fn dist3(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}
