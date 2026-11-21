//! A hand-written area for the spike (POC_PLAN.md, step 0): a slab of ground, a garden wall and a
//! pond, 64 m square, with detail that grows as the viewer shrinks.
//!
//! One file, built two ways: to a WebAssembly module by `spike/build.sh`, and into the host's own
//! code (`spike/host/src/garden.rs`), so the two can be compared. For their answers to agree bit
//! for bit it uses only operations IEEE 754 makes exact (`+ - * /`, `sqrt`, `floor`, comparisons)
//! and its own `min` and `max`: the standard ones may differ between machines on the sign of a
//! zero. Nothing from `libm` (no `sin`, `exp`, `powf`), whose results differ from one to the next.
//!
//! The raw interface of the spike, before the WIT world of step 1:
//! - `alloc(bytes) -> ptr`: room in the module's memory, for the points and the answers;
//! - `sample(points, count, size, out)`: `count` points, three `f32` each (x, y up, z, in metres),
//!   for a viewer `size` metres tall; `out` gets for each a distance (`f32`) and a material (`u32`);
//! - the import `host.seed() -> i64`: the area's seed.

pub const SOIL: u32 = 1;
pub const STONE: u32 = 2;
pub const WATER: u32 = 3;
pub const GRASS: u32 = 4;

/// How wide the area is, along x and along z, in metres.
pub const WIDTH: f32 = 64.0;

/// The distance to the nearest surface at `p` (negative inside), and its material, for a viewer
/// `size` metres tall.
pub fn sample(p: [f32; 3], size: f32, seed: u64) -> (f32, u32) {
    let seed = (seed ^ (seed >> 32)) as u32;
    // A detail is worth adding only if it is larger than about two of the viewer's voxels, which
    // are a twentieth of their height (POC_PLAN.md, "Detail per size").
    let finest = size * 0.1;
    let ground = ground(p, finest, seed);
    let water = water(p);
    let wall = wall(p, finest, seed);
    let mut best = (ground, if p[1] > -0.4 { GRASS } else { SOIL });
    if wall < best.0 {
        best = (wall, STONE);
    }
    if water < best.0 {
        best = (water, WATER);
    }
    best
}

const POND: [f32; 2] = [20.0, 40.0];
const POND_RADIUS: f32 = 9.0;
const WATER_LEVEL: f32 = -0.3;

/// Gently rolling ground about y = 0, sunk into a bowl under the pond.
fn ground(p: [f32; 3], finest: f32, seed: u32) -> f32 {
    let r = length2(p[0] - POND[0], p[2] - POND[1]);
    let t = clamp(1.0 - r / (POND_RADIUS + 3.0), 0.0, 1.0);
    let bowl = 2.0 * t * t * (3.0 - 2.0 * t);
    let height = 0.6 * noise([p[0] * 0.05, 0.0, p[2] * 0.05], seed) - bowl;
    // Lumps, then clods, then grains: each octave a fifteenth as tall as it is wide.
    (p[1] - height - detail(p, 4.0, finest, 0.066, seed ^ 0x9e37)) * 0.8
}

/// The pond: still water filling the bowl up to its level.
fn water(p: [f32; 3]) -> f32 {
    max(p[1] - WATER_LEVEL, length2(p[0] - POND[0], p[2] - POND[1]) - POND_RADIUS)
}

/// The garden wall along x = 44, a metre thick and three high, rough stone.
fn wall(p: [f32; 3], finest: f32, seed: u32) -> f32 {
    let centre = [44.5, 1.0, 32.0];
    let half = [0.5, 2.0, 28.0];
    let q = [abs(p[0] - centre[0]) - half[0], abs(p[1] - centre[1]) - half[1], abs(p[2] - centre[2]) - half[2]];
    let outside = length3(max(q[0], 0.0), max(q[1], 0.0), max(q[2], 0.0));
    let inside = min(max(q[0], max(q[1], q[2])), 0.0);
    outside + inside - 0.05 + detail(p, 1.0, finest, 0.04, seed ^ 0x7f4a)
}

/// Octaves of noise from `widest` metres down to `finest`, each `steep` times as tall as wide, the
/// last faded in as the viewer shrinks so that nothing jumps when the size changes.
fn detail(p: [f32; 3], widest: f32, finest: f32, steep: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut wide = widest;
    let mut octave = 0;
    while wide >= finest && octave < 16 {
        let fade = clamp(wide / finest - 1.0, 0.0, 1.0);
        let inv = 1.0 / wide;
        sum += fade * steep * wide * noise([p[0] * inv, p[1] * inv, p[2] * inv], seed.wrapping_add(octave));
        wide *= 0.5;
        octave += 1;
    }
    sum
}

/// Value noise in -1..1, smooth, one value at each whole point.
fn noise(p: [f32; 3], seed: u32) -> f32 {
    let (fx, fy, fz) = (p[0].floor(), p[1].floor(), p[2].floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (tx, ty, tz) = (fade(p[0] - fx), fade(p[1] - fy), fade(p[2] - fz));
    let v = |dx: i32, dy: i32, dz: i32| corner(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz), seed);
    let x00 = lerp(v(0, 0, 0), v(1, 0, 0), tx);
    let x10 = lerp(v(0, 1, 0), v(1, 1, 0), tx);
    let x01 = lerp(v(0, 0, 1), v(1, 0, 1), tx);
    let x11 = lerp(v(0, 1, 1), v(1, 1, 1), tx);
    lerp(lerp(x00, x10, ty), lerp(x01, x11, ty), tz)
}

fn corner(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    (h >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
}

fn fade(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

fn abs(a: f32) -> f32 {
    if a < 0.0 { -a } else { a }
}

fn clamp(a: f32, lo: f32, hi: f32) -> f32 {
    min(max(a, lo), hi)
}

fn length2(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

fn length3(x: f32, y: f32, z: f32) -> f32 {
    (x * x + y * y + z * z).sqrt()
}

#[cfg(target_arch = "wasm32")]
mod exports {
    #[link(wasm_import_module = "host")]
    unsafe extern "C" {
        fn seed() -> i64;
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn alloc(bytes: u32) -> *mut u8 {
        let words = (bytes as usize).div_ceil(8);
        let mut room = Vec::<u64>::with_capacity(words);
        let ptr = room.as_mut_ptr() as *mut u8;
        std::mem::forget(room);
        ptr
    }

    /// # Safety
    /// `points` holds `count * 3` floats and `out` room for `count * 2` words, both from `alloc`.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn sample(points: *const f32, count: u32, size: f32, out: *mut u32) {
        let seed = unsafe { seed() } as u64;
        let points = unsafe { std::slice::from_raw_parts(points, count as usize * 3) };
        let out = unsafe { std::slice::from_raw_parts_mut(out, count as usize * 2) };
        for (p, o) in points.chunks_exact(3).zip(out.chunks_exact_mut(2)) {
            let (d, m) = super::sample([p[0], p[1], p[2]], size, seed);
            o[0] = d.to_bits();
            o[1] = m;
        }
    }
}
