//! What still holds together, and what holds it up (`WORLD_PLAN.md`, phase G): the arithmetic of
//! support, apart from the world that asks it (`live::falls`).
//!
//! - **Cells**: an element's solid, sampled at the centres of a grid in its own frame, about
//!   `CELLS` along its longest side, solid where the surface is within a quarter of a cell.
//! - **Parts**: the grid's solid cells joined face to face (`Grid::parts`), numbered from the
//!   lowest cell up, so every machine numbers them alike.
//! - **A plane between**: a part comes away as a new piece cut along a plane that has it on one side
//!   and the rest on the other (`separating_plane`), so a piece is still its element with a clip.
//!   A part no plane can part from the rest (interlocking) is left joined.
//! - **Footing**: whether a centre of mass stands over the points that hold it (`overhang`), and if
//!   not, which edge it tips over, asked along sixteen directions. A cross-section holds what is
//!   above it while the mass overhangs it by at most `SECTION_GRIP` of the section's width that way
//!   (wood holds a little in tension); a body resting on its contacts has no such grip.
//!
//! Only IEEE arithmetic (`+ - * /`, `sqrt`) and constants: the same bits on every machine.

use crate::cut::ClipPlane;

/// Cells along an element's longest side.
pub const CELLS: usize = 48;
/// Most cells in a grid, whatever its shape.
pub const MAX_CELLS: usize = 48 * 48 * 48;
/// A cross-section holds what is above it while the mass overhangs it by at most this share of the
/// section's width in that direction: a notch half through holds, one about three-fifths through
/// does not.
pub const SECTION_GRIP: f32 = 0.25;

/// Sixteen directions round the horizontal, as (x, z), from +x towards +z.
pub const DIRECTIONS: [[f32; 2]; 16] = [
    [1.0, 0.0],
    [0.923_879_5, 0.382_683_43],
    [0.707_106_77, 0.707_106_77],
    [0.382_683_43, 0.923_879_5],
    [0.0, 1.0],
    [-0.382_683_43, 0.923_879_5],
    [-0.707_106_77, 0.707_106_77],
    [-0.923_879_5, 0.382_683_43],
    [-1.0, 0.0],
    [-0.923_879_5, -0.382_683_43],
    [-0.707_106_77, -0.707_106_77],
    [-0.382_683_43, -0.923_879_5],
    [0.0, -1.0],
    [0.382_683_43, -0.923_879_5],
    [0.707_106_77, -0.707_106_77],
    [0.923_879_5, -0.382_683_43],
];

/// A grid of cells over an element's local box.
#[derive(Clone, Debug, PartialEq)]
pub struct Grid {
    /// The corner of the first cell, in the element's frame.
    pub lo: [f32; 3],
    /// A cell's side, in the element's frame.
    pub cell: f32,
    pub dims: [usize; 3],
    pub solid: Vec<bool>,
}

impl Grid {
    /// A grid over `lo..hi` (grown by a cell each way), `cells` along its longest side, all empty;
    /// `None` if the box is empty or not finite.
    pub fn over(lo: [f32; 3], hi: [f32; 3], cells: usize) -> Option<Grid> {
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let longest = ext[0].max(ext[1]).max(ext[2]);
        if !(longest.is_finite() && longest > 1e-6) || ext.iter().any(|e| !(e.is_finite() && *e >= 0.0)) {
            return None;
        }
        let mut cell = longest / cells.max(2) as f32;
        let mut dims = [0usize; 3];
        loop {
            for a in 0..3 {
                dims[a] = ((ext[a] / cell) as usize + 3).max(3);
            }
            if dims[0] * dims[1] * dims[2] <= MAX_CELLS {
                break;
            }
            cell *= 1.25;
        }
        let lo = [lo[0] - cell, lo[1] - cell, lo[2] - cell];
        Some(Grid { lo, cell, dims, solid: vec![false; dims[0] * dims[1] * dims[2]] })
    }

    pub fn len(&self) -> usize {
        self.solid.len()
    }

    pub fn is_empty(&self) -> bool {
        self.solid.is_empty()
    }

    pub fn index(&self, c: [usize; 3]) -> usize {
        (c[2] * self.dims[1] + c[1]) * self.dims[0] + c[0]
    }

