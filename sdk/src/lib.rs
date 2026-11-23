//! Everything an area's program needs (POC_PLAN.md, step 1). An area is one Rust file: a type that
//! is an `Area`, and `area!` naming it. The engine asks it, a batch of points at a time, what is at
//! each for a viewer of a size (`Area::sample`): how far to the nearest surface and that surface's
//! material. It asks once what the area is (`Area::describe`): its name, the places it means for
//! one size, its borders. The engine decides everything else: physics, who reaches where, what
//! players build.
//!
//! An area is 64 m square (`WIDTH`): x and z from 0 to 64, y up, in metres, the ground about y = 0.
//!
//! **Detail grows as the viewer shrinks.** A viewer `size` tall sees detail down to about a tenth
//! of their height (`View::finest`): a giant (17 m) sees smooth stone, a person (1.7 m) its blocks,
//! the tiny (1.7 cm) its cracks and grains. Add a detail only where `View::sees` says it is seen,
//! and fade it in by that, so nothing jumps as a viewer grows or shrinks, and so what a smaller
//! viewer sees is the larger one's world with more in it, never another world.
//!
//! **Distances are never too long.** A distance may be less than the true one, never more: the
//! engine steps that far without looking. The sums here keep that; scaling a distance up does not.
//!
//! **The same everywhere.** An area must give the same answers, bit for bit, on every machine and
//! natively as in WebAssembly. So it uses only what IEEE 754 makes exact: `+ - * /`, `sqrt`,
//! `floor`, comparisons, and this crate's own `min`, `max`, `abs`, `sin`, `cos`, `atan2`. Never the
//! standard library's `sin`, `exp`, `powf` and the like (their results differ from one machine to
//! the next), its `min`, `max` or `clamp` (they may differ on the sign of a zero), `mul_add`, or a
//! float's bits.
//!
//! The crate needs nothing but `std`, and builds with the bindings it carries (made ahead of time
//! from `wit/region.wit`), so an area's build is this crate and one file.

#[doc(hidden)]
#[allow(clippy::all, dead_code, unused_imports, unused_unsafe)]
#[rustfmt::skip]
pub mod bindings;
#[doc(hidden)]
pub mod rt;

use std::ops::{Add, Div, Mul, Neg, Sub};

pub use bindings::plateau::region::types::{Answer, Borders, Description, Edge, Material, Place, Point, Prop, PropKind, Size};

/// How wide an area is, along x and along z, in metres.
pub const WIDTH: f32 = 64.0;

/// The three sizes' heights, in metres.
pub const TINY: f32 = 0.017;
pub const PERSON: f32 = 1.7;
pub const GIANT: f32 = 17.0;

/// An area: made once when it is loaded, with its seed, then asked what is where.
pub trait Area: 'static {
    fn new(seed: u64) -> Self;
    /// What the area is: its name, its places, its borders. Asked once.
    fn describe(&self) -> Description;
    /// What is at `at` for `view`: the distance to the nearest surface (negative inside) and its
    /// material.
    fn sample(&self, at: Point, view: &View) -> Answer;
    /// Smooth GPU-rendered props (reeds, cattails, flowers, tussocks, sticks, mushrooms) that stay
    /// razor-sharp at every distance without voxel aliasing. Asked once on load.
    fn props(&self) -> Vec<Prop> {
        Vec::new()
    }
}

impl Prop {
    /// A smooth capsule from `a` to `b` of radius `r`.
    pub const fn capsule(a: Point, b: Point, r: f32, material: Material) -> Prop {
        Prop { kind: PropKind::Capsule, material, a, b, r, yaw: 0.0 }
    }

    /// A smooth ellipsoid centred at `center` with half-sizes `radii`, turned by `yaw` radians.
    pub const fn ellipsoid(center: Point, radii: Point, yaw: f32, material: Material) -> Prop {
        Prop { kind: PropKind::Ellipsoid, material, a: center, b: radii, r: 0.0, yaw }
    }

