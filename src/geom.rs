use crate::protocol::V3;
use std::f64::consts::PI;

pub fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn dot(a: V3, b: V3) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn len(a: V3) -> f64 {
    dot(a, a).sqrt()
}
pub fn dist(a: V3, b: V3) -> f64 {
    len(sub(a, b))
}
pub fn dist2d(a: V3, b: V3) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2)).sqrt()
}
pub fn wrap(a: f64) -> f64 {
    let mut x = a % (2.0 * PI);
    if x > PI {
        x -= 2.0 * PI;
    }
    if x < -PI {
        x += 2.0 * PI;
    }
    x
}
pub fn yaw_to(from: V3, to: V3) -> f64 {
    (to[1] - from[1]).atan2(to[0] - from[0])
}
pub fn pitch_to(from: V3, to: V3) -> f64 {
    let h = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
    (to[2] - from[2]).atan2(h.max(1e-6))
}

/// Point where a projectile of constant `speed` fired from `from` meets a target moving with `vel`.
/// Returns (aim point, time of flight). Falls back to the target position when no solution exists.
pub fn intercept(from: V3, tpos: V3, tvel: V3, speed: f64) -> (V3, f64) {
    let r = sub(tpos, from);
    let a = dot(tvel, tvel) - speed * speed;
    let b = 2.0 * dot(r, tvel);
    let c = dot(r, r);
    let mut t = -1.0;
    if a.abs() < 1e-9 {
        if b.abs() > 1e-9 {
            t = -c / b;
        }
    } else {
        let disc = b * b - 4.0 * a * c;
        if disc >= 0.0 {
            let s = disc.sqrt();
            let t1 = (-b - s) / (2.0 * a);
            let t2 = (-b + s) / (2.0 * a);
            let (lo, hi) = if t1 < t2 { (t1, t2) } else { (t2, t1) };
            t = if lo > 0.0 { lo } else { hi };
        }
    }
    if !(t > 0.0) || t > 3.0 {
        let tt = (len(r) / speed).min(3.0);
        return (tpos, tt);
    }
    ([tpos[0] + tvel[0] * t, tpos[1] + tvel[1] * t, tpos[2] + tvel[2] * t], t)
}

/// Low-arc mortar launch pitch to reach horizontal distance `d` and height difference `dz`.
pub fn mortar_pitch(d: f64, dz: f64, v: f64, g: f64) -> Option<f64> {
    let v2 = v * v;
    let disc = v2 * v2 - g * (g * d * d + 2.0 * dz * v2);
    if disc < 0.0 || d < 1e-6 {
        return None;
    }
    Some(((v2 - disc.sqrt()) / (g * d)).atan())
}
