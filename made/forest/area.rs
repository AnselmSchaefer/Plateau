//! The wood of stools: an old beech wood where trees were felled long ago and their stumps left
//! standing, broad as rooms and three metres tall. A ring of three stumps stands round a mossy
//! stone in a clearing; tall living trees stand about them, their crowns high overhead, and to the
//! north-east an old oak, hollow at its foot.
//!
//! Each size has its own wood:
//! - **The tiny** find, at the foot of each stump where the moss is thickest, an arched door a
//!   centimetre and a half wide with a round window above it: each opens on a little room inside
//!   the stump, its floor carpeted with moss, a table and a bed of moss within. Fallen leaves are
//!   rafts and sheets, bark furrows are canyons, and the rings on a stump's top are grooves.
//! - **A person** walks among the trunks, roots and mushrooms, sees the bark and the split checks
//!   across the stumps' tops, and walks in at the arched door of the old oak to stand in the
//!   hollow inside.
//! - **A giant** walks beneath the crowns, and steps up onto a stump to sit: the north stump, the
//!   broadest, is the giants' stool by the stone table.

use plateau_sdk::*;
use std::cell::Cell;

const TABLE: Point = point(31.0, 0.0, 31.0);
const TABLE_R: f32 = 3.4;
const TABLE_H: f32 = 1.3;

const OAK: Point = point(48.0, 0.0, 13.0);
/// The oak's door faces the clearing.
const OAK_DOOR: Point = point(-0.6866, 0.0, 0.7270);
const HOLLOW_R: f32 = 1.5;

const START: Point = point(32.0, 0.0, 60.0);

/// The living trees: where they stand, and their trunks' radii.
const TREES: [(f32, f32, f32); 13] = [
    (8.0, 8.0, 1.0),
    (20.0, 5.0, 0.8),
    (6.0, 27.0, 1.1),
    (9.0, 46.0, 0.9),
    (4.0, 59.0, 0.8),
    (18.0, 55.0, 1.0),
    (58.0, 28.0, 1.0),
    (60.0, 44.0, 0.9),
    (50.0, 56.0, 1.1),
    (59.0, 6.0, 0.8),
    (30.0, 6.0, 0.9),
    (48.0, 44.0, 0.8),
    (14.0, 17.0, 0.9),
];

struct Stump {
    x: f32,
    z: f32,
    base: f32,
    r: f32,
    h: f32,
    /// Which way the tiny's door faces.
    dir: Point,
    /// Which way the check across the top runs.
    crack: Point,
    roots: [Point; 5],
    seed: u32,
}

struct Tree {
    x: f32,
    z: f32,
    base: f32,
    r: f32,
    flare: f32,
    flare_h: f32,
    limb_y: f32,
    limb_r: f32,
    crown: [Point; 3],
    radii: [f32; 3],
    centre_y: f32,
    crown_reach: f32,
    reach: f32,
    roots: Vec<Point>,
    root_len: f32,
    root_r: f32,
    root_rise: f32,
    seed: u32,
}

impl Tree {
    fn finish(mut self) -> Tree {
        let centre = self.crown[0];
        self.centre_y = centre.y;
        let mut cr = 0.0;
        let mut flat = self.r + self.flare + self.root_len + self.root_r + 0.5;
        for k in 0..3 {
            cr = max(cr, (self.crown[k] - centre).length() + self.radii[k] + 0.8);
            flat = max(flat, self.crown[k].flat_length() + self.radii[k] + 0.8);
        }
        self.crown_reach = cr;
        self.reach = flat;
        self
    }
}

struct Forest {
    seed: u32,
    stumps: Vec<Stump>,
    oak: Tree,
    trees: Vec<Tree>,
    table_base: f32,
}

