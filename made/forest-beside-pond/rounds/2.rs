//! A forest of squat old trees. Giants have sat on them for so long that their crowns have grown
//! flat and broad: each is a stool. At the foot of each, between two roots, is a door two
//! centimetres wide, and behind it a house for the tiny: a hall with a hearth, a stair to a loft
//! with a bed, a store room behind.
//!
//! - **The tiny** find the doors by their lanterns and the dark stone set in moss about them.
//! - **A person** walks the paths beneath the crowns, over the log across the pool, and into the
//!   hollow of the grandmother tree.
//! - **A giant** sees a meadow of stools.

use plateau_sdk::*;

const WEST_GROUND: [f32; 65] = [
    1.153, 1.174, 1.167, 1.232, 1.147, 1.062, 0.989, 0.911, 0.884, 0.892, 0.926,
    0.941, 0.948, 0.909, 0.924, 0.899, 1.016, 1.071, 1.131, 1.154, 1.203, 1.223,
    1.254, 1.290, 1.380, 1.291, 1.303, 1.377, 1.294, 1.215, 1.163, 1.109, 1.063,
    1.074, 0.957, 0.894, 0.822, 0.845, 0.825, 0.825, 0.928, 0.963, 0.987, 0.966,
    0.979, 1.041, 1.046, 1.056, 1.010, 0.981, 0.975, 1.025, 0.986, 0.981, 0.960,
    1.029, 1.038, 1.063, 1.080, 1.062, 1.056, 1.041, 1.063, 1.017, 1.038,
];

/// The grandmother tree.
const OLD_X: f32 = 33.0;
const OLD_Z: f32 = 22.0;
const OLD_CROWN: [(Point, f32); 5] = [
    (point(0.0, 11.5, 0.0), 3.4),
    (point(2.6, 10.4, 0.6), 2.5),
    (point(-2.4, 10.6, 1.2), 2.5),
    (point(0.4, 10.2, -2.6), 2.4),
    (point(0.2, 10.1, 2.5), 2.3),
];

const POOL: Point = point(48.0, 0.0, 42.0);
const POOL_GROUND: f32 = 0.45;
const LEVEL: f32 = 0.15;
const POOL_REACH: f32 = 4.2;

const LOG_A: Point = point(44.2, 0.42, 40.5);
const LOG_B: Point = point(51.8, 0.42, 43.2);
const LOG_R: f32 = 0.36;

const COUNT: usize = 26;
const SPOTS: [(f32, f32); COUNT] = [
    (10.0, 9.0), (18.5, 7.0), (27.0, 9.0), (40.0, 8.0), (49.0, 9.5), (57.5, 8.0),
    (13.0, 18.0), (22.0, 17.0), (43.5, 17.5), (53.0, 19.0),
    (9.5, 27.0), (18.0, 26.5), (45.0, 27.0), (56.5, 29.0),
    (13.0, 35.0), (23.0, 36.0), (38.5, 36.0), (57.5, 38.5),
    (20.0, 49.5), (11.0, 47.0), (37.5, 47.0),
    (10.0, 56.0), (21.0, 58.0), (43.0, 55.0), (52.0, 52.0), (56.5, 58.5),
];
/// The trees of the named places, by their number in `SPOTS`.
const GREAT: usize = 12;
const LOFT: usize = 15;
const HEARTH: usize = 20;

const PATH: [(Point, Point); 6] = [
    (point(32.0, 0.0, 66.0), point(32.0, 0.0, 56.0)),
    (point(32.0, 0.0, 56.0), point(29.0, 0.0, 46.0)),
    (point(29.0, 0.0, 46.0), point(33.0, 0.0, 36.0)),
    (point(33.0, 0.0, 36.0), point(33.0, 0.0, 22.0)),
    (point(29.0, 0.0, 46.0), point(8.0, 0.0, 40.0)),
    (point(33.0, 0.0, 41.0), point(44.0, 0.0, 40.6)),
];

const BOULDERS: [(f32, f32, f32); 6] =
    [(25.5, 52.0, 0.8), (41.0, 42.5, 0.7), (28.0, 15.0, 1.0), (52.0, 34.5, 0.9), (15.5, 12.5, 0.8), (37.8, 30.0, 0.9)];

/// The trunk's flare at its foot, and how far in from the foot's face the hall's middle is.
const FLARE: f32 = 0.3;
const HALL_IN: f32 = 0.22;
/// Half the crown's thickness.
const CROWN_HALF: f32 = 0.875;
const LOFT_TOP: f32 = 0.042;

const DOOR_MESH: u32 = 0;
const FLY_MESH: u32 = 1;
const FLIES: u32 = 6;

struct Tree {
    /// The foot of the trunk's middle.
    at: Point,
    /// Out through the door, across the ground; `yaw` is its angle.
    dir: Point,
    yaw: f32,
    /// How far in front of the trunk's middle the hall's middle is, and that point.
    hall: f32,
    porch: Point,
    top: f32,
    r: f32,
    crown: f32,
    seed: u32,
    /// 1 or -1: the house, or its mirror image.
    hand: f32,
    roots: [(Point, Point, f32); 5],
    limbs: [(Point, Point); 4],
}

impl Tree {
    /// To the tree's own frame: x across its door, z out through it.
    fn local(&self, p: Point) -> Point {
        let q = p - self.at;
        point(q.x * self.dir.z - q.z * self.dir.x, q.y, q.x * self.dir.x + q.z * self.dir.z)
    }

    fn world(&self, l: Point) -> Point {
        self.at + point(self.dir.z * l.x + self.dir.x * l.z, l.y, self.dir.z * l.z - self.dir.x * l.x)
    }

    /// A point of the house, about the middle of its hall's floor.
    fn inside(&self, v: Point) -> Point {
        self.world(point(v.x * self.hand, v.y, v.z + self.hall))
    }

    /// How far out the trunk's face is at a height, in front, where the door is.
    fn face(&self, y: f32) -> f32 {
        let t = clamp(1.0 - y / 1.1, 0.0, 1.0);
        self.r + FLARE * t * t
    }

