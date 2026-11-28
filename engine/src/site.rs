//! Site & cavity measurement (Phase C; AGENTS.md §1, §2).
//!
//! A player points at a place (a cave they carved, a hollowed trunk, a pond's edge, a slope) and the
//! engine measures it, so a standalone element can be made for exactly that place:
//!
//! - **`SiteContext`**: what is around a point at the pointing player's size: how the ground lies
//!   (a grid of heights), what it is made of, water, headroom, and the opening aimed at, if any.
//! - **`Cavity`**: an opening's own measure: where its rim is, which way it faces, its outline as
//!   `CONTOUR` distances around its middle, how deep it goes, how much wall is round it, its floor,
//!   and whether a body of the measuring player's size gets in through it.
//!
//! Everything is read from the live `Region` by rays (`Sight`), so carves, placed SDF elements,
//! promoted trunks and (through `scene::Combined`) solid procedural geometry are all measured the
//! same way, at every size. As in `body`, *a distance is not a contact*: an area's distances may be
//! shorter than the true ones, so every ray looks for the surface itself.
//!
//! **Deterministic**: IEEE arithmetic and square roots only, the same polynomial `sin`/`cos` as
//! `compose`, no libm, no maps walked in no order: the same world measured from the same probe gives
//! the same bits on every machine, and `to_text` / `from_text` keep them exactly.
//!
//! **Local metres.** An element is written in metres at a person's scale (`ELEMENT_GUIDE.md`), so a
//! site's measures are given in the local frame (`SiteContext::frame`) of an element placed there by
//! whoever measured it: a tiny's 2.5 cm doorway is 2.5 m tall to the element made for it. For an
//! opening the frame stands on the sill, in the rim's plane: +X right and +Y up as seen from
//! outside, +Z out of the opening towards whoever comes in.

use std::path::Path;

use wasmtime::bail;

use crate::body::{Body, RADIUS, STEP};
use crate::compose::{PlacementTransform, cos_f32, sin_f32};
use crate::laws::{Verdict, length};
use crate::{Answer, LARGEST, Material, Region, Result, SIZES, SMALLEST, chunk};

/// How many points an opening's outline is measured at, evenly round its middle, from its right
/// (seen from outside) upwards.
pub const CONTOUR: usize = 48;
/// The ground's heights are measured on a square grid of this many points a side.
pub const GRID: usize = 7;
/// Current site file format version.
pub const SITE_VERSION: u32 = 1;

/// What a site gives an element placed for it (`SiteContext::params`, `host.params` of
/// `plateau:region@0.2.0`): this many numbers, each at its index below, lengths in the frame's
/// local metres (`ELEMENT_GUIDE.md`, "Params"). Without an opening, all but `P_OPENING` are 0.
pub const PARAMS: usize = P_RADII + SPOKES;
/// 1 if there is an opening, else 0.
pub const P_OPENING: usize = 0;
/// Its outline's width and height.
pub const P_WIDTH: usize = 1;
pub const P_HEIGHT: usize = 2;
/// How high its middle is above the sill (the origin): the middle the radii are measured from is
/// (0, this, 0).
pub const P_MIDDLE: usize = 3;
/// How far in it goes along -Z, and 1 if it goes on through, open behind.
pub const P_DEPTH: usize = 4;
pub const P_THROUGH: usize = 5;
/// The least wall there is beyond its outline at the rim.
pub const P_WALL: usize = 6;
/// 1 for a circle, less the less round it is (`Cavity::roundness`).
pub const P_ROUND: usize = 7;
/// From floor to ceiling inside, and 1 if whoever measured it walks in through it.
pub const P_HEADROOM: usize = 8;
pub const P_WALKABLE: usize = 9;
/// The first of `SPOKES` distances from the middle to the outline, in the rim's plane (z = 0), at
/// even angles anticlockwise from straight right (+X) as seen from outside: the `k`th towards
/// (cos a, sin a) for a = k / SPOKES of a turn. The whole outline as it was measured (`CONTOUR`
/// points): fewer would cut a square doorway's corners.
pub const P_RADII: usize = 10;
pub const SPOKES: usize = CONTOUR;

/// Person height in metres: a site's local frame is scaled so whoever measured it is this tall.
const PERSON: f32 = SIZES[1].1;
/// An opening is looked for and measured within this many of the player's heights.
const FAR: f32 = 4.0;
/// The grid's spacing, in the player's heights.
const GRID_STEP: f32 = 0.5;

/// What is pointed at: a point of a surface, the way out of that surface, where it is seen from (if
/// anywhere), and how tall whoever points is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteProbe {
    pub target: [f32; 3],
    pub normal: [f32; 3],
    pub from: Option<[f32; 3]>,
    pub size: f32,
}

/// The lie of the land about a site.
#[derive(Clone, Debug, PartialEq)]
pub struct Ground {
    /// World height of the ground at the frame's middle.
    pub elevation: f32,
    /// How steep it is: metres risen a metre across (`0.0` flat, `1.0` a 45° slope).
    pub rise: f32,
    /// Which way it rises, across the ground (world x, z; length one, or zero where it is flat).
    pub uphill: [f32; 2],
    /// The grid's spacing, and where its first row lies, in local metres (its columns run from
    /// `-3` to `+3` spacings along local x, its rows `z0` to `z0 + 6` spacings along local z).
    pub step: f32,
    pub z0: f32,
    /// The ground's height at each point of the grid, in local metres above the frame (`None` where
    /// something solid stands higher than was looked, or there is no ground), row by row.
    pub heights: Vec<Option<f32>>,
}

/// The nearest water about a site.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SiteWater {
    /// World height of its surface.
    pub level: f32,
    /// How far across the ground it is (metres), and which way (world x, z).
    pub distance: f32,
    pub toward: [f32; 2],
    /// How deep it gets within what was looked at, in metres.
    pub depth: f32,
}

/// An opening in something solid, measured (world metres).
#[derive(Clone, Debug, PartialEq)]
pub struct Cavity {
    /// The middle of the opening, in the plane of its rim: the outermost plane in which wall still
    /// goes all the way round, a little in from where it starts to flare.
    pub center: [f32; 3],
    /// The way out of it (length one); level for anything that can be walked into.
    pub axis: [f32; 3],
    /// How far the wall is from `center` at each of `CONTOUR` angles in the rim's plane.
    pub radii: Vec<f32>,
    /// How much solid there is beyond the wall's surface at each of those angles, in the rim's
    /// plane, up to `0.6` of the mean radius (`0` where it cannot be told, as behind a mesh).
    pub wall: Vec<f32>,
    /// How far in it goes from the rim along `axis`, and whether nothing stops it within what is
    /// looked at (a tunnel through).
    pub depth: f32,
    pub through: bool,
    /// The height from floor to ceiling a body's height in (or half of `depth`, if that is less).
    pub headroom: f32,
    /// What its wall is mostly made of.
    pub material: Material,
    /// Whether a body of the measuring player's size gets in through it as it is.
    pub walkable: bool,
}

/// A measured site: all an element needs to be made for it.
#[derive(Clone, Debug, PartialEq)]
pub struct SiteContext {
    /// The area's name, if known.
    pub area: String,
    pub target: [f32; 3],
    pub normal: [f32; 3],
    /// The height of whoever measured it.
    pub size: f32,
    /// Where an element made for this site is placed (its local frame, see the module).
    pub frame: PlacementTransform,
    pub ground: Ground,
    /// What the surfaces round about are made of, most first, as shares of one.
    pub materials: Vec<(Material, f32)>,
    pub water: Option<SiteWater>,
    /// Clear height above the target, if there is anything over it within six heights.
    pub overhead: Option<f32>,
    pub cavity: Option<Cavity>,
    /// What is round about in the world, in words (what the site is in and who owns it, carves,
    /// water let in, elements near by), from the world graph: written by `live::World::neighbourhood`,
    /// which knows the world; `measure` knows only the region, and leaves it empty.
    pub near: Vec<String>,
}