/// The forest floor before it is levelled about the stumps: gentle swells, rising to the edges
/// and a little to the north.
fn raw(seed: u32, x: f32, z: f32) -> f32 {
    let n1 = 0.7 * noise(point(x / 16.0, 0.0, z / 16.0), seed ^ 0x1111);
    let n2 = 0.3 * noise(point(x / 6.0, 0.0, z / 6.0), seed ^ 0x2222);
    let edge = min(min(x, WIDTH - x), min(z, WIDTH - z));
    let rim = 1.2 * (1.0 - between(edge, 0.0, 10.0));
    n1 + n2 + rim + 0.02 * (32.0 - z)
}

fn dist(x: f32, z: f32, cx: f32, cz: f32) -> f32 {
    ((x - cx) * (x - cx) + (z - cz) * (z - cz)).sqrt()
}

/// The ground levelled to `base` within `r` of a thing's middle, eased back beyond.
fn flatten(h: f32, x: f32, z: f32, cx: f32, cz: f32, r: f32, base: f32) -> f32 {
    mix(h, base, between(dist(x, z, cx, cz), r + 3.0, r + 0.8))
}

fn ext2(a: f32, b: f32) -> f32 {
    (max(a, 0.0) * max(a, 0.0) + max(b, 0.0) * max(b, 0.0)).sqrt() + min(max(a, b), 0.0)
}

fn box2(x: f32, y: f32, hx: f32, hy: f32) -> f32 {
    ext2(abs(x) - hx, abs(y) - hy)
}

fn grow(seed: u32, i: usize, x: f32, z: f32, r: f32) -> Tree {
    let mut s = Seeds::of(seed as u64, 100 + i);
    let cy = 30.5 + s.next(-1.0, 1.0);
    let a0 = s.next(0.0, TAU);
    let mut crown = [point(0.0, cy, 0.0); 3];
    let mut radii = [6.0 + s.next(0.0, 0.8); 3];
    for k in 1..3 {
        let a = a0 + (k - 1) as f32 * PI * s.next(0.7, 1.3);
        let d = s.next(2.5, 3.5);
        crown[k] = point(d * cos(a), cy + s.next(-2.0, 0.5), d * sin(a));
        radii[k] = s.next(4.0, 5.0);
    }
    let b = s.next(0.0, TAU);
    let roots = (0..4)
        .map(|k| {
            let a = b + k as f32 * TAU / 4.0 + s.next(-0.3, 0.3);
            point(cos(a), 0.0, sin(a))
        })
        .collect();
    Tree {
        x,
        z,
        base: raw(seed, x, z),
        r,
        flare: 0.5 * r,
        flare_h: 2.0,
        limb_y: 22.5,
        limb_r: 0.25 * r + 0.05,
        crown,
        radii,
        centre_y: 0.0,
        crown_reach: 0.0,
        reach: 0.0,
        roots,
        root_len: 1.6,
        root_r: 0.25 + 0.15 * r,
        root_rise: 0.35,
        seed: seed ^ (0xb00 + i as u32 * 131),
    }
    .finish()
}