    /// A smooth upright cylinder centred at `center` of radius `r` and half-height `half_h`, with
    /// an optional V-notch facing `notch` radians (for lily pads).
    pub const fn cylinder(center: Point, r: f32, half_h: f32, notch: Option<f32>, material: Material) -> Prop {
        match notch {
            Some(yaw) => Prop { kind: PropKind::Cylinder, material, a: center, b: point(r, half_h, 0.0), r: 1.0, yaw },
            None => Prop { kind: PropKind::Cylinder, material, a: center, b: point(r, half_h, 0.0), r: 0.0, yaw: 0.0 },
        }
    }

    /// A smooth cone with base centred at `foot`, base radius `r`, and `height` (negative points down).
    pub const fn cone(foot: Point, height: f32, r: f32, material: Material) -> Prop {
        Prop { kind: PropKind::Cone, material, a: foot, b: point(0.0, height, 0.0), r, yaw: 0.0 }
    }
}

/// Makes the type an area: `area!(Garden);` once, in the area's file. Built for WebAssembly it
/// exports the world's functions; built for the machine (the engine's tests), it gives
/// `played(seed)`, the area to play natively (`native`).
#[macro_export]
macro_rules! area {
    ($ty:ty) => {
        /// The canonical ABI's allocator, exported for the engine to give the area its points.
        #[cfg(target_arch = "wasm32")]
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn cabi_realloc(old: *mut u8, old_len: usize, align: usize, new_len: usize) -> *mut u8 {
            unsafe { $crate::rt::realloc(old, old_len, align, new_len) }
        }

        /// The area, played without WebAssembly.
        #[cfg(not(target_arch = "wasm32"))]
        pub fn played(seed: u64) -> ::std::boxed::Box<dyn $crate::native::Played> {
            ::std::boxed::Box::new(<$ty as $crate::Area>::new(seed))
        }

        #[cfg(target_arch = "wasm32")]
        const _: () = {
            thread_local!(static AREA: ::std::cell::OnceCell<$ty> = const { ::std::cell::OnceCell::new() });

            fn with<R>(f: impl FnOnce(&$ty) -> R) -> R {
                AREA.with(|a| f(a.get_or_init(|| <$ty as $crate::Area>::new($crate::bindings::plateau::region::host::seed()))))
            }

            struct Exports;

            impl $crate::bindings::Guest for Exports {
                fn sample(points: ::std::vec::Vec<$crate::Point>, size: f32) -> ::std::vec::Vec<$crate::Answer> {
                    with(|a| $crate::sample_all(a, &points, size))
                }

                fn describe() -> $crate::Description {
                    with(|a| $crate::Area::describe(a))
                }

                fn props() -> ::std::vec::Vec<$crate::Prop> {
                    with(|a| $crate::Area::props(a))
                }
            }

            $crate::__export_region_impl!(Exports with_types_in $crate::bindings);
        };
    };
}

/// A batch answered: one answer a point, for a viewer `size` tall. Used by `area!`.
#[doc(hidden)]
pub fn sample_all<A: Area>(area: &A, points: &[Point], size: f32) -> Vec<Answer> {
    let view = View::new(size);
    points.iter().map(|&p| area.sample(p, &view)).collect()
}

/// An area played without WebAssembly, its source compiled for the machine it runs on: the
/// engine's tests play the areas of `areas/` so, as stand-ins for their modules (`area!` gives
/// each a `played(seed)`).
#[cfg(not(target_arch = "wasm32"))]
#[doc(hidden)]
pub mod native {
    use crate::{Answer, Area, Description, Point, Prop};

    /// An area as the engine's tests play it, as the world's exports are called.
    pub trait Played {
        fn sample(&self, points: &[Point], size: f32, out: &mut Vec<Answer>);
        fn describe(&self) -> Description;
        fn props(&self) -> Vec<Prop> {
            Vec::new()
        }
    }

