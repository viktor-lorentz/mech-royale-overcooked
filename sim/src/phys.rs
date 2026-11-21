//! Mech movement, jump/jet, torso slew, energy.
use crate::map::World;
use crate::sim::*;

pub fn wrap(a: f64) -> f64 {
    let mut x = a % std::f64::consts::TAU;
    if x > std::f64::consts::PI {
        x -= std::f64::consts::TAU;
    }
    if x < -std::f64::consts::PI {
        x += std::f64::consts::TAU;
    }
    x
}

/// Parameters of a walking body.
#[derive(Clone, Copy)]
pub struct Body {
    pub r: f64,
    pub h: f64,
    pub step_up: f64,
    pub max_slope: f64,
}

impl World {
    fn walk_ok(&self, b: &Body, x0: f64, y0: f64, x: f64, y: f64, feet: f64, grounded: bool) -> bool {
        if !self.in_bounds(x, y, b.r) {
            return false;
        }
        let g = self.ground(x, y);
        if g - feet > b.step_up {
            return false;
        }
        if grounded {
            let g0 = self.ground(x0, y0);
            let rise = g - g0;
            if rise > 0.0 {
                let d = ((x - x0).powi(2) + (y - y0).powi(2)).sqrt().max(1e-6);
                if (rise / d).atan() > b.max_slope && g - feet > 0.05 {
                    return false;
                }
            }
        }
        !self.hits_wall(x, y, feet, b.r, b.h, b.step_up)
    }

    /// Move horizontally by (dx,dy) with sliding. Returns (new x, new y, blocked).
    pub fn walk(&self, b: &Body, x: f64, y: f64, feet: f64, grounded: bool, dx: f64, dy: f64) -> (f64, f64, bool) {
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-9 {
            return (x, y, false);
        }
        let n = ((len / 0.5).ceil() as usize).max(1);
        let (sx, sy) = (dx / n as f64, dy / n as f64);
        let (mut cx, mut cy) = (x, y);
        let mut blocked = false;
        for _ in 0..n {
            if self.walk_ok(b, cx, cy, cx + sx, cy + sy, feet, grounded) {
                cx += sx;
                cy += sy;
            } else {
                blocked = true;
                let mut moved = false;
                if sx.abs() > 1e-9 && self.walk_ok(b, cx, cy, cx + sx, cy, feet, grounded) {
                    cx += sx;
                    moved = true;
                }
                if sy.abs() > 1e-9 && self.walk_ok(b, cx, cy, cx, cy + sy, feet, grounded) {
                    cy += sy;
                    moved = true;
                }
                if !moved {
                    break;
                }
            }
        }
        (cx, cy, blocked)
    }
}

impl Sim {
    pub fn body_of(&self, i: usize) -> Body {
        let ms = &self.mechs[i].ms;
        Body { r: ms.radius, h: ms.height, step_up: ms.step_up, max_slope: ms.max_slope }
    }

    pub fn spend_energy(&mut self, i: usize, amt: f64) {
        let m = &mut self.mechs[i];
        m.energy = (m.energy - amt).max(0.0);
        m.en_idle = 0.0;
    }

    pub fn emit_sound(&mut self, kind: &str, pos: [f64; 3], src: i64) {
        self.sounds.push(SoundEv { tick: self.tick, pos, kind: kind.to_string(), src });
    }

