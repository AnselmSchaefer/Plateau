//! Standalone door in a frame (`elements/hobbit-door/element.rs`), round unless made for an opening.
//!
//! **Given a site's params** (`plateau:region@0.2.0`, `make/ELEMENT_GUIDE.md` "Params"), it makes
//! itself for that opening, whatever its shape: its frame follows the outline the params give, a
//! little into the wall all round, and its leaf fills it. Placed where the site is
//! (`plateau_engine::fit::fit_to_site`), it fits as it is, and fits again when the opening changes
//! and it is given the new one's.
//!
//! **Given none**, it is round, made for no opening in particular, to fit any round one
//! (`plateau_engine::fit::fit_transform` turns and scales it until its frame goes a little into the
//! wall all round): the frame's outer edge is a circle about `MIDDLE`.
//!
//! The rim's plane is z = 0, outside is +Z, the sill at the origin. The green leaf hangs from a hinge
//! at the left of the frame and swings away from whoever opens it with `F`; a small lamp inside is
//! lit while it stands open.
//!
//! Pure procedural `Scene` (`sdf_bounds() == None`): the frame is static and solid, the leaf (`id = 1`)
//! solid but while it swings (so it never shoves anyone), its planks, straps and knob (`id = 2`) and
//! the stones round the frame not solid at all.

use plateau_sdk::*;

/// The middle of the round opening it is made for given no params: its radius above the sill.
const MIDDLE: f32 = 1.05;
/// How far the frame goes beyond the outline into the wall, and in from it, and how far the leaf
/// reaches in from it: for the round door, its outer and inner radius are 1.13 and 0.95, its leaf's
/// 0.985. All are for an opening of `MIDDLE`'s radius, and grow and shrink with the opening's.
const BEYOND: f32 = 0.08;
const WITHIN_RIM: f32 = 0.10;
const LEAF_IN: f32 = 0.065;
/// How far the frame goes in and out of the rim's plane.
const WITHIN: f32 = -0.20;
const OUT: f32 = 0.12;
/// The leaf's half thickness.
const THICK: f32 = 0.045;
/// How far it swings, in radians, and how long it takes, in seconds.
const SWING: f32 = 1.75;
const SECONDS: f32 = 1.1;
const SIDES: usize = 48;
/// The site's params this door reads (`plateau_engine::site`'s `P_*`): whether there is an opening,
/// how high its middle is above the sill, how deep it goes, and the first of `SPOKES` radii from
/// the middle to the outline, anticlockwise from straight right.
const P_OPENING: usize = 0;
const P_MIDDLE: usize = 3;
const P_DEPTH: usize = 4;
const P_RADII: usize = 10;
const SPOKES: usize = 48;

/// The opening the door is made for: how high its middle is above the sill, and how far its outline
/// is from the middle at each of `SPOKES` angles.
#[derive(Clone, Copy)]
struct Opening {
    middle: f32,
    radii: [f32; SPOKES],
    depth: f32,
    /// Made for no opening in particular.
    round: bool,
}

impl Opening {
    fn round() -> Opening {
        Opening { middle: MIDDLE, radii: [MIDDLE; SPOKES], depth: 2.0, round: true }
    }

    /// The opening `params` give, or the round one if they give none (or none it can use).
    fn of(params: &[f32]) -> Opening {
        if params.len() < P_RADII + SPOKES || param(params, P_OPENING, 0.0) != 1.0 {
            return Opening::round();
        }
        let mut radii = [0.0f32; SPOKES];
        for (k, r) in radii.iter_mut().enumerate() {
            *r = params[P_RADII + k];
        }
        let middle = params[P_MIDDLE];
        let sane = |v: f32| v > 0.05 && v < 50.0;
        if !radii.iter().all(|&r| sane(r)) || !sane(middle) {
            return Opening::round();
        }
        Opening { middle, radii, depth: max(params[P_DEPTH], 0.1), round: false }
    }

