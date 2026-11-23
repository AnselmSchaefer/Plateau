//! The headland: low fields rise northward to a headland that ends in a cliff, with a lighthouse on
//! it, a sea stack off it, and smugglers' tunnels in its rock.
//!
//! - **The tiny** go in at two clefts, marked with moss, in the back wall of the sea cave: to a hall
//!   with a hoard, a store, and a hidden pool, all joined by tunnels two fingers wide.
//! - **A person** climbs the path through the dry-stone wall to the lighthouse, and up its winding
//!   stair to the lamp gallery; or walks along the beach and the cut path to the sea cave.
//! - **A giant** wades out to the sea stack and sits in it as in a chair.

use plateau_sdk::*;

const SEA: f32 = -0.5;
/// The lighthouse's foot.
const BASE: f32 = 8.0;
const LIGHT: Point = point(24.0, BASE, 25.5);
/// The lamp's floor, above the lighthouse's foot.
const LAMP: f32 = 11.0;
const RISE: f32 = 0.216;
const STEPS: usize = 50;
/// The sea cave's floor, and its back wall, where the clefts are.
const FLOOR: f32 = 0.5;
const BACK: f32 = 37.0;
const CLEFT: f32 = 26.8;
const SECOND: f32 = 0.8;
const STACK: Point = point(49.0, 0.0, 14.0);
const SEAT: Point = point(48.0, 2.5, 15.0);

const COAST: [(f32, f32); 26] = [
    (-20.0, 47.0),
    (0.0, 46.0),
    (3.5, 45.2),
    (6.9, 42.9),
    (12.2, 37.3),
    (13.6, 34.6),
    (12.9, 32.5),
    (13.2, 28.0),
    (14.2, 23.4),
    (17.4, 18.7),
    (21.0, 16.8),
    (26.5, 16.0),
    (33.0, 17.5),
    (37.0, 20.6),
    (39.2, 24.8),
    (40.0, 29.5),
    (37.3, 31.6),
    (39.9, 36.2),
    (43.5, 39.5),
    (47.9, 42.8),
    (52.0, 44.6),
    (56.0, 45.6),
    (64.0, 46.0),
    (84.0, 46.5),
    (84.0, 90.0),
    (-20.0, 90.0),
];

const CLIFF: [(f32, f32); 18] = [
    (13.0, 34.0),
    (13.2, 28.0),
    (14.4, 23.2),
    (18.0, 18.0),
    (21.0, 16.8),
    (26.5, 16.0),
    (33.0, 17.5),
    (37.0, 20.6),
    (39.2, 24.8),
    (40.0, 30.0),
    (36.2, 31.5),
    (33.1, 30.1),
    (31.1, 26.1),
    (27.8, 23.1),
    (25.2, 22.0),
    (22.1, 22.4),
    (18.2, 25.8),
    (15.8, 32.5),
];

/// The cut path along the cliff's foot, from the cave's mouth to the beach.
const PATH: [(f32, f32); 6] = [(40.6, 26.8), (41.5, 29.5), (40.7, 31.8), (38.7, 33.7), (36.2, 35.2), (35.0, 36.2)];

/// The cut path's inland side, where the rock slopes down from it into the cove at the cliff's foot.
const INNER: [(f32, f32); 8] = [(40.6, 26.8), (41.5, 29.5), (40.7, 31.8), (38.7, 33.7), (36.2, 35.2), (35.0, 36.2), (33.0, 34.0), (34.0, 28.0)];

/// A tread of the lighthouse's stair: its direction, its middle's height, half its thickness.
struct Tread {
    c: f32,
    s: f32,
    y: f32,
    half: f32,
}

/// What lies at a spot of the map.
struct Land {
    /// Across the ground to the coast: negative inland.
    c: f32,
    /// Across the ground to the cliff's band of rock: negative in it.
    g: f32,
    /// The height the land rises to well inland.
    t: f32,
    /// The ground's height, without its lumps.
    h: f32,
    /// How far the lighthouse, the path and the stack are.
    light: f32,
    path: f32,
    stack: f32,
}

struct Headland {
    seed: u32,
    treads: Vec<Tread>,
    boat: Point,
}

impl Area for Headland {
    fn new(seed: u64) -> Headland {
        let seed = (seed ^ (seed >> 32)) as u32;
        let mut treads = Vec::new();
        for k in 0..STEPS + 3 {
            let angle = PI * 0.5 + (k as f32 + 2.0) * TAU / 16.0;
            let top = if k < STEPS { 0.2 + RISE * (k as f32 + 1.0) } else { LAMP };
            let bottom = if k < 8 { 0.0 } else { top - 0.3 };
            treads.push(Tread { c: cos(angle), s: sin(angle), y: (top + bottom) * 0.5, half: (top - bottom) * 0.5 });
        }
        let mut area = Headland { seed, treads, boat: point(42.6, 0.0, 41.7) };
        area.boat.y = area.land(area.boat.x, area.boat.z).h + 0.2;
        area
    }