    impl<T: Area> Played for T {
        fn sample(&self, points: &[Point], size: f32, out: &mut Vec<Answer>) {
            *out = crate::sample_all(self, points, size);
        }
        fn describe(&self) -> Description {
            Area::describe(self)
        }
        fn props(&self) -> Vec<Prop> {
            Area::props(self)
        }
    }
}

/// Who is looking: how tall they are, and the finest detail worth giving them.
#[derive(Clone, Copy, Debug)]
pub struct View {
    /// The viewer's height, in metres.
    pub size: f32,
    /// The finest detail they see, in metres across: a tenth of their height, about two of the
    /// engine's cells near them (a cell is a twentieth).
    pub finest: f32,
}

impl View {
    pub fn new(size: f32) -> View {
        View { size, finest: size * 0.1 }
    }

    /// How much of a detail `width` metres across this viewer sees: none at `finest` or below, all
    /// of it from twice that, in between growing. Multiply the detail by it, so it fades in as the
    /// viewer shrinks.
    pub fn sees(&self, width: f32) -> f32 {
        clamp(width / self.finest - 1.0, 0.0, 1.0)
    }
}

// Points as vectors.

/// A point, or a vector, from its three coordinates.
pub const fn point(x: f32, y: f32, z: f32) -> Point {
    Point { x, y, z }
}

impl Point {
    pub fn length(self) -> f32 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn dot(self, o: Point) -> f32 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    /// Each coordinate made positive.
    pub fn abs(self) -> Point {
        point(abs(self.x), abs(self.y), abs(self.z))
    }

    /// Each coordinate's larger with `o`'s.
    pub fn max(self, o: Point) -> Point {
        point(max(self.x, o.x), max(self.y, o.y), max(self.z, o.z))
    }

    pub fn min(self, o: Point) -> Point {
        point(min(self.x, o.x), min(self.y, o.y), min(self.z, o.z))
    }

    /// The largest coordinate.
    pub fn largest(self) -> f32 {
        max(self.x, max(self.y, self.z))
    }

    /// How far it is across the ground, ignoring height.
    pub fn flat_length(self) -> f32 {
        (self.x * self.x + self.z * self.z).sqrt()
    }

    /// Turned about the vertical by `angle` radians, counter-clockwise seen from above.
    pub fn turned(self, angle: f32) -> Point {
        let (s, c) = (sin(angle), cos(angle));
        point(c * self.x + s * self.z, self.y, c * self.z - s * self.x)
    }

    /// Each coordinate scaled by `o`'s.
    pub fn scaled(self, o: Point) -> Point {
        point(self.x * o.x, self.y * o.y, self.z * o.z)
    }
}