/// A ray: from a point, along a direction of length one, at most so far.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Ray {
    pub from: [f32; 3],
    pub along: [f32; 3],
    pub far: f32,
}

impl Ray {
    fn at(&self, t: f32) -> [f32; 3] {
        add(self.from, scale(self.along, t))
    }
}

/// Where a ray meets a surface, and what it is (`t == 0`: it started inside).
#[derive(Clone, Copy, Debug)]
pub(crate) struct Hit {
    pub t: f32,
    pub material: Material,
}

/// Rays and rings of rays through an area, as a viewer of a size sees it.
pub(crate) struct Sight<'a> {
    pub area: &'a mut dyn Region,
    pub size: f32,
    out: Vec<Answer>,
}

impl<'a> Sight<'a> {
    pub fn new(area: &'a mut dyn Region, size: f32) -> Sight<'a> {
        Sight { area, size, out: Vec::new() }
    }

    pub fn ask(&mut self, points: &[[f32; 3]]) -> Result<&[Answer]> {
        chunk::sample(self.area, points, self.size, &mut self.out)?;
        Ok(&self.out)
    }

    /// Where each ray first meets something solid. Water is no more than air to them, unless `wet`.
    pub fn cast(&mut self, rays: &[Ray], wet: bool) -> Result<Vec<Option<Hit>>> {
        let close = 0.0015 * self.size;
        let least = 0.003 * self.size;
        let mut t = vec![0.0f32; rays.len()];
        let mut before = vec![0.0f32; rays.len()];
        let mut found: Vec<Option<Hit>> = vec![None; rays.len()];
        let mut active: Vec<usize> = (0..rays.len()).collect();
        let mut refine: Vec<(usize, f32, f32)> = Vec::new();
        let mut out = Vec::new();
        for _ in 0..600 {
            if active.is_empty() {
                break;
            }
            let points: Vec<[f32; 3]> = active.iter().map(|&k| rays[k].at(t[k])).collect();
            chunk::sample(self.area, &points, self.size, &mut out)?;
            let mut still = Vec::with_capacity(active.len());
            for (&k, a) in active.iter().zip(&out) {
                let water = a.material == Material::Water && !wet;
                if !water && a.distance <= close {
                    found[k] = Some(Hit { t: t[k], material: a.material });
                    if a.distance < 0.0 && t[k] > 0.0 {
                        refine.push((k, before[k], t[k]));
                    }
                    continue;
                }
                // Inside water, an answer says how far its surface is, not the floor's.
                let step = if water && a.distance <= 0.0 {
                    clamp(-a.distance, close, self.size * 0.05)
                } else {
                    max(a.distance, least)
                };
                if t[k] < rays[k].far {
                    before[k] = t[k];
                    t[k] = min(t[k] + step, rays[k].far);
                    still.push(k);
                }
            }
            active = still;
        }
        for _ in 0..8 {
            if refine.is_empty() {
                break;
            }
            let points: Vec<[f32; 3]> = refine.iter().map(|&(k, o, i)| rays[k].at((o + i) * 0.5)).collect();
            chunk::sample(self.area, &points, self.size, &mut out)?;
            for (r, a) in refine.iter_mut().zip(&out) {
                let mid = (r.1 + r.2) * 0.5;
                if a.distance < 0.0 && (wet || a.material != Material::Water) {
                    r.2 = mid;
                } else {
                    r.1 = mid;
                }
            }
        }
        for (k, _, inside) in refine {
            if let Some(h) = &mut found[k] {
                h.t = inside;
            }
        }
        Ok(found)
    }

    /// The wall all round `center` in the plane across `axis`, seen by `n` rays at most `far` long,
    /// or `None` if it does not go all the way round (one ray in 24 may miss, for a crack).
    fn ring(&mut self, center: [f32; 3], axis: [f32; 3], n: usize, far: f32) -> Result<Option<Ring>> {
        let (right, up) = basis(axis);
        let rays: Vec<Ray> = (0..n).map(|k| Ray { from: center, along: spoke(right, up, k, n), far }).collect();
        let hits = self.cast(&rays, false)?;
        let mut radii: Vec<Option<f32>> = Vec::with_capacity(n);
        let mut tally = [0usize; Material::ALL.len()];
        let mut missed = 0;
        for h in &hits {
            match h {
                Some(h) if h.t <= 0.0 => return Ok(None),
                Some(h) => {
                    radii.push(Some(h.t));
                    tally[h.material as usize] += 1;
                }
                None => {
                    radii.push(None);
                    missed += 1;
                }
            }
        }
        if missed > n / 24 {
            return Ok(None);
        }
        let filled: Vec<f32> = (0..n)
            .map(|k| {
                radii[k].unwrap_or_else(|| {
                    let near = |step: usize| (1..n).find_map(|i| radii[(k + i * step) % n]).unwrap_or(0.0);
                    (near(1) + near(n - 1)) * 0.5
                })
            })
            .collect();
        Ok(Some(Ring { center, axis, radii: filled, tally }))
    }
}

/// A wall's outline about a point, in a plane.
#[derive(Clone, Debug)]
struct Ring {
    center: [f32; 3],
    axis: [f32; 3],
    radii: Vec<f32>,
    tally: [usize; Material::ALL.len()],
}

impl Ring {
    fn area(&self) -> f32 {
        outline_area(&self.radii)
    }

    /// The radius of the circle as large as it.
    fn mean(&self) -> f32 {
        (self.area() / std::f32::consts::PI).sqrt()
    }

    /// The middle of what it goes round.
    fn centroid(&self) -> [f32; 3] {
        let (right, up) = basis(self.axis);
        let n = self.radii.len();
        let (mut sx, mut sy, mut sa) = (0.0f32, 0.0f32, 0.0f32);
        for k in 0..n {
            let a = flat(&self.radii, k);
            let b = flat(&self.radii, (k + 1) % n);
            let w = a[0] * b[1] - a[1] * b[0];
            sx += (a[0] + b[0]) * w;
            sy += (a[1] + b[1]) * w;
            sa += w;
        }
        if sa <= 1e-20 {
            return self.center;
        }
        let k = 1.0 / (3.0 * sa);
        add(self.center, add(scale(right, sx * k), scale(up, sy * k)))
    }

    fn material(&self) -> Material {
        let mut best = 0;
        for (k, &n) in self.tally.iter().enumerate() {
            if n > self.tally[best] {
                best = k;
            }
        }
        Material::ALL[best]
    }
}

/// Where on an outline its point `k` is, about its middle (right, up).
fn flat(radii: &[f32], k: usize) -> [f32; 2] {
    let [c, s] = turn(k, radii.len());
    [radii[k] * c, radii[k] * s]
}

/// The smallest circle round `points`, near enough: its middle and radius.
pub(crate) fn circle(points: &[[f32; 2]]) -> ([f32; 2], f32) {
    let mut c = [0.0f32; 2];
    let farthest = |c: [f32; 2]| {
        let (mut best, mut d2) = (c, -1.0f32);
        for p in points {
            let d = (p[0] - c[0]) * (p[0] - c[0]) + (p[1] - c[1]) * (p[1] - c[1]);
            if d > d2 {
                (best, d2) = (*p, d);
            }
        }
        (best, max(d2, 0.0).sqrt())
    };
    // Bădoiu–Clarkson: step towards the farthest point, less each time.
    for i in 1..=400 {
        let (p, _) = farthest(c);
        let k = 1.0 / (i as f32 + 1.0);
        c = [c[0] + (p[0] - c[0]) * k, c[1] + (p[1] - c[1]) * k];
    }
    (c, farthest(c).1)
}

fn outline_area(radii: &[f32]) -> f32 {
    let n = radii.len();
    let wedge = sin_f32(std::f32::consts::TAU / n as f32);
    (0..n).map(|k| 0.5 * radii[k] * radii[(k + 1) % n] * wedge).sum()
}

impl Cavity {
    /// Right and up across the opening, seen from outside.
    pub fn across(&self) -> ([f32; 3], [f32; 3]) {
        basis(self.axis)
    }

