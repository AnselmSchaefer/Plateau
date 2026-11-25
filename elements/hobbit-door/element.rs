//! Standalone round door in a round frame (`elements/hobbit-door/element.rs`).
//!
//! Made for no opening in particular, to fit any round one (`plateau_engine::fit::fit_transform`
//! turns and scales it until its frame goes a little into the wall all round): the frame's outer edge
//! is a circle about `MIDDLE`, the rim's plane is z = 0, outside is +Z. The green leaf hangs from a
//! hinge at the left of the frame and swings away from whoever opens it with `F`; a small lamp
//! inside is lit while it stands open.
//!
//! Pure procedural `Scene` (`sdf_bounds() == None`): the frame is static and solid, the leaf (`id = 1`)
//! solid but while it swings (so it never shoves anyone), its planks, straps and knob (`id = 2`) and
//! the stones round the frame not solid at all.

use plateau_sdk::*;

/// The middle of the opening: on the opening's axis, its radius above the sill.
const MIDDLE: Point = Point { x: 0.0, y: 1.05, z: 0.0 };
/// The frame's outer and inner radius, and how far it goes in and out of the rim's plane.
const OUTER: f32 = 1.13;
const INNER: f32 = 0.95;
const WITHIN: f32 = -0.20;
const OUT: f32 = 0.12;
/// The leaf: a little wider than the frame's inside, so nothing shows past it, and its half thickness.
const LEAF: f32 = 0.985;
const THICK: f32 = 0.045;
/// Where the leaf hangs.
const HINGE: Point = Point { x: -INNER, y: 1.05, z: 0.0 };
/// How far it swings, in radians, and how long it takes, in seconds.
const SWING: f32 = 1.75;
const SECONDS: f32 = 1.1;
const SIDES: usize = 48;

struct HobbitDoor {
    seed: u64,
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

/// A ring about `c`, between two radii and two depths, from side `first` for `count` sides, its ends
/// closed if `ends`.
#[allow(clippy::too_many_arguments)]
fn ring(m: &mut MeshBuilder, c: Point, inner: f32, outer: f32, z0: f32, z1: f32, first: usize, count: usize, ends: bool, material: Material) {
    let at = |k: usize, r: f32, z: f32| {
        let a = TAU * (k % SIDES) as f32 / SIDES as f32;
        c + point(r * cos(a), r * sin(a), z)
    };
    for k in first..first + count {
        let a = TAU * (k as f32 + 0.5) / SIDES as f32;
        let (n, front) = (point(cos(a), sin(a), 0.0), point(0.0, 0.0, 1.0));
        face(m, [at(k, outer, z0), at(k + 1, outer, z0), at(k + 1, outer, z1), at(k, outer, z1)], n, material);
        face(m, [at(k, inner, z0), at(k + 1, inner, z0), at(k + 1, inner, z1), at(k, inner, z1)], -n, material);
        face(m, [at(k, inner, z1), at(k + 1, inner, z1), at(k + 1, outer, z1), at(k, outer, z1)], front, material);
        face(m, [at(k, inner, z0), at(k + 1, inner, z0), at(k + 1, outer, z0), at(k, outer, z0)], -front, material);
    }
    if ends {
        for (k, way) in [(first, -1.0), (first + count, 1.0)] {
            let a = TAU * (k % SIDES) as f32 / SIDES as f32;
            let n = point(-sin(a), cos(a), 0.0) * way;
            face(m, [at(k, inner, z0), at(k, outer, z0), at(k, outer, z1), at(k, inner, z1)], n, material);
        }
    }
}

/// A closed round slab about `c`.
fn disc(m: &mut MeshBuilder, c: Point, r: f32, half: f32, material: Material) {
    let at = |k: usize, z: f32| {
        let a = TAU * (k % SIDES) as f32 / SIDES as f32;
        c + point(r * cos(a), r * sin(a), z)
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
            Command::SetTransform(instance(1, 1, HINGE, self.turn(), point(1.0, 1.0, 1.0), shut_or_open)),
            Command::SetTransform(instance(2, 2, HINGE, self.turn(), point(1.0, 1.0, 1.0), false)),
        ]
    }

    fn lamp(&self) -> Command {
        let lit = if self.open { 46.0 } else { 0.0 };
        Command::SetLight(light(1, point(0.0, MIDDLE.y + 0.35, -1.1), [1.0, 0.76, 0.42], lit, 6.0))
    }
}