impl Area for Forest {
    fn new(seed: u64) -> Forest {
        let seed = (seed ^ (seed >> 32)) as u32;
        let specs = [
            (31.0, 21.0, 3.6, 3.4, -PI / 2.0),
            (40.0, 35.0, 3.0, 2.7, PI / 4.0),
            (21.0, 35.0, 3.2, 3.0, 3.0 * PI / 4.0),
        ];
        let mut stumps = Vec::new();
        for (i, &(x, z, r, h, door)) in specs.iter().enumerate() {
            let mut s = Seeds::of(seed as u64, 10 + i);
            let mut roots = [point(0.0, 0.0, 0.0); 5];
            for (k, root) in roots.iter_mut().enumerate() {
                let a = door + 1.2 + k as f32 * (TAU - 2.4) / 4.0 + s.next(-0.15, 0.15);
                *root = point(cos(a), 0.0, sin(a));
            }
            let ca = door + s.next(1.8, TAU - 1.8);
            stumps.push(Stump {
                x,
                z,
                base: raw(seed, x, z),
                r,
                h,
                dir: point(cos(door), 0.0, sin(door)),
                crack: point(cos(ca), 0.0, sin(ca)),
                roots,
                seed: seed ^ (0x5170 + i as u32 * 77),
            });
        }

        let mut so = Seeds::of(seed as u64, 99);
        let door_a = atan2(OAK_DOOR.z, OAK_DOOR.x);
        let oak_roots: Vec<Point> = (0..5)
            .map(|k| {
                let a = door_a + 1.0 + k as f32 * (TAU - 2.0) / 4.0 + so.next(-0.1, 0.1);
                point(cos(a), 0.0, sin(a))
            })
            .collect();
        let a0 = so.next(0.0, TAU);
        let mut crown = [point(0.0, 33.0, 0.0); 3];
        let mut radii = [8.5; 3];
        for k in 1..3 {
            let a = a0 + (k - 1) as f32 * PI * so.next(0.8, 1.2);
            let d = so.next(4.0, 5.0);
            crown[k] = point(d * cos(a), so.next(31.5, 33.5), d * sin(a));
            radii[k] = so.next(6.0, 6.5);
        }
        let oak = Tree {
            x: OAK.x,
            z: OAK.z,
            base: raw(seed, OAK.x, OAK.z),
            r: 3.0,
            flare: 1.2,
            flare_h: 3.2,
            limb_y: 25.0,
            limb_r: 1.0,
            crown,
            radii,
            centre_y: 0.0,
            crown_reach: 0.0,
            reach: 0.0,
            roots: oak_roots,
            root_len: 3.0,
            root_r: 0.7,
            root_rise: 0.8,
            seed: seed ^ 0x0a4c,
        }
        .finish();

        let trees = TREES.iter().enumerate().map(|(i, &(x, z, r))| grow(seed, i, x, z, r)).collect();

        Forest { seed, stumps, oak, trees, table_base: raw(seed, TABLE.x, TABLE.z) }
    }

    fn describe(&self) -> Description {
        let house = |s: &Stump| point(s.x + s.dir.x * (s.r - 0.3), s.base, s.z + s.dir.z * (s.r - 0.3));
        let a = &self.stumps[0];
        Description {
            name: "The wood of stools".into(),
            places: vec![
                Place::new("the giants' stool by the stone table", Size::Giant, point(a.x, a.base + a.h, a.z)),
                Place::new("the little house in the east stump", Size::Tiny, house(&self.stumps[1])),
                Place::new("the little house in the west stump", Size::Tiny, house(&self.stumps[2])),
                Place::new("the hollow in the old oak", Size::Person, point(OAK.x, self.oak.base, OAK.z)),
            ],
            borders: Borders::of_ground(|x, z| self.height(x, z)),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let mut a = self.ground(p, view);
        for s in &self.stumps {
            a = a.or(self.stump(s, p, view));
        }
        a = a.or(self.table(p, view));
        a = a.or(self.tree(&self.oak, p, view, true));
        for t in &self.trees {
            a = a.or(self.tree(t, p, view, false));
        }
        a
    }
}

area!(Forest);

impl Forest {
    /// The ground's height, levelled about the stumps, the oak and the table.
    fn height(&self, x: f32, z: f32) -> f32 {
        let mut h = raw(self.seed, x, z);
        for s in &self.stumps {
            h = flatten(h, x, z, s.x, s.z, s.r + 0.35, s.base);
        }
        h = flatten(h, x, z, OAK.x, OAK.z, self.oak.r + self.oak.flare, self.oak.base);
        flatten(h, x, z, TABLE.x, TABLE.z, TABLE_R + 0.35, self.table_base)
    }

    /// One far from the stumps, the oak and the table; nought at their feet, where the ground lies
    /// smooth and level, so a tiny door meets it without a step.
    fn calm(&self, x: f32, z: f32) -> f32 {
        let mut c: f32 = 1.0;
        for s in &self.stumps {
            c = min(c, between(dist(x, z, s.x, s.z) - (s.r + 0.35), 0.3, 1.5));
        }
        c = min(c, between(dist(x, z, OAK.x, OAK.z) - (self.oak.r + self.oak.flare), 0.3, 1.5));
        min(c, between(dist(x, z, TABLE.x, TABLE.z) - (TABLE_R + 0.35), 0.3, 1.5))
    }

