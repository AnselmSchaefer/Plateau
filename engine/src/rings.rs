//! Which chunks a viewer sees (POC_PLAN.md, step 2): the finest about them, coarser further off;
//! and, while some are still being made, which to show instead.
//!
//! Each level is a block of four chunks a side about the viewer, its middle on the next level's
//! grid, so it is two of the next level's chunks a side and lies within the next level's block.
//! Each level leaves out the part of its block the level below fills, so every point near the
//! viewer is in exactly one chunk, and the levels nest without gaps. The finest level's voxel is
//! the power of two at or below a twentieth of the viewer's height; the coarsest (`COARSEST`, a
//! metre) has a block that holds the whole area from anywhere in it.
//!
//! **Blocks lag.** A level's block follows the viewer only once they are `LAG` past the middle
//! between two of its places (`Rings`), so walking to and fro about that middle does not swap its
//! chunks to and fro. A lag under a sixth keeps every level within the next.
//!
//! **What is shown never overlaps** (`shown`): a chunk wanted but not yet made is stood in for by
//! the coarser chunk about it, if that is made, or by the finer chunks within it, if those are;
//! never by both, and never with the chunks it is replacing still shown beside it.

use std::collections::{HashMap, HashSet};

use crate::WIDTH;
use crate::chunk::{CELLS, Key, VOXELS_A_HEIGHT};

/// The coarsest level: voxels of a metre, chunks of 32 m, a block of 128 m about the viewer.
pub const COARSEST: i32 = 0;
/// The finest: voxels of a quarter of a millimetre, for the smallest viewer.
pub const FINEST: i32 = -12;
/// How far past the middle between two places of a level's block the viewer goes before it
/// follows them, as a fraction of the way between the places.
pub const LAG: f64 = 0.15;
/// How many levels finer a chunk not yet made may be stood in for by the chunks within it.
const FINER_AT_MOST: u32 = 3;

/// The finest level a viewer `size` tall needs: the power of two at or below a twentieth of it
/// (a giant's half a metre, a person's 6 cm, the tiny's half a millimetre).
pub fn finest(size: f32) -> i32 {
    ((size / VOXELS_A_HEIGHT).log2().floor() as i32).clamp(FINEST, COARSEST)
}

/// The blocks about a viewer, remembered from one moment to the next so they can lag.
#[derive(Clone, Debug, Default)]
pub struct Rings {
    /// Each level's block, as its middle on the next level's grid.
    middles: HashMap<i32, [i32; 3]>,
}

impl Rings {
    /// Every chunk a viewer with their eye at `eye`, `size` tall, sees, finest first: only chunks
    /// over the area (x and z from 0 to `WIDTH`).
    pub fn wanted(&mut self, eye: [f64; 3], size: f32) -> Vec<Key> {
        let finest = finest(size);
        // From the coarsest down, each block lagging, and kept within the one about it.
        let mut middles = Vec::new();
        let mut outer: Option<[i32; 3]> = None;
        for level in (finest..=COARSEST).rev() {
            let between = 2.0 * 2f64.powi(level) * CELLS as f64;
            let exact = eye.map(|c| c / between);
            let mut middle = match self.middles.get(&level) {
                Some(&m) => [0, 1, 2].map(|a| if (exact[a] - m[a] as f64).abs() > 0.5 + LAG { exact[a].round() as i32 } else { m[a] }),
                None => exact.map(|c| c.round() as i32),
            };
            if let Some(o) = outer {
                middle = [0, 1, 2].map(|a| middle[a].clamp(2 * o[a] - 1, 2 * o[a] + 1));
            }
            self.middles.insert(level, middle);
            middles.push((level, middle));
            outer = Some(middle);
        }
        let mut keys = Vec::new();
        // The block of the level below, as the middle of its two chunks a side on this level's grid.
        let mut inner: Option<[i32; 3]> = None;
        for &(level, middle) in middles.iter().rev() {
            for x in 2 * middle[0] - 2..2 * middle[0] + 2 {
                for y in 2 * middle[1] - 2..2 * middle[1] + 2 {
                    for z in 2 * middle[2] - 2..2 * middle[2] + 2 {
                        let at = [x, y, z];
                        if let Some(m) = inner
                            && (0..3).all(|a| at[a] >= m[a] - 1 && at[a] < m[a] + 1)
                        {
                            continue;
                        }
                        let key = Key { level, at };
                        if over(key) {
                            keys.push(key);
                        }
                    }
                }
            }
            inner = Some(middle);
        }
        keys
    }

