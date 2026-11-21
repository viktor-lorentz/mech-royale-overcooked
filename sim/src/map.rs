//! Static world: terrain heightfield, prisms, trees, with grid indexes and geometric queries.
use serde_json::Value;
use std::path::Path;

pub const CELL: f64 = 16.0;
pub const EPS: f64 = 1e-4;

pub struct Prism {
    pub kind: u8, // 0 building, 1 stadium, 2 bridge
    pub base: f64,
    pub top: f64,
    pub ring: Vec<[f64; 2]>,
    pub holes: Vec<Vec<[f64; 2]>>,
    pub bb: [f64; 4],
}

pub struct Tree {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub h: f64,
    pub cr: f64,
    pub cb: f64,
    pub tr: f64,
    pub cx: f64,
    pub cy: f64,
}

pub struct World {
    pub terrain: Vec<i16>,
    pub nx: usize,
    pub ny: usize,
    pub minx: f64,
    pub miny: f64,
    pub step: f64,
    pub bounds: [f64; 4],
    pub max_ground: f64,
    pub prisms: Vec<Prism>,
    pub trees: Vec<Tree>,
    gw: usize,
    gh: usize,
    gx0: f64,
    gy0: f64,
    pcells: Vec<Vec<u32>>,
    tcells: Vec<Vec<u32>>,
    pub canopy_density: f64,
    pub canopy_block_t: f64,
}

fn ring_from(flat: &Value) -> Vec<[f64; 2]> {
    let a: Vec<f64> = flat.as_array().map(|a| a.iter().filter_map(|v| v.as_f64()).collect()).unwrap_or_default();
    a.chunks(2).filter(|c| c.len() == 2).map(|c| [c[0], c[1]]).collect()
}

pub fn point_in_poly(poly: &[[f64; 2]], x: f64, y: f64) -> bool {
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

fn dist2_seg(px: f64, py: f64, a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 < 1e-12 { 0.0 } else { (((px - a[0]) * dx + (py - a[1]) * dy) / l2).clamp(0.0, 1.0) };
    let (qx, qy) = (a[0] + t * dx, a[1] + t * dy);
    (px - qx) * (px - qx) + (py - qy) * (py - qy)
}

/// Closest point on polygon boundary (ring) to p.
fn closest_on_poly(poly: &[[f64; 2]], px: f64, py: f64) -> ([f64; 2], f64) {
    let n = poly.len();
    let mut best = (poly[0], f64::MAX);
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[j], poly[i]);
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let l2 = dx * dx + dy * dy;
        let t = if l2 < 1e-12 { 0.0 } else { (((px - a[0]) * dx + (py - a[1]) * dy) / l2).clamp(0.0, 1.0) };
        let q = [a[0] + t * dx, a[1] + t * dy];
        let d = (px - q[0]).powi(2) + (py - q[1]).powi(2);
        if d < best.1 {
            best = (q, d);
        }
        j = i;
    }
    best
}