    /// The trunk, roots and limbs, without bark.
    fn wood(&self, l: Point, view: &View) -> f32 {
        let u = clamp((l.y - (self.top - 2.5)) / 1.4, 0.0, 1.0);
        let rr = self.face(l.y) + 0.3 * u * u;
        let (lo, hi) = (-0.7, self.top - 1.0);
        let col = max(l.flat_length() - rr, abs(l.y - (lo + hi) * 0.5) - (hi - lo) * 0.5) * 0.85;
        let seen = view.sees(0.4);
        if seen == 0.0 {
            return col;
        }
        let mut roots = 1000.0;
        for &(a, b, r) in self.roots.iter() {
            roots = min(roots, capsule(l, a, b, r));
        }
        let mut limbs = 1000.0;
        for &(a, b) in self.limbs.iter() {
            limbs = min(limbs, capsule(l, a, b, 0.15));
        }
        let full = smooth_union(smooth_union(col, roots, 0.35), limbs, 0.2);
        mix(col, full, seen)
    }
}

struct Forest {
    seed: u32,
    old: Point,
    old_roots: [(Point, Point); 6],
    trees: Vec<Tree>,
    boulders: Vec<(Point, f32)>,
    open: Vec<bool>,
    dark: Vec<bool>,
}

/// The ground at a spot: its height, how much of its roughness it has, how far the nearest trunk's
/// middle is, and the tree whose porch it is on, if any.
struct Lay {
    h: f32,
    along: f32,
    near: f32,
    porch: Option<usize>,
}

impl Area for Forest {
    fn new(seed: u64) -> Forest {
        let seed = (seed ^ (seed >> 32)) as u32;
        let none = point(0.0, 0.0, 0.0);
        let mut f = Forest {
            seed,
            old: none,
            old_roots: [(none, none); 6],
            trees: Vec::with_capacity(COUNT),
            boulders: Vec::new(),
            open: vec![false; COUNT],
            dark: vec![false; COUNT],
        };
        f.old = point(OLD_X, f.raw(OLD_X, OLD_Z), OLD_Z);
        for k in 0..6 {
            let a = (k as f32 + 0.5) * TAU / 6.0;
            let d = point(sin(a), 0.0, cos(a));
            f.old_roots[k] = (d * 1.6 + point(0.0, 1.3, 0.0), d * 4.6 + point(0.0, -0.25, 0.0));
        }
        for i in 0..COUNT {
            let t = f.plant(i);
            f.trees.push(t);
        }
        for &(x, z, r) in BOULDERS.iter() {
            let y = f.lay(x, z).h;
            f.boulders.push((point(x, y - 0.3 * r, z), r));
        }
        f
    }

    fn describe(&self) -> Description {
        let great = &self.trees[GREAT];
        Description {
            name: "The forest of stools".into(),
            places: vec![
                Place::new("the hearth behind the root door", Size::Tiny, self.trees[HEARTH].inside(point(0.06, 0.0, -0.03))),
                Place::new("the loft in the bark", Size::Tiny, self.trees[LOFT].inside(point(-0.085, LOFT_TOP, -0.04))),
                Place::new("the hollow of the grandmother tree", Size::Person, self.old + point(0.1, 0.0, 0.3)),
                Place::new("the great stool", Size::Giant, great.at + point(0.0, great.top, 0.0)),
            ],
            borders: Borders::of_ground(|x, z| self.lay(x, z).h),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let mut solid = self.ground(p, view).or(self.grandmother(p, view));
        for t in self.trees.iter() {
            solid = solid.or(self.tree(t, p, view));
        }
        solid = solid.or(self.rocks(p, view)).or(self.log(p, view));
        let water = max(p.y - LEVEL, point(p.x - POOL.x, 0.0, p.z - POOL.z).flat_length() - POOL_REACH);
        let water = Answer::new(water, Material::Water).cut(solid.distance);
        solid.or(water)
    }

    fn scene(&self) -> Scene {
        self.build_scene()
    }

    fn tick(&mut self, _dt: f32, player: &PlayerState) -> Vec<Command> {
        let mut out = Vec::new();
        for k in 0..FLIES {
            let at = self.fly(k, player.time);
            let glow = 0.5 + 0.5 * sin(player.time * 1.7 + k as f32 * 2.1);
            out.push(Command::SetTransform(instance(1 + k, FLY_MESH, at, Quat::IDENTITY, point(1.0, 1.0, 1.0), false)));
            out.push(Command::SetLight(light(1200 + k, at, [0.8, 1.0, 0.35], 15.0 + 90.0 * glow, 3.0)));
        }
        // The doors open for the tiny, and for nobody else.
        for i in 0..self.trees.len() {
            let t = &self.trees[i];
            let d = (player.feet - t.world(point(0.0, 0.0, t.face(0.0)))).length();
            let want = player.size < 0.1 && d < 0.3;
            if want != self.open[i] {
                self.open[i] = want;
                out.push(Command::SetTransform(self.door(i, want)));
            }
        }
        out
    }

    fn interact(&mut self, player: &PlayerState) -> Vec<Command> {
        let mut best = 1000.0;
        let mut which = 0;
        for i in 0..self.trees.len() {
            let d = (player.feet - self.trees[i].porch).length();
            if d < best {
                best = d;
                which = i;
            }
        }
        let mut out = Vec::new();
        if player.size > 6.0 {
            if best < 6.0 && player.feet.y > self.trees[which].at.y + 2.5 {
                out.push(Command::Say("The crown sighs and settles. Far below, someone's cups rattle.".into()));
            }
        } else if player.size > 0.3 {
            if best < 2.2 {
                out.push(Command::Say("A lantern no bigger than an acorn. Somebody very small could knock.".into()));
            }
        } else if best < 1.0 {
            self.dark[which] = !self.dark[which];
            let dark = self.dark[which];
            out.push(Command::SetLight(self.lamp(which, dark)));
            out.push(Command::Say(if dark { "You pinch out the lantern." } else { "The lantern takes the flame." }.into()));
        }
        out
    }
}

area!(Forest);

/// A distance brought nearer by `a`: room for detail that is not worked out this far away. Only for
/// what is joined to the rest by a plain union: where it starts, the distance jumps (shorter).
fn slack(d: f32, a: f32) -> f32 {
    if d > 0.0 { max(d - a, 0.001) } else { min(d + a, -0.001) }
}

/// A distance roughened by `rough` (never more than `amp` either way) near its surface, fading
/// from `lo` to `hi` away from it into the most the roughness could be: without a jump, so it may
/// be blended with others.
fn roughened(d: f32, amp: f32, lo: f32, hi: f32, rough: impl Fn() -> f32) -> f32 {
    let off = if d > 0.0 { -amp } else { amp };
    let near = between(abs(d), hi, lo);
    if near <= 0.0 {
        d + off
    } else if near >= 1.0 {
        d + rough()
    } else {
        d + mix(off, rough(), near)
    }
}

impl Forest {
    fn plant(&self, i: usize) -> Tree {
        let mut s = Seeds::of(self.seed as u64 ^ 0x7ee5, i);
        let (sx, sz) = SPOTS[i];
        let (x, z) = (sx + s.next(-0.5, 0.5), sz + s.next(-0.5, 0.5));
        let r = s.next(1.0, 1.25);
        let home = i == HEARTH || i == LOFT;
        let top = if i == GREAT { 4.2 } else if home { 4.4 } else { s.next(4.1, 4.4) };
        let crown = if i == GREAT { 4.3 } else { s.next(3.2, 3.6) };
        let yaw = atan2(32.0 - x, 58.0 - z) + s.next(-0.4, 0.4);
        let dir = point(sin(yaw), 0.0, cos(yaw));
        let hall = r + FLARE - HALL_IN;
        let (px, pz) = (x + dir.x * hall, z + dir.z * hall);
        let y = self.base(px, pz);
        let none = point(0.0, 0.0, 0.0);
        let mut roots = [(none, none, 0.0); 5];
        for (k, root) in roots.iter_mut().enumerate() {
            let a = (k as f32 + 0.5) * TAU / 5.0 + s.next(-0.1, 0.1);
            let d = point(sin(a), 0.0, cos(a));
            let reach = r + s.next(0.8, 1.35);
            *root = (d * (r * 0.6) + point(0.0, s.next(0.6, 0.85), 0.0), d * reach + point(0.0, -0.2, 0.0), s.next(0.2, 0.26));
        }
        let mut limbs = [(none, none); 4];
        let turn = s.next(0.0, TAU);
        for (k, limb) in limbs.iter_mut().enumerate() {
            let a = turn + k as f32 * TAU / 4.0 + s.next(-0.3, 0.3);
            let d = point(sin(a), 0.0, cos(a));
            *limb = (d * (r * 0.5) + point(0.0, top - 2.6, 0.0), d * 2.2 + point(0.0, top - 1.5, 0.0));
        }
        Tree {
            at: point(x, y, z),
            dir,
            yaw,
            hall,
            porch: point(px, y, pz),
            top,
            r,
            crown,
            seed: hash(i as i32, 5, 9, self.seed),
            hand: if s.below(2) == 0 { 1.0 } else { -1.0 },
            roots,
            limbs,
        }
    }

