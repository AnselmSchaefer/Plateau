The castle reads clearly as a ruin from every size, but the pictures showed four flaws, and this version fixes them. These changes haven't been through the engine yet, so they still need to be checked against the laws.

**What looked wrong:**
- **Curtain walls:** they were striped white and green from above. The grass only covered the very top of each wall, and the rounded edges dropped below that line, so the stone showed through in bands.
- **Fallen stone:** the piece of wall in the bailey was a level box with a flat green top, and looked like a test shape.
- **Keep:** it was a plain box, and its broken corner was too shallow to see.
- **Pebbles:** to the tiny they were all identical round domes.

**What I changed:**
- **Walls:** the grass now drapes over the tops in turf of uneven depth, down to about a metre over the edges, so they read as grassed-over hills.
- **Keep:** it now has a wider plinth at its base, buttresses at its corners (ending below the broken top), and a deeper break at the north-west corner (about 2 m lower). I made the door deeper so it still cuts cleanly through the plinth. The top stays well above anything a giant can jump to, so the hall is still closed to giants.
- **Fallen stone:** it is now tilted, half sunk into the ground and chipped by noise, with uneven moss on it. A smaller broken piece lies beside it.
- **Pebbles:** they are now flattened, so to the tiny they look like low boulders instead of domes.