    fn describe(&self) -> Description {
        let mut borders = Borders::of_ground(|x, z| self.land(x, z).h);
        borders.north.water = Some(SEA);
        borders.east.water = Some(SEA);
        borders.west.water = Some(SEA);
        Description {
            name: "The smugglers' headland".into(),
            places: vec![
                Place::new("the smugglers' hall behind the cleft", Size::Tiny, point(BACK - 0.5, FLOOR, CLEFT)),
                Place::new("the hidden pool", Size::Tiny, point(BACK - 0.86, FLOOR - 0.03, CLEFT + 0.36)),
                Place::new("the giant's seat on the sea stack", Size::Giant, SEAT),
                Place::new("the lamp gallery", Size::Person, point(LIGHT.x - 2.217, BASE + LAMP, LIGHT.z - 0.918)),
            ],
            borders,
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let l = self.land(p.x, p.z);
        let way = along_path(p.x, p.z);
        let corridor = intersect(way - 0.95, abs(p.y - (FLOOR + 1.1)) - 1.8);
        let rocks = self.cliff(p, &l, view).or(self.boulders(p, &l, view)).cut(corridor);
        let solid = self.ground(p, &l, view).or(rocks);
        let solid = self.cave(solid, p, way, view);
        let solid = solid.or(self.stack(p, view)).or(self.skerry(p, view));
        let solid = solid.or(self.field_wall(p, &l, view)).or(self.plants(p, &l, view));
        let solid = solid.or(self.tower(p, view)).or(self.boat(p, view)).or(self.cargo(p, view));
        let solid = self.tunnels(solid, p, view);
        let water = Answer::new(self.water(p, view), Material::Water).cut(solid.distance);
        solid.or(water)
    }
}

area!(Headland);

impl Headland {
    fn land(&self, x: f32, z: f32) -> Land {
        let f = point(x, 0.0, z);
        let c = polygon(x, z, &COAST) + 0.4 * noise(f / 4.5, self.seed ^ 0xc0);
        let g = polygon(x, z, &CLIFF) + 0.35 * noise(f / 3.2, self.seed ^ 0xc1);
        let light = (f - LIGHT).flat_length();
        let stack = (f - STACK).flat_length();
        let mut t = 0.7 + 0.35 * noise(f / 8.0, self.seed ^ 0xc2) + 7.3 * between(z, 50.0, 28.0);
        t = mix(t, BASE, between(light, 9.0, 4.0));
        let mut h = if c < 0.0 { SEA + (t - SEA) * ease(-c / 7.5) } else { SEA - 1.6 * ease(c / 7.0) };
        h = mix(h, -1.8, between(stack, 13.0, 8.0));
        let path = path_distance(x, z);
        h -= 0.06 * between(path, 1.3, 0.5) * between(light, 4.0, 6.0);
        Land { c, g, t, h, light, path, stack }
    }