    /// How large it is against the round one: what every measure across it is scaled by.
    fn scale(&self) -> f32 {
        self.radii.iter().sum::<f32>() / (SPOKES as f32 * MIDDLE)
    }

    /// How far the outline is from the middle `k` `SIDES`ths of a turn anticlockwise from the
    /// right, between the spokes it lies between.
    fn at(&self, k: usize) -> f32 {
        let k = k % SIDES;
        let f = (k * SPOKES) as f32 / SIDES as f32;
        let i = f.floor() as usize;
        let t = f - i as f32;
        self.radii[i % SPOKES] * (1.0 - t) + self.radii[(i + 1) % SPOKES] * t
    }

    fn middle(&self) -> Point {
        point(0.0, self.middle, 0.0)
    }
}

struct HobbitDoor {
    seed: u64,
    opening: Opening,
    /// Where the leaf hangs.
    hinge: Point,
    /// Whether it is to be open, how far it is (0 to 1), and which way (1 inwards, -1 outwards).
    open: bool,
    done: f32,
    way: f32,
    moving: bool,
}

/// A flat-shaded quad, whichever way its corners go round, facing `out`.
fn face(m: &mut MeshBuilder, [a, b, c, d]: [Point; 4], out: Point, material: Material) {
    let [a, b, c, d] = if (b - a).cross(c - a).dot(out) < 0.0 { [a, d, c, b] } else { [a, b, c, d] };
    let [a, b, c, d] = [a, b, c, d].map(|p| m.vertex(p, out, material));
    m.quad(a, b, c, d);
}

/// The way `k` `SIDES`ths of a turn anticlockwise from the right.
fn way(k: usize) -> (f32, f32) {
    let a = TAU * (k % SIDES) as f32 / SIDES as f32;
    (cos(a), sin(a))
}

/// A ring about `c`, between the radii `inner(k)` and `outer(k)` at each side `k` and two depths,
/// from side `first` for `count` sides, its ends closed if `ends`.
#[allow(clippy::too_many_arguments)]
fn ring(
    m: &mut MeshBuilder,
    c: Point,
    inner: &dyn Fn(usize) -> f32,
    outer: &dyn Fn(usize) -> f32,
    z0: f32,
    z1: f32,
    first: usize,
    count: usize,
    ends: bool,
    material: Material,
) {
    let at = |k: usize, r: f32, z: f32| {
        let (x, y) = way(k);
        c + point(r * x, r * y, z)
    };
    for k in first..first + count {
        let a = TAU * (k as f32 + 0.5) / SIDES as f32;
        let (n, front) = (point(cos(a), sin(a), 0.0), point(0.0, 0.0, 1.0));
        let (i0, i1, o0, o1) = (inner(k), inner(k + 1), outer(k), outer(k + 1));
        face(m, [at(k, o0, z0), at(k + 1, o1, z0), at(k + 1, o1, z1), at(k, o0, z1)], n, material);
        face(m, [at(k, i0, z0), at(k + 1, i1, z0), at(k + 1, i1, z1), at(k, i0, z1)], -n, material);
        face(m, [at(k, i0, z1), at(k + 1, i1, z1), at(k + 1, o1, z1), at(k, o0, z1)], front, material);
        face(m, [at(k, i0, z0), at(k + 1, i1, z0), at(k + 1, o1, z0), at(k, o0, z0)], -front, material);
    }
    if ends {
        for (k, side) in [(first, -1.0), (first + count, 1.0)] {
            let (x, y) = way(k);
            let n = point(-y, x, 0.0) * side;
            face(m, [at(k, inner(k), z0), at(k, outer(k), z0), at(k, outer(k), z1), at(k, inner(k), z1)], n, material);
        }
    }
}