    /// The ground before any tree levelled it.
    fn raw(&self, x: f32, z: f32) -> f32 {
        let n = 0.5
            + 0.38 * noise(point(x / 13.0, 0.0, z / 13.0), self.seed ^ 0x11)
            + 0.13 * noise(point(x / 5.1, 0.0, z / 5.1), self.seed ^ 0x12);
        let rp = point(x - POOL.x, 0.0, z - POOL.z).flat_length();
        let n = mix(n, POOL_GROUND, 1.0 - between(rp, 4.5, 8.5));
        if x >= 7.0 {
            return n;
        }
        mix(edge_height(&WEST_GROUND, z), n, between(x, 0.0, 7.0))
    }

    fn base(&self, x: f32, z: f32) -> f32 {
        let r = point(x - OLD_X, 0.0, z - OLD_Z).flat_length();
        mix(self.raw(x, z), self.old.y, 1.0 - between(r, 4.0, 8.0))
    }

    fn lay(&self, x: f32, z: f32) -> Lay {
        let old = point(x - OLD_X, 0.0, z - OLD_Z).flat_length();
        let mut lay = Lay { h: self.base(x, z), along: between(old, 3.5, 5.0), near: max(old - 2.2, 0.0), porch: None };
        let mut near = lay.near * lay.near;
        for (i, t) in self.trees.iter().enumerate() {
            let (dx, dz) = (x - t.at.x, z - t.at.z);
            let d2 = dx * dx + dz * dz;
            near = min(near, d2);
            if d2 < 20.0 {
                let d = point(x - t.porch.x, 0.0, z - t.porch.z).flat_length();
                if d < 2.2 {
                    lay.h = mix(lay.h, t.at.y, 1.0 - between(d, 0.8, 2.2));
                    lay.along = min(lay.along, between(d, 0.35, 1.2));
                    lay.porch = Some(i);
                }
            }
        }
        lay.near = near.sqrt();
        let rp = point(x - POOL.x, 0.0, z - POOL.z).flat_length();
        lay.h -= 0.9 * ease(1.0 - rp / 4.5);
        lay
    }

    /// How much of its roughness the ground has: none right at the area's edges.
    fn rough(&self, x: f32, z: f32, lay: &Lay) -> f32 {
        let edge = min(min(x, WIDTH - x), min(z, WIDTH - z));
        lay.along * between(edge, 0.0, 2.0)
    }

    fn surface(&self, x: f32, z: f32, view: &View, lay: &Lay) -> f32 {
        lay.h + self.rough(x, z, lay) * detail(point(x, 0.0, z), view, 1.5, 0.05, self.seed ^ 0x9e37)
    }

    fn path(&self, x: f32, z: f32) -> f32 {
        let p = point(x, 0.0, z);
        let mut d = 1000.0;
        for &(a, b) in PATH.iter() {
            d = min(d, capsule(p, a, b, 0.0));
        }
        d
    }