    /// The ground: fields and the beach, the sea's bed, and pebbles for the tiny.
    fn ground(&self, p: Point, l: &Land, view: &View) -> Answer {
        let f = point(p.x, 0.0, p.z);
        let dy = p.y - l.h;
        let near = between(abs(dy), 1.0, 0.6);
        let calm = between(l.light, 3.0, 5.0);
        let lumps = if near > 0.0 { calm * detail(f, view, 1.5, 0.03, self.seed ^ 0x9e37) } else { 0.0 };
        let surface = l.h + mix(0.09, lumps, near);
        let n = noise(f / 1.7, self.seed ^ 0x21);
        let sandy = l.c > -(2.3 + 0.9 * n) && l.h < 0.95 + 0.2 * n;
        let material = if l.c > 0.0 && (l.stack < 9.0 + n || l.g < 1.6 + n) {
            Material::Stone
        } else if sandy {
            Material::Sand
        } else if l.path < 0.75 + 0.3 * n {
            Material::Soil
        } else {
            let rise = clamp(-l.c / 7.5, 0.0, 1.0);
            let steep = (l.t - SEA) * 6.0 * rise * (1.0 - rise) / 7.5;
            if steep > 0.75 && n > 0.15 { Material::Soil } else { Material::Grass }
        };
        let ground = Answer::new((p.y - surface) * 0.36, material);
        let seen = view.sees(0.015);
        if seen == 0.0 || abs(p.y - surface) > 0.03 {
            return ground;
        }
        let pebbles = scatter(p, 0.07, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.012) * seen;
            sphere(point(q.x, p.y - surface + r * 0.4, q.z), r)
        });
        ground.or(Answer::new(pebbles * 0.36, Material::Stone))
    }

    /// Rock made rough, near its surface only.
    fn roughened(&self, d: f32, p: Point, view: &View, salt: u32) -> f32 {
        // The roughness is steeper than a distance grows: made shorter for whoever sees it.
        let short = mix(0.95, 0.85, view.sees(0.2));
        if d < -0.5 {
            return d * short;
        }
        let near = between(d, 0.35, 0.15);
        let rough = if near > 0.0 { detail(p, view, 0.8, 0.04, self.seed ^ salt) } else { 0.0 };
        (d + 0.4 * mix(-0.07, rough, near)) * short
    }

    /// The cliff: a wall of bedded rock at the coast, bare on top, sinking under the grass inland.
    fn cliff(&self, p: Point, l: &Land, view: &View) -> Answer {
        if l.c > 6.0 {
            return Answer::new((l.c - 1.4) * 0.5, Material::Stone);
        }
        let f = point(p.x, 0.0, p.z);
        let fall = between(p.z, 25.0, 37.0);
        let top = mix(l.t + 0.45, 2.8, fall) - 1.2 * between(l.g, -0.8, 0.8) - 1.2 * max(l.g - 0.8, 0.0)
            + 0.28 * noise(f / 1.8, self.seed ^ 0x31);
        let above = (p.y - top) * 0.36;
        if above > 2.0 {
            return Answer::new(above - 0.1, Material::Stone);
        }
        let batter = 0.1 * (clamp(p.y, -3.0, 9.0) - 4.0);
        let sway = 2.0 * noise(f / 5.0, self.seed ^ 0x32);
        let beds = 0.1 * sin(p.y * 2.6 + sway) + 0.03 * view.sees(0.5) * sin(p.y * 8.0 + 2.0 * sway);
        let wall = (l.c + batter + 0.45 * noise(p / 2.6, self.seed ^ 0x33) + beds) * 0.45;
        let d = self.roughened(round_max(wall, above, 0.15), p, view, 0x34);
        let patch = noise(p / 1.3, self.seed ^ 0x35);
        let material = if patch > 0.5 && p.y > top - 0.25 && p.y > 2.0 { Material::Moss } else { Material::Stone };
        Answer::new(d, material)
    }

    /// Boulders fallen along the shore.
    fn boulders(&self, p: Point, l: &Land, view: &View) -> Answer {
        let seen = view.sees(1.0);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Stone);
        }
        let dy = p.y - l.h;
        let far = max((dy - 1.0) * 0.33, max((l.c - 5.0) * 0.6, (-3.5 - l.c) * 0.6));
        if far > 0.4 {
            return Answer::new(far, Material::Stone);
        }
        let mask = between(l.c, -3.5, -2.0) * between(l.c, 5.0, 3.0) * seen;
        let lump = 0.06 * noise(p / 0.4, self.seed ^ 0x41);
        let d = scatter(p, 2.2, self.seed ^ 0x42, |q, s| {
            let r = s.next(0.25, 0.7);
            if s.next(0.0, 1.0) > 0.45 {
                return 1.5;
            }
            let r = r * mask;
            sphere(point(q.x, dy + 0.35 * r, q.z), r) + 0.2 * (1.0 - mask)
        });
        Answer::new((min(d, 1.5) + lump) * 0.33, Material::Stone)
    }

    /// The sea cave cut into the cliff, its floor, and the shelf that leads to it from the beach.
    fn cave(&self, solid: Answer, p: Point, way: f32, view: &View) -> Answer {
        let shape = rounded_cuboid(p - point(40.0, 1.6, 26.8), point(3.0, 1.9, 1.9), 0.8);
        let platform = square(p.x - 38.9, p.z - 26.8, 2.7, 2.7);
        let shelf_flat = min(way - 1.2, platform);
        if shape > 4.0 && shelf_flat > 4.0 {
            return solid.or(Answer::new((shelf_flat - 2.6) * 0.7, Material::Stone)).cut((shape - 0.5) * 0.5);
        }
        let f = point(p.x, 0.0, p.z);
        // Near the clefts the wall and the floor are plain, so the tiny's way in is level.
        let mouth = capsule(p, point(BACK, FLOOR, CLEFT), point(BACK, FLOOR, CLEFT + SECOND), 0.001);
        let calm = between(mouth, 0.35, 1.1);
        let near = between(shape, 1.5, 1.0);
        let rough = if near > 0.0 {
            calm * (0.3 * noise(p / 1.1, self.seed ^ 0x51) + detail(p, view, 0.5, 0.03, self.seed ^ 0x52))
                + detail(p, view, 0.04, 0.04, self.seed ^ 0x53)
        } else {
            0.0
        };
        let hollow = (shape + mix(-0.36, rough, near)) * 0.45;
        let mut rock = solid.cut(hollow);
        if hollow < 0.1 {
            rock.material = Material::Stone;
            // Moss about each cleft, and a dark shadow in its middle: what a person sees of it.
            let up = p.y - FLOOR;
            if abs(p.x - BACK) < 0.2 && up > -0.05 {
                let ragged = 0.12 * noise(p / 0.15, self.seed ^ 0x54);
                for v in [0.0, SECOND] {
                    let side = abs(p.z - CLEFT - v);
                    if side < 0.1 - 0.3 * max(up - 0.2, 0.0) && up < 0.4 {
                        rock.material = Material::Mud;
                    } else if (side * side + up * up).sqrt() < 0.45 + ragged && side < 0.4 {
                        rock.material = Material::Moss;
                    }
                }
            }
        }
        // The path dips where it crosses the cove at the cliff's foot.
        let dip = 0.6 * between(point(p.x - 39.9, 0.0, p.z - 32.4).flat_length(), 3.2, 1.2);
        let base = FLOOR - dip;
        let level = between(mouth, 0.5, 1.5);
        let dy = abs(p.y - base);
        let close = between(dy, 0.6, 0.3);
        let lumps = if close > 0.0 {
            level * (0.04 * noise(f / 0.7, self.seed ^ 0x55) + detail(f, view, 0.3, 0.03, self.seed ^ 0x56))
                + detail(f, view, 0.03, 0.03, self.seed ^ 0x57)
        } else {
            0.0
        };
        let top = base + mix(0.06, lumps, close);
        let edge = shelf_flat + 0.15 * noise(f / 1.1, self.seed ^ 0x58);
        let shelf = max(max(edge * 0.65, (p.y - top) * 0.6), -4.0 - p.y);
        let sand = p.x < 39.4 + 0.5 * noise(f / 0.8, self.seed ^ 0x59) && p.z > 24.0 && p.z < 30.0;
        let rock = rock.or(Answer::new(shelf, if sand { Material::Sand } else { Material::Stone }));
        // On its inland side the rock slopes down from it into the cove: the way out of the water.
        let inner = polygon(p.x, p.z, &INNER);
        let slope = (p.y - (top - 0.8 * max(edge, 0.0))) * 0.5;
        let skirt = max(max(inner * 0.65, slope), max((edge - 2.2) * 0.65, -4.0 - p.y));
        rock.or(Answer::new(skirt, Material::Stone))
    }

    /// The smugglers' tunnels, for those small enough to see them.
    fn tunnels(&self, solid: Answer, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        if seen == 0.0 {
            return solid;
        }
        let near = cuboid(p - point(36.45, FLOOR + 0.15, 27.1), point(0.85, 0.4, 0.95));
        if near > 0.05 {
            return Answer::new(max(solid.distance, -near), solid.material);
        }
        // Into the rock, up from the cave's floor, and along its back wall.
        let q = point(BACK - p.x, p.y - FLOOR, p.z - CLEFT);
        let rough = between(q.x, 0.04, 0.25) * detail(p, view, 0.05, 0.06, self.seed ^ 0x61);
        let mut best = (1000.0f32, 0.0f32);
        let mut add = |o: (f32, f32)| {
            if o.0 < best.0 {
                best = o;
            }
        };
        // The hall, the store, the pool's cave.
        add(chamber(q, point(0.52, 0.0, 0.0), point(0.2, 0.11, 0.15), rough));
        add(chamber(q, point(0.72, 0.03, -0.44), point(0.14, 0.1, 0.12), rough));
        add(chamber(q, point(0.86, -0.03, 0.46), point(0.17, 0.12, 0.17), rough));
        // Hall to store, hall to pool, store round the back to pool, pool to the second cleft.
        add(bore(q, point(0.60, 0.0, -0.158), point(0.66, 0.03, -0.312), rough));
        add(bore(q, point(0.62, 0.0, 0.158), point(0.78, -0.03, 0.282), rough));
        add(bore(q, point(0.868, 0.03, -0.44), point(1.1, 0.03, -0.2), rough));
        add(bore(q, point(1.1, 0.03, -0.2), point(1.1, -0.03, 0.2), rough));
        add(bore(q, point(1.1, -0.03, 0.2), point(0.94, -0.03, 0.282), rough));
        add(bore(q, point(0.682, -0.03, 0.46), point(0.42, 0.0, SECOND), rough));
        // The two clefts, and a shaft that lets the day into the hall.
        let wiggle = 0.006 * noise(point(q.x / 0.05, 0.0, 3.5), self.seed ^ 0x62) * between(q.x, -0.02, 0.06);
        add(slot(q, wiggle, 0.12, 0.24));
        add(slot(q, SECOND - wiggle, 0.16, 0.28));
        add((capsule(q, point(0.45, 0.12, 0.03), point(-0.1, 0.32, 0.03), 0.009), 1.0));
        let pit = sphere(q - point(0.86, -0.03 + 0.105, 0.46), 0.12);
        add((pit, 1.0));
        let hollow = best.0 * 0.6;
        let mut carved = solid.cut(hollow);
        if best.0 < 0.03 && q.x > 0.02 {
            carved.material = if best.1 < 0.005 {
                Material::Soil
            } else if pit < 0.01 && q.y < -0.025 {
                Material::Sand
            } else {
                Material::Stone
            };
        }
        let mut out = Answer::new(mix(solid.distance, carved.distance, seen), carved.material);
        // The pool.
        let pool = max(q.y - (-0.031), (q - point(0.86, 0.0, 0.46)).flat_length() - 0.075);
        let pool = max(pool, -0.06 - q.y);
        out = out.or(Answer::new(pool + (1.0 - seen) * 0.1, Material::Water).cut(out.distance));
        // The hoard: crates and casks, and pit props under the roof.
        let inside = cuboid(q - point(0.68, 0.08, -0.2), point(0.26, 0.14, 0.42));
        if inside > 0.03 {
            return out;
        }
        let crate_at = |x: f32, y: f32, z: f32, r: f32| cuboid((q - point(x, y, z)).turned(z * 9.0), point(r, r, r));
        let mut hoard = min(crate_at(0.685, 0.010, 0.055, 0.012), crate_at(0.683, 0.031, 0.052, 0.010));
        hoard = min(hoard, crate_at(0.672, 0.008, 0.088, 0.010));
        hoard = min(hoard, min(cask(q - point(0.685, 0.011, -0.03), 0.009, 0.013), cask(q - point(0.665, 0.011, -0.055), 0.009, 0.013)));
        hoard = min(hoard, cask(q - point(0.69, 0.011, -0.065), 0.009, 0.013));
        hoard = min(hoard, min(cask(q - point(0.68, 0.041, -0.525), 0.010, 0.013), cask(q - point(0.715, 0.041, -0.53), 0.010, 0.013)));
        hoard = min(hoard, crate_at(0.77, 0.043, -0.52, 0.015));
        let post = |z: f32| cuboid(q - point(0.5, 0.07, z), point(0.006, 0.09, 0.006));
        let beam = cuboid(q - point(0.5, 0.148, 0.0), point(0.007, 0.007, 0.16));
        hoard = min(hoard, min(beam, min(post(-0.132), post(0.132))));
        let grain = 0.0006 * view.sees(0.004) * noise(point(q.x / 0.004, q.y / 0.02, q.z / 0.004), self.seed ^ 0x63);
        out = out.or(Answer::new((hoard + grain) * 0.6 + (1.0 - seen) * 0.1, Material::Wood));
        let lantern = sphere(q - point(0.683, 0.0445, 0.052), 0.005);
        out.or(Answer::new(lantern + (1.0 - seen) * 0.1, Material::Sand))
    }

    /// The sea stack: undercut by the waves all round, flat on top, a crag behind like a chair's back.
    fn stack(&self, p: Point, view: &View) -> Answer {
        let q = p - STACK;
        let bound = cylinder(q - point(0.0, 1.5, 0.0), 6.3, 6.2);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let f = point(p.x, 0.0, p.z);
        let y = p.y;
        let wall = (q.flat_length() - 4.5
            + 0.35 * noise(f / 2.5, self.seed ^ 0x71)
            + 0.55 * between(y, 1.3, -0.2)
            - 1.1 * between(y, -0.7, -2.0)
            + 0.1 * noise(p / 1.3, self.seed ^ 0x72)
            + 0.04 * sin(y * 4.0))
            * 0.45;
        let d = p - SEAT;
        let from_seat = d.flat_length();
        let top = SEAT.y + 0.1 * noise(f / 1.6, self.seed ^ 0x73) * between(from_seat, 2.2, 3.2);
        let body = round_max(wall, (y - top) * 0.8, 0.15);
        let along = (d.x - d.z) * 0.707_106_8;
        let crest = 2.2 + 4.2 * between(along, 1.0, 4.6) + 0.35 * noise(f / 1.2, self.seed ^ 0x74);
        let inner = (3.3 + 0.15 * (clamp(y, 2.5, 7.0) - 2.5) - from_seat + 0.15 * noise(p / 0.9, self.seed ^ 0x75)) * 0.6;
        let crag = round_max(max(wall, inner), (y - crest) * 0.34, 0.12);
        let rock = min(body, crag);
        // The seat itself is worn smooth; all else is rough.
        let worn = between(from_seat, 0.8, 2.6) * 0.8 + 0.2;
        let near = between(rock, 0.35, 0.15);
        let rough = if near > 0.0 && rock > -0.5 { detail(p, view, 0.8, 0.04, self.seed ^ 0x76) * worn } else { 0.0 };
        let out = (rock + 0.4 * mix(-0.07, rough, near)) * 0.85;
        let patch = noise(f / 1.1, self.seed ^ 0x77);
        let material = if patch > 0.5 && y > 2.2 && from_seat > 1.5 { Material::Moss } else { Material::Stone };
        Answer::new(out, material)
    }

    /// The skerry to the west.
    fn skerry(&self, p: Point, view: &View) -> Answer {
        let q = p - point(9.3, 0.0, 13.0);
        let bound = cylinder(q, 4.6, 4.4);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let f = point(p.x, 0.0, p.z);
        let wall = (q.flat_length() - 2.6
            + 0.4 * noise(f / 1.6, self.seed ^ 0x81)
            + 0.2 * noise(p / 1.1, self.seed ^ 0x82)
            + 0.07 * sin(p.y * 3.1)
            + 0.22 * clamp(p.y, -3.0, 3.0))
            * 0.42;
        let top = 2.1 + 0.6 * noise(f / 1.4, self.seed ^ 0x83);
        let d = round_max(wall, (p.y - top) * 0.45, 0.15);
        let patch = noise(p / 0.8, self.seed ^ 0x85);
        let material = if patch > 0.4 && p.y > 1.5 { Material::Moss } else { Material::Stone };
        Answer::new(self.roughened(d, p, view, 0x84), material)
    }

    /// The dry-stone wall across the headland's neck, open where the path goes through.
    fn field_wall(&self, p: Point, l: &Land, view: &View) -> Answer {
        let seen = view.sees(0.7);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Stone);
        }
        let flat = min(
            min(segment(p.x, p.z, (15.5, 41.0), (20.5, 42.6)), segment(p.x, p.z, (20.5, 42.6), (26.0, 43.0))),
            min(segment(p.x, p.z, (30.4, 43.0), (36.0, 43.4)), segment(p.x, p.z, (36.0, 43.4), (40.0, 42.6))),
        );
        let dy = p.y - l.h;
        let coarse = max(flat - 0.32, abs(dy - 0.15) - 0.8 * seen) + 0.05 * (1.0 - seen);
        if coarse > 0.8 {
            return Answer::new((coarse - 0.2) * 0.42, Material::Stone);
        }
        let stones = 0.06 * noise(point(p.x / 0.3, p.y / 0.17, p.z / 0.3), self.seed ^ 0x91)
            + detail(p, view, 0.15, 0.04, self.seed ^ 0x92) * between(coarse, 0.5, 0.2);
        let patch = noise(p / 0.6, self.seed ^ 0x93);
        let material = if patch > 0.4 && dy > 0.6 { Material::Moss } else { Material::Stone };
        Answer::new((coarse + stones) * 0.35, material)
    }

    /// Gorse and tussocks on the fields.
    fn plants(&self, p: Point, l: &Land, view: &View) -> Answer {
        let seen = view.sees(0.8);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Leaf);
        }
        let dy = p.y - l.h;
        let far = max((dy - 1.0) * 0.33, (l.c + 4.5) * 0.6);
        if far > 0.3 {
            return Answer::new(far, Material::Leaf);
        }
        let mask = seen * between(l.c, -4.5, -6.5) * between(l.light, 4.5, 6.5) * between(l.g, 1.0, 2.5);
        let leaves = 0.05 * noise(p / 0.25, self.seed ^ 0xa1);
        let gorse = scatter(p, 4.6, self.seed ^ 0xa2, |q, s| {
            let r = s.next(0.45, 0.85);
            if s.next(0.0, 1.0) > 0.42 {
                return 3.0;
            }
            let (x, z) = (p.x - q.x, p.z - q.z);
            let start = point(x - 32.0, 0.0, z - 60.0).flat_length();
            let m = mask * between(path_distance(x, z), 1.8, 2.6) * between(start, 5.0, 6.0);
            // A clump of three, the middle one highest.
            let a = s.next(0.0, TAU);
            let o = point(cos(a), 0.0, sin(a)) * (0.6 * r * m);
            let u = point(q.x, dy + 0.15 * r * m, q.z);
            let clump = min(sphere(u, r * m), min(sphere(u - o, 0.75 * r * m), sphere(u + o, 0.65 * r * m)));
            clump + 0.3 * (1.0 - m)
        });
        let gorse = min(gorse, 2.5);
        let fine = if gorse < 0.2 { detail(p, view, 0.06, 0.06, self.seed ^ 0xa3) * between(gorse, 0.2, 0.1) } else { 0.0 };
        let gorse = Answer::new((gorse + leaves + fine) * 0.3, Material::Leaf);
        let tufts = scatter(p, 1.5, self.seed ^ 0xa4, |q, s| {
            let r = s.next(0.16, 0.34);
            if s.next(0.0, 1.0) > 0.6 {
                return 1.0;
            }
            let m = mask * between(path_distance(p.x - q.x, p.z - q.z), 0.9, 1.4);
            sphere(point(q.x, dy + 0.55 * r * m, q.z), r * m) + 0.2 * (1.0 - m)
        });
        let blades = if dy < 0.4 { detail(p, view, 0.08, 0.06, self.seed ^ 0xa5) } else { -0.01 };
        gorse.or(Answer::new((min(tufts, 1.0) + blades) * 0.3, Material::Grass))
    }

    /// The lighthouse: a tapered tower with a winding stair inside, a gallery and a lamp on top.
    fn tower(&self, p: Point, view: &View) -> Answer {
        let q = p - LIGHT;
        let bound = cylinder(q - point(0.0, 7.2, 0.0), 3.4, 10.6);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let seen = view.sees(0.8);
        let r = q.flat_length();
        let y = q.y;
        let plinth = cylinder(q - point(0.0, -1.4, 0.0), 2.9, 1.6);
        let body = max(r - (2.35 - 0.035 * clamp(y, 0.0, 11.0)), abs(y - 5.2) - 5.7) * 0.98;
        let gallery = min(cylinder(q - point(0.0, 10.7, 0.0), 3.1, 0.3), cylinder(q - point(0.0, 10.1, 0.0), 2.6, 0.36));
        let drum = cylinder(q - point(0.0, 12.4, 0.0), 1.95, 1.8);
        let eave = cylinder(q - point(0.0, 14.1, 0.0), 2.35, 0.25);
        let dome = max(sphere(q - point(0.0, 14.0, 0.0), 1.9), 14.0 - y);
        let ball = min(sphere(q - point(0.0, 16.2, 0.0), 0.42), capsule(q, point(0.0, 15.6, 0.0), point(0.0, 17.0, 0.0), 0.14));
        let roof = min(min(eave, dome), ball);
        let frame = cuboid(q - point(0.0, 1.5, 2.2), point(0.85, 1.4, 0.3));

        let band = ((y - 1.6) / 2.3).floor() as i32;
        let striped = if y < 1.6 {
            Material::Stone
        } else if y > 9.4 {
            Material::Sand
        } else if band & 1 == 1 {
            Material::Wood
        } else {
            Material::Sand
        };
        let mut shell = Answer::new(body, striped);
        shell = shell.or(Answer::new(min(min(plinth, gallery), frame), Material::Stone));
        shell = shell.or(Answer::new(drum, Material::Sand)).or(Answer::new(roof, Material::Wood));
        if seen == 0.0 {
            return shell;
        }

        // Courses of dressed stone on the outside.
        let depth = 0.025 * view.sees(0.2);
        if depth > 0.0 && abs(body) < 0.2 && r > 1.75 && y < 10.0 {
            let fade = between(y, 10.0, 9.7) * between(r, 1.75, 1.85);
            let course = (y / 0.5).floor();
            let bed = abs(y - (y / 0.5 + 0.5).floor() * 0.5) - 0.015;
            let shift = if (course as i32) & 1 == 1 { 0.5 } else { 0.0 };
            let t = atan2(q.z, q.x) / TAU * 18.0 + shift;
            let joint = abs(t - (t + 0.5).floor()) * (TAU * r / 18.0) - 0.015;
            let mortar = min(bed, joint);
            shell.distance = max(shell.distance, min(-mortar, shell.distance + depth * fade) * 0.95);
        }

        // What is hollow: the well, the door, the windows, the lamp's panes and its door.
        let well = cylinder(q - point(0.0, 7.0, 0.0), 1.55, 6.8);
        let door = max(max(abs(q.x) - 0.55, abs(y - 1.35) - 1.15), abs(q.z - 2.0) - 0.9);
        let window = |out: f32, side: f32, at: f32| max(max(abs(side) - 0.26, abs(y - at) - 0.48), abs(out - 2.0) - 0.7);
        let windows = min(window(-q.z, q.x, 4.2), min(window(q.x, q.z, 6.4), window(q.z, q.x, 8.6)));
        let angle = atan2(q.z, q.x);
        let eighth = TAU / 8.0;
        let af = angle - (angle / eighth + 0.5).floor() * eighth;
        let panes = max(max(abs(y - 12.8) - 0.9, max(1.3 - r, r - 2.2)), 0.18 - r * abs(sin(af)));
        let (dx, dz) = (-0.923_88, -0.382_683);
        let (out, side) = (q.x * dx + q.z * dz, q.z * dx - q.x * dz);
        let lamp_door = max(max(abs(side) - 0.48, abs(y - 12.25) - 1.25), abs(out - 1.75) - 0.65);
        let hollow = min(min(well, door), min(min(windows, panes), lamp_door)) * 0.95 + (1.0 - seen) * 2.5;
        let mut all = shell.cut(hollow);

        // The stair about its newel.
        let gone = (1.0 - seen) * 0.5;
        let newel = cylinder(q - point(0.0, 5.85, 0.0), 0.24, 6.15);
        all = all.or(Answer::new(newel + gone, Material::Stone));
        if r < 2.4 && y < 11.6 {
            let mut best = 0.6;
            for t in &self.treads {
                let high = abs(y - t.y) - t.half;
                if high * 0.95 >= best {
                    continue;
                }
                let (u, w) = (q.x * t.c + q.z * t.s, q.z * t.c - q.x * t.s);
                let wedge = max(abs(w) - (0.21 * max(u, 0.0) + 0.05), max(u - 1.7, 0.05 - u));
                best = min(best, max(wedge, high) * 0.95);
            }
            all = all.or(Answer::new(best + gone, Material::Wood));
        } else {
            // Never nearer than this to a tread from here.
            all = all.or(Answer::new(max(r - 1.75, y - 11.05) + gone, Material::Wood));
        }
        // The lamp: a lens of ringed glass.
        let lens = (cylinder(q - point(0.0, 12.45, 0.0), 0.5, 0.7) - 0.03 * sin(y * 18.0)) * 0.8;
        all = all.or(Answer::new(lens + gone, Material::Sand));
        // The gallery's railing.
        let sixteenth = TAU / 16.0;
        let ap = angle - (angle / sixteenth + 0.5).floor() * sixteenth;
        let (px, pz) = (r * cos(ap) - 2.95, r * sin(ap));
        let posts = max((px * px + pz * pz).sqrt() - 0.09, abs(y - 11.4) - 0.6);
        let rails = min(torus(q - point(0.0, 12.0, 0.0), 2.95, 0.1), torus(q - point(0.0, 11.5, 0.0), 2.95, 0.09));
        all = all.or(Answer::new(min(posts, rails) + gone, Material::Wood));
        // Worn and weathered, for the small.
        if abs(all.distance) < 0.05 {
            let fade = between(abs(all.distance), 0.05, 0.02);
            let worn = detail(p, view, 0.06, 0.03, self.seed ^ 0xb1) * fade;
            all = Answer::new((all.distance + worn) * 0.8, all.material);
        } else {
            all = Answer::new(all.distance * 0.8, all.material);
        }
        all
    }

    /// A rowing boat drawn up on the beach.
    fn boat(&self, p: Point, view: &View) -> Answer {
        let seen = view.sees(1.0);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Wood);
        }
        let q = (p - self.boat).turned(0.8);
        let bound = sphere(q, 1.9);
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let y = q.y;
        let arc = |w: f32| (q.x * q.x + w * w).sqrt() - 2.175;
        let plan = max(arc(q.z + 1.575), arc(q.z - 1.575));
        let side = plan + 0.45 * (0.3 - clamp(y, -0.35, 0.35));
        let outer = max(side, abs(y - 0.02) - 0.32);
        let inner = max(side + 0.2, -0.06 - y);
        let hull = cut(outer, inner);
        let thwart = |x: f32| max(cuboid(q - point(x, 0.13, 0.0), point(0.14, 0.09, 0.7)), side);
        let seats = min(thwart(-0.45), thwart(0.5));
        let planks = 0.012 * view.sees(0.1) * sin(y * 40.0 + 2.0 * q.x * q.x);
        Answer::new((min(hull, seats) + planks) * 0.65 + (1.0 - seen) * 0.4, Material::Wood)
    }

    /// Casks and a crate left in the sea cave.
    fn cargo(&self, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.7);
        if seen == 0.0 {
            return Answer::new(1000.0, Material::Wood);
        }
        let bound = cuboid(p - point(38.6, 1.0, 26.8), point(1.4, 0.9, 2.0));
        if bound > 0.5 {
            return Answer::new(bound, Material::Wood);
        }
        let casks = min(
            cask(p - point(38.5, FLOOR + 0.4, 28.15), 0.34, 0.46),
            min(cask(p - point(39.3, FLOOR + 0.4, 28.2), 0.32, 0.46), cask(p - point(38.95, FLOOR + 0.35, 27.65), 0.26, 0.4)),
        );
        let chest = rounded_cuboid((p - point(38.6, FLOOR + 0.32, 25.45)).turned(0.4), point(0.5, 0.38, 0.35), 0.04);
        let d = min(casks, chest);
        let near = between(d, 0.1, 0.05);
        let grain = if near > 0.0 { detail(point(p.x, p.y * 0.15, p.z), view, 0.08, 0.05, self.seed ^ 0xd1) } else { 0.0 };
        Answer::new((d + mix(-0.01, grain, near)) * 0.75 + (1.0 - seen) * 0.8, Material::Wood)
    }

    /// The sea, with a low swell that stills toward the area's edges.
    fn water(&self, p: Point, view: &View) -> f32 {
        let inside = min(min(p.x, WIDTH - p.x), min(p.z, WIDTH - p.z));
        let open = between(inside, 0.0, 4.0);
        let swell = 0.025 * sin(p.x * 0.8 + p.z * 0.35) + 0.02 * sin(p.z * 1.1 - p.x * 0.5);
        let d = p.y - SEA - open * swell;
        let near = between(abs(d), 0.15, 0.08);
        let ripples = if near > 0.0 { detail(point(p.x, 0.0, p.z), view, 0.4, 0.015, self.seed ^ 0xe1) } else { 0.0 };
        (d - mix(0.012, ripples * open, near)) * 0.85
    }
}

