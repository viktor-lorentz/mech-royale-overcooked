//! Coarse walkability grid + A* so the bot routes around buildings instead of grinding into them.
use crate::protocol::V3;
use crate::world::World;
use std::cmp::Ordering;
use std::collections::BinaryHeap;

pub const CELL: f64 = 3.0;

pub struct Nav {
    pub w: usize,
    pub h: usize,
    pub min: [f64; 2],
    blocked: Vec<bool>,
    near: Vec<bool>, // adjacent to an obstacle (soft cost)
}

#[derive(Copy, Clone, PartialEq)]
struct Node {
    f: f64,
    i: usize,
}
impl Eq for Node {}
impl Ord for Node {
    fn cmp(&self, o: &Self) -> Ordering {
        o.f.partial_cmp(&self.f).unwrap_or(Ordering::Equal)
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl Nav {
    pub fn build(world: &World) -> Nav {
        let b = world.bounds;
        let min = [b[0] + 8.0, b[1] + 8.0];
        let w = ((b[2] - b[0] - 16.0) / CELL) as usize;
        let h = ((b[3] - b[1] - 16.0) / CELL) as usize;
        let mut blocked = vec![false; w * h];
        const R: f64 = 2.7;
        let offs: [[f64; 2]; 9] = {
            let d = R * 0.7071;
            [[0.0, 0.0], [R, 0.0], [-R, 0.0], [0.0, R], [0.0, -R], [d, d], [-d, d], [d, -d], [-d, -d]]
        };
        for j in 0..h {
            for i in 0..w {
                let x = min[0] + (i as f64 + 0.5) * CELL;
                let y = min[1] + (j as f64 + 0.5) * CELL;
                let g = world.ground(x, y);
                let mut bad = false;
                // slope
                let s = 1.5;
                let dx = (world.ground(x + s, y) - world.ground(x - s, y)).abs() / (2.0 * s);
                let dy = (world.ground(x, y + s) - world.ground(x, y - s)).abs() / (2.0 * s);
                if dx.max(dy) > 1.0 {
                    bad = true;
                }
                if !bad {
                    'o: for o in offs.iter() {
                        let gg = world.ground(x + o[0], y + o[1]);
                        if (gg - g).abs() > 2.2 {
                            bad = true;
                            break 'o;
                        }
                        for z in [1.5, 4.5, 8.0] {
                            if world.solid(x + o[0], y + o[1], gg + z) {
                                bad = true;
                                break 'o;
                            }
                        }
                    }
                }
                blocked[j * w + i] = bad;
            }
        }
        let mut near = vec![false; w * h];
        for j in 1..h.saturating_sub(1) {
            for i in 1..w.saturating_sub(1) {
                if blocked[j * w + i] {
                    continue;
                }
                'n: for dj in -1i32..=1 {
                    for di in -1i32..=1 {
                        if blocked[((j as i32 + dj) as usize) * w + (i as i32 + di) as usize] {
                            near[j * w + i] = true;
                            break 'n;
                        }
                    }
                }
            }
        }
        Nav { w, h, min, blocked, near }
    }

    pub fn cell_of(&self, x: f64, y: f64) -> Option<usize> {
        let i = ((x - self.min[0]) / CELL).floor();
        let j = ((y - self.min[1]) / CELL).floor();
        if i < 0.0 || j < 0.0 || i as usize >= self.w || j as usize >= self.h {
            return None;
        }
        Some(j as usize * self.w + i as usize)
    }
    pub fn center(&self, c: usize) -> [f64; 2] {
        [self.min[0] + ((c % self.w) as f64 + 0.5) * CELL, self.min[1] + ((c / self.w) as f64 + 0.5) * CELL]
    }
    pub fn is_blocked(&self, x: f64, y: f64) -> bool {
        self.cell_of(x, y).map(|c| self.blocked[c]).unwrap_or(true)
    }