impl Prism {
    pub fn contains_xy(&self, x: f64, y: f64) -> bool {
        if x < self.bb[0] || x > self.bb[2] || y < self.bb[1] || y > self.bb[3] {
            return false;
        }
        if !point_in_poly(&self.ring, x, y) {
            return false;
        }
        for h in &self.holes {
            if point_in_poly(h, x, y) {
                return false;
            }
        }
        true
    }
    pub fn circle_overlaps(&self, x: f64, y: f64, r: f64) -> bool {
        if x < self.bb[0] - r || x > self.bb[2] + r || y < self.bb[1] - r || y > self.bb[3] + r {
            return false;
        }
        if self.contains_xy(x, y) {
            return true;
        }
        let r2 = r * r;
        let edges = |poly: &Vec<[f64; 2]>| -> bool {
            let n = poly.len();
            if n < 2 {
                return false;
            }
            let mut j = n - 1;
            for i in 0..n {
                if dist2_seg(x, y, poly[j], poly[i]) < r2 {
                    return true;
                }
                j = i;
            }
            false
        };
        if edges(&self.ring) {
            return true;
        }
        for h in &self.holes {
            if edges(h) {
                return true;
            }
        }
        false
    }
    /// First t in [0,1] at which segment a->b is inside the solid.
    pub fn seg_hit(&self, a: [f64; 3], b: [f64; 3]) -> Option<f64> {
        let (minx, maxx) = (a[0].min(b[0]), a[0].max(b[0]));
        let (miny, maxy) = (a[1].min(b[1]), a[1].max(b[1]));
        if maxx < self.bb[0] || minx > self.bb[2] || maxy < self.bb[1] || miny > self.bb[3] {
            return None;
        }
        let dz = b[2] - a[2];
        let (mut t0, mut t1) = (0.0f64, 1.0f64);
        if dz.abs() < 1e-12 {
            if a[2] < self.base || a[2] > self.top {
                return None;
            }
        } else {
            let ta = (self.base - a[2]) / dz;
            let tb = (self.top - a[2]) / dz;
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        }
        if t0 > t1 {
            return None;
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let mut ts: Vec<f64> = Vec::with_capacity(16);
        ts.push(t0);
        ts.push(t1);
        let mut add_poly = |poly: &Vec<[f64; 2]>| {
            let n = poly.len();
            if n < 2 {
                return;
            }
            let mut j = n - 1;
            for i in 0..n {
                let (p, q) = (poly[j], poly[i]);
                let (ex, ey) = (q[0] - p[0], q[1] - p[1]);
                let den = dx * ey - dy * ex;
                if den.abs() > 1e-12 {
                    let t = ((p[0] - a[0]) * ey - (p[1] - a[1]) * ex) / den;
                    let u = ((p[0] - a[0]) * dy - (p[1] - a[1]) * dx) / den;
                    if t > t0 && t < t1 && (0.0..=1.0).contains(&u) {
                        ts.push(t);
                    }
                }
                j = i;
            }
        };
        add_poly(&self.ring);
        for h in &self.holes {
            add_poly(h);
        }
        ts.sort_by(|x, y| x.partial_cmp(y).unwrap());
        for w in ts.windows(2) {
            let m = 0.5 * (w[0] + w[1]);
            if w[1] - w[0] < 1e-9 && ts.len() > 2 {
                continue;
            }
            if self.contains_xy(a[0] + dx * m, a[1] + dy * m) {
                return Some(w[0]);
            }
        }
        None
    }
}

impl World {
    pub fn load(dir: &Path) -> Result<World, String> {
        let wj = std::fs::read_to_string(dir.join("v2/world.json")).map_err(|e| format!("world.json: {e}"))?;
        let v: Value = serde_json::from_str(&wj).map_err(|e| e.to_string())?;
        let raw = std::fs::read(dir.join("v2/terrain.i16")).map_err(|e| format!("terrain.i16: {e}"))?;
        let t = &v["terrain"];
        let nx = t["nx"].as_u64().unwrap_or(1070) as usize;
        let ny = t["ny"].as_u64().unwrap_or(724) as usize;
        let terrain: Vec<i16> = raw.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
        if terrain.len() < nx * ny {
            return Err("terrain too short".into());
        }
        let b = &v["bounds"];
        let bounds = [
            b["minX"].as_f64().unwrap_or(-512.0),
            b["minY"].as_f64().unwrap_or(-333.0),
            b["maxX"].as_f64().unwrap_or(555.0),
            b["maxY"].as_f64().unwrap_or(388.0),
        ];
        let mut prisms = vec![];
        for p in v["prisms"].as_array().cloned().unwrap_or_default() {
            let ring = ring_from(&p["ring"]);
            if ring.len() < 3 {
                continue;
            }
            let holes: Vec<Vec<[f64; 2]>> =
                p["holes"].as_array().map(|a| a.iter().map(ring_from).filter(|h| h.len() >= 3).collect()).unwrap_or_default();
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
            prisms.push(Prism {
                kind,
                base: p["base"].as_f64().unwrap_or(0.0),
                top: p["top"].as_f64().unwrap_or(0.0),
                ring,
                holes,
                bb,
            });
        }
        let mut trees = vec![];
        let tr = &v["trees"];
        let fields: Vec<String> =
            tr["fields"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect()).unwrap_or_default();
        let data: Vec<f64> = tr["data"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default();
        let fi = |n: &str| fields.iter().position(|f| f == n);
        if !fields.is_empty() {
            let (ix, iy, iz, ih, icr, icb, itr) = (
                fi("x").unwrap_or(0),
                fi("y").unwrap_or(1),
                fi("z").unwrap_or(2),
                fi("height").unwrap_or(3),
                fi("crownR").unwrap_or(4),
                fi("crownBase").unwrap_or(5),
                fi("trunkR").unwrap_or(6),
            );
            let (icx, icy) = (fi("crownX"), fi("crownY"));
            for row in data.chunks(fields.len()) {
                if row.len() < fields.len() {
                    break;
                }
                let x = row[ix];
                let y = row[iy];
                trees.push(Tree {
                    x,
                    y,
                    z: row[iz],
                    h: row[ih],
                    cr: row[icr],
                    cb: row[icb],
                    tr: row[itr],
                    cx: icx.map(|i| row[i]).unwrap_or(x),
                    cy: icy.map(|i| row[i]).unwrap_or(y),
                });
            }
        }
        let max_ground = terrain.iter().map(|&h| h as f64 * 0.01).fold(f64::MIN, f64::max);
        let gx0 = bounds[0] - 40.0;
        let gy0 = bounds[1] - 40.0;
        let gw = ((bounds[2] - bounds[0] + 80.0) / CELL).ceil() as usize + 1;
        let gh = ((bounds[3] - bounds[1] + 80.0) / CELL).ceil() as usize + 1;
        let mut w = World {
            terrain,
            nx,
            ny,
            minx: t["minX"].as_f64().unwrap_or(-513.0),
            miny: t["minY"].as_f64().unwrap_or(-334.0),
            step: t["step"].as_f64().unwrap_or(1.0),
            bounds,
            max_ground,
            prisms,
            trees,
            gw,
            gh,
            gx0,
            gy0,
            pcells: vec![vec![]; gw * gh],
            tcells: vec![vec![]; gw * gh],
            canopy_density: v["rules"]["canopyDensity"].as_f64().unwrap_or(0.35),
            canopy_block_t: v["rules"]["canopyBlockT"].as_f64().unwrap_or(0.25),
        };
        for (i, p) in w.prisms.iter().enumerate() {
            let (c0x, c0y) = w.cell_of(p.bb[0], p.bb[1]);
            let (c1x, c1y) = w.cell_of(p.bb[2], p.bb[3]);
            for cy in c0y..=c1y {
                for cx in c0x..=c1x {
                    w.pcells[cy * w.gw + cx].push(i as u32);
                }
            }
        }
        for (i, t) in w.trees.iter().enumerate() {
            let rr = t.cr.max(t.tr);
            let (c0x, c0y) = w.cell_of(t.x.min(t.cx) - rr, t.y.min(t.cy) - rr);
            let (c1x, c1y) = w.cell_of(t.x.max(t.cx) + rr, t.y.max(t.cy) + rr);
            for cy in c0y..=c1y {
                for cx in c0x..=c1x {
                    w.tcells[cy * w.gw + cx].push(i as u32);
                }
            }
        }
        Ok(w)
    }

    fn cell_of(&self, x: f64, y: f64) -> (usize, usize) {
        let cx = (((x - self.gx0) / CELL).floor().max(0.0) as usize).min(self.gw - 1);
        let cy = (((y - self.gy0) / CELL).floor().max(0.0) as usize).min(self.gh - 1);
        (cx, cy)
    }

    pub fn ground(&self, x: f64, y: f64) -> f64 {
        let fx = ((x - self.minx) / self.step).clamp(0.0, (self.nx - 1) as f64 - 1e-9);
        let fy = ((y - self.miny) / self.step).clamp(0.0, (self.ny - 1) as f64 - 1e-9);
        let (ix, iy) = (fx as usize, fy as usize);
        let (tx, ty) = (fx - ix as f64, fy - iy as f64);
        let h = |i: usize, j: usize| self.terrain[j * self.nx + i] as f64 * 0.01;
        let a = h(ix, iy) * (1.0 - tx) + h(ix + 1, iy) * tx;
        let b = h(ix, iy + 1) * (1.0 - tx) + h(ix + 1, iy + 1) * tx;
        a * (1.0 - ty) + b * ty
    }

    pub fn in_bounds(&self, x: f64, y: f64, margin: f64) -> bool {
        x >= self.bounds[0] + margin && x <= self.bounds[2] - margin && y >= self.bounds[1] + margin && y <= self.bounds[3] - margin
    }

    fn cells_along(&self, a: [f64; 3], b: [f64; 3], out: &mut Vec<usize>, trees: bool) {
        let _ = trees;
        out.clear();
        let (ax, ay) = ((a[0] - self.gx0) / CELL, (a[1] - self.gy0) / CELL);
        let (bx, by) = ((b[0] - self.gx0) / CELL, (b[1] - self.gy0) / CELL);
        let mut cx = ax.floor() as i64;
        let mut cy = ay.floor() as i64;
        let ex = bx.floor() as i64;
        let ey = by.floor() as i64;
        let (dx, dy) = (bx - ax, by - ay);
        let sx = if dx > 0.0 { 1 } else { -1 };
        let sy = if dy > 0.0 { 1 } else { -1 };
        let tdx = if dx.abs() < 1e-12 { f64::MAX } else { 1.0 / dx.abs() };
        let tdy = if dy.abs() < 1e-12 { f64::MAX } else { 1.0 / dy.abs() };
        let mut tmx = if dx.abs() < 1e-12 {
            f64::MAX
        } else if dx > 0.0 {
            (cx as f64 + 1.0 - ax) * tdx
        } else {
            (ax - cx as f64) * tdx
        };
        let mut tmy = if dy.abs() < 1e-12 {
            f64::MAX
        } else if dy > 0.0 {
            (cy as f64 + 1.0 - ay) * tdy
        } else {
            (ay - cy as f64) * tdy
        };
        let max_it = ((ex - cx).abs() + (ey - cy).abs() + 2) as usize;
        for _ in 0..max_it {
            if cx >= 0 && cy >= 0 && (cx as usize) < self.gw && (cy as usize) < self.gh {
                out.push(cy as usize * self.gw + cx as usize);
            }
            if cx == ex && cy == ey {
                break;
            }
            if tmx < tmy {
                cx += sx;
                tmx += tdx;
            } else {
                cy += sy;
                tmy += tdy;
            }
        }
    }

    /// Nearest prism hit along a->b (t in [0,1]).
    pub fn ray_prisms(&self, a: [f64; 3], b: [f64; 3]) -> Option<f64> {
        let mut cells = Vec::with_capacity(32);
        self.cells_along(a, b, &mut cells, false);
        let mut seen: Vec<u32> = Vec::with_capacity(16);
        let mut best: Option<f64> = None;
        for c in cells {
            for &pi in &self.pcells[c] {
                if seen.contains(&pi) {
                    continue;
                }
                seen.push(pi);
                if let Some(t) = self.prisms[pi as usize].seg_hit(a, b) {
                    if best.map_or(true, |bt| t < bt) {
                        best = Some(t);
                    }
                }
            }
        }
        best
    }

    pub fn ray_terrain(&self, a: [f64; 3], b: [f64; 3]) -> Option<f64> {
        if a[2].min(b[2]) > self.max_ground {
            return None;
        }
        let hl = ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        let n = ((hl / 1.5).ceil() as usize).max(1);
        let f = |t: f64| -> f64 {
            let (x, y, z) = (a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t);
            z - self.ground(x, y)
        };
        let mut prev_t = 0.0;
        if f(0.0) < 0.0 {
            return Some(0.0);
        }
        for i in 1..=n {
            let t = i as f64 / n as f64;
            if f(t) < 0.0 {
                let (mut lo, mut hi) = (prev_t, t);
                for _ in 0..8 {
                    let m = 0.5 * (lo + hi);
                    if f(m) < 0.0 {
                        hi = m;
                    } else {
                        lo = m;
                    }
                }
                return Some(hi);
            }
            prev_t = t;
        }
        None
    }

    /// First solid (prism or terrain) hit.
    pub fn raycast(&self, a: [f64; 3], b: [f64; 3]) -> Option<f64> {
        let p = self.ray_prisms(a, b);
        let t = self.ray_terrain(a, b);
        match (p, t) {
            (Some(x), Some(y)) => Some(x.min(y)),
            (x, None) => x,
            (None, y) => y,
        }
    }

    /// Line of sight. `foliage`: also apply trunk blocking and canopy attenuation.
    pub fn los(&self, a: [f64; 3], b: [f64; 3], foliage: bool) -> bool {
        if self.raycast(a, b).is_some() {
            return false;
        }
        if foliage {
            return self.foliage_clear(a, b);
        }
        true
    }

    fn foliage_clear(&self, a: [f64; 3], b: [f64; 3]) -> bool {
        let mut cells = Vec::with_capacity(32);
        self.cells_along(a, b, &mut cells, true);
        let mut seen: Vec<u32> = Vec::with_capacity(16);
        let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        let mut chord = 0.0;
        for c in cells {
            for &ti in &self.tcells[c] {
                if seen.contains(&ti) {
                    continue;
                }
                seen.push(ti);
                let t = &self.trees[ti as usize];
                // trunk: vertical cylinder radius tr from z to z+cb (or h)
                let top = t.z + if t.cb > 0.5 { t.cb } else { t.h };
                if t.tr > 0.0 && seg_vcyl(a, b, t.x, t.y, t.tr, t.z, top).is_some() {
                    return false;
                }
                // crown: ellipsoid
                if t.cr > 0.0 && t.h > t.cb {
                    let cz = t.z + 0.5 * (t.cb + t.h);
                    let rz = 0.5 * (t.h - t.cb);
                    // scale to unit sphere
                    let o = [(a[0] - t.cx) / t.cr, (a[1] - t.cy) / t.cr, (a[2] - cz) / rz];
                    let dd = [d[0] / t.cr, d[1] / t.cr, d[2] / rz];
                    let qa = dd[0] * dd[0] + dd[1] * dd[1] + dd[2] * dd[2];
                    let qb = 2.0 * (o[0] * dd[0] + o[1] * dd[1] + o[2] * dd[2]);
                    let qc = o[0] * o[0] + o[1] * o[1] + o[2] * o[2] - 1.0;
                    let disc = qb * qb - 4.0 * qa * qc;
                    if disc > 0.0 && qa > 1e-12 {
                        let s = disc.sqrt();
                        let t0 = ((-qb - s) / (2.0 * qa)).clamp(0.0, 1.0);
                        let t1 = ((-qb + s) / (2.0 * qa)).clamp(0.0, 1.0);
                        chord += (t1 - t0) * len;
                    }
                }
            }
        }
        if chord <= 0.0 {
            return true;
        }
        (-self.canopy_density * chord).exp() >= self.canopy_block_t
    }

    /// Highest surface supporting a point at (x,y) whose top is reachable from `feet`.
    pub fn support(&self, x: f64, y: f64, feet: f64, step_up: f64) -> f64 {
        let mut s = self.ground(x, y);
        let (cx, cy) = self.cell_of(x, y);
        for &pi in &self.pcells[cy * self.gw + cx] {
            let p = &self.prisms[pi as usize];
            if p.top <= feet + step_up + EPS && p.top > s && p.contains_xy(x, y) {
                s = p.top;
            }
        }
        s
    }

    fn near_prisms<F: FnMut(&Prism)>(&self, x: f64, y: f64, r: f64, mut f: F) {
        let (c0x, c0y) = self.cell_of(x - r, y - r);
        let (c1x, c1y) = self.cell_of(x + r, y + r);
        for cy in c0y..=c1y {
            for cx in c0x..=c1x {
                for &pi in &self.pcells[cy * self.gw + cx] {
                    // avoid double counting across cells: only process in the cell containing max(bb min, query min)
                    let p = &self.prisms[pi as usize];
                    let (ox, oy) = self.cell_of(p.bb[0].max(x - r), p.bb[1].max(y - r));
                    if ox == cx && oy == cy {
                        f(p);
                    }
                }
            }
        }
    }

    /// Does a cylinder (feet at z, radius r, height h) overlap a wall prism (can't be stepped onto / is in the head space)?
    pub fn hits_wall(&self, x: f64, y: f64, feet: f64, r: f64, h: f64, step_up: f64) -> bool {
        let mut hit = false;
        self.near_prisms(x, y, r, |p| {
            if hit {
                return;
            }
            if p.top > feet + step_up + EPS && p.base < feet + h && p.circle_overlaps(x, y, r) {
                hit = true;
            }
        });
        hit
    }

    /// Highest prism top below `feet_old + eps` overlapping the circle (for edge landing) with a flag whether center is inside.
    pub fn landing_surface(&self, x: f64, y: f64, r: f64, feet_old: f64, feet_new: f64) -> Option<(f64, bool, [f64; 2])> {
        let mut best: Option<(f64, bool, [f64; 2])> = None;
        self.near_prisms(x, y, r, |p| {
            if p.top <= feet_old + 0.05 && p.top >= feet_new - 1e-6 && p.circle_overlaps(x, y, r) {
                let inside = p.contains_xy(x, y);
                let push = if inside {
                    [0.0, 0.0]
                } else {
                    let (q, _) = closest_on_poly(&p.ring, x, y);
                    let (dx, dy) = (x - q[0], y - q[1]);
                    let l = (dx * dx + dy * dy).sqrt().max(1e-9);
                    [dx / l, dy / l]
                };
                if best.map_or(true, |b| p.top > b.0) {
                    best = Some((p.top, inside, push));
                }
            }
        });
        best
    }

    /// Lowest ceiling (prism base) above head within the footprint, between `from` and `to` heights.
    pub fn ceiling(&self, x: f64, y: f64, r: f64, head_old: f64, head_new: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        self.near_prisms(x, y, r, |p| {
            if p.base >= head_old - 0.05 && p.base < head_new && p.circle_overlaps(x, y, r) {
                if best.map_or(true, |b| p.base < b) {
                    best = Some(p.base);
                }
            }
        });
        best
    }

    /// Is (x,y) free open ground (no prism footprint within `r`)?
    pub fn open_ground(&self, x: f64, y: f64, r: f64) -> bool {
        let mut free = true;
        self.near_prisms(x, y, r, |p| {
            if p.circle_overlaps(x, y, r) {
                free = false;
            }
        });
        free
    }

    pub fn slope_at(&self, x: f64, y: f64) -> f64 {
        let gx = (self.ground(x + 1.0, y) - self.ground(x - 1.0, y)) * 0.5;
        let gy = (self.ground(x, y + 1.0) - self.ground(x, y - 1.0)) * 0.5;
        (gx * gx + gy * gy).sqrt().atan()
    }

    /// Highest roof top covering (x,y) among building prisms (None if none).
    pub fn roof_top(&self, x: f64, y: f64) -> Option<(f64, usize)> {
        let (cx, cy) = self.cell_of(x, y);
        let mut best: Option<(f64, usize)> = None;
        for &pi in &self.pcells[cy * self.gw + cx] {
            let p = &self.prisms[pi as usize];
            if p.contains_xy(x, y) && best.map_or(true, |b| p.top > b.0) {
                best = Some((p.top, pi as usize));
            }
        }
        best
    }
}

/// Segment vs vertical cylinder; returns entry t.
pub fn seg_vcyl(a: [f64; 3], b: [f64; 3], cx: f64, cy: f64, r: f64, z0: f64, z1: f64) -> Option<f64> {
    let (dx, dy, dz) = (b[0] - a[0], b[1] - a[1], b[2] - a[2]);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    if dz.abs() < 1e-12 {
        if a[2] < z0 || a[2] > z1 {
            return None;
        }
    } else {
        let ta = (z0 - a[2]) / dz;
        let tb = (z1 - a[2]) / dz;
        t0 = t0.max(ta.min(tb));
        t1 = t1.min(ta.max(tb));
        if t0 > t1 {
            return None;
        }
    }
    let (ox, oy) = (a[0] - cx, a[1] - cy);
    let qa = dx * dx + dy * dy;
    let qc = ox * ox + oy * oy - r * r;
    if qa < 1e-12 {
        return if qc <= 0.0 { Some(t0) } else { None };
    }
    let qb = 2.0 * (ox * dx + oy * dy);
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let u0 = (-qb - s) / (2.0 * qa);
    let u1 = (-qb + s) / (2.0 * qa);
    let lo = t0.max(u0);
    let hi = t1.min(u1);
    if lo <= hi {
        Some(lo)
    } else {
        None
    }
}