impl Add for Point {
    type Output = Point;
    fn add(self, o: Point) -> Point {
        point(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}

impl Sub for Point {
    type Output = Point;
    fn sub(self, o: Point) -> Point {
        point(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}

impl Mul<f32> for Point {
    type Output = Point;
    fn mul(self, k: f32) -> Point {
        point(self.x * k, self.y * k, self.z * k)
    }
}

impl Div<f32> for Point {
    type Output = Point;
    fn div(self, k: f32) -> Point {
        point(self.x / k, self.y / k, self.z / k)
    }
}

impl Neg for Point {
    type Output = Point;
    fn neg(self) -> Point {
        point(-self.x, -self.y, -self.z)
    }
}

impl PartialEq for Point {
    fn eq(&self, o: &Point) -> bool {
        self.x == o.x && self.y == o.y && self.z == o.z
    }
}

// Numbers, the same on every machine.

/// The smaller of two, the same everywhere (the standard `f32::min` may differ on the sign of a
/// zero from one machine to the next).
pub fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

pub fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

pub fn abs(a: f32) -> f32 {
    if a < 0.0 { -a } else { a }
}

pub fn clamp(a: f32, lo: f32, hi: f32) -> f32 {
    min(max(a, lo), hi)
}

/// From `a` to `b` as `t` goes from 0 to 1.
pub fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// `t` from 0 to 1, eased in and out; 0 below, 1 above.
pub fn ease(t: f32) -> f32 {
    let t = clamp(t, 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// How far `x` has gone from `a` to `b`, eased: 0 at `a`, 1 at `b`.
pub fn between(x: f32, a: f32, b: f32) -> f32 {
    ease((x - a) / (b - a))
}

pub const PI: f32 = std::f32::consts::PI;
pub const TAU: f32 = std::f32::consts::TAU;

/// The sine, to within about a ten-millionth, the same everywhere.
pub fn sin(x: f32) -> f32 {
    // Into -pi..pi, then -pi/2..pi/2 by symmetry, then the series to x^11.
    let turns = x * (1.0 / TAU);
    let mut r = (turns - (turns + 0.5).floor()) * TAU;
    if r > PI / 2.0 {
        r = PI - r;
    } else if r < -PI / 2.0 {
        r = -PI - r;
    }
    let r2 = r * r;
    r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0 + r2 * (-1.0 / 39_916_800.0))))))
}

pub fn cos(x: f32) -> f32 {
    sin(x + PI / 2.0)
}

/// The angle of `(x, y)` from the x axis, in radians from -pi to pi, to within about a
/// hundred-thousandth, the same everywhere.
pub fn atan2(y: f32, x: f32) -> f32 {
    let (ax, ay) = (abs(x), abs(y));
    if ax == 0.0 && ay == 0.0 {
        return 0.0;
    }
    let z = min(ax, ay) / max(ax, ay);
    let z2 = z * z;
    let mut a = z * (0.999_866 + z2 * (-0.330_299_5 + z2 * (0.180_141 + z2 * (-0.085_133 + z2 * 0.020_835_1))));
    if ay > ax {
        a = PI / 2.0 - a;
    }
    if x < 0.0 {
        a = PI - a;
    }
    if y < 0.0 { -a } else { a }
}

// Distances to shapes, each about the origin: move the point, not the shape (`at - centre`).

/// A ball of radius `r`.
pub fn sphere(p: Point, r: f32) -> f32 {
    p.length() - r
}

/// A box reaching `half` from its middle along each axis.
pub fn cuboid(p: Point, half: Point) -> f32 {
    let q = p.abs() - half;
    q.max(point(0.0, 0.0, 0.0)).length() + min(q.largest(), 0.0)
}

/// A box with its edges rounded by `r`, within the same `half`.
pub fn rounded_cuboid(p: Point, half: Point, r: f32) -> f32 {
    cuboid(p, half - point(r, r, r)) - r
}

/// An upright cylinder of radius `r`, reaching `half_height` above and below its middle.
pub fn cylinder(p: Point, r: f32, half_height: f32) -> f32 {
    let (qx, qy) = (p.flat_length() - r, abs(p.y) - half_height);
    (max(qx, 0.0) * max(qx, 0.0) + max(qy, 0.0) * max(qy, 0.0)).sqrt() + min(max(qx, qy), 0.0)
}

/// A rod of radius `r` from `a` to `b`, rounded at its ends.
pub fn capsule(p: Point, a: Point, b: Point, r: f32) -> f32 {
    let (pa, ba) = (p - a, b - a);
    let h = clamp(pa.dot(ba) / ba.dot(ba), 0.0, 1.0);
    (pa - ba * h).length() - r
}

/// A flat ring lying on the ground: `big` from its middle to the middle of its rod, `small` the
/// rod's radius.
pub fn torus(p: Point, big: f32, small: f32) -> f32 {
    let q = p.flat_length() - big;
    (q * q + p.y * p.y).sqrt() - small
}

// Distances summed. Each keeps distances never too long, given distances that are not.

/// Both shapes.
pub fn union(a: f32, b: f32) -> f32 {
    min(a, b)
}

/// Both shapes, blended where they meet over about `k` metres.
pub fn smooth_union(a: f32, b: f32, k: f32) -> f32 {
    let h = max(k - abs(a - b), 0.0) / k;
    min(a, b) - h * h * k * 0.25
}

/// `a` with `b` cut out of it.
pub fn cut(a: f32, b: f32) -> f32 {
    max(a, -b)
}

/// `a` with `b` cut out of it, the cut's edge rounded over about `k` metres.
pub fn smooth_cut(a: f32, b: f32, k: f32) -> f32 {
    -smooth_union(-a, b, k)
}

/// Only where both are.
pub fn intersect(a: f32, b: f32) -> f32 {
    max(a, b)
}

/// The shape grown by `r` all round: its edges rounded.
pub fn round(d: f32, r: f32) -> f32 {
    d - r
}

/// A shell `thickness` thick about the shape's surface.
pub fn shell(d: f32, thickness: f32) -> f32 {
    abs(d) - thickness * 0.5
}

// Answers summed: as the distances, each keeping the material of the surface that is nearest.

impl Answer {
    pub fn new(distance: f32, material: Material) -> Answer {
        Answer { distance, material }
    }