/// A closed slab about `c`, `r(k)` from it at each side `k`.
fn disc(m: &mut MeshBuilder, c: Point, r: &dyn Fn(usize) -> f32, half: f32, material: Material) {
    let at = |k: usize, z: f32| {
        let (x, y) = way(k);
        c + point(r(k) * x, r(k) * y, z)
    };
    for k in 0..SIDES {
        let a = TAU * (k as f32 + 0.5) / SIDES as f32;
        face(m, [at(k, -half), at(k + 1, -half), at(k + 1, half), at(k, half)], point(cos(a), sin(a), 0.0), material);
        m.flat_tri(c + point(0.0, 0.0, half), at(k, half), at(k + 1, half), material);
        m.flat_tri(c - point(0.0, 0.0, half), at(k + 1, -half), at(k, -half), material);
    }
}

impl HobbitDoor {
    fn turn(&self) -> Quat {
        let t = self.done * self.done * (3.0 - 2.0 * self.done);
        Quat::from_y(self.way * SWING * t)
    }

    fn leaf(&self) -> Vec<Command> {
        let shut_or_open = !self.moving;
        vec![
            Command::SetTransform(instance(1, 1, self.hinge, self.turn(), point(1.0, 1.0, 1.0), shut_or_open)),
            Command::SetTransform(instance(2, 2, self.hinge, self.turn(), point(1.0, 1.0, 1.0), false)),
        ]
    }

    fn lamp(&self) -> Command {
        let lit = if self.open { 46.0 } else { 0.0 };
        let o = &self.opening;
        let at = point(0.0, o.middle + 0.35 * o.scale(), -min(1.1, 0.55 * o.depth));
        Command::SetLight(light(1, at, [1.0, 0.76, 0.42], lit, 6.0))
    }
}

impl Element for HobbitDoor {
    fn new(seed: u64) -> Self {
        let opening = Opening::of(&params());
        let k = opening.scale();
        let hinge = point(-(opening.at(SIDES / 2) - WITHIN_RIM * k), opening.middle, 0.0);
        HobbitDoor { seed, opening, hinge, open: false, done: 0.0, way: 1.0, moving: false }
    }

    fn name(&self) -> &str {
        "Hobbit Door"
    }

