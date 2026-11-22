//! The heron's pond: a round pond in a shallow meadow, its water dotted with lily pads and ringed
//! with reeds, a mossy stone on its north shore, bushes about the meadow, and to the north-west an
//! old alder, broken off at the height of a giant's knee, with a heron's nest of sticks on its stump.
//!
//! Each size has its own pond:
//! - **The tiny** walk on the water and step from lily pad to lily pad. The pads ride low, two
//!   millimetres proud of the water, an easy step up. Reeds are a forest to them. On the mossy
//!   stone's south face, where the moss is thickest, a crack a little over a centimetre wide leads
//!   into a room inside the stone.
//! - **A person** wades the pond among the pads, the reeds and the shore stones. They walk in at a
//!   low door in the alder's trunk to the hollow inside it, under the nest they cannot reach.
//! - **A giant** steps up onto the broken alder and into the heron's nest, where two eggs lie.

use plateau_sdk::*;
use core::cell::Cell;

const BUSHES: usize = 8;

struct Heron {
    seed: u32,
    /// The ground's height about the mossy stone, flattened: the floor of the room inside it.
    floor: f32,
    /// The ground's height about the alder, flattened.
    base: f32,
    /// Loose sticks poking out of the nest, from end to end, relative to the alder's foot.
    sticks: [(Point, Point); 10],
    /// Bushes about the meadow: three lobes each, a middle and a radius.
    bushes: [(Point, f32); BUSHES * 3],
}

const MIDDLE: Point = point(32.0, 0.0, 32.0);
const START: Point = point(32.0, 0.0, 60.0);

const POND: Point = point(30.0, 0.0, 33.0);
const LEVEL: f32 = -0.3;
/// The water reaches no further than this from the pond's middle: the shore is nearer.
const POND_REACH: f32 = 8.6;

/// The mossy stone: a block of rock on the north shore, its south face to the pond.
const BOULDER: Point = point(31.0, 0.0, 21.0);
const BHALF: Point = point(1.6, 1.3, 1.2);
const FACE_Z: f32 = 22.2;
const CRACK_X: f32 = 31.0;
const ROOM_Z: f32 = 21.88;

/// The broken alder, its nest on top.
const SNAG: Point = point(17.0, 0.0, 19.0);
const TRUNK_R: f32 = 3.0;
/// The nest's floor, above the alder's foot: out of a person's reach, a step for a giant.
const NEST: f32 = 3.4;
const SPIRE_A: Point = point(3.3, 2.0, 0.3);
const SPIRE_B: Point = point(4.6, 9.0, 1.2);

impl Area for Heron {
    fn new(seed: u64) -> Heron {
        let s32 = (seed ^ (seed >> 32)) as u32;
        let zero = point(0.0, 0.0, 0.0);
        let mut h = Heron {
            seed: s32,
            floor: 0.0,
            base: 0.0,
            sticks: [(zero, zero); 10],
            bushes: [(point(-100.0, -100.0, -100.0), 0.0); BUSHES * 3],
        };
        h.floor = h.raw(BOULDER.x, BOULDER.z);
        h.base = h.raw(SNAG.x, SNAG.z);
        for i in 0..10 {
            let mut s = Seeds::of(seed, 300 + i);
            let a = i as f32 * TAU / 10.0 + s.next(-0.2, 0.2);
            let (c, sn) = (cos(a), sin(a));
            let r0 = s.next(2.4, 3.0);
            let r1 = s.next(4.2, 4.8);
            let y0 = s.next(2.95, 3.3);
            let y1 = s.next(2.95, 3.65);
            let tw = s.next(-0.9, 0.9);
            let from = point(c * r0 - sn * tw * 0.3, y0, sn * r0 + c * tw * 0.3);
            let to = point(c * r1 + sn * tw, y1, sn * r1 - c * tw);
            h.sticks[i] = (from, to);
        }
        // Bushes, kept clear of the start, the pond, the stone and the alder.
        let mut found = 0;
        let mut k = 0;
        while found < BUSHES && k < 400 {
            let mut s = Seeds::of(seed, 500 + k);
            k += 1;
            let x = s.next(5.0, 59.0);
            let z = s.next(5.0, 59.0);
            let r = s.next(1.3, 2.2);
            let a = s.next(0.0, TAU);
            let at = point(x, 0.0, z);
            if (at - START).flat_length() < 12.0
                || (at - POND).flat_length() < 14.0
                || (at - SNAG).flat_length() < 13.0
                || (at - BOULDER).flat_length() < 8.0
            {
                continue;
            }
            let mut clash = false;
            for j in 0..found {
                if (at - h.bushes[j * 3].0).flat_length() < 7.0 {
                    clash = true;
                }
            }
            if clash {
                continue;
            }
            let y = h.height(x, z);
            h.bushes[found * 3] = (point(x, y + r * 0.55, z), r);
            h.bushes[found * 3 + 1] = (point(x + cos(a) * r * 0.8, y + r * 0.35, z + sin(a) * r * 0.8), r * 0.7);
            h.bushes[found * 3 + 2] = (point(x + cos(a + 2.4) * r * 0.75, y + r * 0.3, z + sin(a + 2.4) * r * 0.75), r * 0.6);
            found += 1;
        }
        h
    }