    /// The forest floor and the mushrooms on it.
    fn ground(&self, p: Point, view: &View) -> Answer {
        let h = self.height(p.x, p.z);
        self.floor(p, h, view).or(self.mushrooms(p, h, view))
    }

    /// The forest floor: grass in the glades, soil and leaf litter under the trees, moss at the
    /// feet of the stumps; for the tiny, fallen leaves.
    fn floor(&self, p: Point, h: f32, view: &View) -> Answer {
        let above = p.y - h;
        if above > 0.4 {
            return Answer::new((above - 0.12) * 0.55, Material::Soil);
        }
        if above < -0.4 {
            return Answer::new((above + 0.12) * 0.55, Material::Soil);
        }
        let flat = point(p.x, 0.0, p.z);
        let calm = self.calm(p.x, p.z);
        let surface = h + calm * detail(flat, view, 0.8, 0.05, self.seed ^ 0x9e37);
        let glade = max(between((flat - TABLE).flat_length(), 17.0, 10.0), between((flat - START).flat_length(), 12.0, 5.0));
        let g = 0.5 * noise(flat / 5.0, self.seed ^ 0x31) + glade;
        let material = if calm < 0.6 && noise(flat / 0.8, self.seed ^ 0x6e) > -0.3 {
            Material::Moss
        } else if g > 0.35 {
            Material::Grass
        } else if noise(flat / 2.3, self.seed ^ 0x44) > 0.35 {
            Material::Leaf
        } else {
            Material::Soil
        };
        // The lumps' octaves add up in slope with the swells: the distance made shorter.
        let ground = Answer::new((p.y - surface) * 0.55, material);
        // Fallen leaves, two to five centimetres long: sheets to the tiny, unseen by anyone else,
        // and none at the stumps' feet where the doors are.
        let seen = view.sees(0.02) * calm;
        let off = p.y - surface;
        if seen == 0.0 || abs(off) > 0.03 {
            return ground;
        }
        let leaves = scatter(p, 0.07, self.seed ^ 0x77, |q, s| {
            if s.next(0.0, 1.0) > 0.55 {
                return 1.0;
            }
            let r = s.next(0.012, 0.028) * seen;
            let lq = q.turned(s.next(0.0, TAU));
            cylinder(point(lq.x, off - 0.001, lq.z * 1.8), r, 0.001) / 1.8
        });
        ground.or(Answer::new(leaves * 0.55, Material::Leaf))
    }

    /// Stout brown-capped mushrooms among the trees, knee-high to a person, unseen by a giant;
    /// none at the feet of the stumps, the oak and the table, nor about the start.
    fn mushrooms(&self, p: Point, h: f32, view: &View) -> Answer {
        let seen = view.sees(0.35);
        if seen == 0.0 {
            return Answer::new(1.0e9, Material::Wood);
        }
        let off = p.y - h;
        if off > 0.8 {
            return Answer::new((off - 0.75) * 0.5, Material::Wood);
        }
        if off < -0.15 {
            return Answer::new(1.0e9, Material::Wood);
        }
        let cap_best = Cell::new(1.0e9_f32);
        let all = scatter(p, 2.6, self.seed ^ 0x3a5, |q, s| {
            if s.next(0.0, 1.0) > 0.4 {
                return 2.0;
            }
            let rc = s.next(0.18, 0.28) * seen;
            let stem_h = s.next(0.15, 0.35) * seen;
            let lean = s.next(-0.04, 0.04) * seen;
            let place = p - q;
            if self.calm(place.x, place.z) < 1.0 || (place - START).flat_length() < 4.0 {
                return 2.0;
            }
            let lq = point(q.x, off, q.z);
            let stem = capsule(lq, point(0.0, -0.06, 0.0), point(lean, stem_h, 0.0), rc * 0.42);
            let c = lq - point(lean, stem_h, 0.0);
            let cap = max(sphere(c, rc), -c.y - 0.02);
            cap_best.set(min(cap_best.get(), cap));
            min(stem, cap)
        });
        let material = if cap_best.get() <= all + 0.001 { Material::Wood } else { Material::Sand };
        Answer::new(all * 0.55, material)
    }

