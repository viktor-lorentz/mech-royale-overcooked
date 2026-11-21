//! Static map: terrain heightfield + extruded building prisms, with fast line-of-sight / shot tests.
use crate::protocol::V3;
use serde_json::Value;

pub struct Prism {
    pub base: f64,
    pub top: f64,
    pub ring: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
    pub bb: [f64; 4], // minx miny maxx maxy
    pub kind: u8,     // 0 building 1 stadium 2 bridge
}

pub struct World {
    nx: usize,
    ny: usize,
    minx: f64,
    miny: f64,
    step: f64,
    terr: Vec<i16>,
    pub prisms: Vec<Prism>,
    gmin: [f64; 2],
    gcell: f64,
    gw: usize,
    gh: usize,
    grid: Vec<Vec<u32>>,
    pub bounds: [f64; 4],
}

fn pts(flat: &Value) -> Vec<[f64; 2]> {
    let a: Vec<f64> = flat.as_array().map(|v| v.iter().filter_map(|x| x.as_f64()).collect()).unwrap_or_default();
    a.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
}

fn in_poly(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (xi, yi, xj, yj) = (poly[i][0], poly[i][1], poly[j][0], poly[j][1]);
        if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
            inside = !inside;
        }
        j = i;
    }
    inside
}

impl World {
    pub fn load(dir: &str) -> Option<World> {
        let w: Value = serde_json::from_slice(&std::fs::read(format!("{dir}/v2/world.json")).ok()?).ok()?;
        let raw = std::fs::read(format!("{dir}/v2/terrain.i16")).ok()?;
        let t = &w["terrain"];
        let nx = t["nx"].as_u64()? as usize;
        let ny = t["ny"].as_u64()? as usize;
        let terr: Vec<i16> = raw.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        if terr.len() < nx * ny {
            return None;
        }
        let mut prisms = Vec::new();
        for p in w["prisms"].as_array()? {
            let ring = pts(&p["ring"]);
            if ring.len() < 3 {
                continue;
            }
            let holes: Vec<Vec<[f64; 2]>> = p["holes"].as_array().map(|h| h.iter().map(pts).collect()).unwrap_or_default();
            let mut bb = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
            for q in &ring {
                bb[0] = bb[0].min(q[0]);
                bb[1] = bb[1].min(q[1]);
                bb[2] = bb[2].max(q[0]);
                bb[3] = bb[3].max(q[1]);
            }
            let kind = match p["kind"].as_str().unwrap_or("building") {
                "stadium" => 1,
                "bridge" => 2,
                _ => 0,
            };
            prisms.push(Prism { base: p["base"].as_f64()?, top: p["top"].as_f64()?, ring, holes, bb, kind });
        }
        let b = &w["bounds"];
        let bounds = [b["minX"].as_f64()?, b["minY"].as_f64()?, b["maxX"].as_f64()?, b["maxY"].as_f64()?];
        let gcell = 16.0;
        let gmin = [bounds[0] - 16.0, bounds[1] - 16.0];
        let gw = ((bounds[2] - gmin[0]) / gcell).ceil() as usize + 2;
        let gh = ((bounds[3] - gmin[1]) / gcell).ceil() as usize + 2;
        let mut grid = vec![Vec::new(); gw * gh];
        for (i, p) in prisms.iter().enumerate() {
            let x0 = ((p.bb[0] - gmin[0]) / gcell).floor().max(0.0) as usize;
            let x1 = (((p.bb[2] - gmin[0]) / gcell).floor() as usize).min(gw - 1);
            let y0 = ((p.bb[1] - gmin[1]) / gcell).floor().max(0.0) as usize;
            let y1 = (((p.bb[3] - gmin[1]) / gcell).floor() as usize).min(gh - 1);
            for gy in y0..=y1 {
                for gx in x0..=x1 {
                    grid[gy * gw + gx].push(i as u32);
                }
            }
        }
        Some(World {
            nx,
            ny,
            minx: t["minX"].as_f64()?,
            miny: t["minY"].as_f64()?,
            step: t["step"].as_f64().unwrap_or(1.0),
            terr,
            prisms,
            gmin,
            gcell,
            gw,
            gh,
            grid,
            bounds,
        })
    }