    /// The way from the middle to the outline's point `k`, in the rim's plane (length one).
    pub fn spoke(&self, k: usize) -> [f32; 3] {
        let (right, up) = self.across();
        spoke(right, up, k, self.radii.len())
    }

    /// The radius of the circle as large as the opening.
    pub fn mean_radius(&self) -> f32 {
        (outline_area(&self.radii) / std::f32::consts::PI).sqrt()
    }

    pub fn area(&self) -> f32 {
        outline_area(&self.radii)
    }

    /// The outline's span right and up from its middle: `[left, right, bottom, top]`.
    pub fn span(&self) -> [f32; 4] {
        let mut s = [0.0f32; 4];
        for k in 0..self.radii.len() {
            let [x, y] = flat(&self.radii, k);
            s = [min(s[0], x), max(s[1], x), min(s[2], y), max(s[3], y)];
        }
        s
    }

    pub fn width(&self) -> f32 {
        let s = self.span();
        s[1] - s[0]
    }

    pub fn height(&self) -> f32 {
        let s = self.span();
        s[3] - s[2]
    }

    /// How far below the middle its floor is, and the floor's world height for one walked into.
    pub fn drop(&self) -> f32 {
        self.radii[CONTOUR * 3 / 4]
    }

    pub fn floor(&self) -> f32 {
        self.center[1] - self.drop()
    }

    /// `1` for a circle, less the more the outline's distance from its own middle varies: its least
    /// over its most, about the middle of the smallest circle round it.
    pub fn roundness(&self) -> f32 {
        let (c, most) = self.circle();
        let mut least = f32::INFINITY;
        for k in 0..self.radii.len() {
            let [x, y] = flat(&self.radii, k);
            least = min(least, ((x - c[0]) * (x - c[0]) + (y - c[1]) * (y - c[1])).sqrt());
        }
        if most > 0.0 { least / most } else { 0.0 }
    }

    /// How much of the box round it the opening fills: 0.79 a circle, about 0.9 an arch, 1 a square.
    pub fn fullness(&self) -> f32 {
        let b = self.width() * self.height();
        if b > 0.0 { self.area() / b } else { 0.0 }
    }

    /// The smallest circle round the outline: its middle (right, up from `center`), and its radius.
    pub fn circle(&self) -> ([f32; 2], f32) {
        circle(&(0..self.radii.len()).map(|k| flat(&self.radii, k)).collect::<Vec<_>>())
    }

    /// Whether it is gone into on foot (and not, as a hole in the ground, dropped into).
    pub fn upright(&self) -> bool {
        abs(self.axis[1]) < 0.5
    }

    /// What its outline is most like, in words.
    pub fn shape(&self) -> &'static str {
        if self.roundness() > 0.86 {
            "round"
        } else if self.fullness() > 0.94 {
            "square-cornered"
        } else if self.fullness() > 0.82 {
            "arched"
        } else {
            "irregular"
        }
    }
}

impl SiteContext {
    /// Measures the site `probe` points at in `area` (as it is now, carves and all; give it
    /// `scene::Combined` to see solid procedural geometry too).
    pub fn measure(area: &mut dyn Region, probe: &SiteProbe) -> Result<SiteContext> {
        let h = probe.size;
        if !(SMALLEST..=LARGEST).contains(&h) || !h.is_finite() {
            bail!("a site is measured by someone {SMALLEST} to {LARGEST} m tall, not {h}");
        }
        if !probe.target.iter().chain(&probe.normal).all(|c| c.is_finite()) || probe.from.is_some_and(|f| !f.iter().all(|c| c.is_finite())) {
            bail!("a site's target must be finite, got {probe:?}");
        }
        let normal = unit(probe.normal).unwrap_or([0.0, 1.0, 0.0]);
        let probe = SiteProbe { normal, ..*probe };
        let mut sight = Sight::new(area, h);
        let cavity = cavity(&mut sight, &probe)?;
        let frame = match &cavity {
            Some(c) => {
                let (right, up) = c.across();
                let sill = add(c.center, scale(up, -c.drop()));
                PlacementTransform::with_rot(sill, quat(right, up, c.axis), h / PERSON)
            }
            None => {
                // Facing out of a wall or down a slope; on the level, towards whoever looks.
                let level = |v: [f32; 3]| unit([v[0], 0.0, v[2]]);
                let steep = normal[0] * normal[0] + normal[2] * normal[2] > 0.06;
                let seen = probe.from.and_then(|f| level(sub(f, probe.target)));
                let out = if steep { level(normal) } else { None }.or(seen).unwrap_or([0.0, 0.0, 1.0]);
                let (right, up) = basis(out);
                PlacementTransform::with_rot(probe.target, quat(right, up, out), h / PERSON)
            }
        };
        let base = match &cavity {
            Some(c) => add(c.center, scale(c.axis, 0.02 * h)),
            None => add(probe.target, scale(normal, 0.05 * h)),
        };
        let overhead = sight.cast(&[Ray { from: base, along: UP, far: 6.0 * h }], false)?[0].map(|hit| hit.t);
        let (ground, water) = ground(&mut sight, frame, cavity.is_some(), overhead)?;
        let materials = materials(&mut sight, &probe, cavity.as_ref())?;
        Ok(SiteContext { area: String::new(), target: probe.target, normal, size: h, frame, ground, materials, water, overhead, cavity, near: Vec::new() })
    }

    /// The opening's outline in the frame's local metres (x right, y up from the sill, at z = 0).
    pub fn outline(&self) -> Vec<[f32; 2]> {
        let Some(c) = &self.cavity else { return Vec::new() };
        let inv = 1.0 / self.frame.scale;
        (0..c.radii.len())
            .map(|k| {
                let [x, y] = flat(&c.radii, k);
                [x * inv, (y + c.drop()) * inv]
            })
            .collect()
    }

    /// What an element placed for this site is given (`host.params`): `PARAMS` numbers, laid out
    /// as `P_OPENING` and those after it say, in the frame's local metres.
    pub fn params(&self) -> Vec<f32> {
        let mut p = vec![0.0f32; PARAMS];
        let Some(c) = &self.cavity else { return p };
        let l = |m: f32| self.local(m);
        p[P_OPENING] = 1.0;
        p[P_WIDTH] = l(c.width());
        p[P_HEIGHT] = l(c.height());
        p[P_MIDDLE] = l(c.drop());
        p[P_DEPTH] = l(c.depth);
        p[P_THROUGH] = c.through as u8 as f32;
        p[P_WALL] = l(c.wall.iter().copied().fold(f32::INFINITY, min));
        p[P_ROUND] = c.roundness();
        p[P_HEADROOM] = l(c.headroom);
        p[P_WALKABLE] = c.walkable as u8 as f32;
        let n = c.radii.len();
        for k in 0..SPOKES {
            p[P_RADII + k] = l(c.radii[k * n / SPOKES]);
        }
        if !p[P_WALL].is_finite() {
            p[P_WALL] = 0.0;
        }
        p
    }

