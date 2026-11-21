//! Which chunks a viewer sees (POC_PLAN.md, step 2): the finest about them, coarser further off.
//!
//! Each level is a block of four chunks a side about the viewer, its middle on the next level's
//! grid, so it is two of the next level's chunks a side and lies within the next level's block.
//! Each level leaves out the part of its block the level below fills, so every point near the
//! viewer is in exactly one chunk, and the levels nest without gaps. The finest level's voxel is
//! the power of two at or below a twentieth of the viewer's height; the coarsest (`COARSEST`, a
//! metre) has a block that holds the whole area from anywhere in it.

use crate::WIDTH;
use crate::chunk::{CELLS, Key, VOXELS_A_HEIGHT};

/// The coarsest level: voxels of a metre, chunks of 32 m, a block of 128 m about the viewer.
pub const COARSEST: i32 = 0;
/// The finest: voxels of a quarter of a millimetre, for the smallest viewer.
pub const FINEST: i32 = -12;

/// The finest level a viewer `size` tall needs: the power of two at or below a twentieth of it
/// (a giant's half a metre, a person's 6 cm, the tiny's half a millimetre).
pub fn finest(size: f32) -> i32 {
    ((size / VOXELS_A_HEIGHT).log2().floor() as i32).clamp(FINEST, COARSEST)
}

/// Every chunk a viewer with their eye at `eye`, `size` tall, sees, finest first. Only chunks over
/// the area (x and z from 0 to `WIDTH`).
pub fn wanted(eye: [f64; 3], size: f32) -> Vec<Key> {
    let mut keys = Vec::new();
    // The block of the level below, as the middle of its two chunks a side on this level's grid.
    let mut inner: Option<[i32; 3]> = None;
    for level in finest(size)..=COARSEST {
        let side = 2f64.powi(level) * CELLS as f64;
        let middle = eye.map(|c| (c / (2.0 * side)).round() as i32);
        for x in 2 * middle[0] - 2..2 * middle[0] + 2 {
            for y in 2 * middle[1] - 2..2 * middle[1] + 2 {
                for z in 2 * middle[2] - 2..2 * middle[2] + 2 {
                    let at = [x, y, z];
                    if let Some(m) = inner
                        && (0..3).all(|a| at[a] >= m[a] - 1 && at[a] < m[a] + 1)
                    {
                        continue;
                    }
                    let over = |i: i32| (i as f64 + 1.0) * side > 0.0 && (i as f64) * side < WIDTH as f64;
                    if over(x) && over(z) {
                        keys.push(Key { level, at });
                    }
                }
            }
        }
        inner = Some(middle);
    }
    keys
}

/// The space `wanted` fills for a viewer at `eye`: the coarsest block, as its lowest and highest
/// corners (x and z cut to the area).
pub fn reach(eye: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let side = 2f64.powi(COARSEST) * CELLS as f64;
    let middle = eye.map(|c| (c / (2.0 * side)).round() * 2.0 * side);
    let (mut lo, mut hi) = (middle.map(|m| m - 2.0 * side), middle.map(|m| m + 2.0 * side));
    for a in [0, 2] {
        lo[a] = lo[a].max(0.0);
        hi[a] = hi[a].min(WIDTH as f64);
    }
    (lo, hi)
}