/// Across the ground to the path from the start to the lighthouse's door.
fn path_distance(x: f32, z: f32) -> f32 {
    let middle = mix(24.0, 32.0, between(z, 30.0, 58.0)) + 0.8 * sin(z * 0.35) * between(z, 27.0, 34.0);
    max(abs(x - middle) * 0.85, 28.2 - z)
}

/// Across the ground to the middle of the cut path at the cliff's foot.
fn along_path(x: f32, z: f32) -> f32 {
    let mut d = 1000.0;
    for k in 0..PATH.len() - 1 {
        d = min(d, segment(x, z, PATH[k], PATH[k + 1]));
    }
    d
}

/// A room of the tunnels on a level floor `at`: how far, and how high above its floor.
fn chamber(q: Point, at: Point, half: Point, rough: f32) -> (f32, f32) {
    let walls = rounded_cuboid(q - point(at.x, at.y + half.y - 0.06, at.z), half, 0.05);
    (max(walls + rough, at.y - q.y + 0.3 * rough), q.y - at.y)
}

/// A tunnel whose floor goes from `a` to `b`: how far, and how high above its floor.
fn bore(q: Point, a: Point, b: Point, rough: f32) -> (f32, f32) {
    let lift = point(0.0, 0.013, 0.0);
    let (pa, ba) = (q - lift - a, b - a);
    let t = clamp(pa.dot(ba) / ba.dot(ba), 0.0, 1.0);
    let floor = a.y + ba.y * t;
    let tube = (pa - ba * t).length() - 0.022;
    (max(tube + rough * 0.7, (floor - q.y) * 0.95 + 0.3 * rough), q.y - floor)
}