    /// A stump: a giant's stool, its top sawn flat with rings and a check, bark down its sides,
    /// roots spreading; and for the tiny, a door, a window and a room at its foot.
    fn stump(&self, s: &Stump, p: Point, view: &View) -> Answer {
        let q = point(p.x - s.x, p.y - s.base, p.z - s.z);
        let rho = q.flat_length();
        let bound = max(rho - (s.r + 2.7), max(q.y - s.h - 0.3, -q.y - 3.3));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let u = q.x * s.dir.x + q.z * s.dir.z;
        let v = q.z * s.dir.x - q.x * s.dir.z;
        let house_box = cuboid(point(v, q.y - 0.1, u - (s.r + 0.1)), point(0.25, 0.2, 0.7));
        let seen_house = view.sees(0.06);

        // The body flares a little at its foot; its top edge is worn round.
        let rr = s.r + 0.35 * ease(1.0 - q.y / 1.2);
        let e = 0.12;
        let body = max(ext2(rho - rr + e, q.y - s.h + e) - e, -3.0 - q.y) * 0.85;
        let mut roots = 1.0e9_f32;
        for dir in &s.roots {
            let from = *dir * (s.r * 0.55) + point(0.0, 0.5, 0.0);
            let to = *dir * (s.r + 1.9) + point(0.0, -0.45, 0.0);
            roots = min(roots, capsule(q, from, to, 0.5));
        }
        let coarse = smooth_union(body, roots, 0.5);
        if coarse > 0.25 {
            return Answer::new(coarse - 0.1, Material::Wood);
        }
        if coarse < -0.6 && house_box > 0.05 {
            // Deep inside: no nearer than the house could be.
            let d = (coarse + 0.1) * 0.55;
            return Answer::new(mix(d, max(d, -house_box), seen_house), Material::Wood);
        }

        // Bark, furrowed up and down, on the sides; none on the sawn top.
        let top_t = between(q.y, s.h - 0.3, s.h - 0.1);
        let bark = detail(point(p.x, p.y * 0.25, p.z), view, 0.6, 0.06, s.seed) * (1.0 - top_t);
        let mut d = (coarse + bark) * 0.55;

        // The growth rings on the top, grooves to the tiny.
        let depth = 0.003 * view.sees(0.02) * top_t;
        if depth > 0.0 {
            let t = (rho + 0.005 * noise(q / 0.15, s.seed ^ 0x3)) / 0.028;
            let ring = (abs(t - (t + 0.5).floor()) * 0.028 - 0.0015) * 0.8;
            d = max(d, min(-ring, d + depth));
        }

        // A check split across the top from near the heart to the rim, and down the rim.
        let seen_crack = view.sees(0.3);
        if seen_crack > 0.0 && q.y > s.h - 1.2 {
            let along = q.x * s.crack.x + q.z * s.crack.z;
            let side = q.z * s.crack.x - q.x * s.crack.z;
            let w = 0.04 * clamp((q.y - (s.h - 0.4)) / 0.4, 0.0, 1.0);
            let crack = max(max(abs(side) - w, (s.h - 0.4) - q.y), s.r * 0.25 - along) * 0.9;
            d = mix(d, cut(d, crack), seen_crack);
        }

        // Moss low on the north side, and always about the tiny's door.
        let north = -q.z / max(rho, 0.01);
        let mouth = point(s.dir.x * (s.r + 0.35), 0.0, s.dir.z * (s.r + 0.35));
        let patch = noise(p / 0.9, s.seed ^ 0x6d) + 0.7 * north + 1.5 * between(-(q - mouth).length(), -0.6, -0.25);
        let moss = between(patch, 0.15, 0.35) * between(q.y, s.h * 0.75, s.h * 0.45) * (1.0 - top_t);
        let mut material = if moss > 0.5 { Material::Moss } else { Material::Wood };
        let tuft_seen = view.sees(0.006) * moss;
        if tuft_seen > 0.0 {
            let tufts = tuft_seen * 0.003 * (0.6 + 0.4 * noise(p / 0.005, s.seed ^ 0x2b));
            d = (d - tufts) * (1.0 - 0.5 * tuft_seen);
        }

        // The tiny's house: an arched door, a round window, a room behind with a table and a bed,
        // for the tiny alone.
        if seen_house > 0.0 {
            if house_box < 0.05 {
                let arch = min(box2(v, q.y - 0.013, 0.008, 0.013), (v * v + (q.y - 0.026) * (q.y - 0.026)).sqrt() - 0.008);
                let through = abs(u - (s.r + 0.175)) - 0.425;
                let passage = max(arch, through);
                let window = max((v * v + (q.y - 0.085) * (q.y - 0.085)).sqrt() - 0.009, through);
                let room = rounded_cuboid(point(v, q.y - 0.065, u - (s.r - 0.3)), point(0.12, 0.065, 0.12), 0.04);
                let rough = detail(p, view, 0.04, 0.06, s.seed ^ 0x4c) * between(q.y, 0.004, 0.03);
                let room = (room + rough) * 0.6;
                let hollow = min(min(passage, window), room);
                let mut cut_d = cut(d, hollow);
                if seen_house > 0.5 && -hollow > d {
                    material = if q.y < 0.004 && u < s.r - 0.15 { Material::Moss } else { Material::Wood };
                }
                // A round table on one leg, and a bed of moss along the far side.
                let tq = point(v - 0.065, q.y, u - (s.r - 0.36));
                let table = min(
                    cylinder(tq - point(0.0, 0.026, 0.0), 0.022, 0.0025),
                    cylinder(tq - point(0.0, 0.012, 0.0), 0.004, 0.014),
                );
                let bed = rounded_cuboid(point(v + 0.07, q.y - 0.004, u - (s.r - 0.34)), point(0.022, 0.008, 0.045), 0.004);
                let furniture = min(table, bed);
                if furniture < cut_d {
                    cut_d = furniture;
                    if seen_house > 0.5 {
                        material = if table < bed { Material::Wood } else { Material::Moss };
                    }
                }
                d = mix(d, cut_d, seen_house);
            } else {
                d = mix(d, max(d, -house_box), seen_house);
            }
        }
        Answer::new(d, material)
    }

