//! The walled garden (POC_PLAN.md, step 1), the first area, written by hand: a lawn inside a stone
//! wall 2.4 m high, a gate to the south, a pond with lily pads, and a giant's bench. Outside the
//! wall the ground rises into low hills to the area's edges.
//!
//! Each size has its own garden:
//! - **The tiny** find a crack at the foot of the east wall, by a patch of moss, a little over a
//!   centimetre wide: it leads into a cave inside the wall. Lily pads are rafts; pebbles are
//!   boulders; the mortar between the stones is a ravine, the grain of the bench's wood ridges.
//! - **A person** walks in at the gate, around the pond, and under the bench, whose seat is far
//!   out of reach. The crack is too small to see; the moss shows where it is.
//! - **A giant** steps over the wall, and sits on the bench: its seat is 4.5 m up.

use plateau_sdk::*;

struct Garden {
    seed: u32,
    /// The ground's height at the crack, where the cave's floor is.
    floor: f32,
}

/// The wall runs round a square about the middle, its middle line `RING` from it each way.
const MIDDLE: Point = point(32.0, 0.0, 32.0);
const RING: f32 = 22.0;
const THICK: f32 = 0.3;
const BOTTOM: f32 = -0.6;
const TOP: f32 = 2.4;
const COURSE: f32 = 0.3;
const BLOCK: f32 = 0.6;

/// The crack, in the east wall's inner face (x = 53.7), and the cave behind it.
const CRACK_Z: f32 = 30.0;
const CAVE: Point = point(54.0, 0.0, 30.15);

const POND: Point = point(22.0, 0.0, 36.0);
const LEVEL: f32 = -0.3;
/// The water's edge reaches no further than this from the pond's middle: the shore is nearer,
/// where the ground rises above the level.
const POND_REACH: f32 = 7.7;

const BENCH: Point = point(40.0, 0.0, 18.0);
const SEAT: f32 = 4.5;

impl Area for Garden {
    fn new(seed: u64) -> Garden {
        let seed = (seed ^ (seed >> 32)) as u32;
        let mut garden = Garden { seed, floor: 0.0 };
        garden.floor = garden.height(53.9, CRACK_Z);
        garden
    }

    fn describe(&self) -> Description {
        Description {
            name: "The walled garden".into(),
            places: vec![
                Place::new("the cave behind the crack", Size::Tiny, point(CAVE.x, self.floor, CAVE.z)),
                Place::new("the giant's seat", Size::Giant, point(BENCH.x, SEAT, BENCH.z)),
            ],
            borders: Borders::of_ground(|x, z| self.height(x, z)),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let solid = self.ground(p, view).or(self.wall(p, view));
        let solid = self.cave(solid, p, view);
        let water = Answer::new(self.water(p), Material::Water).cut(solid.distance);
        solid.or(self.bench(p, view)).or(water).or(self.pads(p, view))
    }
}

area!(Garden);

impl Garden {
    /// The ground's height, without its roughness: a lawn nearly flat inside the wall, the pond's
    /// bowl sunk into it, and hills rising outside.
    fn height(&self, x: f32, z: f32) -> f32 {
        let lawn = 0.1 * noise(point(x / 9.0, 0.0, z / 9.0), self.seed);
        let outside = max(abs(x - MIDDLE.x), abs(z - MIDDLE.z)) - (RING + THICK);
        let hills = 0.9 * between(outside, 0.0, 8.0) * (0.6 + noise(point(x / 12.0, 0.0, z / 12.0), self.seed ^ 0x51));
        let r = point(x - POND.x, 0.0, z - POND.z).flat_length();
        let bowl = 1.4 * ease(1.0 - r / 10.0);
        lawn + hills - bowl
    }