impl Element for HobbitDoor {
    fn new(seed: u64) -> Self {
        HobbitDoor { seed, open: false, done: 0.0, way: 1.0, moving: false }
    }

    fn name(&self) -> &str {
        "Hobbit Door"
    }

    fn scene(&self) -> Scene {
        let mut s = Seeds::of(self.seed, 17);
        // Mesh 0: the frame, and the sill stone it stands on, sunk into the floor. Static, solid.
        let mut frame = MeshBuilder::new();
        for k in 0..12 {
            let shade = s.next(-12.0, 12.0);
            frame.tint((172.0 + shade) as u8, (146.0 + shade) as u8, (118.0 + shade) as u8);
            ring(&mut frame, MIDDLE, INNER, OUTER, WITHIN, OUT, k * 4, 4, false, Material::Wood);
        }
        frame.tint(176, 172, 166);
        frame.cuboid(point(0.0, -0.14, -0.03), point(0.34, 0.10, 0.14), Material::Stone);

        // Mesh 1: the leaf, about its hinge (so it turns there).
        let middle = point(INNER, 0.0, 0.0);
        let mut leaf = MeshBuilder::new();
        leaf.tint(88, 236, 128);
        disc(&mut leaf, middle, LEAF, THICK, Material::Wood);

        // Mesh 2: its trim: plank joints, a rail round its edge, strap hinges, and the knob in its
        // very middle, on both sides.
        let mut trim = MeshBuilder::new();
        for side in [-1.0f32, 1.0] {
            let z = side * THICK;
            trim.tint(48, 120, 66);
            for k in -3i32..=3 {
                let x = k as f32 * 0.235;
                let half = (0.82f32 * 0.82 - x * x).sqrt();
                trim.cuboid(middle + point(x, 0.0, z), point(0.010, half, 0.006), Material::Wood);
            }
            trim.tint(70, 190, 100);
            let (z0, z1) = if side > 0.0 { (z - 0.01, z + 0.022) } else { (z - 0.022, z + 0.01) };
            ring(&mut trim, middle, 0.80, 0.93, z0, z1, 0, SIDES, false, Material::Wood);
            trim.tint(96, 96, 104);
            for y in [-0.42f32, 0.42] {
                trim.cuboid(point(0.52, y, z + side * 0.018), point(0.40, 0.040, 0.012), Material::Stone);
            }
            trim.tint(255, 214, 96);
            trim.ellipsoid(middle + point(0.0, 0.0, z + side * 0.07), point(0.075, 0.075, 0.06), 12, 8, Material::Sand);
            trim.sweep_tube(&[(middle + point(0.0, 0.0, z), 0.03), (middle + point(0.0, 0.0, z + side * 0.06), 0.03)], 8, true, Material::Sand);
        }

        // Mesh 3: dressed stones round the frame, against the wall's face; only to look at.
        let mut stones = MeshBuilder::new();
        for k in 0..16 {
            let shade = s.next(-16.0, 16.0);
            stones.tint((204.0 + shade) as u8, (200.0 + shade) as u8, (190.0 + shade) as u8);
            let out = s.next(0.115, 0.15);
            ring(&mut stones, MIDDLE, OUTER - 0.03, OUTER + s.next(0.09, 0.14), WITHIN * 0.3, out, k * 3, 3, true, Material::Stone);
        }

        let one = point(1.0, 1.0, 1.0);
        let mut sc = SceneBuilder::new();
        let frame = sc.add_mesh(frame.build());
        let leaf = sc.add_mesh(leaf.build());
        let trim = sc.add_mesh(trim.build());
        let stones = sc.add_mesh(stones.build());
        sc.place(frame, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, true);
        sc.place(stones, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, false);
        sc.spawn(1, leaf, HINGE, Quat::IDENTITY, one, true);
        sc.spawn(2, trim, HINGE, Quat::IDENTITY, one, false);
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
        let near = p.feet.length() < 5.0;
        if !aimed && !near {
            return Vec::new();
        }
        if self.open && abs(p.feet.z) < THICK + 0.16 * p.size && abs(p.feet.x) < INNER {
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
        commands.push(Command::Say(if self.open { "The round door swings open.".into() } else { "The round door swings shut.".into() }));
        commands
    }
}

element!(HobbitDoor);