    /// The site with its opening `wide` times as wide and `high` times as high about its sill, all
    /// else as it was: one like it, to judge an element over a range of openings (`plateau
    /// check-element`). Its headroom grows with it; whoever measured it walks in only if it grew.
    pub fn varied(&self, wide: f32, high: f32) -> SiteContext {
        let mut s = self.clone();
        let Some(c) = &mut s.cavity else { return s };
        let n = c.radii.len();
        let drop = c.drop();
        // About the middle, which stays over the sill, `high` times as far up.
        let scaled: Vec<[f32; 2]> = (0..n).map(|k| {
            let [x, y] = flat(&c.radii, k);
            [x * wide, y * high]
        }).collect();
        let radii: Vec<f32> = (0..n)
            .map(|j| {
                let [dx, dy] = turn(j, n);
                let mut far = 0.0f32;
                for k in 0..n {
                    let (p, q) = (scaled[k], scaled[(k + 1) % n]);
                    let (ex, ey) = (q[0] - p[0], q[1] - p[1]);
                    let den = dx * ey - dy * ex;
                    if abs(den) < 1e-12 {
                        continue;
                    }
                    let t = (p[0] * ey - p[1] * ex) / den;
                    let u = (p[0] * dy - p[1] * dx) / den;
                    if t > 0.0 && (-1e-6..=1.0 + 1e-6).contains(&u) {
                        far = max(far, t);
                    }
                }
                far
            })
            .collect();
        let (_, up) = c.across();
        let sill = add(c.center, scale(up, -drop));
        c.center = add(sill, scale(up, drop * high));
        c.radii = radii;
        c.headroom *= high;
        c.walkable &= wide >= 1.0 && high >= 1.0;
        s
    }

    /// A world length in the frame's local metres.
    pub fn local(&self, world: f32) -> f32 {
        world / self.frame.scale
    }

    /// One line for a player: what was measured.
    pub fn summary(&self) -> String {
        match &self.cavity {
            Some(c) => format!(
                "{} opening in {}: {} wide, {} high, {}{}, {}",
                c.shape(),
                c.material.name(),
                length(c.width()),
                length(c.height()),
                length(c.depth),
                if c.through { "+ deep, open behind" } else { " deep" },
                if c.walkable {
                    "you fit in"
                } else if c.upright() {
                    "you do not fit in"
                } else {
                    "opens upwards"
                }
            ),
            None => {
                let mat = self.materials.first().map_or("nothing", |m| m.0.name());
                let water = match &self.water {
                    Some(w) if w.distance < 0.1 * self.size => format!(", in water up to {} deep", length(w.depth)),
                    Some(w) => format!(", water {} off, up to {} deep", length(w.distance), length(w.depth)),
                    None => String::new(),
                };
                format!("{mat}, rising {} a metre{water}; no opening here", length(self.ground.rise))
            }
        }
    }

    /// The site in words and numbers for whoever writes an element for it, in local metres.
    pub fn describe_for_prompt(&self) -> String {
        let l = |m: f32| self.local(m);
        let mut s = format!(
            "The site was measured by someone {} tall. Everything below is in your element's **local metres**: whoever measured it is \
             1.7 m tall in them (the engine places your element at scale {} and turns and moves it into place). +Y is up, +Z faces out \
             towards whoever comes to it, +X is to the right as they look at it.\n\n",
            length(self.size),
            self.frame.scale
        );
        let mats: Vec<String> = self.materials.iter().map(|(m, k)| format!("{} {:.0}%", m.name(), k * 100.0)).collect();
        s += &format!("- **Round about**: {}.\n", mats.join(", "));
        match self.overhead {
            Some(o) => s += &format!("- **Overhead**: something solid {:.2} m up.\n", l(o)),
            None => s += "- **Overhead**: open.\n",
        }
        if let Some(w) = &self.water {
            let right = self.frame.local_to_world_dir([1.0, 0.0, 0.0]);
            let out = self.frame.local_to_world_dir([0.0, 0.0, 1.0]);
            let along = |v: [f32; 3]| w.toward[0] * v[0] + w.toward[1] * v[2];
            s += &format!(
                "- **Water**: its surface is at local y = {:.3}, up to {:.2} m deep, nearest at about local ({:.2}, {:.2}) in x, z.\n",
                l(w.level - self.frame.pos[1]),
                l(w.depth),
                l(w.distance * along(right)),
                l(w.distance * along(out)),
            );
        }
        let g = &self.ground;
        s += &format!(
            "- **Ground**: rises {:.2} m a metre. Heights in local y on a grid, rows from local z = {:.2} to {:.2} (nearest the {} first), \
             columns from local x = {:.2} to {:.2}, {:.2} m apart (`--` where something stands in the way or there is no ground):\n\n```text\n",
            g.rise,
            g.z0,
            g.z0 + g.step * (GRID - 1) as f32,
            if self.cavity.is_some() { "opening" } else { "back" },
            -g.step * (GRID / 2) as f32,
            g.step * (GRID / 2) as f32,
            g.step,
        );
        for row in g.heights.chunks(GRID) {
            let cells: Vec<String> = row.iter().map(|y| y.map_or("    --".into(), |y| format!("{y:6.2}"))).collect();
            s += &format!("{}\n", cells.join(" "));
        }
        s += "```\n";
        if !self.near.is_empty() {
            s += "\n### Round about, in the world\n\nWhat the world knows of the site and what is near it (places in local metres, as above):\n\n";
            for line in &self.near {
                s += &format!("- {line}\n");
            }
        }
        if let Some(c) = &self.cavity {
            let span = c.span();
            let up = c.drop();
            s += &format!(
                "\n### The opening\n\nThe origin (0, 0, 0) is on the opening's sill: its floor, in the middle, in the plane of its rim (z = 0). The wall is at \
                 z <= 0 all round; outside is z > 0.\n\n\
                 - **Outline**: {} ({:.0}% round), {:.3} m wide (x from {:.3} to {:.3}), {:.3} m high (y from {:.3} to {:.3}); mean radius {:.3} m \
                 about its middle (0, {:.3}).\n\
                 - **Depth**: {:.2} m in along -Z{}; {:.2} m from floor to ceiling inside.\n\
                 - **Wall**: {}; at least {:.3} m of it beyond the outline at the rim.\n\
                 - **Getting in**: {}.\n",
                c.shape(),
                c.roundness() * 100.0,
                l(c.width()),
                l(span[0]),
                l(span[1]),
                l(c.height()),
                l(span[2] + up),
                l(span[3] + up),
                l(c.mean_radius()),
                l(up),
                l(c.depth),
                if c.through { " and on, open behind" } else { "" },
                l(c.headroom),
                c.material.name(),
                l(c.wall.iter().copied().fold(f32::INFINITY, min)),
                if c.walkable {
                    "someone 1.7 m tall (0.51 m across) walks in through it; your element must leave them room to"
                } else {
                    "someone 1.7 m tall does not get in through it as it is (too small, too shallow, or it opens upwards)"
                }
            );
            s += &format!(
                "\nThe outline itself, (x, y) at z = 0, anticlockwise from its right as seen from outside (use it as it is; go a little beyond it \
                 into the wall):\n\n```rust\nconst OPENING: [(f32, f32); {CONTOUR}] = [\n"
            );
            for row in self.outline().chunks(6) {
                let cells: Vec<String> = row.iter().map(|[x, y]| format!("({x:.4}, {y:.4}),")).collect();
                s += &format!("    {}\n", cells.join(" "));
            }
            s += "];\n```\n";
            s += &format!(
                "\nYour element is given these measures when it runs, too: `params()`, {PARAMS} numbers (\"Params\" in the guide), the \
                 outline's radii among them. Build from those, not from the numbers above, so it fits every opening like this one.\n"
            );
        }
        s
    }