    /// The space the chunks last wanted fill: the coarsest block, as its lowest and highest
    /// corners (x and z cut to the area).
    pub fn reach(&self) -> Option<([f64; 3], [f64; 3])> {
        let m = self.middles.get(&COARSEST)?;
        let side = 2f64.powi(COARSEST) * CELLS as f64;
        let (mut lo, mut hi) = (m.map(|c| (2 * c - 2) as f64 * side), m.map(|c| (2 * c + 2) as f64 * side));
        for a in [0, 2] {
            lo[a] = lo[a].max(0.0);
            hi[a] = hi[a].min(WIDTH as f64);
        }
        Some((lo, hi))
    }
}

/// Every chunk a viewer at `eye`, `size` tall, sees, with no blocks remembered to lag.
pub fn wanted(eye: [f64; 3], size: f32) -> Vec<Key> {
    Rings::default().wanted(eye, size)
}

/// Whether a chunk lies over the area, across the ground.
pub fn over(key: Key) -> bool {
    let side = key.side();
    let over = |i: i32| (i as f64 + 1.0) * side > 0.0 && (i as f64) * side < WIDTH as f64;
    over(key.at[0]) && over(key.at[2])
}

/// What to show of the chunks `wanted`, when only those `made` says are made (with a surface or
/// without): chunks that never overlap, filling as much of the wanted space as can be, each
/// wanted chunk not yet made stood in for by the coarser chunk about it or the finer ones within
/// it. With how many wanted chunks have nothing shown in their space.
pub fn shown(wanted: &[Key], made: &dyn Fn(Key) -> bool) -> (Vec<Key>, usize) {
    let wanted_set: HashSet<Key> = wanted.iter().copied().collect();
    // Every chunk about a wanted one, up to the coarsest.
    let mut about: HashSet<Key> = HashSet::new();
    for k in wanted {
        let mut p = *k;
        while p.level < COARSEST {
            p = p.parent();
            if !about.insert(p) {
                break;
            }
        }
    }
    let roots: HashSet<Key> = wanted_set.iter().chain(&about).filter(|k| k.level == COARSEST).copied().collect();
    let look = Look { wanted: &wanted_set, about: &about, made };
    let (mut shown, mut holes) = (Vec::new(), 0);
    for root in roots {
        let (keys, h) = look.show(root);
        shown.extend(keys);
        holes += h;
    }
    (shown, holes)
}

struct Look<'a> {
    wanted: &'a HashSet<Key>,
    about: &'a HashSet<Key>,
    made: &'a dyn Fn(Key) -> bool,
}

impl Look<'_> {
    /// What to show in `key`'s space, and how many wanted chunks there have nothing shown.
    fn show(&self, key: Key) -> (Vec<Key>, usize) {
        if !over(key) {
            return (Vec::new(), 0);
        }
        if self.wanted.contains(&key) {
            return match self.finer(key, 0) {
                Some(keys) => (keys, 0),
                None => (Vec::new(), 1),
            };
        }
        if !self.about.contains(&key) {
            return (Vec::new(), 0);
        }
        let (mut keys, mut holes) = (Vec::new(), 0);
        for child in key.children() {
            let (k, h) = self.show(child);
            keys.extend(k);
            holes += h;
        }
        // Not all of it made yet: the coarser chunk instead, if that is.
        if holes > 0 && (self.made)(key) {
            return (vec![key], 0);
        }
        (keys, holes)
    }

    /// `key` if it is made, or else the finer chunks made within it, a few levels down at most.
    fn finer(&self, key: Key, depth: u32) -> Option<Vec<Key>> {
        if !over(key) {
            return Some(Vec::new());
        }
        if (self.made)(key) {
            return Some(vec![key]);
        }
        if depth == FINER_AT_MOST {
            return None;
        }
        let mut keys = Vec::new();
        for child in key.children() {
            keys.extend(self.finer(child, depth + 1)?);
        }
        Some(keys)
    }
}