/// A cleft 2.2 cm wide in the cave's back wall, narrowing upwards.
fn slot(q: Point, across: f32, middle: f32, half: f32) -> (f32, f32) {
    let up = clamp(q.y / 0.06, 0.0, 1.0);
    let sides = (abs(q.z - across) - 0.011 * (1.0 - 0.5 * up)) / 1.3;
    (max(max(sides, abs(q.y - 0.03) - 0.03), abs(q.x - middle) - half), q.y)
}

/// A cask standing on its end.
fn cask(q: Point, r: f32, half: f32) -> f32 {
    let t = clamp(q.y / half, -1.0, 1.0);
    max(q.flat_length() - r * (1.0 - 0.18 * t * t), abs(q.y) - half) * 0.9
}

/// Where both are, the edge between them rounded by `r`. It falls off across the edge as fast as
/// either shape does, so a ray that stops by the edge stops close by the rock.
fn round_max(a: f32, b: f32, r: f32) -> f32 {
    let (u, v) = (max(a + r, 0.0), max(b + r, 0.0));
    min(max(a, b), -r) + (u * u + v * v).sqrt()
}

/// Across the ground to a line from `a` to `b`.
fn segment(x: f32, z: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (ex, ez) = (b.0 - a.0, b.1 - a.1);
    let (wx, wz) = (x - a.0, z - a.1);
    let t = clamp((wx * ex + wz * ez) / (ex * ex + ez * ez), 0.0, 1.0);
    let (dx, dz) = (wx - ex * t, wz - ez * t);
    (dx * dx + dz * dz).sqrt()
}