    /// Nearest unblocked cell to a point (spiral search).
    pub fn nearest_free(&self, x: f64, y: f64) -> Option<usize> {
        let c0 = self.cell_of(x.clamp(self.min[0] + 1.0, self.min[0] + self.w as f64 * CELL - 1.0), y.clamp(self.min[1] + 1.0, self.min[1] + self.h as f64 * CELL - 1.0))?;
        if !self.blocked[c0] {
            return Some(c0);
        }
        let (ci, cj) = ((c0 % self.w) as i32, (c0 / self.w) as i32);
        for r in 1..40i32 {
            let mut best: Option<(i32, usize)> = None;
            for dj in -r..=r {
                for di in -r..=r {
                    if dj.abs().max(di.abs()) != r {
                        continue;
                    }
                    let (i, j) = (ci + di, cj + dj);
                    if i < 0 || j < 0 || i >= self.w as i32 || j >= self.h as i32 {
                        continue;
                    }
                    let c = j as usize * self.w + i as usize;
                    if !self.blocked[c] {
                        let d = di * di + dj * dj;
                        if best.map(|b| d < b.0).unwrap_or(true) {
                            best = Some((d, c));
                        }
                    }
                }
            }
            if let Some((_, c)) = best {
                return Some(c);
            }
        }
        None
    }

    /// A* from `from` to `to` (world xy). Returns waypoints (cell centers), excluding the start cell.
    pub fn path(&self, from: V3, to: [f64; 2], max_expand: usize) -> Option<Vec<[f64; 2]>> {
        let s = self.nearest_free(from[0], from[1])?;
        let g = self.nearest_free(to[0], to[1])?;
        if s == g {
            return Some(vec![self.center(g)]);
        }
        let n = self.w * self.h;
        let mut gs = vec![f64::MAX; n];
        let mut came = vec![usize::MAX; n];
        let mut open = BinaryHeap::new();
        let gc = self.center(g);
        let hfn = |c: usize| {
            let p = self.center(c);
            ((p[0] - gc[0]).powi(2) + (p[1] - gc[1]).powi(2)).sqrt()
        };
        gs[s] = 0.0;
        open.push(Node { f: hfn(s), i: s });
        let mut exp = 0;
        let mut found = false;
        let mut visited: Vec<usize> = vec![s];
        while let Some(Node { i, .. }) = open.pop() {
            if i == g {
                found = true;
                break;
            }
            exp += 1;
            if exp > max_expand {
                break;
            }
            let (ci, cj) = ((i % self.w) as i32, (i / self.w) as i32);
            for dj in -1i32..=1 {
                for di in -1i32..=1 {
                    if di == 0 && dj == 0 {
                        continue;
                    }
                    let (ni, nj) = (ci + di, cj + dj);
                    if ni < 0 || nj < 0 || ni >= self.w as i32 || nj >= self.h as i32 {
                        continue;
                    }
                    let nc = nj as usize * self.w + ni as usize;
                    if self.blocked[nc] {
                        continue;
                    }
                    if di != 0 && dj != 0 {
                        // no corner cutting
                        if self.blocked[cj as usize * self.w + ni as usize] || self.blocked[nj as usize * self.w + ci as usize] {
                            continue;
                        }
                    }
                    let step = if di != 0 && dj != 0 { 1.414 * CELL } else { CELL };
                    let cost = gs[i] + step + if self.near[nc] { 2.5 } else { 0.0 };
                    if cost < gs[nc] {
                        if gs[nc] == f64::MAX {
                            visited.push(nc);
                        }
                        gs[nc] = cost;
                        came[nc] = i;
                        open.push(Node { f: cost + hfn(nc), i: nc });
                    }
                }
            }
        }
        let end = if found {
            g
        } else {
            // partial: best explored cell closest to goal
            let mut best = s;
            let mut bd = f64::MAX;
            for &c in &visited {
                {
                    let d = hfn(c);
                    if d < bd {
                        bd = d;
                        best = c;
                    }
                }
            }
            best
        };
        let mut out = Vec::new();
        let mut c = end;
        while c != s && c != usize::MAX {
            out.push(self.center(c));
            c = came[c];
        }
        out.reverse();
        if out.is_empty() {
            None
        } else {
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn path_across_campus() {
        let w = World::load(&format!("{}/kit/arena", env!("CARGO_MANIFEST_DIR"))).unwrap();
        let t = std::time::Instant::now();
        let nav = Nav::build(&w);
        eprintln!("nav build {:?} ({}x{})", t.elapsed(), nav.w, nav.h);
        let free = (0..nav.w * nav.h).filter(|&c| !nav.blocked[c]).count();
        eprintln!("free cells {}/{}", free, nav.w * nav.h);
        let a = nav.center(nav.nearest_free(-217.0, -22.0).unwrap());
        let t = std::time::Instant::now();
        let p = nav.path([a[0], a[1], 0.0], [41.0, -72.0], 60000).expect("path");
        eprintln!("path len {} in {:?}", p.len(), t.elapsed());
        assert!(p.len() > 10);
    }
}