    fn ground(&self, p: Point, view: &View) -> Answer {
        if p.y > 4.5 {
            return Answer::new((p.y - 1.7) * 0.65, Material::Grass);
        }
        let lay = self.lay(p.x, p.z);
        let over = p.y - lay.h;
        if abs(over) > 2.5 {
            return Answer::new(slack(over, 0.16) * 0.65, Material::Grass);
        }
        let material = self.soil(p, &lay);
        if abs(over) > 0.4 {
            return Answer::new(slack(over, 0.16) * 0.65, material);
        }
        let surface = self.surface(p.x, p.z, view, &lay);
        let ground = Answer::new((p.y - surface) * 0.65, material);
        // Pebbles, boulders to the tiny; low enough that they can always jump back over one.
        let seen = view.sees(0.015) * self.rough(p.x, p.z, &lay);
        if seen <= 0.02 || abs(p.y - surface) > 0.03 {
            return ground;
        }
        let pebbles = scatter(p, 0.07, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.011) * seen;
            sphere(point(q.x, p.y - surface + r * 0.3, q.z), r)
        });
        ground.or(Answer::new(pebbles * 0.6, Material::Stone))
    }

    fn soil(&self, p: Point, lay: &Lay) -> Material {
        if p.x < 4.5 {
            return Material::Grass;
        }
        if let Some(i) = lay.porch {
            let t = &self.trees[i];
            let l = t.local(p);
            let z = l.z - t.hall;
            if abs(l.x) < 0.135 && abs(z) < 0.105 {
                return Material::Wood;
            }
            // A little flagged way out from the door, where a person's eye may catch it.
            if z > 0.1 && z < 1.0 && abs(l.x + 0.05 * sin(z * 6.0) * between(z, 0.25, 0.5)) < 0.07 {
                return Material::Stone;
            }
        }
        let n = noise(point(p.x / 1.3, 0.0, p.z / 1.3), self.seed ^ 0x3c);
        let rp = point(p.x - POOL.x, 0.0, p.z - POOL.z).flat_length();
        if rp < 3.7 + 0.5 * n {
            return Material::Mud;
        }
        if self.path(p.x, p.z) < 0.8 + 0.3 * n {
            return Material::Soil;
        }
        if lay.near < 2.9 + 0.8 * n {
            return if n > -0.1 { Material::Moss } else { Material::Soil };
        }
        if noise(point(p.x / 3.1, 0.0, p.z / 3.1), self.seed ^ 0x3d) > 0.35 {
            return Material::Moss;
        }
        Material::Grass
    }

    fn bark(&self, p: Point, view: &View, seed: u32) -> f32 {
        detail(point(p.x, p.y * 0.25, p.z), view, 0.5, 0.05, seed)
    }

    /// Leaves on a crown: `d` roughened near its surface, and never too long.
    fn leaves(&self, d: f32, p: Point, view: &View, seed: u32) -> Answer {
        let d = roughened(d, 0.17, 0.3, 0.65, || detail(p, view, 0.8, 0.1, seed));
        Answer::new(d * 0.5, Material::Leaf)
    }

    fn tree(&self, t: &Tree, p: Point, view: &View) -> Answer {
        let q = p - t.at;
        let bound = cylinder(q - point(0.0, 2.2, 0.0), t.crown + 0.5, 3.2);
        if bound > 0.6 {
            return Answer::new(bound, Material::Leaf);
        }
        let l = t.local(p);
        let coarse = t.wood(l, view);
        // The door's arch across the trunk's front: no bark there, stone set in moss.
        let arch = if l.z > 0.0 {
            let y = l.y - clamp(l.y, -0.5, 0.16);
            (l.x * l.x + y * y).sqrt()
        } else {
            1000.0
        };
        let bare = between(arch, 0.05, 0.25);
        let wood = roughened(coarse, 0.07, 0.2, 0.4, || if bare > 0.0 { bare * self.bark(p, view, t.seed) } else { 0.0 }) * 0.65;
        let mut material = Material::Wood;
        if abs(coarse) < 0.4 && l.z > t.hall + 0.12 && arch < 0.17 {
            material = if arch < 0.07 { Material::Stone } else { Material::Moss };
        } else if abs(coarse) < 0.4 && l.y < 1.2 && arch > 0.17 {
            let n = noise(p / 0.6, t.seed ^ 0x6d);
            if n + (0.45 - l.y) * 0.8 > 0.35 {
                material = Material::Moss;
            }
        }
        let trunk = self.house(t, Answer::new(wood, material), p, l, view);
        // The crown, its top pressed flat: lumpy at its rim, level in its middle. Its lumps fade
        // out far from it without a jump, for the trunk is blended into it.
        let c = cylinder(q - point(0.0, t.top - CROWN_HALF, 0.0), t.crown - 0.8, CROWN_HALF - 0.8) - 0.8;
        let lumpy = roughened(c, 0.28, 0.95, 1.6, || {
            0.28 * between(q.flat_length(), 1.3, 2.7) * noise(q / 1.7, t.seed ^ 0x1f)
        });
        let leaf = self.leaves(lumpy, p, view, t.seed ^ 0x2f);
        trunk.smooth_or(leaf, 0.4)
    }

    /// The house cut out of a trunk's foot, for those small enough to see its door.
    fn house(&self, t: &Tree, trunk: Answer, p: Point, l: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        if seen == 0.0 {
            return trunk;
        }
        let h = point(l.x * t.hand, l.y, l.z - t.hall);
        let near = cuboid(h - point(0.0, 0.05, 0.25), point(0.2, 0.2, 0.65));
        if near > 0.05 {
            return trunk;
        }
        let rough = detail(p, view, 0.02, 0.06, t.seed ^ 0x4c);
        let hall = (rounded_cuboid(h - point(0.0, 0.015, 0.0), point(0.13, 0.075, 0.10), 0.03) + rough) * 0.7;
        let store = (rounded_cuboid(h - point(0.0, 0.005, -0.2), point(0.07, 0.06, 0.07), 0.03) + rough) * 0.7;
        // One arched way, 2.2 cm wide and 3.4 cm high, from outside through the hall to the store.
        let y = h.y - clamp(h.y, -0.05, 0.023);
        let way = max((h.x * h.x + y * y).sqrt() - 0.011, abs(h.z - 0.26) - 0.46);
        let hollow = union(union(hall, store), way);
        let mut left = Answer::new(mix(trunk.distance, cut(trunk.distance, hollow), seen), trunk.material);
        if cuboid(h - point(0.0, 0.04, -0.08), point(0.16, 0.08, 0.24)) < 0.03 {
            let things = self.furniture(h);
            left = left.or(Answer::new(things.distance + (1.0 - seen) * 0.06, things.material));
        }
        left
    }

    /// What stands in a house, about the middle of its hall's floor.
    fn furniture(&self, h: Point) -> Answer {
        // The loft, and the stair up to it: steps of 3 mm.
        let mut wood = cuboid(h - point(-0.10, LOFT_TOP - 0.0025, 0.0), point(0.0505, 0.0025, 0.12));
        for k in 0..14 {
            let top = 0.003 * (k as f32 + 1.0);
            let front = 0.06 - 0.009 * k as f32;
            let (cy, hy) = ((top - 0.01) * 0.5, (top + 0.01) * 0.5);
            let (cz, hz) = ((front - 0.12) * 0.5, (front + 0.12) * 0.5);
            wood = min(wood, cuboid(h - point(-0.043, cy, cz), point(0.009, hy, hz)));
        }
        // The table and its stools; the bed's headboard.
        let table = h - point(0.06, 0.0, 0.045);
        wood = min(wood, cylinder(table - point(0.0, 0.0085, 0.0), 0.014, 0.0012));
        wood = min(wood, cylinder(table - point(0.0, 0.003, 0.0), 0.003, 0.006));
        wood = min(wood, cylinder(point(abs(table.x) - 0.023, table.y - 0.001, table.z), 0.004, 0.005));
        wood = min(wood, cuboid(h - point(-0.105, 0.05, 0.066), point(0.011, 0.011, 0.0018)));
        // Acorns in the store.
        wood = min(wood, sphere(h - point(0.045, 0.009, -0.235), 0.012));
        wood = min(wood, sphere(h - point(-0.05, 0.008, -0.22), 0.011));
        let bed = rounded_cuboid(h - point(-0.105, 0.045, 0.045), point(0.010, 0.005, 0.02), 0.002);
        // The hearth and its chimney.
        let block = cuboid(h - point(0.126, 0.018, 0.0), point(0.018, 0.026, 0.024));
        let mouth = cuboid(h - point(0.104, 0.008, 0.0), point(0.013, 0.010, 0.011));
        let chimney = cuboid(h - point(0.126, 0.06, 0.0), point(0.014, 0.045, 0.014));
        let hearth = union(cut(block, mouth), chimney);
        Answer::new(wood, Material::Wood).or(Answer::new(bed, Material::Moss)).or(Answer::new(hearth, Material::Stone))
    }

    /// The grandmother tree, hollow, a door in her south side a person can walk through.
    fn grandmother(&self, p: Point, view: &View) -> Answer {
        let q = p - self.old;
        let bound = cylinder(q - point(0.0, 7.2, 0.0), 6.2, 8.6);
        if bound > 0.8 {
            return Answer::new(bound, Material::Leaf);
        }
        let t = clamp(1.0 - q.y / 2.2, 0.0, 1.0);
        let mut coarse = max(q.flat_length() - (2.3 + 0.9 * t * t), abs(q.y - 4.5) - 5.5) * 0.75;
        let mut roots = 1000.0;
        for &(a, b) in self.old_roots.iter() {
            roots = min(roots, capsule(q, a, b, 0.38));
        }
        let limbs = min(
            capsule(q, point(0.8, 6.6, 0.2), point(3.0, 9.4, 0.8), 0.5),
            capsule(q, point(-0.8, 6.9, 0.2), point(-2.8, 9.6, 1.3), 0.5),
        );
        coarse = smooth_union(smooth_union(coarse, roots, 0.6), limbs, 0.4);
        let mut d = roughened(coarse, 0.07, 0.2, 0.4, || self.bark(p, view, self.seed ^ 0x7f4a)) * 0.7;
        let seen = view.sees(1.0);
        let mut table = 1000.0;
        if seen > 0.0 && q.y < 4.0 {
            let room = cylinder(q - point(0.0, 0.9, 0.0), 1.25, 1.3) - 0.3;
            let room = roughened(room, 0.05, 0.2, 0.4, || detail(p, view, 0.4, 0.05, self.seed ^ 0x4d));
            let y = q.y - clamp(q.y, -0.5, 1.45);
            let door = max((q.x * q.x + y * y).sqrt() - 0.5, abs(q.z - 2.5) - 2.6);
            d = mix(d, cut(d, union(room * 0.7, door)), seen);
            table = cylinder(q - point(-0.75, 0.1, -0.6), 0.3, 0.32) + (1.0 - seen) * 0.4;
        }
        let mut material = Material::Wood;
        if q.y < 2.5 && abs(coarse) < 0.5 {
            let n = noise(p / 0.8, self.seed ^ 0x6e);
            if n + (0.9 - q.y) * 0.5 > 0.4 {
                material = Material::Moss;
            }
        }
        let trunk = Answer::new(d, material).or(Answer::new(table, Material::Wood));
        let mut c = 1000.0;
        for &(at, r) in OLD_CROWN.iter() {
            c = smooth_union(c, sphere(q - at, r), 1.0);
        }
        let lumpy = roughened(c, 0.3, 1.1, 1.8, || 0.3 * noise(q / 2.0, self.seed ^ 0x21));
        let leaf = self.leaves(lumpy, p, view, self.seed ^ 0x22);
        trunk.smooth_or(leaf, 0.5)
    }

    fn rocks(&self, p: Point, view: &View) -> Answer {
        let seen = view.sees(1.2);
        let mut best = Answer::new(1000.0, Material::Stone);
        if seen == 0.0 {
            return best;
        }
        for &(at, r) in self.boulders.iter() {
            let q = p - at;
            let d = sphere(q, r) + r * (1.0 - seen);
            if d > 0.3 {
                best = best.or(Answer::new(d - 0.09, Material::Stone));
                continue;
            }
            let d = if d > -0.3 { (d + detail(p, view, 0.6, 0.06, self.seed ^ 0x55)) * 0.7 } else { slack(d, 0.08) * 0.7 };
            let mossy = q.y / r + 0.5 * noise(p / 0.5, self.seed ^ 0x56) > 0.6;
            best = best.or(Answer::new(d, if mossy { Material::Moss } else { Material::Stone }));
        }
        best
    }

    /// A fallen log across the pool.
    fn log(&self, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.7);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Wood);
        }
        let d = capsule(p, LOG_A, LOG_B, LOG_R) + LOG_R * (1.0 - seen);
        if d > 0.2 {
            return Answer::new(d - 0.07, Material::Wood);
        }
        let d = if d > -0.2 { d + detail(p, view, 0.3, 0.05, self.seed ^ 0x57) } else { slack(d, 0.04) };
        let mossy = p.y > 0.6 && noise(p / 0.5, self.seed ^ 0x58) > 0.15;
        Answer::new(d * 0.7, if mossy { Material::Moss } else { Material::Wood })
    }

    // What moves and shines.

    fn door(&self, i: usize, open: bool) -> Instance {
        let t = &self.trees[i];
        let at = t.world(point(-0.011, 0.0, t.face(0.034) - 0.001));
        let rot = Quat::from_y(t.yaw + if open { 1.5 } else { 0.0 }).mul(Quat::from_axis_angle(point(1.0, 0.0, 0.0), PI * 0.5));
        instance(100 + i as u32, DOOR_MESH, at, rot, point(1.0, 1.0, 1.0), false)
    }

    fn lamp(&self, i: usize, dark: bool) -> Light {
        let t = &self.trees[i];
        let at = t.world(point(0.032, 0.055, t.face(0.06) + 0.04));
        light(1000 + i as u32, at, [1.0, 0.72, 0.35], if dark { 0.0 } else { 60.0 }, 1.8)
    }

    fn fly(&self, k: u32, time: f32) -> Point {
        const HOMES: [(f32, f32); 6] = [(33.0, 30.0), (30.5, 45.0), (46.0, 41.0), (50.0, 44.0), (24.0, 41.0), (36.0, 49.5)];
        let (x, z) = HOMES[k as usize % 6];
        let f = k as f32;
        let (x, z) = (x + 1.6 * sin(time * 0.31 + f * 1.3), z + 1.6 * sin(time * 0.23 + f * 2.7));
        point(x, 1.7 + 0.4 * sin(time * 0.53 + f), z)
    }

    /// Whether something small may grow at a spot.
    fn free(&self, x: f32, z: f32, lay: &Lay) -> bool {
        let edge = min(min(x, WIDTH - x), min(z, WIDTH - z));
        let rp = point(x - POOL.x, 0.0, z - POOL.z).flat_length();
        let start = point(x - 32.0, 0.0, z - 60.0).flat_length();
        let old = point(x - OLD_X, 0.0, z - OLD_Z).flat_length();
        edge > 1.0 && rp > 3.5 && start > 2.5 && old > 5.0 && lay.near > 1.9 && lay.along > 0.6 && self.path(x, z) > 1.1
    }

    fn build_scene(&self) -> Scene {
        let mut s = SceneBuilder::new();
        let one = point(1.0, 1.0, 1.0);
        let up = point(0.0, 1.0, 0.0);

        // The order of the first two is `DOOR_MESH`, `FLY_MESH`.
        let mut m = MeshBuilder::new();
        m.tint(190, 70, 50);
        let mut arch: Vec<(f32, f32)> = Vec::new();
        for k in 0..=8 {
            let a = PI + k as f32 * PI / 8.0;
            arch.push((0.011 + 0.011 * cos(a), -0.023 + 0.011 * sin(a)));
        }
        arch.push((0.022, 0.0));
        arch.push((0.0, 0.0));
        m.extrude(&arch, -0.0008, 0.0008, Material::Wood);
        m.tint(240, 200, 90);
        m.ellipsoid(point(0.018, 0.0012, -0.015), point(0.0011, 0.0011, 0.0011), 6, 4, Material::Sand);
        s.add_mesh(m.build());

        let mut m = MeshBuilder::new();
        m.tint(235, 255, 120);
        m.ellipsoid(point(0.0, 0.0, 0.0), point(0.008, 0.007, 0.012), 8, 6, Material::Sand);
        s.add_mesh(m.build());

        let cluster = s.add_mesh(leaf_cluster(self.seed));
        let cluster2 = s.add_mesh(leaf_cluster(self.seed ^ 0x99));
        let fern = s.add_mesh(fern());
        let tuft = s.add_mesh(tuft(self.seed));
        let toadstool = s.add_mesh(mushroom([225, 70, 50]));
        let brown = s.add_mesh(mushroom([200, 160, 110]));
        let bell = s.add_mesh(bluebell());
        let fallen = s.add_mesh(fallen_leaf());
        let window = s.add_mesh(window());
        let lantern = s.add_mesh(lantern());
        let embers = s.add_mesh(embers());
        let big_lantern = s.add_mesh(hanging_lantern());

        for k in 0..FLIES {
            let at = self.fly(k, 0.0);
            s.spawn(1 + k, FLY_MESH, at, Quat::IDENTITY, one, false);
            s.light(1200 + k, at, [0.8, 1.0, 0.35], 60.0, 3.0);
        }

        let mut r = Seeds::of(self.seed as u64 ^ 0x5ce0, 3);
        let person = View::new(PERSON);
        for (i, t) in self.trees.iter().enumerate() {
            let d = self.door(i, false);
            let rot = Quat { x: d.rot_x, y: d.rot_y, z: d.rot_z, w: d.rot_w };
            s.spawn(d.id, DOOR_MESH, d.pos, rot, one, false);
            // Two round windows by the door, lying against the sloping foot.
            let lean = Quat::from_y(t.yaw).mul(Quat::from_axis_angle(point(1.0, 0.0, 0.0), PI * 0.5 - 0.46));
            for side in [-0.042, 0.042] {
                s.place(window, t.world(point(side, 0.06, t.face(0.06) - 0.002)), lean, one, false);
            }
            let facing = Quat::from_y(t.yaw);
            s.place(lantern, t.world(point(0.024, 0.055, t.face(0.055) - 0.003)), facing, point(0.032, 0.032, 0.032), false);
            let l = self.lamp(i, false);
            s.light(l.id, l.pos, [l.r, l.g, l.b], l.intensity, l.radius);
            s.place(embers, t.inside(point(0.112, 0.001, 0.0)), facing, one, false);
            s.light(1100 + i as u32, t.inside(point(0.085, 0.03, 0.0)), [1.0, 0.6, 0.28], 30.0, 0.45);

            // The crown's leaves: about its rim, and a few on the seat.
            for k in 0..22 {
                let a = r.next(0.0, TAU);
                let out = point(sin(a), 0.0, cos(a));
                let (pos, tilt) = if k % 4 == 0 {
                    (t.at + out * r.next(0.4, t.crown - 0.9) + up * (t.top - 0.06), 0.1)
                } else {
                    let el = r.next(-0.6, 1.2);
                    let at = t.at + out * (t.crown - 0.8 + 0.72 * cos(el)) + up * (t.top - CROWN_HALF + 0.72 * sin(el));
                    (at, (PI * 0.5 - el) * 0.8)
                };
                let turn = Quat::from_axis_angle(point(out.z, 0.0, -out.x), tilt).mul(Quat::from_y(r.next(0.0, TAU)));
                let k2 = r.next(0.8, 1.35);
                let low = if k % 4 == 0 { 0.5 } else { 1.0 };
                s.place(if k % 2 == 0 { cluster } else { cluster2 }, pos, turn, point(k2, k2 * low, k2), false);
            }
            // Toadstools about the roots, clear of the door.
            for k in 0..5 {
                let a = r.next(0.6, TAU - 0.6);
                let d = t.r + r.next(0.5, 1.5);
                let w = t.world(point(sin(a) * d, 0.0, cos(a) * d));
                let lay = self.lay(w.x, w.z);
                let y = self.surface(w.x, w.z, &person, &lay) - 0.01;
                let k2 = if k == 0 { r.next(0.6, 1.0) } else { r.next(0.15, 0.5) };
                let kind = if i % 3 == 0 { toadstool } else { brown };
                s.place(kind, point(w.x, y, w.z), Quat::from_euler(r.next(0.0, TAU), r.next(-0.1, 0.1), 0.0), point(k2, k2, k2), false);
            }
            // Fallen leaves.
            for _ in 0..5 {
                let a = r.next(0.5, TAU - 0.5);
                let d = t.r + r.next(0.9, 2.8);
                let w = t.world(point(sin(a) * d, 0.0, cos(a) * d));
                let lay = self.lay(w.x, w.z);
                let y = self.surface(w.x, w.z, &person, &lay) + 0.02;
                let k2 = r.next(0.7, 1.3);
                s.place(fallen, point(w.x, y, w.z), Quat::from_euler(r.next(0.0, TAU), r.next(-0.2, 0.2), r.next(-0.2, 0.2)), point(k2, k2, k2), false);
            }
        }

        // The grandmother's leaves, and the lantern in her hollow.
        for k in 0..90 {
            let (at, rad) = OLD_CROWN[k % 5];
            let a = r.next(0.0, TAU);
            let y = r.next(-0.7, 1.0);
            let flat = (1.0 - y * y).sqrt();
            let n = point(sin(a) * flat, y, cos(a) * flat);
            let pos = at + n * (rad - 0.1);
            let mut buried = false;
            for (j, &(o, or)) in OLD_CROWN.iter().enumerate() {
                if j != k % 5 && (pos - o).length() < or - 0.3 {
                    buried = true;
                }
            }
            if buried {
                continue;
            }
            let tilt = atan2(flat, y);
            let turn = Quat::from_axis_angle(point(cos(a), 0.0, -sin(a)), tilt).mul(Quat::from_y(r.next(0.0, TAU)));
            let k2 = r.next(1.3, 2.2);
            s.place(if k % 2 == 0 { cluster } else { cluster2 }, self.old + pos, turn, point(k2, k2, k2), false);
        }
        s.place(big_lantern, self.old + point(0.0, 2.55, 0.0), Quat::IDENTITY, one, false);
        s.light(1300, self.old + point(0.0, 2.0, 0.0), [1.0, 0.75, 0.42], 900.0, 6.0);
        s.light(1301, self.old + point(0.0, 1.4, 4.0), [1.0, 0.75, 0.42], 300.0, 4.0);

        // The forest's floor.
        for k in 0..620 {
            let (x, z) = (r.next(1.0, 63.0), r.next(1.0, 63.0));
            let lay = self.lay(x, z);
            if !self.free(x, z, &lay) {
                continue;
            }
            let at = point(x, self.surface(x, z, &person, &lay) - 0.02, z);
            let turn = Quat::from_y(r.next(0.0, TAU));
            let shade = lay.near < 4.5;
            let (mesh, k2) = if shade {
                if k % 5 < 3 { (fern, r.next(0.6, 1.2)) } else { (bell, r.next(0.7, 1.2)) }
            } else if k % 6 == 0 {
                (bell, r.next(0.7, 1.2))
            } else {
                (tuft, r.next(0.7, 1.5))
            };
            s.place(mesh, at, turn, point(k2, k2, k2), false);
        }
        // Rushes about the pool.
        for k in 0..40 {
            let a = k as f32 * TAU / 40.0 + r.next(-0.06, 0.06);
            let d = r.next(2.9, 3.6);
            let (x, z) = (POOL.x + sin(a) * d, POOL.z + cos(a) * d);
            if capsule(point(x, 0.42, z), LOG_A, LOG_B, LOG_R) < 0.3 {
                continue;
            }
            let lay = self.lay(x, z);
            let k2 = r.next(1.3, 2.1);
            s.place(tuft, point(x, self.surface(x, z, &person, &lay) - 0.02, z), Quat::from_y(a), point(k2 * 0.7, k2 * 1.6, k2 * 0.7), false);
        }
        s.build()
    }
}