```rust
//! The ruined castle: a curtain wall forty metres square, broken and grassed over, a dry ditch
//! (wet at its bottom) about it, a causeway to the south and the gap where the gatehouse was. In
//! the bailey stand the roofless keep, its walls nine to twelve metres high, and the earth-filled
//! stump of the east tower. Outside the ditch the ground rolls up into low hills.
//!
//! Each size has its own castle:
//! - **The tiny** find the walls are hills: two metres of grassed stone, their mortar ravines,
//!   their fallen stones mountains. At the foot of the south wall's outer face, by a patch of moss,
//!   a crack a little over a centimetre wide leads to a hollow inside the wall.
//! - **A person** crosses the causeway, walks the worn path through the gate and into the keep by
//!   its narrow door: an empty hall, a fallen beam on a heap of rubble, a cold hearth, arrow slits
//!   high above, the ledge where the upper floor was.
//! - **A giant** steps over the broken walls and up onto the tower stump, 3.6 m high, and from
//!   there looks down into the keep over its jagged top.

use plateau_sdk::*;

struct Castle {
    seed: u32,
    /// The ground's height at the crack: the hollow's floor.
    floor: f32,
    /// The ground's height in the middle of the keep.
    keep_floor: f32,
    /// The ground's height where a person stands in the hall.
    hall: f32,
    /// The top of the tower stump.
    stump_top: f32,
}

/// The curtain wall runs round a square about the middle, its middle line `RING` from it.
const MIDDLE: Point = point(32.0, 0.0, 30.0);
const RING: f32 = 20.0;
const THICK: f32 = 1.0;
const BOTTOM: f32 = -1.2;
const WALL_MAX: f32 = 3.2;
const GATE_X: f32 = 32.0;

/// The crack, in the south wall's outer face (z = 51), and the hollow behind it.
const CRACK: Point = point(20.0, 0.0, 51.0);
const CAVE: Point = point(20.0, 0.0, 50.4);

const KEEP: Point = point(24.0, 0.0, 22.0);
const KEEP_HALF: f32 = 6.0;
const KEEP_INNER: f32 = 4.5;

const STUMP: Point = point(40.0, 0.0, 24.0);
const STUMP_R: f32 = 5.5;
const STUMP_H: f32 = 3.6;
/// The broken stubs of the stump's parapet.
const STUBS: usize = 7;

/// A piece of the wall fallen into the bailey.
const FALLEN: Point = point(34.0, 0.0, 39.0);

/// The water in the ditch's bottom.
const LEVEL: f32 = -0.7;

/// Where the curtain wall has fallen down to the ground.
const BREACHES: [Point; 3] = [point(12.0, 0.0, 34.0), point(44.0, 0.0, 10.0), point(52.0, 0.0, 40.0)];

impl Area for Castle {
    fn new(seed: u64) -> Castle {
        let seed = (seed ^ (seed >> 32)) as u32;
        let mut c = Castle { seed, floor: 0.0, keep_floor: 0.0, hall: 0.0, stump_top: 0.0 };
        c.floor = c.height(CRACK.x, 50.95);
        c.keep_floor = c.height(KEEP.x, KEEP.z);
        c.hall = c.height(KEEP.x, KEEP.z + 2.0);
        c.stump_top = c.height(STUMP.x, STUMP.z) + STUMP_H;
        c
    }

    fn describe(&self) -> Description {
        Description {
            name: "The ruined castle".into(),
            places: vec![
                Place::new("the hollow inside the south wall", Size::Tiny, point(CAVE.x, self.floor, CAVE.z)),
                Place::new("the keep's empty hall", Size::Person, point(KEEP.x, self.hall, KEEP.z + 2.0)),
                Place::new("the stump of the east tower", Size::Giant, point(STUMP.x, self.stump_top, STUMP.z)),
            ],
            borders: Borders::of_ground(|x, z| self.height(x, z)),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let solid = self.ground(p, view).or(self.wall(p, view));
        let solid = self.cave(solid, p, view);
        let solid = solid
            .or(self.keep(p, view))
            .or(self.inside(p, view))
            .or(self.stump(p, view))
            .or(self.rubble(p, view));
        let water = Answer::new(p.y - LEVEL, Material::Water).cut(solid.distance);
        solid.or(water)
    }
}

area!(Castle);

impl Castle {
    /// The ground's height, without its roughness: a nearly flat bailey, the ditch about the wall
    /// (but not across the causeway), hills beyond. Calm and level at the crack.
    fn height(&self, x: f32, z: f32) -> f32 {
        let (qx, qz) = (x - MIDDLE.x, z - MIDDLE.z);
        let o = square(qx, qz, RING + THICK, RING + THICK);
        let calm = between(point(x - CRACK.x, 0.0, z - CRACK.z).flat_length(), 0.5, 2.5);
        let bailey = 0.15 * calm * noise(point(x / 10.0, 0.0, z / 10.0), self.seed);
        let ditch = 1.1 * ease(1.0 - abs(o - 4.0) / 3.0);
        let causeway = 1.0 - between(z, 44.0, 48.0) * (1.0 - between(abs(x - GATE_X), 4.0, 7.0));
        let hills = 1.4 * between(o, 7.0, 15.0) * (0.6 + noise(point(x / 11.0, 0.0, z / 11.0), self.seed ^ 0x51));
        bailey + hills - ditch * causeway
    }

    /// The ground: grass, a worn path to the keep, mud in the ditch, flags in the keep; pebbles
    /// for the tiny.
    fn ground(&self, p: Point, view: &View) -> Answer {
        let (qx, qz) = (p.x - MIDDLE.x, p.z - MIDDLE.z);
        let ring = abs(square(qx, qz, RING, RING));
        let along_wall = between(ring, THICK + 0.2, THICK + 1.0);
        let h = self.height(p.x, p.z);
        let surface = h + along_wall * detail(point(p.x, 0.0, p.z), view, 1.5, 0.05, self.seed ^ 0x9e37);
        let flat = point(p.x, 0.0, p.z);
        let track = min(
            capsule(flat, point(GATE_X, 0.0, 63.0), point(GATE_X, 0.0, 46.0), 0.0),
            capsule(flat, point(GATE_X, 0.0, 46.0), point(KEEP.x, 0.0, KEEP.z + KEEP_HALF + 0.5), 0.0),
        );
        let path = track - (0.8 + 0.3 * noise(p / 1.3, self.seed ^ 0x3c));
        let in_keep = square(p.x - KEEP.x, p.z - KEEP.z, KEEP_INNER, KEEP_INNER);
        let material = if in_keep < 0.0 {
            Material::Stone
        } else if h < -0.5 {
            Material::Mud
        } else if path < 0.0 {
            Material::Soil
        } else {
            Material::Grass
        };
        let ground = Answer::new((p.y - surface) * 0.65, material);
        let seen = view.sees(0.015);
        if seen == 0.0 || abs(p.y - surface) > 0.03 {
            return ground;
        }
        // Pebbles, flattened as worn stones lie, half sunk.
        let pebbles = scatter(p, 0.06, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.012) * seen;
            let flat = s.next(1.2, 1.9);
            sphere(point(q.x, (p.y - surface + r * 0.3) * flat, q.z), r) / flat
        });
        ground.or(Answer::new(pebbles * 0.65, Material::Stone))
    }

    /// The curtain wall's broken top: worn up and down, down to the ground at the breaches, high
    /// enough over the hollow.
    fn wall_top(&self, x: f32, z: f32) -> f32 {
        let mut t = 2.0 + 1.0 * noise(point(x / 6.0, 0.0, z / 6.0), self.seed ^ 0x21) + 0.35 * noise(point(x / 2.3, 0.0, z / 2.3), self.seed ^ 0x22);
        for b in BREACHES.iter() {
            let d = point(x - b.x, 0.0, z - b.z).flat_length();
            t -= 3.4 * ease(1.0 - d / 6.0);
        }
        let over_cave = 2.4 - 0.3 * point(x - CRACK.x, 0.0, z - 50.0).flat_length();
        clamp(max(t, over_cave), -2.0, WALL_MAX)
    }

    /// The curtain wall: stone in courses, its shoulders rounded and grassed over, the gate gone.
    fn wall(&self, p: Point, view: &View) -> Answer {
        let (qx, qz) = (p.x - MIDDLE.x, p.z - MIDDLE.z);
        let band = abs(square(qx, qz, RING, RING)) - THICK;
        let reach = max(band, max(p.y - (WALL_MAX + 0.1), BOTTOM - p.y));
        if reach > 0.6 {
            return Answer::new(reach - 0.05, Material::Stone);
        }
        let top = self.wall_top(p.x, p.z);
        let vert = (p.y - top) * 0.45;
        let body = max(-smooth_union(-band, -vert, 0.8), BOTTOM - p.y);
        let gate = cuboid(p - point(GATE_X, 1.5, MIDDLE.z + RING), point(4.0, 3.2, 1.6));
        let coarse = cut(body, gate);
        if coarse > 0.1 {
            return Answer::new(coarse - 0.05, Material::Stone);
        }
        let mut d = (coarse + detail(p, view, 0.3, 0.025, self.seed ^ 0x7f4a)) * 0.85;
        let along = if abs(qz) > abs(qx) { p.x } else { p.z };
        d = masonry(d, p.y, along, 0.35, 0.7, 0.03 * view.sees(0.035), (top - 0.3 - p.y) * 0.45);
        // Turf over the broken top, hanging down its rounded shoulders unevenly.
        let turf = top - 0.9 + 0.4 * noise(p / 1.2, self.seed ^ 0x23);
        let material = if p.y > turf { Material::Grass } else { Material::Stone };
        let mut wall = Answer::new(d, material);
        // Moss at the wall's foot in patches, and always about the crack's mouth.
        let foot = self.height(p.x, p.z);
        let patch = noise(p / 0.7, self.seed ^ 0x6d) + 1.5 * between(-(p - point(CRACK.x, self.floor, CRACK.z)).length(), -0.4, -0.15);
        let moss = between(patch, 0.25, 0.45) * between(foot + 0.35 - p.y, 0.0, 0.1);
        if moss > 0.0 {
            wall.material = Material::Moss;
            let seen = view.sees(0.006) * moss;
            let tufts = seen * 0.004 * (0.6 + 0.4 * noise(p / 0.005, self.seed ^ 0x2b));
            wall = Answer::new((wall.distance - tufts) * (1.0 - 0.5 * seen), wall.material);
        }
        wall
    }

    /// The crack in the south wall's outer face and the hollow it leads to, for the tiny alone.
    fn cave(&self, solid: Answer, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        if seen == 0.0 {
            return solid;
        }
        let f = self.floor;
        let near = cuboid(p - point(CRACK.x, f + 0.12, 50.7), point(0.45, 0.3, 0.55));
        if near > 0.05 {
            return solid;
        }
        let rough = detail(p, view, 0.05, 0.08, self.seed ^ 0x4c);
        let room = (rounded_cuboid(p - point(CAVE.x, f + 0.12, CAVE.z), point(0.3, 0.12, 0.17), 0.09) + rough) * 0.6;
        let wiggle = 0.012 * noise(point(0.0, p.y / 0.04, p.z / 0.04), self.seed ^ 0x19);
        let up = clamp((p.y - f) / 0.2, 0.0, 1.0);
        let slot = (abs(p.x - CRACK.x - wiggle) - 0.007 * (1.0 - 0.6 * up)) / 1.5;
        let crack = intersect(intersect(slot, abs(p.y - (f + 0.1)) - 0.1), abs(p.z - 50.8) - 0.32);
        let hollow = union(room, crack);
        let mut cut_out = solid.cut(hollow);
        if hollow < 0.02 && p.z < 50.98 {
            cut_out.material = if p.y > f + 0.01 { Material::Stone } else { Material::Soil };
        }
        Answer::new(mix(solid.distance, cut_out.distance, seen), cut_out.material)
    }

    /// The keep's jagged top, broken down lowest at its north-west corner.
    fn keep_top(&self, x: f32, z: f32) -> f32 {
        let worn = ease(1.0 - point(x - (KEEP.x - KEEP_HALF), 0.0, z - (KEEP.z - KEEP_HALF)).flat_length() / 5.0);
        self.keep_floor + 11.0 + 0.6 * (1.0 + noise(point(x / 4.0, 0.0, z / 4.0), self.seed ^ 0x31)) - 2.0 * worn
    }

    /// The keep: a roofless square tower on a plinth, buttressed at its corners, a narrow arched
    /// door to the south, a window to the east, arrow slits, a hearth, the ledge of its lost upper
    /// floor; ivy up its west side.
    fn keep(&self, p: Point, view: &View) -> Answer {
        let g = self.keep_floor;
        let q = p - KEEP;
        let sq = square(q.x, q.z, KEEP_HALF, KEEP_HALF);
        let reach = max(square(q.x, q.z, KEEP_HALF + 0.85, KEEP_HALF + 0.85), max(p.y - (g + 12.4), g - 1.5 - p.y));
        if reach > 0.6 {
            return Answer::new(reach - 0.05, Material::Stone);
        }
        let top = self.keep_top(p.x, p.z);
        // The broken top rounded into the faces, so no sharp rim stands where the top's distance
        // (made shorter for its slopes) meets the face's.
        let body = max(-smooth_union(-sq, -(p.y - top) * 0.65, 0.3), g - 1.5 - p.y);
        let corner = rounded_cuboid(point(abs(q.x) - KEEP_HALF, p.y - (g + 4.0), abs(q.z) - KEEP_HALF), point(0.8, 5.5, 0.8), 0.1);
        let plinth = rounded_cuboid(point(q.x, p.y - (g - 0.45), q.z), point(6.35, 1.05, 6.35), 0.15);
        let solid = min(body, min(corner, plinth));
        let lower = max(square(q.x, q.z, KEEP_INNER, KEEP_INNER), g - 0.6 - p.y);
        let upper = max(square(q.x, q.z, KEEP_INNER + 0.35, KEEP_INNER + 0.35), g + 5.0 - p.y);
        let door_z = KEEP_HALF - 0.75;
        let door = cuboid(q - point(0.0, g + 1.0, door_z), point(0.6, 1.3, 1.4));
        let ay = q.y - (g + 2.3);
        let arch = max((q.x * q.x + ay * ay).sqrt() - 0.6, abs(q.z - door_z) - 1.4);
        let window = cuboid(q - point(door_z, g + 8.0, 0.0), point(1.0, 0.9, 0.6));
        let hearth = cuboid(q - point(-KEEP_INNER, g + 0.65, 1.0), point(0.55, 0.75, 0.8));
        let mut coarse = smooth_cut(solid, min(lower, upper), 0.3);
        coarse = cut(coarse, min(min(door, arch), min(window, hearth)));
        let slit = view.sees(0.35);
        if slit > 0.0 {
            let (w, h) = (0.175 * slit, 0.8 * slit);
            let s1 = cuboid(q - point(0.0, g + 5.8, 0.0), point(w, h, 7.0));
            let s2 = cuboid(q - point(0.0, g + 5.8, 0.0), point(7.0, h, w));
            coarse = cut(coarse, min(s1, s2));
        }
        if coarse > 0.1 {
            return Answer::new(coarse - 0.05, Material::Stone);
        }
        let mut d = coarse + detail(p, view, 0.4, 0.025, self.seed ^ 0x2f);
        let along = if abs(q.z) > abs(q.x) { p.x } else { p.z };
        d = masonry(d, p.y - g, along, 0.45, 0.9, 0.035 * view.sees(0.04), (top - 0.3 - p.y) * 0.65);
        // Ivy on the outer face, thickest to the west, up to about five metres.
        let face = between(sq, -0.5, -0.15);
        let ivy = face
            * between(noise(p / 1.8, self.seed ^ 0x5a) + 0.3 * between(-q.x, 2.0, 6.0), 0.0, 0.8)
            * between(g + 5.0 + 2.5 * noise(p / 3.5, self.seed ^ 0x5b) - p.y, 0.0, 1.0);
        let leaf = 0.05 * ivy * view.sees(0.05);
        let d = (d - leaf) * 0.8;
        let moss = between(noise(p / 0.9, self.seed ^ 0x5c), 0.1, 0.3) * between(g + 0.8 - p.y, 0.0, 0.2);
        let material = if ivy > 0.45 {
            Material::Leaf
        } else if moss > 0.5 {
            Material::Moss
        } else {
            Material::Stone
        };
        Answer::new(d, material)
    }

    /// In the hall: a heap of fallen stone in the north-east corner, and a beam from the lost floor
    /// lying with one end up on it.
    fn inside(&self, p: Point, view: &View) -> Answer {
        let g = self.keep_floor;
        let q = p - KEEP;
        let reach = max(square(q.x, q.z, KEEP_HALF, KEEP_HALF), max(p.y - (g + 1.5), g - 1.8 - p.y));
        if reach > 0.5 {
            return Answer::new(reach, Material::Stone);
        }
        let heap = (sphere(q - point(3.4, g - 1.6, -3.4), 2.6) + detail(p, view, 0.5, 0.06, self.seed ^ 0x61)) * 0.8;
        let beam = capsule(q, point(-3.6, g + 0.18, -1.0), point(2.7, g + 0.9, -2.7), 0.22);
        let grain = detail(p, view, 0.06, 0.05, self.seed ^ 0x63);
        Answer::new(heap, Material::Stone).or(Answer::new((beam + grain) * 0.85, Material::Wood))
    }

    /// The stump of the east tower, filled with earth and grassed on top, the broken stubs of its
    /// parapet about the rim: too high for anyone but a giant.
    fn stump(&self, p: Point, view: &View) -> Answer {
        let q = p - STUMP;
        let r = q.flat_length();
        let top = self.stump_top;
        let reach = max(r - STUMP_R, max(p.y - (top + 1.3), BOTTOM - p.y));
        if reach > 0.6 {
            return Answer::new(reach - 0.05, Material::Stone);
        }
        let drum = max(-smooth_union(-(r - STUMP_R), -(p.y - top), 0.2), BOTTOM - p.y);
        // The parapet, fallen but for a few stubs of masonry, each of its own height: whole shapes,
        // their distances true, their edges rounded.
        let mut parapet = 1.0e9;
        for k in 0..STUBS {
            let mut s = Seeds::of((self.seed ^ 0x4141) as u64, k);
            let a = (k as f32 + s.next(0.0, 0.6)) * TAU / STUBS as f32;
            let h = s.next(0.3, 1.2);
            let len = s.next(0.6, 1.2);
            let l = q.turned(a) - point(STUMP_R - 0.5, top + h * 0.5 - 0.25, 0.0);
            parapet = min(parapet, rounded_cuboid(l, point(0.35, h * 0.5 + 0.25, len), 0.1));
        }
        let coarse = min(drum, parapet);
        if coarse > 0.1 {
            return Answer::new(coarse - 0.05, Material::Stone);
        }
        let mut d = (coarse + detail(p, view, 0.3, 0.025, self.seed ^ 0x43)) * 0.85;
        let along = (atan2(q.z, q.x) + PI) * STUMP_R;
        d = masonry(d, p.y, along, 0.35, TAU * STUMP_R / 50.0, 0.03 * view.sees(0.035), (top - 0.3 - p.y) * 0.85);
        let foot = self.height(p.x, p.z);
        let moss = between(noise(p / 0.8, self.seed ^ 0x45), 0.15, 0.35) * between(foot + 0.5 - p.y, 0.0, 0.2);
        let material = if p.y > top - 0.2 && r < STUMP_R - 0.9 {
            Material::Grass
        } else if moss > 0.5 {
            Material::Moss
        } else {
            Material::Stone
        };
        Answer::new(d, material)
    }

    /// A piece of wall fallen whole into the bailey, tilted and half sunk, a broken piece beside
    /// it, and stones tumbled at the curtain's feet, rubble to a person, unseen by a giant.
    fn rubble(&self, p: Point, view: &View) -> Answer {
        let b = p - FALLEN;
        let reach = b.length() - 4.4;
        let mut best = if reach > 0.5 {
            Answer::new(reach, Material::Stone)
        } else {
            // Turned about the vertical, then tipped over on its side.
            let t = b.turned(0.6);
            let (s, c) = (sin(0.3), cos(0.3));
            let l = point(t.x, c * t.y - s * t.z, s * t.y + c * t.z);
            let big = rounded_cuboid(l - point(0.0, 0.25, 0.0), point(1.8, 1.0, 1.2), 0.2);
            let piece = rounded_cuboid((b - point(2.7, 0.0, 1.5)).turned(1.3), point(0.8, 0.45, 0.6), 0.15);
            let chips = 0.12 * noise(p / 0.9, self.seed ^ 0x72);
            let coarse = min(big, piece) + chips;
            let mut d = coarse + detail(p, view, 0.3, 0.025, self.seed ^ 0x71);
            d = masonry(d, l.y, l.x, 0.35, 0.7, 0.03 * view.sees(0.035), 0.95 - l.y);
            let mossy = l.y > 0.8 + 0.35 * noise(p / 0.8, self.seed ^ 0x73) && big < piece;
            Answer::new(d * 0.75, if mossy { Material::Moss } else { Material::Stone })
        };
        let seen = view.sees(0.3);
        if seen > 0.0 {
            let ring = abs(square(p.x - MIDDLE.x, p.z - MIDDLE.z, RING, RING));
            let h = self.height(p.x, p.z);
            let bound = max(abs(ring - 2.75) - 2.3, (abs(p.y - h) - 0.6) * 0.65);
            if bound > 0.3 {
                best = best.or(Answer::new(bound, Material::Stone));
            } else {
                let stones = scatter(p, 2.5, self.seed ^ 0x99, |q, s| {
                    let r = s.next(0.15, 0.45) * seen;
                    let chance = s.next(0.0, 1.0);
                    let place = p - q;
                    let rp = abs(square(place.x - MIDDLE.x, place.z - MIDDLE.z, RING, RING));
                    let by_crack = point(place.x - CRACK.x, 0.0, place.z - CRACK.z).flat_length();
                    let in_gate = abs(place.x - GATE_X) < 5.0 && place.z > 44.0;
                    if chance > 0.6 || rp < 1.3 || rp > 4.5 || by_crack < 2.5 || in_gate {
                        return 1.0;
                    }
                    sphere(point(q.x * 0.8, p.y - h + r * 0.3, q.z), r)
                });
                best = best.or(Answer::new(stones * 0.65, Material::Stone));
            }
        }
        best
    }
}

/// Courses and joints cut into a stone face as deep as `depth`, closed below `cap` (positive
/// under the top), so the groove runs into the top without a jump.
fn masonry(d: f32, y: f32, along: f32, course_h: f32, block: f32, depth: f32, cap: f32) -> f32 {
    if depth <= 0.0 {
        return d;
    }
    let course = (y / course_h).floor();
    let bed = abs(y - (y / course_h + 0.5).floor() * course_h) - 0.014;
    let shift = if (course as i32) & 1 == 1 { 0.5 } else { 0.0 };
    let t = along / block + shift;
    let joint = abs(t - (t + 0.5).floor()) * block - 0.014;
    let mortar = min(bed, joint);
    max(d, min(min(-mortar, d + depth), cap))
}

/// The distance to a square `hx` by `hz` about the origin, across the ground.
fn square(x: f32, z: f32, hx: f32, hz: f32) -> f32 {
    let (qx, qz) = (abs(x) - hx, abs(z) - hz);
    (max(qx, 0.0) * max(qx, 0.0) + max(qz, 0.0) * max(qz, 0.0)).sqrt() + min(max(qx, qz), 0.0)
}
```