    /// The stone table in the clearing: a broad boulder, flat-topped, mossy.
    fn table(&self, p: Point, view: &View) -> Answer {
        let q = point(p.x - TABLE.x, p.y - self.table_base, p.z - TABLE.z);
        let rho = q.flat_length();
        let bound = max(rho - (TABLE_R + 0.9), max(q.y - TABLE_H - 0.3, -q.y - 1.6));
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let inv = 1.0 / max(rho, 0.5);
        let rim = TABLE_R + 0.35 * noise(point(q.x * inv * 1.5, 0.0, q.z * inv * 1.5), self.seed ^ 0x7a);
        let top = TABLE_H - 0.12 * rho * rho / (TABLE_R * TABLE_R);
        let e = 0.3;
        let body = max(ext2(rho - rim + e, q.y - top + e) - e, -1.5 - q.y) * 0.75;
        if body > 0.25 {
            return Answer::new(body - 0.08, Material::Stone);
        }
        let rough = detail(p, view, 0.8, 0.04, self.seed ^ 0x5e);
        let mut d = (body + rough) * 0.6;
        let m = between(noise(p / 1.2, self.seed ^ 0x99) + 0.6 * between(q.y, TABLE_H - 0.6, TABLE_H - 0.2), 0.3, 0.5);
        let material = if m > 0.5 { Material::Moss } else { Material::Stone };
        let seen = view.sees(0.006) * m;
        if seen > 0.0 {
            let tufts = seen * 0.003 * (0.6 + 0.4 * noise(p / 0.005, self.seed ^ 0x2c));
            d = (d - tufts) * (1.0 - 0.5 * seen);
        }
        Answer::new(d, material)
    }