// The meshes.

fn leaf_cluster(seed: u32) -> MeshData {
    let mut m = MeshBuilder::new();
    let mut s = Seeds::of(seed as u64, 41);
    for k in 0..18 {
        let a = k as f32 * 2.4 + s.next(-0.3, 0.3);
        let rise = s.next(0.1, 1.0);
        let len = s.next(0.3, 0.5);
        let out = point(sin(a), 0.0, cos(a));
        let dir = out * (1.0 - 0.6 * rise) + point(0.0, 0.25 + 0.7 * rise, 0.0);
        let root = out * s.next(0.0, 0.18);
        let hint = point(0.0, 1.0, 0.0) - out * 0.6;
        let g = s.next(170.0, 255.0);
        m.tint((g * s.next(0.7, 1.0)) as u8, g as u8, (g * 0.7) as u8);
        let droop = |t: f32| root + dir * (len * t) + point(0.0, -0.18 * t * t, 0.0);
        m.ribbon(
            &[(droop(0.0), 0.012, hint), (droop(0.35), 0.075, hint), (droop(0.7), 0.06, hint), (droop(1.0), 0.005, hint)],
            Material::Leaf,
        );
    }
    m.build()
}

fn fern() -> MeshData {
    let mut m = MeshBuilder::new();
    let hint = point(0.0, 1.0, 0.0);
    for k in 0..9 {
        let a = k as f32 * TAU / 9.0 + 0.2 * sin(k as f32 * 3.0);
        let out = point(sin(a), 0.0, cos(a));
        let side = point(cos(a), 0.0, -sin(a));
        let size = 0.8 + 0.2 * sin(k as f32 * 5.0);
        let at = |t: f32| out * (0.55 * t * size) + point(0.0, (0.75 * t - 0.55 * t * t) * size, 0.0);
        m.tint(150 + (k * 8) as u8, 215, 130);
        m.ribbon(&[(at(0.0), 0.012, hint), (at(0.33), 0.010, hint), (at(0.66), 0.008, hint), (at(1.0), 0.003, hint)], Material::Leaf);
        for j in 1..8 {
            let t = j as f32 / 8.0;
            let w = 0.17 * (1.0 - t * 0.85) * size;
            for d in [-1.0, 1.0] {
                let p = at(t);
                let q = p + side * (w * d) + out * (w * 0.35) + point(0.0, -0.02, 0.0);
                m.ribbon(&[(p, 0.024, hint), (q, 0.004, hint)], Material::Leaf);
            }
        }
    }
    m.build()
}