    pub fn coords(&self, i: usize) -> [usize; 3] {
        let x = i % self.dims[0];
        let y = (i / self.dims[0]) % self.dims[1];
        let z = i / (self.dims[0] * self.dims[1]);
        [x, y, z]
    }

    /// The centre of cell `i`, in the element's frame.
    pub fn centre(&self, i: usize) -> [f32; 3] {
        let c = self.coords(i);
        [0, 1, 2].map(|a| self.lo[a] + (c[a] as f32 + 0.5) * self.cell)
    }

    /// The solid cells joined face to face: each cell's part (0 for empty, else from 1), and how
    /// many parts. Parts are numbered in the order their first cell comes.
    pub fn parts(&self) -> (Vec<u32>, u32) {
        let mut label = vec![0u32; self.solid.len()];
        let mut count = 0u32;
        let mut stack = Vec::new();
        for start in 0..self.solid.len() {
            if !self.solid[start] || label[start] != 0 {
                continue;
            }
            count += 1;
            label[start] = count;
            stack.push(start);
            while let Some(i) = stack.pop() {
                let c = self.coords(i);
                for (a, step) in [(0usize, -1isize), (0, 1), (1, -1), (1, 1), (2, -1), (2, 1)] {
                    let n = c[a] as isize + step;
                    if n < 0 || n as usize >= self.dims[a] {
                        continue;
                    }
                    let mut nc = c;
                    nc[a] = n as usize;
                    let j = self.index(nc);
                    if self.solid[j] && label[j] == 0 {
                        label[j] = count;
                        stack.push(j);
                    }
                }
            }
        }
        (label, count)
    }
}

/// Directions a plane between two parts is looked for along, in the element's frame: its three
/// axes, the twelve between pairs of them, and `extra` (the world's up, in that frame).
fn plane_normals(extra: [f32; 3]) -> Vec<[f32; 3]> {
    const H: f32 = std::f32::consts::FRAC_1_SQRT_2;
    let mut ns = vec![[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]];
    for (a, b) in [(0, 1), (1, 2), (0, 2)] {
        for s in [1.0, -1.0] {
            let mut n = [0.0; 3];
            n[a] = H;
            n[b] = H * s;
            ns.push(n);
        }
    }
    let l = (extra[0] * extra[0] + extra[1] * extra[1] + extra[2] * extra[2]).sqrt();
    if l > 1e-6 {
        ns.insert(0, extra.map(|c| c / l));
    }
    ns
}

/// A plane, in the element's frame, with every cell of `part` on its kept side and every other solid
/// cell on the other, half way across the gap between them: level in the world (`up`, the world's
/// up in the element's frame) if a level plane parts them, so what is not solid (leaves) goes with
/// the part above; else along whichever direction looked at leaves the widest gap. `None` if none
/// parts them.
pub fn separating_plane(grid: &Grid, labels: &[u32], part: u32, up: [f32; 3]) -> Option<ClipPlane> {
    let mut best: Option<(f32, ClipPlane)> = None;
    let half = 0.5 * grid.cell;
    for (k, n) in plane_normals(up).into_iter().enumerate() {
        if k == 1 && best.is_some() {
            break;
        }
        // How far a cell reaches along `n` from its centre.
        let reach = half * (n[0].abs() + n[1].abs() + n[2].abs());
        let (mut part_lo, mut part_hi, mut rest_lo, mut rest_hi) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY, f32::NEG_INFINITY);
        for (i, &l) in labels.iter().enumerate() {
            if l == 0 {
                continue;
            }
            let c = grid.centre(i);
            let d = n[0] * c[0] + n[1] * c[1] + n[2] * c[2];
            if l == part {
                part_lo = part_lo.min(d);
                part_hi = part_hi.max(d);
            } else {
                rest_lo = rest_lo.min(d);
                rest_hi = rest_hi.max(d);
            }
        }
        if !part_lo.is_finite() || !rest_lo.is_finite() {
            continue;
        }
        // The part above the rest along `n`, or below it.
        for (gap, plane) in [
            ((part_lo - reach) - (rest_hi + reach), ClipPlane::new(n, 0.5 * (part_lo + rest_hi))),
            ((rest_lo - reach) - (part_hi + reach), ClipPlane::new(n, 0.5 * (rest_lo + part_hi)).flipped()),
        ] {
            if gap >= 0.0 && best.as_ref().is_none_or(|(g, _)| gap > *g) {
                best = Some((gap, plane));
            }
        }
    }
    best.map(|(_, p)| p)
}