    /// The ground: grass, mud about the pond, and for the tiny, pebbles.
    fn ground(&self, p: Point, view: &View) -> Answer {
        // Lumps and clods, smoothed away along the wall's foot, so the ground meets it level.
        let along_wall = between(abs(square(p.x - MIDDLE.x, p.z - MIDDLE.z, RING, RING)), 0.4, 1.2);
        let surface = self.height(p.x, p.z) + along_wall * detail(point(p.x, 0.0, p.z), view, 1.5, 0.05, self.seed ^ 0x9e37);
        let r = (p - POND).flat_length();
        let shore = 8.0 + 0.6 * noise(p / 1.5, self.seed ^ 0x3c);
        let depth = surface - p.y;
        let material = if r < shore {
            strata(depth, &[(Material::Mud, 0.45), (Material::Soil, 0.8), (Material::Stone, 0.0)])
        } else {
            strata(depth, &[(Material::Grass, 0.08), (Material::Soil, 0.7), (Material::Stone, 0.0)])
        };
        // The lumps' octaves add up to slopes of nearly one where they line up: the distance made
        // shorter to stay never too long (the law found it 1.1 times too long at 0.85).
        let ground = Answer::new((p.y - surface) * 0.7, material);
        // Pebbles, a centimetre or so across, half sunk: boulders to the tiny, unseen by anyone else.
        let seen = view.sees(0.015);
        if seen == 0.0 || abs(p.y - surface) > 0.03 {
            return ground;
        }
        let pebbles = scatter(p, 0.06, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.012) * seen;
            sphere(point(q.x, p.y - surface + r * 0.4, q.z), r)
        });
        // Each is placed by its height above the sloping ground, which shears it: its distance
        // made shorter as the ground's is.
        ground.or(Answer::new(pebbles * 0.7, Material::Stone))
    }

    /// The wall, of stone blocks in courses, a coping on top, the gate cut through to the south;
    /// moss at its foot.
    fn wall(&self, p: Point, view: &View) -> Answer {
        let q = p - MIDDLE;
        let ring = abs(square(q.x, q.z, RING, RING)) - THICK;
        let body = extrude(ring, p.y, (BOTTOM + TOP) * 0.5, (TOP - BOTTOM) * 0.5);
        let coping = extrude(abs(square(q.x, q.z, RING, RING)) - 0.4, p.y, TOP + 0.075, 0.075) - 0.02;
        let gate = cuboid(p - point(MIDDLE.x, 1.2, MIDDLE.z + RING), point(0.75, 1.6, 0.6));
        let coarse = cut(min(body, coping), gate);
        // Rough stone and the mortar between blocks, only near the wall's face.
        if coarse > 0.1 {
            return Answer::new(coarse - 0.05, Material::Stone);
        }
        // The roughness is steeper than a distance grows: the distance made shorter to stay never
        // too long.
        let mut d = (coarse + detail(p, view, 0.3, 0.025, self.seed ^ 0x7f4a)) * 0.85;
        let depth = 0.025 * view.sees(0.03);
        if depth > 0.0 {
            // The courses are horizontal; the joints between blocks run across each wall, half a
            // block along on every other course.
            let along = if abs(q.z) > abs(q.x) { p.x } else { p.z };
            let course = (p.y / COURSE).floor();
            let bed = abs(p.y - (p.y / COURSE + 0.5).floor() * COURSE) - 0.012;
            let shift = if (course as i32) & 1 == 1 { BLOCK * 0.5 } else { 0.0 };
            let t = (along + shift) / BLOCK;
            let joint = abs(t - (t + 0.5).floor()) * BLOCK - 0.012;
            let mortar = min(bed, joint);
            // Cut out where it is mortar, as deep as `depth` into the face, up to the coping: a
            // groove closed at the top, so the distance runs on into the coping's without a jump.
            d = max(d, min(min(-mortar, d + depth), TOP - p.y));
        }
        let mut wall = Answer::new(d, Material::Stone);
        // Moss at the wall's foot in patches, and always about the crack's mouth.
        let foot = self.height(p.x, p.z);
        let patch = noise(p / 0.7, self.seed ^ 0x6d) + 1.5 * between(-(p - point(53.7, self.floor, CRACK_Z)).length(), -0.4, -0.15);
        let moss = between(patch, 0.25, 0.45) * between(foot + 0.35 - p.y, 0.0, 0.1);
        if moss > 0.0 {
            // Moss is a thin skin over the stone wall: 3 cm deep it gives way to the stone core.
            let mat = strata(-wall.distance, &[(Material::Moss, 0.03), (Material::Stone, 0.0)]);
            // A forest of tufts to the tiny.
            // They rise and fall as steeply as a distance grows: the distance halved where they are.
            let seen = view.sees(0.006) * moss;
            let tufts = seen * 0.004 * (0.6 + 0.4 * noise(p / 0.005, self.seed ^ 0x2b));
            wall = Answer::new((wall.distance - tufts) * (1.0 - 0.5 * seen), mat);
        }
        wall
    }

    /// The crack in the east wall's inner face and the cave it leads to, cut out of the wall and
    /// the ground, for those small enough to see them: faded in from a person's half size down.
    fn cave(&self, solid: Answer, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        let near = cuboid(p - point(53.95, self.floor + 0.1, 30.15), point(0.45, 0.3, 0.6));
        if seen == 0.0 || near > 0.05 {
            return solid;
        }
        let f = self.floor;
        let rough = detail(p, view, 0.05, 0.08, self.seed ^ 0x4c);
        let room = (rounded_cuboid(p - point(CAVE.x, f + 0.12, CAVE.z), point(0.17, 0.12, 0.3), 0.09) + rough) * 0.6;
        // The crack: a jagged slot 1.4 cm wide, narrowing upwards, from the face to the room.
        let wiggle = 0.012 * noise(point(p.x / 0.04, p.y / 0.04, 0.0), self.seed ^ 0x19);
        let up = clamp((p.y - f) / 0.2, 0.0, 1.0);
        let slot = (abs(p.z - CRACK_Z - wiggle) - 0.007 * (1.0 - 0.6 * up)) / 1.5;
        let crack = intersect(intersect(slot, abs(p.y - (f + 0.1)) - 0.1), abs(p.x - 53.845) - 0.175);
        let hollow = union(room, crack);
        let mut cut_out = solid.cut(hollow);
        // Inside, a floor of bare earth under walls of stone.
        if hollow < 0.02 && p.x > 53.72 {
            cut_out.material = if p.y > f + 0.01 { Material::Stone } else { Material::Soil };
        }
        Answer::new(mix(solid.distance, cut_out.distance, seen), cut_out.material)
    }

    /// The pond's water: still, up to its level, wherever the ground is below it.
    fn water(&self, p: Point) -> f32 {
        max(p.y - LEVEL, (p - POND).flat_length() - POND_REACH)
    }

    /// Lily pads afloat on the pond: rafts to the tiny, leaves to a person, unseen by a giant. Thin
    /// as a leaf to the tiny, and as thick as a person's eye needs to see them.
    fn pads(&self, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.4);
        let far = Answer::new(max(abs(p.y - LEVEL) - 0.1, (p - POND).flat_length() - 6.0), Material::Leaf);
        // None at all, or none near: the distance to where they could be, which is never too
        // long. Only well away from it, or rays would crawl along its edge.
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Leaf);
        }
        if far.distance > 0.05 {
            return far;
        }
        let half = max(0.004, view.finest * 0.12);
        let pads = scatter(p - POND, 1.6, self.seed ^ 0x11, |q, s| {
            let r = s.next(0.15, 0.3) * seen;
            if s.next(0.0, 1.0) > 0.55 {
                return 1.0;
            }
            // A notch cut into each, as lily pads have.
            let pad = cylinder(q - point(0.0, LEVEL + half, 0.0), r, half);
            let notch = cuboid((q - point(0.0, LEVEL, 0.0)).turned(s.next(0.0, TAU)) - point(r * 0.5, 0.0, 0.0), point(r * 0.5, 0.1, 0.02 * seen));
            cut(pad, notch)
        });
        // Only pads that lie on the water, well inside the shore.
        Answer::new(max(pads, (p - POND).flat_length() - 5.5), Material::Leaf)
    }

    /// The giant's bench: a seat of four planks 4.5 m up on four legs, a backrest behind; the grain
    /// of its wood for the tiny.
    fn bench(&self, p: Point, view: &View) -> Answer {
        let q = p - BENCH;
        let bound = cuboid(q - point(0.0, 4.0, -0.4), point(6.2, 4.4, 2.6));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        // Four planks along x, 4 cm apart, 90 cm thick: thinner, and a giant's view (drawn in voxels
        // of half a metre near it, a metre further off) breaks them up.
        // The gaps between them, and their rounded edges, are detail: closed for whoever is too
        // large to see them, or a coarse view would fall in the gaps and miss the seat. Closed, they
        // overlap: planks that only touched would be a surface inside the seat, where the distance
        // is nought, and a coarse view that asks there (a giant's asks on whole metres) finds none.
        let seen = view.sees(0.04);
        let plank = |k: f32| rounded_cuboid(q - point(0.0, SEAT - 0.45, -1.5 + k), point(6.0, 0.45, 0.48 + 0.04 * (1.0 - seen)), 0.05 * seen);
        let seat = union(union(plank(0.0), plank(1.0)), union(plank(2.0), plank(3.0)));
        let leg = |x: f32, z: f32, top: f32| cuboid(q - point(x, (top - 0.4) * 0.5, z), point(0.4, (top + 0.4) * 0.5, 0.4));
        let front = union(leg(-5.0, 1.4, SEAT - 0.9), leg(5.0, 1.4, SEAT - 0.9));
        let back = union(leg(-5.0, -1.6, 8.0), leg(5.0, -1.6, 8.0));
        let rest = rounded_cuboid(q - point(0.0, 6.8, -1.6), point(5.6, 1.0, 0.4), 0.05);
        let coarse = union(union(seat, rest), union(front, back));
        // The grain runs along the planks: noise drawn out ten times along x.
        // Near the wood, and faded out further off into the most it could add (a centimetre), so the
        // distance never jumps.
        let near = between(coarse, 0.03, 0.02);
        let grain = if near > 0.0 { detail(point(q.x * 0.1, q.y, q.z), view, 0.08, 0.06, self.seed ^ 0x3d) } else { 0.0 };
        Answer::new((coarse + mix(-0.01, grain, near)) * 0.75, Material::Wood)
    }
}

/// The distance to a square `hx` by `hz` about the origin, across the ground.
fn square(x: f32, z: f32, hx: f32, hz: f32) -> f32 {
    let (qx, qz) = (abs(x) - hx, abs(z) - hz);
    (max(qx, 0.0) * max(qx, 0.0) + max(qz, 0.0) * max(qz, 0.0)).sqrt() + min(max(qx, qz), 0.0)
}

/// A shape `d` across the ground stood up from `middle - half` to `middle + half`.
fn extrude(d: f32, y: f32, middle: f32, half: f32) -> f32 {
    let h = abs(y - middle) - half;
    (max(d, 0.0) * max(d, 0.0) + max(h, 0.0) * max(h, 0.0)).sqrt() + min(max(d, h), 0.0)
}