fn tuft(seed: u32) -> MeshData {
    let mut m = MeshBuilder::new();
    let mut s = Seeds::of(seed as u64, 77);
    for k in 0..11 {
        let a = k as f32 * 2.4;
        let out = point(sin(a), 0.0, cos(a));
        let root = out * s.next(0.0, 0.08);
        let h = s.next(0.18, 0.4);
        let lean = s.next(0.05, 0.2);
        m.tint(190 + s.below(50) as u8, 255, 150 + s.below(40) as u8);
        m.ribbon(
            &[
                (root, 0.012, out),
                (root + out * (lean * 0.3) + point(0.0, h * 0.6, 0.0), 0.009, out),
                (root + out * lean + point(0.0, h, 0.0), 0.002, out),
            ],
            Material::Grass,
        );
    }
    m.build()
}

fn mushroom(tint: [u8; 3]) -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(255, 245, 225);
    m.lathe(point(0.0, 0.0, 0.0), &[(0.035, -0.02), (0.028, 0.06), (0.022, 0.15), (0.025, 0.19)], 10, Material::Sand);
    m.lathe(point(0.0, 0.0, 0.0), &[(0.022, 0.172), (0.11, 0.15)], 14, Material::Sand);
    m.tint(tint[0], tint[1], tint[2]);
    m.lathe(
        point(0.0, 0.0, 0.0),
        &[(0.11, 0.15), (0.118, 0.163), (0.105, 0.192), (0.075, 0.222), (0.035, 0.24), (0.0, 0.244)],
        14,
        Material::Sand,
    );
    m.tint(255, 250, 235);
    for k in 0..7 {
        let a = k as f32 * 2.4;
        let d = 0.03 + 0.009 * k as f32;
        m.ellipsoid(point(sin(a) * d, 0.242 - 5.0 * d * d, cos(a) * d), point(0.010, 0.006, 0.010), 6, 4, Material::Sand);
    }
    m.build()
}