/// Across the ground to a polygon's edge, negative inside it.
fn polygon(x: f32, z: f32, pts: &[(f32, f32)]) -> f32 {
    let mut best = 1.0e9;
    let mut inside = false;
    let mut j = pts.len() - 1;
    for i in 0..pts.len() {
        let (xi, zi) = pts[i];
        let (xj, zj) = pts[j];
        let (ex, ez) = (xj - xi, zj - zi);
        let (wx, wz) = (x - xi, z - zi);
        let t = clamp((wx * ex + wz * ez) / (ex * ex + ez * ez), 0.0, 1.0);
        let (dx, dz) = (wx - ex * t, wz - ez * t);
        best = min(best, dx * dx + dz * dz);
        if (zi > z) != (zj > z) && x < ex * wz / ez + xi {
            inside = !inside;
        }
        j = i;
    }
    if inside { -best.sqrt() } else { best.sqrt() }
}

/// Across the ground to a rectangle about the origin.
fn square(x: f32, z: f32, hx: f32, hz: f32) -> f32 {
    let (qx, qz) = (abs(x) - hx, abs(z) - hz);
    (max(qx, 0.0) * max(qx, 0.0) + max(qz, 0.0) * max(qz, 0.0)).sqrt() + min(max(qx, qz), 0.0)
}