    /// One tick of movement for mech i.
    pub fn phys_mech(&mut self, i: usize) {
        let dt = self.cfg.dt;
        let world = self.world.clone();
        let b = self.body_of(i);
        let t = self.t();
        let mut sounds: Vec<(&'static str, [f64; 3])> = vec![];
        let m = &mut self.mechs[i];
        let ms = m.ms.clone();
        // torso slew
        if let Some(a) = m.cmd.aim {
            let cp = a[1].clamp(ms.min_pitch, ms.max_pitch);
            m.cyaw = a[0];
            m.cpitch = cp;
        }
        let charging = m.slots.iter().flatten().any(|w| w.kind == crate::cfg::LASER && w.charging);
        let dy = wrap(m.cyaw - m.yaw);
        let ty = ms.turn_rate * dt;
        m.yaw = wrap(m.yaw + dy.clamp(-ty, ty));
        let dp = m.cpitch - m.pitch;
        let tp = ms.pitch_rate * dt;
        m.pitch = (m.pitch + dp.clamp(-tp, tp)).clamp(ms.min_pitch, ms.max_pitch);

        // ground contact check
        if m.grounded {
            let sf = world.support(m.pos[0], m.pos[1], m.pos[2], b.step_up);
            if sf >= m.pos[2] - b.step_up - 1e-6 {
                m.pos[2] = sf;
                m.vel[2] = 0.0;
            } else {
                m.grounded = false;
                m.vel[2] = 0.0;
            }
        }
        // desired horizontal velocity
        let mut mv = m.cmd.mv;
        let ml = (mv[0] * mv[0] + mv[1] * mv[1]).sqrt();
        if ml > 1.0 {
            mv = [mv[0] / ml, mv[1] / ml];
        }
        let moving = ml > 0.01;
        let mut speed = ms.walk;
        let mut spend = 0.0;
        let boost = m.cmd.boost;
        let mut boosting_ground = false;
        if boost && m.grounded && moving && m.energy > 0.0 {
            speed = ms.boost;
            spend += ms.boost_drain * dt;
            boosting_ground = true;
        }
        if charging {
            speed *= ms.charge_move;
        }
        let des = [mv[0] * speed, mv[1] * speed];
        let acc = if m.grounded { ms.ground_accel } else { ms.air_accel } * dt;
        let (ex, ey) = (des[0] - m.vel[0], des[1] - m.vel[1]);
        let el = (ex * ex + ey * ey).sqrt();
        if el <= acc {
            m.vel[0] = des[0];
            m.vel[1] = des[1];
        } else {
            m.vel[0] += ex / el * acc;
            m.vel[1] += ey / el * acc;
        }
        // vertical
        let mut jetting = false;
        if m.jump_pending && m.grounded {
            m.vel[2] = ms.jump;
            m.grounded = false;
        }
        m.jump_pending = false;
        if boost && !m.grounded && m.energy > 0.0 {
            jetting = true;
            if m.vel[2] < ms.max_rise {
                m.vel[2] = (m.vel[2] + ms.jet_accel * dt).min(ms.max_rise);
            }
            spend += ms.jet_drain * dt;
        }
        if !m.grounded {
            m.vel[2] -= ms.gravity * dt;
        }
        // horizontal move
        let (ox, oy) = (m.pos[0], m.pos[1]);
        let (dx, dy2) = (m.vel[0] * dt, m.vel[1] * dt);
        let (nx, ny, blocked) = world.walk(&b, ox, oy, m.pos[2], m.grounded, dx, dy2);
        if blocked {
            m.vel[0] = (nx - ox) / dt;
            m.vel[1] = (ny - oy) / dt;
        }
        m.pos[0] = nx;
        m.pos[1] = ny;
        // follow the ground to the new position
        if m.grounded {
            let sf = world.support(nx, ny, m.pos[2], b.step_up);
            if sf >= m.pos[2] - b.step_up - 1e-6 {
                m.pos[2] = sf;
            } else {
                m.grounded = false;
            }
        }
        // vertical move
        if !m.grounded {
            let feet_old = m.pos[2];
            let mut feet_new = feet_old + m.vel[2] * dt;
            if m.vel[2] > 0.0 {
                if let Some(base) = world.ceiling(nx, ny, b.r, feet_old + b.h, feet_new + b.h) {
                    feet_new = (base - b.h - 0.01).max(feet_old.min(feet_new));
                    m.vel[2] = 0.0;
                }
                m.pos[2] = feet_new;
            } else {
                let s = world.support(nx, ny, feet_old, b.step_up);
                let mut land: Option<f64> = None;
                if feet_new <= s {
                    land = Some(s);
                }
                if let Some((top, inside, push)) = world.landing_surface(nx, ny, b.r, feet_old, feet_new) {
                    if top > land.unwrap_or(f64::MIN) {
                        land = Some(top);
                        if !inside {
                            let (px, py, _) = world.walk(&b, nx, ny, top, true, push[0] * 0.6, push[1] * 0.6);
                            m.pos[0] = px;
                            m.pos[1] = py;
                        }
                    }
                }
                if let Some(l) = land {
                    m.pos[2] = l;
                    m.vel[2] = 0.0;
                    m.grounded = true;
                } else {
                    m.pos[2] = feet_new;
                }
            }
            if m.pos[2] < -200.0 {
                m.pos[2] = -200.0;
            }
        }
        m.knocked = false;
        // energy
        if spend > 0.0 {
            m.energy = (m.energy - spend).max(0.0);
            m.en_idle = 0.0;
        } else {
            m.en_idle += dt;
            if m.en_idle >= ms.en_delay {
                m.energy = (m.energy + ms.energy_regen * dt).min(m.max_energy);
            }
        }
        // footsteps
        let sp = (m.vel[0] * m.vel[0] + m.vel[1] * m.vel[1]).sqrt();
        if m.grounded && sp > self.cfg.step_speed {
            m.step_timer += dt;
            if m.step_timer >= self.cfg.step_every {
                m.step_timer = 0.0;
                sounds.push((if boosting_ground { "boost" } else { "step" }, m.pos));
            }
        } else if jetting {
            m.step_timer += dt;
            if m.step_timer >= self.cfg.step_every * 0.5 {
                m.step_timer = 0.0;
                sounds.push(("jet", m.pos));
            }
        }
        let _ = t;
        let id = i as i64;
        for (k, p) in sounds {
            self.emit_sound(k, p, id);
        }
    }

    pub fn separate_mechs(&mut self) {
        let n = self.mechs.len();
        for i in 0..n {
            if !self.mechs[i].alive {
                continue;
            }
            for j in (i + 1)..n {
                if !self.mechs[j].alive {
                    continue;
                }
                let (a, b) = (&self.mechs[i], &self.mechs[j]);
                let (dx, dy) = (b.pos[0] - a.pos[0], b.pos[1] - a.pos[1]);
                let d = (dx * dx + dy * dy).sqrt();
                let rr = a.ms.radius + b.ms.radius;
                if d >= rr {
                    continue;
                }
                if a.pos[2] >= b.pos[2] + b.ms.height || b.pos[2] >= a.pos[2] + a.ms.height {
                    continue;
                }
                let (ux, uy) = if d < 1e-6 { (1.0, 0.0) } else { (dx / d, dy / d) };
                let push = (rr - d) * 0.5;
                let ba = self.body_of(i);
                let bb = self.body_of(j);
                let world = self.world.clone();
                let (fa, fb) = (self.mechs[i].pos[2], self.mechs[j].pos[2]);
                let (ga, gb) = (self.mechs[i].grounded, self.mechs[j].grounded);
                let (ax, ay, _) = world.walk(&ba, self.mechs[i].pos[0], self.mechs[i].pos[1], fa, ga, -ux * push, -uy * push);
                let (bx, by, _) = world.walk(&bb, self.mechs[j].pos[0], self.mechs[j].pos[1], fb, gb, ux * push, uy * push);
                self.mechs[i].pos[0] = ax;
                self.mechs[i].pos[1] = ay;
                self.mechs[j].pos[0] = bx;
                self.mechs[j].pos[1] = by;
            }
        }
    }
}
