The trap was in the grotto. Its walls curved into the floor like a bowl, so the tiny could climb the rough slope in 4 mm steps and get stuck in a pocket of the wall.

The grotto's walls now rise straight from the floor. They are smooth for the first 1.5 cm, which is higher than the tiny can step or jump, and the roughness fades in above that. I shortened the distance there slightly to allow for the fade. Nothing else changed, and I haven't rebuilt or tested it here.

```rust
//! The heron's pond: a wide pond in a meadow, lily pads strung across it, a mossy rock in its
//! north-west corner, and a heron's nest on a dead snag to the north-east.
//!
//! - **The tiny** cross by the lily pads to a shelf of stone at the mossy rock's foot, where a
//!   crack under a mossy arch leads into a grotto.
//! - **A person** follows the path to the beach, wades among the reeds, and shelters in the hollow
//!   willow.
//! - **A giant** wades the pond and steps up into the heron's nest.

use plateau_sdk::*;

const LEVEL: f32 = -0.35;
/// The floor of the grotto, its crack and its arch: the top of the stone shelf before them.
const FLOOR: f32 = -0.25;
const MOUTH: Point = point(20.6, FLOOR, 22.7);
const ROOM: Point = point(20.6, FLOOR, 21.65);

const NEST: Point = point(53.0, 0.0, 17.0);
/// The nest's floor, above the ground at the snag's foot.
const NEST_H: f32 = 4.1;
const WILLOW: Point = point(52.5, 0.0, 45.0);

const POND: (f32, f32) = (29.0, 31.0);
const OUTLINE: [(f32, f32); 15] = [
    (16.4, 43.2),
    (11.0, 36.1),
    (11.0, 25.9),
    (13.6, 19.6),
    (16.2, 17.2),
    (19.8, 15.6),
    (28.0, 14.5),
    (37.0, 16.4),
    (43.0, 19.9),
    (46.1, 25.6),
    (46.2, 33.9),
    (43.2, 40.8),
    (38.9, 44.5),
    (31.4, 46.9),
    (23.4, 46.4),
];
/// How much the outline's corners are rounded.
const ROUNDED: f32 = 2.5;

/// Boulders about the shore and in the water: middles and half sizes.
const STONES: [(f32, f32, f32, f32, f32, f32); 13] = [
    (12.6, -0.3, 38.6, 0.9, 0.6, 0.7),
    (13.9, -0.4, 40.6, 0.5, 0.4, 0.6),
    (27.2, -0.3, 13.9, 1.0, 0.7, 0.8),
    (28.6, -0.35, 14.8, 0.5, 0.4, 0.5),
    (44.6, -0.3, 22.2, 1.1, 0.7, 0.9),
    (46.9, -0.35, 36.2, 0.8, 0.55, 0.6),
    (45.9, -0.4, 37.4, 0.5, 0.4, 0.5),
    (40.2, -0.35, 45.0, 0.7, 0.5, 0.9),
    (20.4, -0.3, 46.6, 0.9, 0.6, 0.7),
    (38.6, -0.75, 27.6, 1.3, 0.9, 1.0),
    (39.9, -0.8, 28.7, 0.7, 0.6, 0.7),
    (35.2, -0.7, 21.4, 0.9, 0.6, 1.1),
    (15.4, -0.6, 28.4, 0.8, 0.6, 0.7),
];

const TREES: [(f32, f32, f32); 7] = [
    (8.0, 9.0, 0.4),
    (34.0, 5.5, 1.9),
    (59.0, 6.0, 2.7),
    (59.5, 31.0, 0.9),
    (6.0, 51.0, 3.5),
    (15.0, 57.0, 5.1),
    (4.6, 30.0, 4.2),
];

struct Pad {
    x: f32,
    z: f32,
    r: f32,
    c: f32,
    s: f32,
    flower: bool,
}

struct Stalk {
    foot: Point,
    tip: Point,
    head: bool,
}

struct Clump {
    x: f32,
    z: f32,
    stalks: Vec<Stalk>,
}

struct Stick {
    a: Point,
    b: Point,
    r: f32,
}

struct Pond {
    seed: u32,
    corners: Vec<(f32, f32)>,
    pads: Vec<Pad>,
    clumps: Vec<Clump>,
    sticks: Vec<Stick>,
    spokes: Vec<(f32, f32)>,
    ring: Vec<(f32, f32)>,
    nest_ground: f32,
    willow_ground: f32,
}

/// How the land lies at a point: the distance to the pond's smooth outline (negative inside), the
/// same with its shore's wobble, and the ground's height.
struct Lie {
    raw: f32,
    s: f32,
    height: f32,
}

impl Area for Pond {
    fn new(seed: u64) -> Pond {
        let short = (seed ^ (seed >> 32)) as u32;
        let corners = OUTLINE
            .iter()
            .map(|&(x, z)| {
                let (dx, dz) = (POND.0 - x, POND.1 - z);
                let l = (dx * dx + dz * dz).sqrt();
                (x + dx / l * ROUNDED, z + dz / l * ROUNDED)
            })
            .collect();
        let spokes = (0..5).map(|i| (cos(i as f32 * TAU / 5.0 + 0.4), sin(i as f32 * TAU / 5.0 + 0.4))).collect();
        let ring = (0..7).map(|i| (cos(i as f32 * TAU / 7.0 + 0.2), sin(i as f32 * TAU / 7.0 + 0.2))).collect();
        let mut pond = Pond {
            seed: short,
            corners,
            pads: pads(seed),
            clumps: clumps(seed),
            sticks: sticks(seed),
            spokes,
            ring,
            nest_ground: 0.0,
            willow_ground: 0.0,
        };
        pond.nest_ground = pond.lie(NEST.x, NEST.z).height;
        pond.willow_ground = pond.lie(WILLOW.x, WILLOW.z).height;
        pond
    }

    fn describe(&self) -> Description {
        Description {
            name: "The heron's pond".into(),
            places: vec![
                Place::new("the grotto in the mossy rock", Size::Tiny, ROOM),
                Place::new("the heron's nest", Size::Giant, point(NEST.x, self.nest_ground + NEST_H, NEST.z)),
                Place::new("the hollow willow", Size::Person, point(WILLOW.x, self.willow_ground, WILLOW.z)),
            ],
            borders: Borders::of_ground(|x, z| self.lie(x, z).height),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let lie = self.lie(p.x, p.z);
        let solid = self.ground(p, view, &lie).or(self.stones(p, view)).or(self.rock(p, view));
        let solid = self.hollow(solid, p, view);
        let solid = solid.or(self.nest(p, view)).or(self.willow(p, view)).or(self.trees(p, view)).or(self.log(p, view));
        let water = Answer::new(max(p.y - LEVEL, lie.raw - 3.0), Material::Water).cut(solid.distance);
        solid.or(water).or(self.reeds(p, view, lie.raw)).or(self.lilies(p, view, lie.raw))
    }
}

area!(Pond);

/// The lily pads: a trail of stepping stones from the beach to the rock's shelf, a drift of others
/// about it, and a few great ones a giant can see.
fn pads(seed: u64) -> Vec<Pad> {
    let mut seeds = Seeds::of(seed, 11);
    let mut pads: Vec<Pad> = Vec::new();
    for &(x, z, r) in [(32.7, 34.6, 1.6), (25.0, 35.2, 1.7), (31.2, 29.8, 1.5), (34.6, 38.6, 1.7), (24.4, 39.6, 1.55)].iter() {
        let a = seeds.next(0.0, TAU);
        pads.push(Pad { x, z, r, c: cos(a), s: sin(a), flower: false });
    }
    let way = [(30.6, 45.6), (30.2, 41.5), (28.2, 37.2), (28.6, 33.4), (25.6, 29.8), (22.2, 27.0), (20.9, 24.6)];
    for w in 0..way.len() - 1 {
        let ((ax, az), (bx, bz)): ((f32, f32), (f32, f32)) = (way[w], way[w + 1]);
        let (dx, dz) = (bx - ax, bz - az);
        let n = ((dx * dx + dz * dz).sqrt() / 0.8).floor() as i32;
        for k in 0..n {
            let t = k as f32 / n as f32;
            let x = ax + dx * t + seeds.next(-0.12, 0.12);
            let z = az + dz * t + seeds.next(-0.12, 0.12);
            let a = seeds.next(0.0, TAU);
            pads.push(Pad { x, z, r: seeds.next(0.27, 0.42), c: cos(a), s: sin(a), flower: false });
        }
    }
    for _ in 0..46 {
        let a = seeds.next(0.0, TAU);
        let k = seeds.next(0.15, 1.0);
        let (x, z) = (29.0 + 10.0 * k * cos(a), 33.5 + 8.5 * k * sin(a));
        let r = seeds.next(0.22, 0.52);
        let turn = seeds.next(0.0, TAU);
        let flower = seeds.next(0.0, 1.0) < 0.5 && r > 0.36;
        let free = pads.iter().all(|o| {
            let (dx, dz, apart) = (x - o.x, z - o.z, r + o.r + 0.05);
            dx * dx + dz * dz > apart * apart
        });
        if free {
            pads.push(Pad { x, z, r, c: cos(turn), s: sin(turn), flower });
        }
    }
    pads
}

/// Clumps of reeds along the shore, away from the rock and the beach.
fn clumps(seed: u64) -> Vec<Clump> {
    let mut seeds = Seeds::of(seed, 23);
    let edges = [0usize, 0, 1, 1, 6, 7, 7, 8, 9, 9, 10, 10, 11, 14];
    let mut clumps = Vec::new();
    for k in 0..24 {
        let e = edges[k % edges.len()];
        let ((ax, az), (bx, bz)) = (OUTLINE[e], OUTLINE[(e + 1) % OUTLINE.len()]);
        let t = seeds.next(0.12, 0.9);
        let (x, z) = (mix(ax, bx, t), mix(az, bz, t));
        let (dx, dz) = (POND.0 - x, POND.1 - z);
        let l = (dx * dx + dz * dz).sqrt();
        let inward = seeds.next(0.2, 1.1);
        let (x, z) = (x + dx / l * inward, z + dz / l * inward);
        let mut stalks = Vec::new();
        for _ in 0..7 {
            let (a, r) = (seeds.next(0.0, TAU), seeds.next(0.05, 0.6));
            let foot = point(x + r * cos(a), LEVEL - 1.0, z + r * sin(a));
            let tip = point(foot.x + seeds.next(-0.2, 0.2), LEVEL + seeds.next(1.0, 1.85), foot.z + seeds.next(-0.2, 0.2));
            stalks.push(Stalk { foot, tip, head: seeds.next(0.0, 1.0) < 0.5 });
        }
        clumps.push(Clump { x, z, stalks });
    }
    clumps
}

/// The nest's sticks, about the middle of the snag's foot.
fn sticks(seed: u64) -> Vec<Stick> {
    let mut seeds = Seeds::of(seed, 37);
    let n = 44;
    let mut sticks = Vec::new();
    for i in 0..n {
        let a = i as f32 * TAU / n as f32 + seeds.next(-0.1, 0.1);
        let (c, s) = (cos(a), sin(a));
        let r = seeds.next(2.35, 3.0);
        let at = point(r * c, NEST_H + seeds.next(-1.0, 0.26), r * s);
        let lean = seeds.next(-0.6, 0.6);
        let (lc, ls) = (cos(lean), sin(lean));
        let way = point(-s * lc + c * ls, seeds.next(-0.14, 0.14), c * lc + s * ls);
        let half = seeds.next(0.65, 1.15);
        sticks.push(Stick { a: at - way * half, b: at + way * half, r: seeds.next(0.075, 0.13) });
    }
    sticks
}

/// A ball stretched to reach `r` along each axis: a distance never too long.
fn ellipsoid(p: Point, r: Point) -> f32 {
    let q = point(p.x / r.x, p.y / r.y, p.z / r.z);
    max((q.length() - 1.0) * min(r.x, min(r.y, r.z)), cuboid(p, r))
}

impl Pond {
    /// The distance across the ground to the pond's smooth outline, negative inside.
    fn outline(&self, x: f32, z: f32) -> f32 {
        let v = &self.corners;
        let mut best = f32::MAX;
        let mut inside = false;
        let mut j = v.len() - 1;
        for i in 0..v.len() {
            let (ax, az) = v[i];
            let (bx, bz) = v[j];
            let (ex, ez) = (bx - ax, bz - az);
            let (wx, wz) = (x - ax, z - az);
            let t = clamp((wx * ex + wz * ez) / (ex * ex + ez * ez), 0.0, 1.0);
            let (dx, dz) = (wx - ex * t, wz - ez * t);
            best = min(best, dx * dx + dz * dz);
            let (up, down, left) = (z >= az, z < bz, ex * wz > ez * wx);
            if (up && down && left) || (!up && !down && !left) {
                inside = !inside;
            }
            j = i;
        }
        let d = best.sqrt();
        (if inside { -d } else { d }) - ROUNDED
    }

    fn lie(&self, x: f32, z: f32) -> Lie {
        let raw = self.outline(x, z);
        let flat = |k: f32, seed: u32| noise(point(x / k, 0.5, z / k), self.seed ^ seed);
        let s = raw + 0.9 * flat(6.0, 0x21) + 0.3 * flat(2.0, 0x35);
        let shore = if s > 0.0 { 0.62 * s / (s + 3.5) } else { 1.15 * s / (4.5 - s) };
        let mut height = LEVEL + shore;
        if s > 2.0 {
            height += between(s, 2.0, 8.0) * 0.15 * flat(9.0, 0x47);
            height += between(s, 6.0, 18.0) * (0.9 + 0.7 * flat(13.0, 0x59));
        }
        Lie { raw, s, height }
    }

    /// The ground: meadow, muddy shore, a beach to the south with a worn path to it.
    fn ground(&self, p: Point, view: &View, lie: &Lie) -> Answer {
        if p.y - lie.height > 2.0 {
            return Answer::new((p.y - lie.height - 0.2) * 0.65, Material::Grass);
        }
        let dw = (p - WILLOW).flat_length();
        let lumps = between(dw, 3.0, 5.0);
        let surface = lie.height + lumps * detail(point(p.x, 0.0, p.z), view, 1.5, 0.05, self.seed ^ 0x9e37);
        let above = p.y - surface;

        let beach = between(abs(p.x - 31.5), 7.0, 4.0) * between(p.z, 42.0, 44.5);
        let wide = 0.7 + 0.4 * noise(point(p.x / 1.5, 0.5, p.z / 1.5), self.seed ^ 0x3c) + 1.6 * beach;
        let from_path = abs(p.x - (31.6 + 0.7 * sin(p.z * 0.45)));
        let south = between(p.z, 44.0, 46.0);
        let worn = between(from_path, 1.5, 0.8) * south;
        let turf = between(lie.s, wide, wide + 0.6) * (1.0 - worn) * between(dw, 1.7, 2.6);
        let material = if turf > 0.5 {
            Material::Grass
        } else if dw < 4.0 {
            Material::Soil
        } else if beach > 0.4 && lie.s < wide + 0.5 {
            Material::Sand
        } else if lie.s < wide + 0.3 {
            Material::Mud
        } else {
            Material::Soil
        };
        let mut ground = Answer::new(above * 0.65, material);

        // Tussocks, knee high to nobody but a hill to the tiny.
        let seen = view.sees(0.5);
        if seen > 0.0 && above < 0.6 {
            let here = seen * between(lie.s, 1.0, 4.0) * (1.0 - between(from_path, 3.2, 0.8) * south) * between(dw, 3.0, 6.0);
            if here > 0.0 {
                let tussocks = scatter(p, 2.2, self.seed ^ 0x71, |q, s| {
                    let r = s.next(0.2, 0.35) * here;
                    if s.next(0.0, 1.0) > 0.6 {
                        return 1.5;
                    }
                    min(sphere(point(q.x, above + r * 0.6, q.z), r), 1.5)
                });
                ground = ground.or(Answer::new(tussocks * 0.6, Material::Grass));
            }
        }
        // Pebbles where there is no turf, half sunk: boulders to the tiny.
        let pebbly = view.sees(0.015) * (1.0 - turf);
        if pebbly > 0.0 && abs(above) < 0.03 {
            let pebbles = scatter(p, 0.06, self.seed ^ 0x77, |q, s| {
                let r = s.next(0.004, 0.012) * pebbly;
                sphere(point(q.x, above + r * 0.4, q.z), r)
            });
            ground = ground.or(Answer::new(pebbles * 0.65, Material::Stone));
        }
        // Blades of grass, a forest to the tiny, sunk out of sight where the turf ends.
        let blady = view.sees(0.005) * clamp(turf * 2.0 - 1.0, 0.0, 1.0);
        if blady > 0.0 && above < 0.16 && above > -0.02 {
            let blades = scatter(p, 0.05, self.seed ^ 0x83, |q, s| {
                let top = -0.01 + s.next(0.025, 0.053) * blady;
                let lean = point(s.next(-0.012, 0.012) * blady, top, s.next(-0.012, 0.012) * blady);
                capsule(point(q.x, above, q.z), point(0.0, -0.03, 0.0), lean, 0.0026)
            });
            ground = ground.or(Answer::new(blades * 0.6, Material::Grass));
        }
        ground
    }

    /// The boulders about the shore.
    fn stones(&self, p: Point, view: &View) -> Answer {
        let high = p.y - 0.6;
        if high > 1.0 {
            return Answer::new(high, Material::Stone);
        }
        let mut d = 1000.0;
        for &(x, y, z, rx, ry, rz) in STONES.iter() {
            let q = p - point(x, y, z);
            let r = point(rx, ry, rz);
            let bound = cuboid(q, r);
            if bound < d {
                d = min(d, ellipsoid(q, r));
            }
        }
        if d > 0.3 {
            return Answer::new(d - 0.08, Material::Stone);
        }
        let rough = detail(p, view, 0.6, 0.05, self.seed ^ 0x91);
        let material = if p.y > LEVEL + 0.12 && noise(p / 0.5, self.seed ^ 0x93) > 0.3 { Material::Moss } else { Material::Stone };
        Answer::new((d + rough) * 0.65, material)
    }

    /// The mossy rock, and the stone shelf at its foot where the pads lead.
    fn rock(&self, p: Point, view: &View) -> Answer {
        let bound = cuboid(p - point(19.6, -0.4, 20.6), point(5.8, 3.5, 5.7));
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let main = ellipsoid(p - point(19.3, -0.4, 19.7), point(4.8, 2.9, 3.0));
        let west = ellipsoid(p - point(17.2, -0.4, 20.6), point(2.6, 1.9, 2.6));
        let east = ellipsoid(p - point(21.8, -0.4, 18.8), point(2.9, 2.2, 2.6));
        let crag = smooth_union(smooth_union(main, west, 0.6), east, 0.6);
        let from_mouth = (p - MOUTH).flat_length();

        let crag = if crag > 0.5 {
            Answer::new(crag - 0.25, Material::Stone)
        } else {
            let rough = detail(p, view, 1.6, 0.05, self.seed ^ 0xa1);
            let strata = 0.02 * view.sees(0.5) * sin(p.y * 9.0 + 2.0 * noise(p / 2.5, self.seed ^ 0xa3));
            let d = (crag + rough + strata) * 0.6;
            let arch = between((p - point(MOUTH.x, FLOOR + 0.2, MOUTH.z)).length(), 0.95, 0.6);
            let patch = between(noise(p / 1.2, self.seed ^ 0xa5), 0.2, 0.4) * between(p.y, LEVEL + 0.1, LEVEL + 0.3);
            let moss = max(arch, patch);
            if moss > 0.3 {
                let thick = between(moss, 0.3, 0.7) * view.sees(0.006) * between(from_mouth, 0.4, 0.7);
                let tufts = if thick > 0.0 && abs(d) < 0.1 { thick * 0.004 * (0.6 + 0.4 * noise(p / 0.005, self.seed ^ 0xa7)) } else { thick * 0.004 };
                Answer::new((d - tufts) * (1.0 - 0.5 * thick), Material::Moss)
            } else {
                Answer::new(d, Material::Stone)
            }
        };

        let shelf = ellipsoid(p - point(MOUTH.x, FLOOR - 0.7, MOUTH.z), point(3.2, 0.7, 3.0));
        let shelf = if shelf > 0.2 {
            shelf - 0.05
        } else {
            // Worn level before the crack, so the tiny walk straight in.
            let worn = between(from_mouth, 0.4, 1.0);
            let rough = if worn > 0.0 { worn * detail(p, view, 0.4, 0.04, self.seed ^ 0xa9) } else { 0.0 };
            (shelf + rough) * 0.75
        };
        crag.or(Answer::new(shelf, Material::Stone))
    }

    /// The shadowed arch in the rock's south face, the crack in it, and the grotto behind.
    fn hollow(&self, solid: Answer, p: Point, view: &View) -> Answer {
        let arch = view.sees(0.6);
        let near = cuboid(p - point(MOUTH.x, FLOOR + 0.15, 22.15), point(0.75, 0.5, 1.05));
        if arch == 0.0 || near > 0.05 {
            return solid;
        }
        let above = p.y - FLOOR;
        let recess = max(ellipsoid(p - point(MOUTH.x, FLOOR + 0.05, MOUTH.z + 0.02), point(0.32, 0.5, 0.34)), -above);
        let mut out = solid.cut(recess);
        out.distance = mix(solid.distance, out.distance, arch);
        if recess < 0.05 && p.z < MOUTH.z + 0.1 {
            out.material = if above > 0.02 { Material::Soil } else { Material::Stone };
        }
        let seen = view.sees(0.06);
        if seen == 0.0 {
            return out;
        }
        let rough = detail(p, view, 0.08, 0.06, self.seed ^ 0xb1);
        let floor = (0.25 * rough - above) * 0.85;
        let q = p - point(ROOM.x, FLOOR + 0.14, ROOM.z);
        // The walls rise sheer from the floor, smooth at their foot and rough only above the
        // tiny's jump, so nobody climbs into a pocket of them; the roof is rounded.
        let wall_rough = rough * between(above, 0.015, 0.065);
        let vault = rounded_cuboid(p - point(ROOM.x, FLOOR, ROOM.z), point(0.5, 0.34, 0.42), 0.16);
        let room = max((vault + wall_rough) * 0.6, floor);
        // A jagged crack, a little over two centimetres wide at its foot, narrowing upwards.
        let wiggle = 0.012 * noise(point(p.z / 0.06, p.y / 0.06, 0.5), self.seed ^ 0xb3);
        let up = clamp(above / 0.19, 0.0, 1.0);
        let slot = (abs(p.x - MOUTH.x - wiggle) - 0.011 * (1.0 - 0.6 * up)) / 1.5;
        let crack = max(max(slot, floor), max(above - 0.19, abs(p.z - 22.4) - 0.5));
        let hollow = min(room, crack);
        let mut inner = out.cut(hollow);
        if hollow < 0.02 && p.z < MOUTH.z - 0.2 {
            inner.material = if above < 0.008 {
                Material::Sand
            } else if noise(p / 0.12, self.seed ^ 0xb5) > 0.25 {
                Material::Moss
            } else {
                Material::Stone
            };
        }
        // Stones grown up from the floor and down from the roof.
        if room < 0.02 {
            let spikes = [
                (-0.34, -0.2, 0.11, 0.016),
                (-0.27, -0.27, 0.06, 0.012),
                (0.33, -0.22, 0.14, 0.018),
                (0.38, 0.08, 0.07, 0.013),
                (-0.38, 0.12, 0.09, 0.015),
                (0.18, -0.3, 0.05, 0.011),
            ];
            let mut d = 1000.0;
            for &(x, z, h, r) in spikes.iter() {
                let at = q - point(x, 0.0, z);
                d = min(d, capsule(at, point(0.0, -0.17, 0.0), point(0.0, -0.14 + h * 0.4, 0.0), r * 1.5));
                d = min(d, capsule(at, point(0.0, -0.17, 0.0), point(0.0, -0.14 + h, 0.0), r * 0.7));
                d = min(d, capsule(at, point(0.02, 0.24, 0.02), point(0.02, 0.2 - h * 0.5, 0.02), r));
            }
            inner = inner.or(Answer::new(d, Material::Stone));
        }
        Answer::new(mix(out.distance, inner.distance, seen), inner.material)
    }

    /// The heron's nest: a great bowl of sticks in the crook of a dead snag.
    fn nest(&self, p: Point, view: &View) -> Answer {
        let q = p - point(NEST.x, self.nest_ground, NEST.z);
        let bound = cuboid(q - point(0.0, 2.2, 0.0), point(4.4, 3.4, 4.4));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let mut wood = capsule(q, point(0.0, -1.0, 0.0), point(0.0, 2.6, 0.0), 1.0);
        for &(c, s) in self.spokes.iter() {
            let root = capsule(q, point(c * 0.5, 0.5, s * 0.5), point(c * 2.2, -0.4, s * 2.2), 0.32);
            let limb = capsule(q, point(-c * 0.3, 1.9, -s * 0.3), point(-c * 2.3, 3.4, -s * 2.3), 0.36);
            wood = smooth_union(smooth_union(wood, root, 0.35), limb, 0.35);
        }
        let wood = if wood > 0.4 {
            wood - 0.1
        } else {
            (wood + detail(point(q.x, q.y * 0.2, q.z), view, 0.35, 0.06, self.seed ^ 0xc1)) * 0.75
        };
        let round_body = cylinder(q - point(0.0, NEST_H - 0.45, 0.0), 2.8, 0.45) - 0.3;
        let bowl = cut(round_body, sphere(q - point(0.0, NEST_H + 8.0, 0.0), 8.0));
        let bowl = if bowl > 0.3 { bowl - 0.06 } else { (bowl + detail(q, view, 0.4, 0.06, self.seed ^ 0xc3)) * 0.75 };
        let lined = q.flat_length() < 2.0 && q.y > NEST_H - 0.3;
        let mut nest = Answer::new(wood, Material::Wood).or(Answer::new(bowl, if lined { Material::Moss } else { Material::Wood }));
        let seen = view.sees(0.3);
        if seen > 0.0 {
            let mut d = 1000.0;
            for k in self.sticks.iter() {
                d = min(d, capsule(q, k.a, k.b, k.r * seen));
            }
            nest = nest.or(Answer::new(d, Material::Wood));
        }
        // Three eggs.
        let seen = view.sees(0.7);
        if seen > 0.0 {
            let mut d = 1000.0;
            for &(x, z, a) in [(1.0, 0.45, 0.3), (0.55, 1.15, 1.6), (1.35, 1.2, 2.5)].iter() {
                let e = (q - point(x, NEST_H + 0.34, z)).turned(a);
                d = min(d, ellipsoid(e, point(0.45, 0.35, 0.35) * seen));
            }
            nest = nest.or(Answer::new(d, Material::Sand));
        }
        nest
    }

    /// The old willow, its trunk hollow, with a door a person walks in by.
    fn willow(&self, p: Point, view: &View) -> Answer {
        let q = p - point(WILLOW.x, self.willow_ground, WILLOW.z);
        let bound = cuboid(q - point(0.0, 6.0, 0.0), point(6.2, 8.0, 6.2));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let mut wood = capsule(q, point(0.0, -1.0, 0.0), point(0.0, 5.6, 0.0), 2.15);
        for &(c, s) in self.ring.iter() {
            let root = capsule(q, point(c * 1.5, 0.6, s * 1.5), point(c * 3.4, -0.5, s * 3.4), 0.42);
            wood = smooth_union(wood, root, 0.5);
        }
        for &(c, s) in self.spokes.iter() {
            let limb = capsule(q, point(c * 0.5, 5.2, s * 0.5), point(c * 3.2, 8.6, s * 3.2), 0.6);
            wood = smooth_union(wood, limb, 0.5);
        }
        let wood = if wood > 0.5 {
            wood - 0.15
        } else {
            (wood + detail(point(q.x, q.y * 0.15, q.z), view, 0.5, 0.06, self.seed ^ 0xd1)) * 0.75
        };
        let inside = cylinder(q - point(0.0, 1.1, 0.0), 1.1, 1.75) - 0.25;
        // The door looks north-west, over the water.
        let along = -0.8 * q.x - 0.6 * q.z;
        let across = 0.6 * q.x - 0.8 * q.z;
        let door = rounded_cuboid(point(along - 2.4, q.y - 0.6, across), point(1.6, 1.55, 0.65), 0.35);
        let wood = cut(wood, smooth_union(inside, door, 0.15));

        let mut leaf = sphere(q - point(0.0, 10.0, 0.0), 3.2);
        for &(x, y, z, r) in [(3.0, 8.8, 0.8, 2.4), (-2.6, 9.0, 1.8, 2.5), (0.6, 8.9, -3.0, 2.4), (-1.9, 9.2, -1.8, 2.3), (1.2, 9.0, 2.8, 2.3)].iter() {
            leaf = smooth_union(leaf, sphere(q - point(x, y, z), r), 0.5);
        }
        for &(c, s) in self.ring.iter() {
            leaf = smooth_union(leaf, ellipsoid(q - point(c * 4.2, 6.7, s * 4.2), point(0.95, 2.3, 0.95)), 0.4);
        }
        let leaf = self.foliage(leaf, p, view);
        let moss = q.y < 1.2 && noise(p / 0.8, self.seed ^ 0xd5) > 0.25;
        Answer::new(wood, if moss { Material::Moss } else { Material::Wood }).or(Answer::new(leaf, Material::Leaf))
    }

    /// Leaves in clumps, over a smooth shape.
    fn foliage(&self, leaf: f32, p: Point, view: &View) -> f32 {
        if leaf > 0.7 { leaf - 0.3 } else { (leaf + detail(p, view, 1.4, 0.1, self.seed ^ 0xd3)) * 0.55 }
    }

    /// The trees along the meadow's edges.
    fn trees(&self, p: Point, view: &View) -> Answer {
        let mut best = Answer::new(1000.0, Material::Leaf);
        for &(x, z, a) in TREES.iter() {
            let q = p - point(x, 0.0, z);
            let bound = cuboid(q - point(0.0, 4.5, 0.0), point(3.3, 5.6, 3.3));
            if bound >= best.distance {
                continue;
            }
            if bound > 0.5 {
                best = Answer::new(bound, Material::Leaf);
                continue;
            }
            let trunk = capsule(q, point(0.0, -1.0, 0.0), point(0.0, 5.8, 0.0), 0.4);
            let trunk = if trunk > 0.3 { trunk - 0.05 } else { (trunk + detail(point(q.x, q.y * 0.2, q.z), view, 0.2, 0.06, self.seed ^ 0xe1)) * 0.75 };
            let (c, s) = (cos(a), sin(a));
            let crown = sphere(q - point(0.0, 7.2, 0.0), 2.1);
            let one = sphere(q - point(1.2 * c, 6.1, 1.2 * s), 1.6);
            let two = sphere(q - point(-1.2 * c, 6.4, -1.2 * s), 1.7);
            let three = sphere(q - point(-1.0 * s, 5.9, 1.0 * c), 1.4);
            let leaf = smooth_union(smooth_union(crown, one, 0.4), smooth_union(two, three, 0.4), 0.4);
            best = best.or(Answer::new(trunk, Material::Wood)).or(Answer::new(self.foliage(leaf, p, view), Material::Leaf));
        }
        best
    }

    /// A fallen trunk, its head in the water.
    fn log(&self, p: Point, view: &View) -> Answer {
        let (a, b) = (point(37.6, LEVEL + 0.4, 48.6), point(34.2, LEVEL + 0.02, 43.8));
        let log = capsule(p, a, b, 0.38);
        if log > 0.8 {
            return Answer::new(log - 0.7, Material::Wood);
        }
        let stub = capsule(p, point(35.6, LEVEL + 0.2, 45.8), point(35.1, LEVEL + 0.9, 46.4), 0.15);
        let d = smooth_union(log, stub, 0.15);
        let d = (d + detail(p, view, 0.2, 0.05, self.seed ^ 0xe5)) * 0.75;
        let moss = noise(p / 0.4, self.seed ^ 0xe7) > 0.3 && p.y > LEVEL + 0.35;
        Answer::new(d, if moss { Material::Moss } else { Material::Wood })
    }

    /// Reeds along the shore, as thick as the viewer's eye needs.
    fn reeds(&self, p: Point, view: &View, raw: f32) -> Answer {
        let seen = view.sees(1.2);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Leaf);
        }
        let far = max(p.y - (LEVEL + 2.4), abs(raw + 0.6) - 3.0);
        if far > 0.5 {
            return Answer::new(far, Material::Leaf);
        }
        let r = max(0.012, view.finest * 0.35) * seen;
        let mut best = Answer::new(1000.0, Material::Leaf);
        for c in self.clumps.iter() {
            let (dx, dz) = (p.x - c.x, p.z - c.z);
            let bound = (dx * dx + dz * dz).sqrt() - 1.05;
            if bound >= best.distance {
                continue;
            }
            if bound > 0.4 {
                best = Answer::new(bound, Material::Leaf);
                continue;
            }
            for k in c.stalks.iter() {
                best = best.or(Answer::new(capsule(p, k.foot, k.tip, r), Material::Leaf));
                if k.head {
                    let way = k.tip - k.foot;
                    let (lo, hi) = (k.foot + way * 0.86, k.foot + way * 0.97);
                    best = best.or(Answer::new(capsule(p, lo, hi, r + 0.025 * seen), Material::Wood));
                }
            }
        }
        best
    }

    /// The lily pads afloat on the pond, some in flower.
    fn lilies(&self, p: Point, view: &View, raw: f32) -> Answer {
        let far = max(abs(p.y - LEVEL) - 0.35, raw);
        if far > 0.5 {
            return Answer::new(far, Material::Leaf);
        }
        // Thin enough for the tiny to step up on; as thick as larger eyes need.
        let half = max(0.0025, view.finest * 0.12);
        let veined = view.sees(0.004);
        let bloom = view.sees(0.3);
        let mut best = Answer::new(1000.0, Material::Leaf);
        for k in self.pads.iter() {
            let seen = view.sees(2.0 * k.r);
            if seen == 0.0 {
                continue;
            }
            let q = point(p.x - k.x, p.y - LEVEL, p.z - k.z);
            let r = k.r * seen;
            if q.flat_length() - r - 0.002 >= best.distance {
                continue;
            }
            let mut d = cylinder(q, r, half);
            if d < 0.4 {
                // A notch, from near the middle to the rim.
                let (x, z) = (k.c * q.x + k.s * q.z, k.c * q.z - k.s * q.x);
                d = cut(d, (abs(z) - 0.18 * (x - 0.15 * r)) * 0.97);
                if veined > 0.0 {
                    if d < 0.02 {
                        let out = q.flat_length();
                        let t = atan2(z, x) * (13.0 / TAU) + 0.5;
                        let off = abs(t - (t + 0.5).floor()) * (TAU / 13.0) * out;
                        let vein = 0.0012 * veined * clamp(1.0 - off / 0.005, 0.0, 1.0) * between(out, 0.05, 0.1);
                        d = (d - vein) * 0.85;
                    } else {
                        d -= 0.0012 * veined;
                    }
                }
            }
            best = best.or(Answer::new(d, Material::Leaf));
            if k.flower && bloom > 0.0 {
                let ball = sphere(q, 0.25);
                let d = if ball > 0.3 {
                    ball
                } else {
                    // Petals six ways, by folding; a bud in the middle.
                    let f = q / bloom;
                    let f = point(abs(f.x), f.y, abs(f.z));
                    let a = capsule(f, point(0.02, 0.01, 0.0), point(0.13, 0.1, 0.0), 0.036);
                    let b = capsule(f, point(0.01, 0.01, 0.0173), point(0.065, 0.1, 0.1126), 0.036);
                    let bud = capsule(f, point(0.0, 0.0, 0.0), point(0.0, 0.1, 0.0), 0.04);
                    min(min(a, b), bud) * bloom
                };
                best = best.or(Answer::new(d, Material::Sand));
            }
        }
        best
    }
}
```