    fn describe(&self) -> Description {
        Description {
            name: "The heron's pond".into(),
            places: vec![
                Place::new("the room inside the mossy stone", Size::Tiny, point(CRACK_X, self.floor, ROOM_Z)),
                Place::new("the hollow in the broken alder", Size::Person, point(SNAG.x, self.base, SNAG.z)),
                Place::new("the heron's nest", Size::Giant, point(SNAG.x - 0.6, self.base + NEST, SNAG.z - 0.6)),
            ],
            borders: Borders::of_ground(|x, z| self.height(x, z)),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let solid = self.ground(p, view).or(self.boulder(p, view));
        let solid = self.cave(solid, p, view);
        let water = Answer::new(self.water(p), Material::Water).cut(solid.distance);
        solid.or(self.snag(p, view)).or(self.bush(p, view)).or(water).or(self.pads(p, view))
    }
}

area!(Heron);

impl Heron {
    /// The meadow's height before it is levelled about the stone and the alder: gentle swells,
    /// the pond's bowl, and low hills rising to the area's edges.
    fn raw(&self, x: f32, z: f32) -> f32 {
        let at = point(x, 0.0, z);
        let lawn = 0.2 * noise(point(x / 10.0, 0.0, z / 10.0), self.seed);
        let rise = between((at - MIDDLE).flat_length(), 20.0, 34.0);
        let hills = 1.4 * rise * (0.55 + 0.45 * noise(point(x / 13.0, 0.0, z / 13.0), self.seed ^ 0x51));
        let r = (at - POND).flat_length();
        let bowl = 1.5 * ease(1.0 - r / 10.0);
        lawn + hills - bowl
    }

    /// The ground's height: level about the mossy stone and about the alder.
    fn height(&self, x: f32, z: f32) -> f32 {
        let h = self.raw(x, z);
        let sb = square(x - BOULDER.x, z - BOULDER.z, BHALF.x, BHALF.z);
        let h = mix(h, self.floor, 1.0 - between(sb, 0.6, 3.0));
        let ds = (point(x, 0.0, z) - SNAG).flat_length() - TRUNK_R;
        mix(h, self.base, 1.0 - between(ds, 1.5, 6.0))
    }