    /// Both: the nearer surface's material.
    pub fn or(self, o: Answer) -> Answer {
        if o.distance < self.distance { o } else { self }
    }

    /// Both, blended where they meet over about `k` metres: the nearer surface's material.
    pub fn smooth_or(self, o: Answer, k: f32) -> Answer {
        let material = if o.distance < self.distance { o.material } else { self.material };
        Answer::new(smooth_union(self.distance, o.distance, k), material)
    }

    /// With a shape `d` cut out of it: what is left keeps its own material, inside the cut too.
    pub fn cut(self, d: f32) -> Answer {
        Answer::new(cut(self.distance, d), self.material)
    }

    pub fn smooth_cut(self, d: f32, k: f32) -> Answer {
        Answer::new(smooth_cut(self.distance, d, k), self.material)
    }

    /// Only where `d` is too.
    pub fn within(self, d: f32) -> Answer {
        Answer::new(intersect(self.distance, d), self.material)
    }

    /// Grown by `r`, or worn away by a negative `r`: rough stone is a cuboid grown by its detail.
    pub fn grown(self, r: f32) -> Answer {
        Answer::new(self.distance - r, self.material)
    }
}

// Noise.

/// A number for each whole point and seed, the same everywhere.
pub fn hash(x: i32, y: i32, z: i32, seed: u32) -> u32 {
    let mut h = seed ^ (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 16;
    h = h.wrapping_mul(0x7feb_352d);
    h ^= h >> 15;
    h = h.wrapping_mul(0x846c_a68b);
    h ^= h >> 16;
    h
}

fn corner(x: i32, y: i32, z: i32, seed: u32) -> f32 {
    (hash(x, y, z, seed) >> 8) as f32 * (2.0 / 16_777_216.0) - 1.0
}

/// Smooth noise from -1 to 1, changing about once a metre: a value at each whole point, eased
/// between. Scale the point to change its size: `noise(p / 4.0, seed)` changes every 4 m.
pub fn noise(p: Point, seed: u32) -> f32 {
    let (fx, fy, fz) = (p.x.floor(), p.y.floor(), p.z.floor());
    let (ix, iy, iz) = (fx as i32, fy as i32, fz as i32);
    let (tx, ty, tz) = (ease(p.x - fx), ease(p.y - fy), ease(p.z - fz));
    let v = |dx: i32, dy: i32, dz: i32| corner(ix.wrapping_add(dx), iy.wrapping_add(dy), iz.wrapping_add(dz), seed);
    let x00 = mix(v(0, 0, 0), v(1, 0, 0), tx);
    let x10 = mix(v(0, 1, 0), v(1, 1, 0), tx);
    let x01 = mix(v(0, 0, 1), v(1, 0, 1), tx);
    let x11 = mix(v(0, 1, 1), v(1, 1, 1), tx);
    mix(mix(x00, x10, ty), mix(x01, x11, ty), tz)
}

/// Roughness from `widest` metres across down to the finest the viewer sees, each octave half as
/// wide as the one before and `steep` times as tall as it is wide (0.05 for worn stone, 0.1 for
/// rough ground), the finest faded in as the viewer shrinks. Add it to a distance to roughen a
/// surface. Its steepness adds up over the octaves the viewer sees, and with it how much shorter
/// the distance must be made to stay never too long: keep `steep` small.
pub fn detail(p: Point, view: &View, widest: f32, steep: f32, seed: u32) -> f32 {
    let mut sum = 0.0;
    let mut wide = widest;
    let mut octave = 0;
    while wide > view.finest && octave < 16 {
        sum += view.sees(wide) * steep * wide * noise(p / wide, seed.wrapping_add(octave));
        wide *= 0.5;
        octave += 1;
    }
    sum
}

/// Things scattered over the ground, one in each square `spacing` metres wide: the distance to the
/// nearest. `thing(q, seeds)` is the distance to one thing from `q`, which is relative to its own
/// place (a random spot near its square's middle, at y = 0); `seeds` are its own, to choose its
/// size and kind. A thing must reach no further than `spacing` from its place.
pub fn scatter(p: Point, spacing: f32, seed: u32, thing: impl Fn(Point, &mut Seeds) -> f32) -> f32 {
    let (cx, cz) = ((p.x / spacing).floor() as i32, (p.z / spacing).floor() as i32);
    let mut best = f32::MAX;
    for dz in -1..=1 {
        for dx in -1..=1 {
            let (i, j) = (cx.wrapping_add(dx), cz.wrapping_add(dz));
            let mut seeds = Seeds::of(seed as u64, hash(i, 0, j, seed) as usize);
            let place = point((i as f32 + seeds.next(0.2, 0.8)) * spacing, 0.0, (j as f32 + seeds.next(0.2, 0.8)) * spacing);
            best = min(best, thing(p - place, &mut seeds));
        }
    }
    best
}

/// A little generator of numbers, the same for the same seed and number (Caprice's own).
#[derive(Clone, Debug)]
pub struct Seeds(u64);

impl Seeds {
    pub fn of(seed: u64, k: usize) -> Seeds {
        // splitmix64: every seed and number gives a number of its own, the same on every machine.
        let mut z = seed ^ (k as u64).wrapping_add(1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Seeds((z ^ (z >> 31)) | 1)
    }

    /// A number from `lo` to `hi`.
    pub fn next(&mut self, lo: f32, hi: f32) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        lo + (hi - lo) * (self.0 % 10_000) as f32 / 10_000.0
    }

    /// A whole number below `n`.
    pub fn below(&mut self, n: u32) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % n as u64) as u32
    }
}

// What an area says of itself.

impl Place {
    pub fn new(name: &str, size: Size, at: Point) -> Place {
        Place { name: name.into(), size, at }
    }
}

impl Edge {
    /// The ground along an edge, `height(s)` at each metre `s` from its first corner to the next;
    /// no water crossing it.
    pub fn along(height: impl Fn(f32) -> f32) -> Edge {
        Edge { ground: (0..=WIDTH as u32).map(|s| height(s as f32)).collect(), water: None }
    }
}

impl Borders {
    /// The four edges of ground whose height at `(x, z)` is `height(x, z)`.
    pub fn of_ground(height: impl Fn(f32, f32) -> f32) -> Borders {
        Borders {
            north: Edge::along(|s| height(s, 0.0)),
            east: Edge::along(|s| height(WIDTH, s)),
            south: Edge::along(|s| height(s, WIDTH)),
            west: Edge::along(|s| height(0.0, s)),
        }
    }
}