    /// Bilinear ground height (m).
    pub fn ground(&self, x: f64, y: f64) -> f64 {
        let fx = ((x - self.minx) / self.step).clamp(0.0, (self.nx - 1) as f64 - 1e-6);
        let fy = ((y - self.miny) / self.step).clamp(0.0, (self.ny - 1) as f64 - 1e-6);
        let (ix, iy) = (fx as usize, fy as usize);
        let (tx, ty) = (fx - ix as f64, fy - iy as f64);
        let h = |i: usize, j: usize| self.terr[j * self.nx + i] as f64 * 0.01;
        let a = h(ix, iy) * (1.0 - tx) + h(ix + 1, iy) * tx;
        let b = h(ix, iy + 1) * (1.0 - tx) + h(ix + 1, iy + 1) * tx;
        a * (1.0 - ty) + b * ty
    }

    fn cell(&self, x: f64, y: f64) -> Option<&Vec<u32>> {
        let gx = ((x - self.gmin[0]) / self.gcell).floor();
        let gy = ((y - self.gmin[1]) / self.gcell).floor();
        if gx < 0.0 || gy < 0.0 || gx as usize >= self.gw || gy as usize >= self.gh {
            return None;
        }
        Some(&self.grid[gy as usize * self.gw + gx as usize])
    }

    /// True when (x,y,z) is inside any solid prism.
    pub fn solid(&self, x: f64, y: f64, z: f64) -> bool {
        if let Some(c) = self.cell(x, y) {
            for &i in c {
                let p = &self.prisms[i as usize];
                if z < p.base || z > p.top || x < p.bb[0] || x > p.bb[2] || y < p.bb[1] || y > p.bb[3] {
                    continue;
                }
                if in_poly(&p.ring, x, y) && !p.holes.iter().any(|h| in_poly(h, x, y)) {
                    return true;
                }
            }
        }
        false
    }

    /// Does the straight segment a->b hit terrain or a building? (shots and sight lines)
    pub fn blocked(&self, a: V3, b: V3) -> bool {
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        if len < 1e-6 {
            return false;
        }
        let n = (len / 1.5).ceil().max(1.0) as usize;
        for i in 1..n {
            let t = i as f64 / n as f64;
            let (x, y, z) = (a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t);
            if z < self.ground(x, y) || self.solid(x, y, z) {
                return true;
            }
        }
        false
    }

    /// Distance along the ray before it hits something (len if clear).
    pub fn first_hit(&self, a: V3, b: V3) -> f64 {
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let n = (len / 1.5).ceil().max(1.0) as usize;
        for i in 1..=n {
            let t = i as f64 / n as f64;
            let (x, y, z) = (a[0] + d[0] * t, a[1] + d[1] * t, a[2] + d[2] * t);
            if z < self.ground(x, y) || self.solid(x, y, z) {
                return len * (i - 1) as f64 / n as f64;
            }
        }
        len
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn world() -> Option<World> {
        World::load(&format!("{}/kit/arena", env!("CARGO_MANIFEST_DIR")))
    }
    #[test]
    fn loads_and_ground_sane() {
        let w = world().expect("map");
        assert!(w.prisms.len() > 200);
        let g = w.ground(0.0, 0.0);
        assert!(g.abs() < 20.0, "ground at origin {g}");
    }
    #[test]
    fn building_blocks_sight() {
        let w = world().expect("map");
        // find a building interior point and shoot straight through it at mid height
        let p = w.prisms.iter().find(|p| p.kind == 0 && p.top - p.base > 12.0).unwrap();
        let c = p.ring.iter().fold([0.0, 0.0], |a, q| [a[0] + q[0] / p.ring.len() as f64, a[1] + q[1] / p.ring.len() as f64]);
        let z = (p.base + p.top) * 0.5;
        let hit = (0..8).any(|k| {
            let ang = k as f64 * 0.785;
            let a = [c[0] + 400.0 * ang.cos(), c[1] + 400.0 * ang.sin(), z];
            let b = [c[0] - 400.0 * ang.cos(), c[1] - 400.0 * ang.sin(), z];
            w.blocked(a, b)
        });
        assert!(hit);
    }
}