/// How a mass at `com` stands over the points `points` that hold it, all seen from above as (x, z):
/// `None` if it stands; else the direction it tips (from the sixteen) and which of `points` it tips
/// over (the farthest that way). `grip` is how far it may overhang, as a share of the points' width
/// that way. Nothing to stand on, it tips (over nothing).
pub fn overhang(points: &[[f32; 2]], com: [f32; 2], grip: f32) -> Option<([f32; 2], usize)> {
    if points.is_empty() {
        return Some(([1.0, 0.0], 0));
    }
    let mut worst: Option<(f32, [f32; 2], usize)> = None;
    for u in DIRECTIONS {
        let (mut hi, mut lo, mut at) = (f32::NEG_INFINITY, f32::INFINITY, 0);
        for (i, p) in points.iter().enumerate() {
            let d = (p[0] - com[0]) * u[0] + (p[1] - com[1]) * u[1];
            if d > hi {
                hi = d;
                at = i;
            }
            lo = lo.min(d);
        }
        let width = (hi - lo).max(0.0);
        // Beyond its farthest point that way, by more than it may.
        let over = -hi - grip * width;
        if over > 0.0 && worst.as_ref().is_none_or(|(w, _, _)| over > *w) {
            worst = Some((over, u, at));
        }
    }
    worst.map(|(_, u, at)| (u, at))
}

/// The mean of `points`.
pub fn mean(points: &[[f32; 3]]) -> Option<[f32; 3]> {
    if points.is_empty() {
        return None;
    }
    let mut s = [0.0f64; 3];
    for p in points {
        for a in 0..3 {
            s[a] += p[a] as f64;
        }
    }
    let n = points.len() as f64;
    Some(s.map(|v| (v / n) as f32))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parts_and_the_plane_between_them() {
        let mut g = Grid::over([0.0; 3], [1.0, 4.0, 1.0], 16).unwrap();
        // A post with a gap a cell high half way up.
        for i in 0..g.len() {
            let c = g.centre(i);
            let in_post = (0.25..0.75).contains(&c[0]) && (0.25..0.75).contains(&c[2]) && (0.0..4.0).contains(&c[1]);
            g.solid[i] = in_post && !(1.9..2.2).contains(&c[1]);
        }
        let (labels, n) = g.parts();
        assert_eq!(n, 2);
        let top = labels[(0..g.len()).find(|&i| g.solid[i] && g.centre(i)[1] > 3.0).unwrap()];
        let plane = separating_plane(&g, &labels, top, [0.0, 1.0, 0.0]).unwrap();
        assert_eq!(plane.normal, [0.0, 1.0, 0.0]);
        assert!((1.9..2.2).contains(&plane.offset), "{plane:?}");
    }

    #[test]
    fn a_mass_overhanging_tips_over_the_far_edge() {
        let disc: Vec<[f32; 2]> = (0..64).map(|k| k as f32 / 64.0 * std::f32::consts::TAU).map(|a| [crate::math::cos(a), crate::math::sin(a)]).collect();
        assert!(overhang(&disc, [0.0, 0.0], 0.0).is_none(), "over its middle it stands");
        let (u, at) = overhang(&disc, [1.3, 0.0], 0.0).expect("beyond its edge it tips");
        assert_eq!(u, [1.0, 0.0]);
        assert_eq!(at, 0, "over the point farthest east");
        // Half a disc, the mass over its straight edge: held with grip, not without.
        let half: Vec<[f32; 2]> = disc.iter().copied().filter(|p| p[0] <= 0.0).collect();
        assert!(overhang(&half, [0.05, 0.0], SECTION_GRIP).is_none());
        let third: Vec<[f32; 2]> = disc.iter().copied().filter(|p| p[0] <= -0.5).collect();
        assert!(overhang(&third, [0.0, 0.0], SECTION_GRIP).is_some());
    }
}