fn bluebell() -> MeshData {
    let mut m = MeshBuilder::new();
    for k in 0..4 {
        let a = k as f32 * 1.7;
        let out = point(sin(a), 0.0, cos(a));
        let root = out * 0.05;
        let h = 0.26 + 0.04 * sin(k as f32 * 4.0);
        m.tint(150, 210, 120);
        m.sweep_tube(
            &[
                (root, 0.005),
                (root + out * 0.01 + point(0.0, h * 0.7, 0.0), 0.004),
                (root + out * 0.06 + point(0.0, h, 0.0), 0.003),
                (root + out * 0.12 + point(0.0, h * 0.97, 0.0), 0.002),
            ],
            5,
            true,
            Material::Leaf,
        );
        m.ribbon(&[(root, 0.012, out), (root + out * 0.12 + point(0.0, 0.14, 0.0), 0.010, out), (root + out * 0.2 + point(0.0, 0.1, 0.0), 0.002, out)], Material::Leaf);
        m.tint(120, 120, 255);
        for j in 0..3 {
            let t = j as f32 / 2.0;
            let at = root + out * (0.06 + 0.06 * t) + point(0.0, h - 0.045 - 0.01 * t, 0.0);
            m.lathe(at, &[(0.022, 0.0), (0.014, 0.012), (0.012, 0.03), (0.0, 0.042)], 8, Material::Sand);
        }
    }
    m.build()
}