    /// The ground: grass, mud about the pond, soil inside the alder. Shore stones and reeds for a
    /// person, pebbles for the tiny.
    fn ground(&self, p: Point, view: &View) -> Answer {
        let sb = square(p.x - BOULDER.x, p.z - BOULDER.z, BHALF.x, BHALF.z);
        // Lumps and clods, smoothed away at the stone's foot, so the ground meets it level.
        let calm = between(sb, 0.2, 1.0);
        let level = self.height(p.x, p.z);
        let surface = level + calm * detail(point(p.x, 0.0, p.z), view, 1.5, 0.05, self.seed ^ 0x9e37);
        let r = (p - POND).flat_length();
        let shore = 8.0 + 0.6 * noise(p / 1.5, self.seed ^ 0x3c);
        let in_trunk = (p - SNAG).flat_length() < TRUNK_R - 0.2;
        let material = if in_trunk {
            Material::Soil
        } else if r < shore {
            Material::Mud
        } else {
            Material::Grass
        };
        let mut ground = Answer::new((p.y - surface) * 0.7, material);
        let h = p.y - surface;

        // Shore stones about the water's edge, a person's stepping stones.
        let ss = view.sees(0.5);
        if ss > 0.0 && r > 5.0 && r < 10.5 && h > -0.5 {
            if h > 1.0 {
                ground = ground.or(Answer::new((h - 0.3) * 0.7, material));
            } else {
                let stones = scatter(p, 2.5, self.seed ^ 0x5a, |q, s| {
                    let place = p - q;
                    let rr = (point(place.x, 0.0, place.z) - POND).flat_length();
                    let rad = s.next(0.25, 0.5) * ss;
                    if s.next(0.0, 1.0) > 0.45 || rr < 6.3 || rr > 9.2 {
                        return 1.0;
                    }
                    sphere(point(q.x, h + rad * 0.45, q.z), rad)
                });
                ground = ground.or(Answer::new(stones * 0.7, Material::Stone));
            }
        }

        // Reeds in clumps about the shore, cattails on some: a person's reed beds, the tiny's
        // forest. None by the mossy stone. They grow as the viewer shrinks, slowly enough that
        // no viewer sees them taller than a larger one by more than a tenth of its height.
        let grow = view.sees(2.0);
        if grow > 0.0 && h > -0.6 {
            let thick = max(view.sees(0.4), 0.2);
            let band = max(max(6.7 - r, r - 10.7), h - 2.4);
            if band > 1.0 {
                ground = ground.or(Answer::new(band * 0.7, Material::Grass));
            } else {
                let heads = Cell::new(1000.0f32);
                let stalks = scatter(p, 1.6, self.seed ^ 0x4ee, |q, s| {
                    let place = p - q;
                    let rr = (point(place.x, 0.0, place.z) - POND).flat_length();
                    let keep = s.next(0.0, 1.0);
                    let by_stone = (point(place.x, 0.0, place.z) - BOULDER).flat_length() < 4.8;
                    if keep > 0.6 || rr < 7.6 || rr > 9.8 || by_stone {
                        return 1.0;
                    }
                    let local = point(q.x, h, q.z);
                    let mut d = 1000.0;
                    for k in 0..4 {
                        let bx = s.next(-0.25, 0.25);
                        let bz = s.next(-0.25, 0.25);
                        let top = s.next(1.3, 2.0) * grow;
                        let lx = s.next(-0.3, 0.3) * grow;
                        let lz = s.next(-0.3, 0.3) * grow;
                        let a = point(bx, -0.4, bz);
                        let b = point(bx + lx, top, bz + lz);
                        d = min(d, capsule(local, a, b, 0.09 * thick));
                        if k < 2 {
                            let c0 = a + (b - a) * 0.72;
                            let c1 = a + (b - a) * 0.88;
                            let hd = capsule(local, c0, c1, 0.15 * thick);
                            if hd < heads.get() {
                                heads.set(hd);
                            }
                            d = min(d, hd);
                        }
                    }
                    d
                });
                let mat = if heads.get() <= stalks + 0.001 { Material::Wood } else { Material::Grass };
                ground = ground.or(Answer::new(stalks * 0.7, mat));
            }
        }

        // Pebbles, a centimetre or so across, half sunk: boulders to the tiny. None at the mossy
        // stone's foot, where the way to the crack is.
        let keep = between(sb, 0.3, 0.8);
        let seen = view.sees(0.015) * keep;
        if seen == 0.0 || abs(h) > 0.03 {
            return ground;
        }
        let pebbles = scatter(p, 0.06, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.012) * seen;
            sphere(point(q.x, h + r * 0.4, q.z), r)
        });
        ground.or(Answer::new(pebbles * 0.7, Material::Stone))
    }

    /// Bushes about the meadow: round lumps of leaves, leafy to a person and the tiny.
    fn bush(&self, p: Point, view: &View) -> Answer {
        let mut d = 1000.0;
        for i in 0..BUSHES {
            let mut b = 1000.0;
            for j in 0..3 {
                let (c, r) = self.bushes[i * 3 + j];
                if r > 0.0 {
                    b = smooth_union(b, sphere(p - c, r), 0.8);
                }
            }
            d = min(d, b);
        }
        if d > 0.5 {
            return Answer::new(d - 0.2, Material::Leaf);
        }
        let leafy = detail(p, view, 0.8, 0.08, self.seed ^ 0x1ea);
        Answer::new((d + leafy) * 0.6, Material::Leaf)
    }

    /// The mossy stone: a block of rock with a rounded crown, rough to a person, grained to the
    /// tiny; moss at its foot and on its crown, thickest about the crack.
    fn boulder(&self, p: Point, view: &View) -> Answer {
        let f = self.floor;
        let q = p - point(BOULDER.x, f + 0.9, BOULDER.z);
        let body = rounded_cuboid(q, BHALF, 0.5);
        let crown = sphere(q - point(-0.4, 0.9, -0.3), 1.3);
        let coarse = smooth_union(body, crown, 0.6);
        if coarse > 0.15 {
            return Answer::new(coarse - 0.06, Material::Stone);
        }
        // Rough, but smooth about the crack's mouth, so the crack opens cleanly.
        let mouth = point(CRACK_X, f + 0.1, FACE_Z);
        let to_mouth = (p - mouth).length();
        let calm = between(to_mouth, 0.25, 0.6);
        let rough = calm * detail(p, view, 0.6, 0.03, self.seed ^ 0x7f4a);
        let mut stone = Answer::new((coarse + rough) * 0.7, Material::Stone);

        let patch = noise(p / 0.7, self.seed ^ 0x6d) + 1.5 * between(-to_mouth, -0.45, -0.2);
        let foot = between(patch, 0.25, 0.45) * between(f + 0.4 - p.y, 0.0, 0.1);
        let top = between(noise(p / 1.1, self.seed ^ 0x6e), 0.0, 0.3) * between(q.y, 1.3, 1.7);
        let moss = max(foot, top);
        if moss > 0.0 {
            stone.material = Material::Moss;
            // A forest of tufts to the tiny: the distance halved where they are.
            let seen = view.sees(0.006) * moss;
            let tufts = seen * 0.004 * (0.6 + 0.4 * noise(p / 0.005, self.seed ^ 0x2b));
            stone = Answer::new((stone.distance - tufts) * (1.0 - 0.5 * seen), stone.material);
        }
        stone
    }

    /// The crack in the mossy stone's south face and the room behind it, for those small enough
    /// to see them.
    fn cave(&self, solid: Answer, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        if seen == 0.0 {
            return solid;
        }
        let f = self.floor;
        let near = cuboid(p - point(CRACK_X, f + 0.12, 21.98), point(0.6, 0.45, 0.6));
        if near > 0.05 {
            return solid;
        }
        // Rough walls and roof over a level floor.
        let lift = between(p.y - f, 0.005, 0.04);
        let rough = lift * detail(p, view, 0.05, 0.08, self.seed ^ 0x4c);
        let room = (rounded_cuboid(p - point(CRACK_X, f + 0.12, ROOM_Z), point(0.3, 0.12, 0.17), 0.09) + rough) * 0.6;
        // The crack: a jagged slot 1.4 cm wide, narrowing upwards, from outside the face into the
        // room.
        let wiggle = 0.012 * noise(point(0.0, p.y / 0.04, p.z / 0.04), self.seed ^ 0x19);
        let up = clamp((p.y - f) / 0.2, 0.0, 1.0);
        let slot = (abs(p.x - CRACK_X - wiggle) - 0.007 * (1.0 - 0.6 * up)) / 1.5;
        let crack = intersect(intersect(slot, abs(p.y - (f + 0.1)) - 0.1), abs(p.z - 22.025) - 0.225);
        let hollow = union(room, crack);
        let mut out = solid.cut(hollow);
        if hollow < 0.02 && p.z < FACE_Z - 0.01 {
            out.material = if p.y < f + 0.01 { Material::Soil } else { Material::Stone };
        }
        Answer::new(mix(solid.distance, out.distance, seen), out.material)
    }

    /// The pond's water: still, up to its level, wherever the ground is below it.
    fn water(&self, p: Point) -> f32 {
        max(p.y - LEVEL, (p - POND).flat_length() - POND_REACH)
    }

    /// Lily pads afloat on the pond, riding low: half in the water, two millimetres proud of it to
    /// the tiny, an easy step from pad to pad. Leaves to a person, unseen by a giant.
    fn pads(&self, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.4);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Leaf);
        }
        let far = Answer::new(max(abs(p.y - LEVEL) - 0.1, (p - POND).flat_length() - 6.0), Material::Leaf);
        if far.distance > 0.05 {
            return far;
        }
        let half = max(0.002, view.finest * 0.12);
        let pads = scatter(p - POND, 1.3, self.seed ^ 0x11, |q, s| {
            let r = s.next(0.18, 0.4) * seen;
            if s.next(0.0, 1.0) > 0.6 {
                return 1.0;
            }
            let pad = cylinder(q - point(0.0, LEVEL, 0.0), r, half);
            let notch = cuboid((q - point(0.0, LEVEL, 0.0)).turned(s.next(0.0, TAU)) - point(r * 0.5, 0.0, 0.0), point(r * 0.5, 0.1, 0.02 * seen));
            cut(pad, notch)
        });
        Answer::new(max(pads, (p - POND).flat_length() - 5.5), Material::Leaf)
    }

    /// The broken alder: a great trunk snapped off low, one splinter still standing, a door at its
    /// foot into the hollow inside; on the stump the heron's nest of sticks, lined with moss, two
    /// eggs in it.
    fn snag(&self, p: Point, view: &View) -> Answer {
        let q = p - point(SNAG.x, self.base, SNAG.z);
        // Far off, the distance to what holds it all: the stump and nest, and the splinter.
        let outer = min(cylinder(q - point(0.0, 1.85, 0.0), 5.1, 2.5), capsule(q, SPIRE_A, SPIRE_B, 0.8));
        if outer > 0.5 {
            return Answer::new(outer, Material::Wood);
        }
        let trunk = cylinder(q - point(0.0, 1.1, 0.0), TRUNK_R, 2.1);
        let flare = cylinder(q - point(0.0, -0.3, 0.0), TRUNK_R + 0.5, 0.3) - 0.1;
        let spire = capsule(q, SPIRE_A, SPIRE_B, 0.55);
        let wood = smooth_union(smooth_union(trunk, flare, 0.8), spire, 0.6);
        // The hollow and its door, cut a little below the ground.
        let hollow = cylinder(q - point(0.0, 1.0, 0.0), 2.0, 1.3);
        let door = rounded_cuboid(q - point(0.0, 0.85, 2.95), point(0.7, 1.15, 1.3), 0.25);
        let wood = cut(wood, union(hollow, door));
        // Bark furrows running up the trunk: noise drawn out five times along y.
        let bark = detail(point(q.x, q.y * 0.2, q.z), view, 0.5, 0.06, self.seed ^ 0xb4);
        let tree = Answer::new((wood + bark) * 0.7, Material::Wood);

        // The nest: a thick mat of sticks on the stump, a rim about it.
        let bowl = cylinder(q - point(0.0, 3.15, 0.0), 3.7, 0.05) - 0.2;
        let rim = torus(q - point(0.0, NEST, 0.0), 3.4, 0.5);
        let body = smooth_union(bowl, rim, 0.4);
        let ws = view.sees(0.25);
        let weave = if ws > 0.0 {
            0.04 * ws * sin((q.x * 0.8 + q.z * 0.6) * 9.0) * sin((q.z * 0.8 - q.x * 0.6) * 9.0 + q.y * 4.0)
        } else {
            0.0
        };
        let twig = detail(q, view, 0.12, 0.06, self.seed ^ 0x2e);
        let lined = q.flat_length() < 2.8 && q.y > NEST - 0.15;
        let nest = Answer::new((body + weave + twig) * 0.55, if lined { Material::Moss } else { Material::Wood });

        // Loose sticks, for a person and the tiny.
        let ss = view.sees(0.32);
        let mut sticks = 1000.0;
        if ss > 0.0 {
            for &(a, b) in self.sticks.iter() {
                sticks = min(sticks, capsule(q, a, b, 0.16 * ss));
            }
        }

        // Two eggs, lying in the moss.
        let egg = |c: Point| capsule(q, c, c + point(0.35, 0.05, 0.2), 0.42);
        let eggs = min(egg(point(1.3, NEST + 0.38, 1.2)), egg(point(0.4, NEST + 0.38, 1.9)));

        tree.or(nest).or(Answer::new(sticks, Material::Wood)).or(Answer::new(eggs, Material::Stone))
    }
}

/// The distance to a rectangle `hx` by `hz` about the origin, across the ground.
fn square(x: f32, z: f32, hx: f32, hz: f32) -> f32 {
    let (qx, qz) = (abs(x) - hx, abs(z) - hz);
    (max(qx, 0.0) * max(qx, 0.0) + max(qz, 0.0) * max(qz, 0.0)).sqrt() + min(max(qx, qz), 0.0)
}