    /// Serializes to a human-readable text representation that keeps every bit.
    pub fn to_text(&self) -> String {
        let v3 = |v: [f32; 3]| format!("{} {} {}", v[0], v[1], v[2]);
        let mut s = format!("plateau-site {SITE_VERSION}\n# {}\n", self.summary());
        if !self.area.is_empty() {
            s += &format!("area {}\n", self.area.replace(['\n', '\r'], " "));
        }
        let q = self.frame.rot;
        s += &format!("target {}\nnormal {}\nsize {}\n", v3(self.target), v3(self.normal), self.size);
        s += &format!("frame {} rot {} {} {} {} scale {}\n", v3(self.frame.pos), q[0], q[1], q[2], q[3], self.frame.scale);
        let g = &self.ground;
        s += &format!("ground {} rise {} uphill {} {} step {} z0 {}\n", g.elevation, g.rise, g.uphill[0], g.uphill[1], g.step, g.z0);
        for row in g.heights.chunks(GRID) {
            let cells: Vec<String> = row.iter().map(|y| y.map_or("-".into(), |y| y.to_string())).collect();
            s += &format!("heights {}\n", cells.join(" "));
        }
        for (m, k) in &self.materials {
            s += &format!("material {} {k}\n", m.name());
        }
        if let Some(w) = &self.water {
            s += &format!("water {} distance {} toward {} {} depth {}\n", w.level, w.distance, w.toward[0], w.toward[1], w.depth);
        }
        if let Some(o) = self.overhead {
            s += &format!("overhead {o}\n");
        }
        if let Some(c) = &self.cavity {
            s += &format!(
                "cavity {} axis {} depth {} through {} headroom {} wall {} walkable {}\n",
                v3(c.center),
                v3(c.axis),
                c.depth,
                c.through as u8,
                c.headroom,
                c.material.name(),
                c.walkable as u8
            );
            let list = |v: &[f32]| v.iter().map(f32::to_string).collect::<Vec<_>>().join(" ");
            s += &format!("contour {}\nmargin {}\n", list(&c.radii), list(&c.wall));
        }
        for line in &self.near {
            s += &format!("near {}\n", line.replace(['\n', '\r'], " "));
        }
        s
    }

    /// Parses and validates a site file.
    pub fn from_text(text: &str) -> Result<SiteContext> {
        let (mut version, mut area, mut target, mut normal, mut size, mut frame) = (None, String::new(), None, None, None, None);
        let (mut ground, mut materials, mut water, mut overhead): (Option<Ground>, _, _, _) = (None, Vec::new(), None, None);
        let mut cavity: Option<Cavity> = None;
        let mut near = Vec::new();
        for (k, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let (word, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            let mut f = Fields { line: k + 1, word, rest: rest.split_whitespace() };
            match word {
                "plateau-site" => {
                    let v = f.num()?;
                    if v != SITE_VERSION as f32 {
                        bail!("line {}: unsupported site version {v} (expected {SITE_VERSION})", f.line);
                    }
                    version = Some(v);
                }
                "area" => {
                    area = rest.trim().to_string();
                    continue;
                }
                "near" => {
                    near.push(rest.trim().to_string());
                    continue;
                }
                "target" => target = Some(f.three()?),
                "normal" => normal = Some(f.three()?),
                "size" => size = Some(f.num()?),
                "frame" => {
                    let pos = f.three()?;
                    f.key("rot")?;
                    let rot = [f.num()?, f.num()?, f.num()?, f.num()?];
                    f.key("scale")?;
                    let scale = f.num()?;
                    let len = (rot[0] * rot[0] + rot[1] * rot[1] + rot[2] * rot[2] + rot[3] * rot[3]).sqrt();
                    if !(0.95..=1.05).contains(&len) || scale <= 0.0 || scale > 100.0 {
                        bail!("line {}: a frame's rot is a unit quaternion and its scale in (0, 100]", f.line);
                    }
                    frame = Some(PlacementTransform { pos, rot, scale });
                }
                "ground" => {
                    let elevation = f.num()?;
                    f.key("rise")?;
                    let rise = f.num()?;
                    f.key("uphill")?;
                    let uphill = [f.num()?, f.num()?];
                    f.key("step")?;
                    let step = f.num()?;
                    f.key("z0")?;
                    ground = Some(Ground { elevation, rise, uphill, step, z0: f.num()?, heights: Vec::new() });
                }
                "heights" => {
                    let line = f.line;
                    let g = ground.as_mut().ok_or_else(|| wasmtime::format_err!("line {line}: `heights` before `ground`"))?;
                    for _ in 0..GRID {
                        g.heights.push(f.maybe()?);
                    }
                }
                "material" => materials.push((f.material()?, f.num()?)),
                "water" => {
                    let level = f.num()?;
                    f.key("distance")?;
                    let distance = f.num()?;
                    f.key("toward")?;
                    let toward = [f.num()?, f.num()?];
                    f.key("depth")?;
                    water = Some(SiteWater { level, distance, toward, depth: f.num()? });
                }
                "overhead" => overhead = Some(f.num()?),
                "cavity" => {
                    let center = f.three()?;
                    f.key("axis")?;
                    let axis = f.three()?;
                    f.key("depth")?;
                    let depth = f.num()?;
                    f.key("through")?;
                    let through = f.num()? != 0.0;
                    f.key("headroom")?;
                    let headroom = f.num()?;
                    f.key("wall")?;
                    let material = f.material()?;
                    f.key("walkable")?;
                    let walkable = f.num()? != 0.0;
                    let len = dot(axis, axis).sqrt();
                    if !(0.99..=1.01).contains(&len) || depth <= 0.0 {
                        bail!("line {}: a cavity's axis is of length one and its depth more than nothing", f.line);
                    }
                    cavity = Some(Cavity { center, axis, radii: Vec::new(), wall: Vec::new(), depth, through, headroom, material, walkable });
                }
                "contour" | "margin" => {
                    let line = f.line;
                    let c = cavity.as_mut().ok_or_else(|| wasmtime::format_err!("line {line}: `{word}` before `cavity`"))?;
                    let list = (0..CONTOUR).map(|_| f.num()).collect::<Result<Vec<f32>>>()?;
                    let contour = word == "contour";
                    if list.iter().any(|&r| r < 0.0 || (contour && r <= 0.0)) {
                        bail!("line {line}: `{word}` has a distance less than it may be");
                    }
                    if contour { c.radii = list } else { c.wall = list }
                }
                _ => bail!("line {}: unknown record `{line}`", f.line),
            }
            f.end()?;
        }
        let missing = |what: &str| wasmtime::format_err!("the site file has no `{what}`");
        version.ok_or_else(|| missing("plateau-site"))?;
        let site = SiteContext {
            area,
            target: target.ok_or_else(|| missing("target"))?,
            normal: normal.ok_or_else(|| missing("normal"))?,
            size: size.ok_or_else(|| missing("size"))?,
            frame: frame.ok_or_else(|| missing("frame"))?,
            ground: ground.ok_or_else(|| missing("ground"))?,
            materials,
            water,
            overhead,
            cavity,
            near,
        };
        if !(SMALLEST..=LARGEST).contains(&site.size) {
            bail!("a site is measured by someone {SMALLEST} to {LARGEST} m tall, not {}", site.size);
        }
        if site.ground.heights.len() != GRID * GRID {
            bail!("a site's ground has {} heights, not {}", site.ground.heights.len(), GRID * GRID);
        }
        if let Some(c) = &site.cavity
            && (c.radii.len() != CONTOUR || c.wall.len() != CONTOUR)
        {
            bail!("a site's cavity needs `contour` and `margin`, {CONTOUR} distances each");
        }
        Ok(site)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, self.to_text())?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<SiteContext> {
        SiteContext::from_text(&std::fs::read_to_string(path)?)
    }
}

/// A line's values, read one after another.
struct Fields<'a> {
    line: usize,
    word: &'a str,
    rest: std::str::SplitWhitespace<'a>,
}

impl<'a> Fields<'a> {
    fn next(&mut self) -> Result<&'a str> {
        self.rest.next().ok_or_else(|| wasmtime::format_err!("line {}: `{}` ends early", self.line, self.word))
    }

    fn num(&mut self) -> Result<f32> {
        let t = self.next()?;
        match t.parse::<f32>() {
            Ok(x) if x.is_finite() => Ok(x),
            _ => bail!("line {}: bad number `{t}`", self.line),
        }
    }

    fn maybe(&mut self) -> Result<Option<f32>> {
        let t = self.next()?;
        match t.parse::<f32>() {
            _ if t == "-" => Ok(None),
            Ok(x) if x.is_finite() => Ok(Some(x)),
            _ => bail!("line {}: bad number `{t}`", self.line),
        }
    }

    fn three(&mut self) -> Result<[f32; 3]> {
        Ok([self.num()?, self.num()?, self.num()?])
    }

    fn key(&mut self, want: &str) -> Result<()> {
        let t = self.next()?;
        if t != want {
            bail!("line {}: expected `{want}` in `{}`, got `{t}`", self.line, self.word);
        }
        Ok(())
    }

    fn material(&mut self) -> Result<Material> {
        let t = self.next()?;
        Material::ALL.into_iter().find(|m| m.name() == t).ok_or_else(|| wasmtime::format_err!("line {}: unknown material `{t}`", self.line))
    }

    fn end(&mut self) -> Result<()> {
        match self.rest.next() {
            Some(t) => bail!("line {}: trailing `{t}` in `{}`", self.line, self.word),
            None => Ok(()),
        }
    }
}