    fn scene(&self) -> Scene {
        let o = self.opening;
        let k = o.scale();
        let middle = o.middle();
        let outer = |i: usize| o.at(i) + BEYOND * k;
        let inner = |i: usize| o.at(i) - WITHIN_RIM * k;
        let leaf_r = |i: usize| o.at(i) - LEAF_IN * k;
        let mut s = Seeds::of(self.seed, 17);
        // Mesh 0: the frame, and the sill stone it stands on, sunk into the floor. Static, solid.
        let mut frame = MeshBuilder::new();
        for q in 0..12 {
            let shade = s.next(-12.0, 12.0);
            frame.tint((172.0 + shade) as u8, (146.0 + shade) as u8, (118.0 + shade) as u8);
            ring(&mut frame, middle, &inner, &outer, WITHIN, OUT, q * 4, 4, false, Material::Wood);
        }
        frame.tint(176, 172, 166);
        frame.cuboid(point(0.0, -0.14, -0.03), point(0.34 * k, 0.10, 0.14), Material::Stone);

        // Mesh 1: the leaf, about its hinge (so it turns there).
        let centre = middle - self.hinge;
        let mut leaf = MeshBuilder::new();
        leaf.tint(88, 236, 128);
        disc(&mut leaf, centre, &leaf_r, THICK, Material::Wood);

        // Mesh 2: its trim: plank joints, a rail round its edge, strap hinges, and the knob in its
        // very middle, on both sides.
        let (top, bottom) = (leaf_r(SIDES / 4), leaf_r(3 * SIDES / 4));
        let (left, right) = (leaf_r(SIDES / 2), leaf_r(0));
        let mut trim = MeshBuilder::new();
        for side in [-1.0f32, 1.0] {
            let z = side * THICK;
            trim.tint(48, 120, 66);
            for j in -3i32..=3 {
                let x = j as f32 * 0.235 * k;
                let across = if x < 0.0 { left } else { right };
                // Within an ellipse a little inside the leaf: a circle of 0.82 in the round door.
                let q = 0.82 / 0.985;
                let reach = 1.0 - (x / (q * across)) * (x / (q * across));
                if reach <= 0.0 {
                    continue;
                }
                let (up, down) = (q * top * reach.sqrt(), q * bottom * reach.sqrt());
                let mid = 0.5 * (up - down);
                trim.cuboid(centre + point(x, mid, z), point(0.010, 0.5 * (up + down), 0.006), Material::Wood);
            }
            trim.tint(70, 190, 100);
            let (z0, z1) = if side > 0.0 { (z - 0.01, z + 0.022) } else { (z - 0.022, z + 0.01) };
            ring(&mut trim, centre, &|i| leaf_r(i) - 0.185 * k, &|i| leaf_r(i) - 0.055 * k, z0, z1, 0, SIDES, false, Material::Wood);
            trim.tint(96, 96, 104);
            for y in [-0.42f32, 0.42] {
                trim.cuboid(point(0.52 * k, y * k, z + side * 0.018), point(0.40 * k, 0.040 * k, 0.012), Material::Stone);
            }
            trim.tint(255, 214, 96);
            trim.ellipsoid(centre + point(0.0, 0.0, z + side * 0.07), point(0.075, 0.075, 0.06), 12, 8, Material::Sand);
            trim.sweep_tube(&[(centre + point(0.0, 0.0, z), 0.03), (centre + point(0.0, 0.0, z + side * 0.06), 0.03)], 8, true, Material::Sand);
        }

        // Mesh 3: dressed stones round the frame, against the wall's face; only to look at.
        let mut stones = MeshBuilder::new();
        for q in 0..16 {
            let shade = s.next(-16.0, 16.0);
            stones.tint((204.0 + shade) as u8, (200.0 + shade) as u8, (190.0 + shade) as u8);
            let out = s.next(0.115, 0.15);
            let wide = s.next(0.09, 0.14);
            ring(&mut stones, middle, &|i| outer(i) - 0.03 * k, &|i| outer(i) + wide * k, WITHIN * 0.3, out, q * 3, 3, true, Material::Stone);
        }

        let one = point(1.0, 1.0, 1.0);
        let mut sc = SceneBuilder::new();
        let frame = sc.add_mesh(frame.build());
        let leaf = sc.add_mesh(leaf.build());
        let trim = sc.add_mesh(trim.build());
        let stones = sc.add_mesh(stones.build());
        sc.place(frame, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, true);
        sc.place(stones, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, false);
        sc.spawn(1, leaf, self.hinge, Quat::IDENTITY, one, true);
        sc.spawn(2, trim, self.hinge, Quat::IDENTITY, one, false);
        sc.build()
    }

    fn tick(&mut self, dt: f32, _p: &PlayerState) -> Vec<Command> {
        if !self.moving {
            return Vec::new();
        }
        let step = dt / SECONDS;
        self.done = if self.open { min(self.done + step, 1.0) } else { max(self.done - step, 0.0) };
        self.moving = if self.open { self.done < 1.0 } else { self.done > 0.0 };
        self.leaf()
    }

    fn interact(&mut self, p: &PlayerState) -> Vec<Command> {
        let aimed = p.target == 1 || p.target == 2;
        let near = p.feet.length() < 5.0 * self.opening.scale();
        if !aimed && !near {
            return Vec::new();
        }
        if self.open && abs(p.feet.z) < THICK + 0.16 * p.size && abs(p.feet.x) < -self.hinge.x {
            return vec![Command::Say("You are standing in the doorway.".into())];
        }
        self.open = !self.open;
        if self.open && self.done <= 0.0 {
            // Away from whoever opens it.
            self.way = if p.feet.z >= 0.0 { 1.0 } else { -1.0 };
        }
        self.moving = true;
        let mut commands = self.leaf();
        commands.push(self.lamp());
        let door = if self.opening.round { "round door" } else { "door" };
        commands.push(Command::Say(format!("The {door} swings {}.", if self.open { "open" } else { "shut" })));
        commands
    }
}

element!(HobbitDoor);