    /// A living tree: a flared trunk on spreading roots, limbs, and a crown high above, out of a
    /// giant's way.
    fn tree(&self, t: &Tree, p: Point, view: &View, hollow: bool) -> Answer {
        let q = point(p.x - t.x, p.y - t.base, p.z - t.z);
        let rho = q.flat_length();
        let bound = max(rho - t.reach, max(q.y - (t.centre_y + t.crown_reach), -q.y - 3.2));
        if bound > 0.5 {
            return Answer::new(bound, Material::Leaf);
        }
        let rr = t.r * (1.0 - 0.006 * clamp(q.y, 0.0, t.centre_y)) + t.flare * ease(1.0 - q.y / t.flare_h);
        let trunk = max(rho - rr, max(q.y - t.centre_y, -3.0 - q.y)) * 0.85;
        let mut roots = 1.0e9_f32;
        for dir in &t.roots {
            let from = *dir * (t.r * 0.55) + point(0.0, t.root_rise, 0.0);
            let to = *dir * (t.r + t.root_len) + point(0.0, -0.4 * t.root_rise - 0.2, 0.0);
            roots = min(roots, capsule(q, from, to, t.root_r));
        }
        let mut wood = smooth_union(trunk, roots, 0.5);
        for k in 1..3 {
            wood = min(wood, capsule(q, point(0.0, t.limb_y, 0.0), t.crown[k], t.limb_r));
        }
        let mut a = if wood > 0.3 {
            Answer::new(wood - 0.08, Material::Wood)
        } else if wood < -0.5 {
            Answer::new((wood + 0.08) * 0.55, Material::Wood)
        } else {
            let bark = detail(point(p.x, p.y * 0.25, p.z), view, 0.5, 0.06, t.seed);
            Answer::new((wood + bark) * 0.55, Material::Wood)
        };
        if hollow {
            a = self.hollow(a, q, view);
        }
        let cb = (q - point(0.0, t.centre_y, 0.0)).length() - t.crown_reach;
        if cb > 0.5 {
            return a.or(Answer::new(cb, Material::Leaf));
        }
        let mut c = 1.0e9_f32;
        for k in 0..3 {
            c = min(c, sphere(q - t.crown[k], t.radii[k]));
        }
        // Leafy lumps, down to no finer than eight centimetres: nobody stands up there.
        let coarse_view = View { size: view.size, finest: max(view.finest, 0.08) };
        let lumps = if c < 1.5 { detail(p, &coarse_view, 2.4, 0.12, t.seed ^ 0x5151) } else { -0.6 };
        a.or(Answer::new((c + lumps) * 0.6, Material::Leaf))
    }

    /// The old oak's hollow: a room inside its foot under a domed roof, and an arched door a
    /// person walks in at and a giant does not.
    fn hollow(&self, a: Answer, q: Point, view: &View) -> Answer {
        let u = q.x * OAK_DOOR.x + q.z * OAK_DOOR.z;
        let v = q.z * OAK_DOOR.x - q.x * OAK_DOOR.z;
        let room = smooth_union(cylinder(q - point(0.0, 1.6, 0.0), HOLLOW_R, 1.6), sphere(q - point(0.0, 3.0, 0.0), HOLLOW_R), 0.8);
        let arch = min(box2(v, q.y - 0.95, 0.6, 0.95), (v * v + (q.y - 1.9) * (q.y - 1.9)).sqrt() - 0.6);
        let door = max(arch, abs(u - 3.0) - 2.4);
        let mut h = min(room, door);
        h = if h < 0.3 {
            (h + detail(q, view, 0.3, 0.05, self.seed ^ 0x0a4) * between(q.y, 0.05, 0.3)) * 0.8
        } else {
            (h - 0.05) * 0.8
        };
        let material = if -h > a.distance {
            if q.y < 0.08 { Material::Soil } else { Material::Wood }
        } else {
            a.material
        };
        Answer::new(cut(a.distance, h), material)
    }
}