const UP: [f32; 3] = [0.0, 1.0, 0.0];
const DOWN: [f32; 3] = [0.0, -1.0, 0.0];

/// The opening `probe` points into, at or beside, if there is one.
fn cavity(sight: &mut Sight, probe: &SiteProbe) -> Result<Option<Cavity>> {
    let h = probe.size;
    let far = FAR * h;
    let Some((seed, axis)) = inside(sight, probe)? else { return Ok(None) };

    // Out along it to about where it opens, following its bends...
    let walked = outwards(sight, seed, axis, 0.15, 160, 24, far, true)?;
    let Some(last) = walked.last() else { return Ok(None) };
    let back = 0.6 * last.mean();
    let within = walked.iter().rev().find(|r| distance(r.center, last.center) >= back).unwrap_or(&walked[0]);
    // ...then, from a little way in, squarely across it and straight out.
    let mut inner = squared(sight, within, far)?;
    if abs(inner.axis[1]) < 0.5
        && let Some(level) = unit([inner.axis[0], 0.0, inner.axis[2]])
    {
        let m = inner.mean();
        for inset in [0.0f32, 0.25, 0.5, 0.9, 1.4] {
            let p = add(inner.center, scale(level, -inset * m));
            if let Some(ring) = sight.ring(p, level, 24, far)? {
                inner = ring;
                break;
            }
        }
    }
    let axis = inner.axis;
    let straight = outwards(sight, inner.center, axis, 0.06, 160, CONTOUR, far, false)?;
    if straight.is_empty() {
        return Ok(None);
    }
    // The rim: the outermost plane where it has not begun to flare (no more than one in ten of its
    // sides leaning away faster than they go out).
    let flares = |k: usize| {
        let (a, b) = (&straight[k - 1], &straight[k]);
        let give = 0.6 * distance(a.center, b.center) + 0.015 * b.mean();
        (0..CONTOUR).filter(|&i| b.radii[i] - a.radii[i] > give).count() > CONTOUR / 10
    };
    let rim = (1..straight.len()).rev().find(|&k| !flares(k)).unwrap_or(straight.len() - 1);
    let mean = straight[rim].mean();
    let depth_from = |sight: &mut Sight, at: [f32; 3]| -> Result<(f32, bool)> {
        let hit = sight.cast(&[Ray { from: at, along: scale(axis, -1.0), far: 12.0 * h }], false)?[0];
        Ok(hit.map_or((12.0 * h, true), |hit| (hit.t, false)))
    };
    let (deep, _) = depth_from(sight, straight[rim].center)?;
    // A little in from there, so there is wall to fit into, and about its own middle.
    let inset = min(0.08 * mean, 0.15 * deep);
    let at = add(straight[rim].center, scale(axis, -inset));
    let Some(ring) = sight.ring(at, axis, CONTOUR, far)? else { return Ok(None) };
    let ring = match sight.ring(ring.centroid(), axis, CONTOUR, far)? {
        Some(centered) => centered,
        None => ring,
    };
    let mean = ring.mean();
    let (depth, through) = depth_from(sight, ring.center)?;
    let upright = abs(axis[1]) < 1e-4;
    if mean < 0.08 * h || mean > 0.9 * far || depth < if upright { 0.3 } else { 0.6 } * mean {
        return Ok(None);
    }
    // How much wall there is beyond the outline.
    let (right, up) = basis(axis);
    let steps = 12usize;
    let points: Vec<[f32; 3]> = (0..CONTOUR)
        .flat_map(|k| (1..=steps).map(move |j| (k, j)))
        .map(|(k, j)| add(ring.center, scale(spoke(right, up, k, CONTOUR), ring.radii[k] + 0.05 * mean * j as f32)))
        .collect();
    let answers = sight.ask(&points)?.to_vec();
    let wall = answers
        .chunks(steps)
        .map(|beyond| {
            let solid = beyond.iter().take_while(|a| a.distance < 0.0 && a.material != Material::Water).count();
            0.05 * mean * solid as f32
        })
        .collect();
    let within = add(ring.center, scale(axis, -min(0.5 * depth, h)));
    let up_down = sight.cast(&[Ray { from: within, along: UP, far: 6.0 * h }, Ray { from: within, along: DOWN, far: 6.0 * h }], false)?;
    let headroom = up_down.iter().map(|hit| hit.map_or(6.0 * h, |hit| hit.t)).sum();
    let material = ring.material();
    let mut found = Cavity { center: ring.center, axis, radii: ring.radii, wall, depth, through, headroom, material, walkable: false };
    found.walkable = passes(sight, &found)?.is_ok();
    Ok(Some(found))
}

/// A point within the opening `probe` points into, at or beside, and roughly the way out from there.
fn inside(sight: &mut Sight, probe: &SiteProbe) -> Result<Option<([f32; 3], [f32; 3])>> {
    let h = probe.size;
    let (target, normal) = (probe.target, probe.normal);
    let back = probe.from.and_then(|f| unit(sub(f, target)));
    let mut seeds = vec![add(target, scale(normal, 0.1 * h)), add(target, scale(normal, 0.3 * h)), target];
    if let Some(b) = back {
        seeds.extend([0.25, 0.6].map(|k| add(target, scale(b, k * h))));
    }
    let answers = sight.ask(&seeds)?.to_vec();
    let mut open: Vec<[f32; 3]> = Vec::new();
    for (&seed, a) in seeds.iter().zip(&answers) {
        if a.distance >= 0.03 * h || a.material == Material::Water {
            open.push(seed);
        }
    }
    // The way out is towards whoever looks in, if they do; else where most rays get out, else away
    // from the surface pointed at. The smallest ring across it is the squarest.
    for ask in 0..3 {
        let mut best: Option<Ring> = None;
        for &seed in &open {
            let axis = match ask {
                0 => back,
                1 => way_out(sight, seed)?,
                _ => Some(normal),
            };
            let Some(axis) = axis else { continue };
            if back.is_some_and(|b| dot(axis, b) < 0.2) {
                continue;
            }
            for axis in [Some(axis), unit([axis[0], 0.0, axis[2]])].into_iter().flatten() {
                if let Some(ring) = sight.ring(seed, axis, 24, FAR * h)?
                    && ring.mean() >= 0.08 * h
                    && best.as_ref().is_none_or(|b| ring.area() < b.area())
                {
                    best = Some(ring);
                }
            }
        }
        if let Some(ring) = best {
            return Ok(Some((ring.center, ring.axis)));
        }
    }
    // Pointed at the wall or flared lip beside it: look across the surface normal and (when aimed
    // obliquely at a curved rim/bark lip) across the viewer's line of sight, nearest first, for
    // where a ray goes well past the surface into an enclosed opening.
    let mut faces: Vec<[f32; 3]> = vec![normal];
    for cand in [back, back.and_then(|b| unit([b[0], 0.0, b[2]]))] {
        if let Some(f) = cand
            && faces.iter().all(|&prev| dot(prev, f) < 0.96)
        {
            faces.push(f);
        }
    }
    for face in faces {
        let (right, up) = basis(face);
        for far in 1..=8 {
            let reach = 0.25 * h * far as f32;
            let over: Vec<[f32; 3]> = (0..16).map(|k| add(target, add(scale(spoke(right, up, k, 16), reach), scale(face, 0.3 * h)))).collect();
            let ray_far = 1.6 * h;
            let rays: Vec<Ray> = over.iter().map(|&from| Ray { from, along: scale(face, -1.0), far: ray_far }).collect();
            let hits = sight.cast(&rays, false)?;
            for (from, hit) in over.iter().zip(&hits) {
                let t = hit.map_or(ray_far, |hit| hit.t);
                if t < 0.6 * h {
                    continue;
                }
                let seed = add(*from, scale(face, -(0.3 * h + 0.5 * (t - 0.3 * h))));
                for axis in [Some(face), unit([face[0], 0.0, face[2]])].into_iter().flatten() {
                    if let Some(ring) = sight.ring(seed, axis, 24, FAR * h)?
                        && ring.mean() >= 0.08 * h
                    {
                        return Ok(Some((seed, axis)));
                    }
                }
            }
        }
    }
    Ok(None)
}