fn fallen_leaf() -> MeshData {
    let mut m = MeshBuilder::new();
    let up = point(0.0, 1.0, 0.0);
    m.tint(235, 165, 80);
    m.ribbon(
        &[
            (point(0.0, 0.0, -0.06), 0.003, up),
            (point(0.0, 0.006, -0.03), 0.022, up),
            (point(0.0, 0.004, 0.0), 0.030, up),
            (point(0.0, 0.008, 0.035), 0.018, up),
            (point(0.0, 0.016, 0.06), 0.002, up),
        ],
        Material::Sand,
    );
    m.build()
}

fn window() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(120, 85, 55);
    m.lathe(
        point(0.0, 0.0, 0.0),
        &[(0.0105, -0.006), (0.0105, 0.002), (0.0095, 0.003), (0.0078, 0.003), (0.0075, 0.0015)],
        16,
        Material::Wood,
    );
    m.cuboid(point(0.0, 0.0018, 0.0), point(0.0076, 0.0005, 0.0006), Material::Wood);
    m.cuboid(point(0.0, 0.0018, 0.0), point(0.0006, 0.0005, 0.0076), Material::Wood);
    m.tint(255, 225, 120);
    m.lathe(point(0.0, 0.0, 0.0), &[(0.0078, 0.0012), (0.0, 0.0016)], 16, Material::Sand);
    m.build()
}

/// A lantern on a bracket, one unit long, out along z.
fn lantern() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(90, 80, 70);
    m.sweep_tube(
        &[
            (point(0.0, 0.0, -0.2), 0.045),
            (point(0.0, 0.22, 0.35), 0.04),
            (point(0.0, 0.3, 0.8), 0.035),
            (point(0.0, 0.15, 1.05), 0.03),
        ],
        6,
        true,
        Material::Wood,
    );
    let at = point(0.0, 0.0, 1.05);
    m.cone(at + point(0.0, 0.0, 0.0), 0.2, 0.22, 8, Material::Wood);
    m.lathe(at, &[(0.0, -0.46), (0.16, -0.45), (0.16, -0.38)], 8, Material::Wood);
    m.tint(255, 215, 110);
    m.lathe(at, &[(0.12, -0.4), (0.17, -0.25), (0.17, -0.1), (0.14, 0.01)], 10, Material::Sand);
    m.build()
}

fn embers() -> MeshData {
    let mut m = MeshBuilder::new();
    for k in 0..5 {
        let a = k as f32 * 2.4;
        let d = 0.0012 * k as f32;
        m.tint(255, 110 + (k * 25) as u8, 40);
        m.ellipsoid(point(sin(a) * d * 0.7, 0.001, cos(a) * d), point(0.0024, 0.0022, 0.0024), 6, 4, Material::Sand);
    }
    m.tint(255, 190, 70);
    m.cone(point(0.0, 0.002, 0.0), 0.009, 0.0022, 6, Material::Sand);
    m.cone(point(0.001, 0.002, 0.003), 0.006, 0.0016, 6, Material::Sand);
    m.build()
}

/// The lantern in the grandmother's hollow, hung from its root.
fn hanging_lantern() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(90, 80, 70);
    m.sweep_tube(&[(point(0.0, 0.1, 0.0), 0.012), (point(0.0, -0.2, 0.0), 0.012), (point(0.0, -0.42, 0.0), 0.012)], 6, true, Material::Wood);
    m.cone(point(0.0, -0.5, 0.0), 0.12, 0.13, 10, Material::Wood);
    m.lathe(point(0.0, 0.0, 0.0), &[(0.0, -0.76), (0.1, -0.75), (0.1, -0.71)], 10, Material::Wood);
    m.tint(255, 220, 130);
    m.lathe(point(0.0, 0.0, 0.0), &[(0.08, -0.72), (0.105, -0.64), (0.1, -0.54), (0.08, -0.49)], 12, Material::Sand);
    m.build()
}