/// The way most rays get out from `at`, or the longest goes if none does.
fn way_out(sight: &mut Sight, at: [f32; 3]) -> Result<Option<[f32; 3]>> {
    let far = 5.0 * sight.size;
    let rays: Vec<Ray> = all_round().into_iter().map(|along| Ray { from: at, along, far }).collect();
    let hits = sight.cast(&rays, false)?;
    let (mut sum, mut longest) = ([0.0f32; 3], (0.0f32, UP));
    for (ray, hit) in rays.iter().zip(&hits) {
        match hit {
            None => sum = add(sum, ray.along),
            Some(hit) if hit.t > longest.0 => longest = (hit.t, ray.along),
            Some(_) => {}
        }
    }
    Ok(unit(sum).or(Some(longest.1)))
}

/// The rings from `start` out along `axis`, a step of `pace` mean radii at a time, as long as there
/// is wall all round and it does not open into somewhere wider; following its middle if `follow`.
#[allow(clippy::too_many_arguments)]
fn outwards(sight: &mut Sight, start: [f32; 3], axis: [f32; 3], pace: f32, steps: usize, n: usize, far: f32, follow: bool) -> Result<Vec<Ring>> {
    let mut rings: Vec<Ring> = Vec::new();
    let (mut at, mut axis) = (start, axis);
    for _ in 0..steps {
        let Some(ring) = sight.ring(at, axis, n, far)? else { break };
        let step = pace * ring.mean();
        if let Some(b) = rings.last() {
            let wider = (0..n).filter(|&k| ring.radii[k] > 1.6 * b.radii[k] + 2.0 * step).count();
            if wider * 3 > n {
                break;
            }
        }
        if follow {
            let middle = ring.centroid();
            if rings.len() >= 4
                && let Some(drift) = unit(sub(middle, rings[rings.len() - 4].center))
                && dot(drift, axis) > 0.5
            {
                axis = unit(add(scale(axis, 0.8), scale(drift, 0.2))).unwrap_or(axis);
            }
            at = add(middle, scale(axis, step));
        } else {
            at = add(at, scale(axis, step));
        }
        rings.push(ring);
    }
    Ok(rings)
}

/// The ring about `ring`'s middle that is squarest across: turned and centered until it is smallest.
fn squared(sight: &mut Sight, ring: &Ring, far: f32) -> Result<Ring> {
    let mut best = ring.clone();
    for lean in [0.35f32, 0.2, 0.1, 0.05, 0.025] {
        for _ in 0..3 {
            let (right, up) = basis(best.axis);
            let mut better = false;
            for side in [right, scale(right, -1.0), up, scale(up, -1.0)] {
                let Some(axis) = unit(add(best.axis, scale(side, lean))) else { continue };
                if dot(axis, ring.axis) > 0.5
                    && let Some(r) = sight.ring(best.center, axis, 24, far)?
                    && r.area() < 0.998 * best.area()
                {
                    (best, better) = (r, true);
                }
            }
            if !better {
                break;
            }
        }
        if let Some(r) = sight.ring(best.centroid(), best.axis, 24, far)? {
            best = r;
        }
    }
    Ok(best)
}

/// Whether a body as tall as whoever looks gets in through `cavity`, upright, from just outside its
/// rim to well within it (or why not): at each step in there is floor to stand on no more than a
/// step up from the last, and nothing solid in the way of it, its whole way in along one line no
/// farther than its radius and a half from the middle.
pub(crate) fn passes(sight: &mut Sight, cavity: &Cavity) -> Result<Verdict> {
    let h = sight.size;
    if !cavity.upright() {
        return Ok(Err("it opens upwards".into()));
    }
    let deepest = min(cavity.depth - (RADIUS + 0.03) * h, 0.6 * h);
    if deepest < 0.32 * h {
        return Ok(Err(format!("it goes only {} in", length(cavity.depth))));
    }
    let (right, _) = cavity.across();
    let body = Body::of_size(h);
    let foot = RADIUS * 0.6 * h;
    let mut why = String::new();
    'lines: for aside in [0.0f32, 0.07, -0.07, 0.14, -0.14, 0.21, -0.21] {
        let line = add(cavity.center, scale(right, aside * h));
        let mut feet: Option<f32> = None;
        let steps = 18;
        for k in 0..=steps {
            let z = 0.2 * h - (0.2 * h + deepest) * k as f32 / steps as f32;
            let p = add(line, scale(cavity.axis, z));
            let top = feet.map_or(p[1], |y| y + 0.98 * STEP * h);
            let rays: Vec<Ray> = [[0.0, 0.0], [foot, 0.0], [-foot, 0.0], [0.0, foot], [0.0, -foot]]
                .map(|[dx, dz]| Ray { from: [p[0] + dx, top, p[2] + dz], along: DOWN, far: 2.0 * h })
                .to_vec();
            let floor = sight.cast(&rays, false)?.into_iter().flatten().filter(|hit| hit.t > 0.0).map(|hit| top - hit.t).fold(f32::NEG_INFINITY, max);
            if floor == f32::NEG_INFINITY {
                if z > 0.0 {
                    continue; // A drop outside is no bar to getting in.
                }
                note(&mut why, aside, || format!("there is no floor {} in from the rim", length(-z)));
                continue 'lines;
            }
            feet = Some(floor);
            let spheres = body.spheres_at([p[0] as f64, floor as f64, p[2] as f64]);
            let touching = body.touching(sight.area, &spheres, 0.95 * body.radius() as f64)?;
            if touching.iter().any(Option::is_some) {
                let place = if z > 0.0 { "outside the rim" } else { "in from the rim" };
                note(&mut why, aside, || format!("there is no room for a body {} tall {} {place}", length(h), length(abs(z))));
                continue 'lines;
            }
        }
        return Ok(Ok(()));
    }
    Ok(Err(why))
}

/// Why the line through the middle is barred is what is said.
fn note(why: &mut String, aside: f32, what: impl Fn() -> String) {
    if aside == 0.0 {
        *why = what();
    }
}

/// How the ground lies about a site's frame, and the nearest water seen on it.
fn ground(sight: &mut Sight, frame: PlacementTransform, opening: bool, overhead: Option<f32>) -> Result<(Ground, Option<SiteWater>)> {
    let h = sight.size;
    let step = GRID_STEP * h;
    let z0 = if opening { 0.25 * h } else { -3.0 * step };
    let (right, out) = (frame.local_to_world_dir([1.0, 0.0, 0.0]), frame.local_to_world_dir([0.0, 0.0, 1.0]));
    let level = |v: [f32; 3]| unit([v[0], 0.0, v[2]]);
    // A hatch's frame leans back; its ground is measured about its edge all the same.
    let (right, out) = (level(right).unwrap_or([1.0, 0.0, 0.0]), level(out).unwrap_or([0.0, 0.0, 1.0]));
    // From as high up as there is nothing over it, but not through a roof.
    let top = frame.pos[1] + overhead.map_or(3.0 * h, |o| min(0.8 * o, 3.0 * h));
    let at = |x: f32, z: f32| add([frame.pos[0], top, frame.pos[2]], add(scale(right, x), scale(out, z)));
    let mut from: Vec<[f32; 3]> = Vec::new();
    for row in 0..GRID {
        for col in 0..GRID {
            from.push(at(step * (col as f32 - (GRID / 2) as f32), z0 + step * row as f32));
        }
    }
    // Farther off, only for water.
    for far in [2.5f32, 3.5, 4.5, 6.0] {
        for k in 0..16 {
            let [c, s] = turn(k, 16);
            from.push(at(far * h * c, far * h * s));
        }
    }
    let rays: Vec<Ray> = from.iter().map(|&from| Ray { from, along: DOWN, far: 9.0 * h }).collect();
    let wet = sight.cast(&rays, true)?;
    let dry = sight.cast(&rays, false)?;
    let mut water: Option<SiteWater> = None;
    let mut deepest = 0.0f32;
    for (k, (w, d)) in wet.iter().zip(&dry).enumerate() {
        let Some(w) = w.filter(|w| w.material == Material::Water && w.t > 0.0) else { continue };
        deepest = max(deepest, d.map_or(9.0 * h - w.t, |d| d.t - w.t));
        let across = [from[k][0] - frame.pos[0], from[k][2] - frame.pos[2]];
        let far = (across[0] * across[0] + across[1] * across[1]).sqrt();
        if water.is_none_or(|best| far < best.distance) {
            let toward = if far > 1e-6 { [across[0] / far, across[1] / far] } else { [0.0, 0.0] };
            water = Some(SiteWater { level: top - w.t, distance: far, toward, depth: 0.0 });
        }
    }
    if let Some(w) = &mut water {
        w.depth = deepest;
    }
    let inv = 1.0 / frame.scale;
    let heights: Vec<Option<f32>> = dry[..GRID * GRID].iter().map(|d| d.filter(|d| d.t > 0.0).map(|d| (top - d.t - frame.pos[1]) * inv)).collect();
    // Its slope: the mean of what it rises from each point of the grid to the next.
    let slope = |next: usize, across: bool| {
        let (mut sum, mut n) = (0.0f32, 0);
        for k in 0..GRID * GRID {
            if if across { k % GRID + 1 < GRID } else { k + GRID < GRID * GRID }
                && let (Some(a), Some(b)) = (heights[k], heights[k + next])
            {
                sum += b - a;
                n += 1;
            }
        }
        if n > 0 { sum / (n as f32 * GRID_STEP * PERSON) } else { 0.0 }
    };
    let (sx, sz) = (slope(1, true), slope(GRID, false));
    let rise = (sx * sx + sz * sz).sqrt();
    let uphill = if rise > 0.01 { [0, 2].map(|a| (right[a] * sx + out[a] * sz) / rise) } else { [0.0; 2] };
    let elevation = heights[if opening { GRID / 2 } else { GRID * GRID / 2 }].map_or(frame.pos[1], |y| frame.pos[1] + y * frame.scale);
    Ok((Ground { elevation, rise, uphill, step: step * inv, z0: z0 * inv, heights }, water))
}

/// What the surfaces about a site are made of, seen from just off it and from inside its opening.
fn materials(sight: &mut Sight, probe: &SiteProbe, cavity: Option<&Cavity>) -> Result<Vec<(Material, f32)>> {
    let h = probe.size;
    let mut eyes = vec![add(probe.target, scale(probe.normal, 0.3 * h))];
    if let Some(c) = cavity {
        eyes = vec![add(c.center, scale(c.axis, 0.6 * h)), c.center];
    }
    let rays: Vec<Ray> = eyes.iter().flat_map(|&from| all_round().into_iter().map(move |along| Ray { from, along, far: FAR * h })).collect();
    let mut tally = [0usize; Material::ALL.len()];
    for hit in sight.cast(&rays, true)?.into_iter().flatten() {
        tally[hit.material as usize] += 1;
    }
    let total: usize = tally.iter().sum();
    let mut shares: Vec<(Material, f32)> =
        Material::ALL.into_iter().filter(|&m| tally[m as usize] > 0).map(|m| (m, tally[m as usize] as f32 / total as f32)).collect();
    shares.sort_by(|a, b| b.1.total_cmp(&a.1).then((a.0 as u8).cmp(&(b.0 as u8))));
    Ok(shares)
}

/// Directions all round (length one): the middles of a 5 by 5 grid on each face of a cube.
fn all_round() -> Vec<[f32; 3]> {
    let mut ways = Vec::with_capacity(150);
    for a in 0..3 {
        for side in [-1.0f32, 1.0] {
            for i in -2..=2 {
                for j in -2..=2 {
                    let mut v = [0.0f32; 3];
                    v[a] = side;
                    v[(a + 1) % 3] = i as f32 * 0.4;
                    v[(a + 2) % 3] = j as f32 * 0.4;
                    ways.extend(unit(v));
                }
            }
        }
    }
    ways
}

/// Right and up across `axis` (which comes towards whoever looks): up as upright as it can be; for
/// a level opening in the ground, north.
pub(crate) fn basis(axis: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let lead = if abs(axis[1]) < 0.9 { UP } else { [0.0, 0.0, -1.0] };
    let right = unit(cross(lead, axis)).unwrap_or([1.0, 0.0, 0.0]);
    (right, cross(axis, right))
}

/// The cosine and sine of `k` `n`ths of a turn.
fn turn(k: usize, n: usize) -> [f32; 2] {
    let a = std::f32::consts::TAU * k as f32 / n as f32;
    [cos_f32(a), sin_f32(a)]
}

fn spoke(right: [f32; 3], up: [f32; 3], k: usize, n: usize) -> [f32; 3] {
    let [c, s] = turn(k, n);
    add(scale(right, c), scale(up, s))
}

/// The turn taking +X, +Y and +Z to `x`, `y` and `z` (length one, square to each other), as a unit
/// quaternion, by square roots alone.
pub(crate) fn quat(x: [f32; 3], y: [f32; 3], z: [f32; 3]) -> [f32; 4] {
    let trace = x[0] + y[1] + z[2];
    let q = if trace > 0.0 {
        let s = (trace + 1.0).sqrt() * 2.0;
        [(y[2] - z[1]) / s, (z[0] - x[2]) / s, (x[1] - y[0]) / s, 0.25 * s]
    } else if x[0] > y[1] && x[0] > z[2] {
        let s = (1.0 + x[0] - y[1] - z[2]).sqrt() * 2.0;
        [0.25 * s, (y[0] + x[1]) / s, (z[0] + x[2]) / s, (y[2] - z[1]) / s]
    } else if y[1] > z[2] {
        let s = (1.0 + y[1] - x[0] - z[2]).sqrt() * 2.0;
        [(y[0] + x[1]) / s, 0.25 * s, (z[1] + y[2]) / s, (z[0] - x[2]) / s]
    } else {
        let s = (1.0 + z[2] - x[0] - y[1]).sqrt() * 2.0;
        [(z[0] + x[2]) / s, (z[1] + y[2]) / s, 0.25 * s, (x[1] - y[0]) / s]
    };
    crate::compose::quat_normalize(q)
}

#[inline]
pub(crate) fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

#[inline]
pub(crate) fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

#[inline]
pub(crate) fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}

#[inline]
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

#[inline]
fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub(a, b);
    dot(d, d).sqrt()
}

/// Length one, or nothing if it is too short to have a direction.
#[inline]
pub(crate) fn unit(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = dot(a, a).sqrt();
    (l > 1e-6).then(|| scale(a, 1.0 / l))
}

#[inline]
pub(crate) fn min(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
pub(crate) fn max(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
pub(crate) fn abs(a: f32) -> f32 {
    if a < 0.0 { -a } else { a }
}

#[inline]
fn clamp(a: f32, lo: f32, hi: f32) -> f32 {
    min(max(a, lo), hi)
}
