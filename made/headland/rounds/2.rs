//! The lighthouse headland: a ridge of heath running north from the south edge, rising to a cliff
//! ten metres above the sea. A whitewashed lighthouse stands back from the edge. Out in the water, a
//! sea stack. Down both flanks the heath falls to sandy shores, so whoever falls in can wade round
//! and walk back up.
//!
//! Each size has its own headland:
//! - **The tiny** find a crack at the foot of the smugglers' rock, a granite outcrop near the cliff
//!   edge. The crack, two centimetres wide, leads to a tunnel that runs down to the smugglers' hold,
//!   where their casks still stand. A second tunnel runs on to a spy-hole in the cliff face, looking
//!   out to sea. Pebbles are boulders to them, the heath's lumps hills.
//! - **A person** walks the trodden path to the lighthouse, in at its narrow door, to the keeper's
//!   room: a table, a sea chest, slit windows above. They see the tower's courses of blocks and the
//!   strata of the cliff.
//! - **A giant** wades out from the shore to the sea stack and climbs onto its turf. It is far
//!   above anyone else standing in the water, and too far from the cliff to drop onto.

use plateau_sdk::*;

struct Headland {
    seed: u32,
    /// The ground's height at the lighthouse.
    gl: f32,
    /// The ground's height at the crack's mouth.
    g0: f32,
    /// Where the cliff is, along z, at the tunnels' x.
    wc: f32,
    /// The smugglers' rock's middle, along z.
    oz: f32,
}

const LEVEL: f32 = -1.5;
const SEA_FLOOR: f32 = -4.0;
const LIGHT: Point = point(27.0, 0.0, 36.0);
const STACK: Point = point(32.0, 0.0, 7.0);
/// The tunnels run north along this x.
const MX: f32 = 38.0;

impl Area for Headland {
    fn new(seed: u64) -> Headland {
        let seed = (seed ^ (seed >> 32)) as u32;
        let mut a = Headland { seed, gl: 0.0, g0: 0.0, wc: 0.0, oz: 0.0 };
        a.wc = a.cline(MX);
        a.oz = a.wc + 2.0;
        a.gl = a.raw_land(LIGHT.x, LIGHT.z);
        a.g0 = a.raw_land(MX, a.oz + 0.8);
        a
    }

    fn describe(&self) -> Description {
        let mut borders = Borders::of_ground(|x, z| self.surface_h(x, z));
        borders.north.water = Some(LEVEL);
        borders.east.water = Some(LEVEL);
        borders.west.water = Some(LEVEL);
        Description {
            name: "The lighthouse headland".into(),
            places: vec![
                Place::new("the smugglers' hold under the rock", Size::Tiny, point(MX, self.g0 - 0.251, self.oz - 0.4)),
                Place::new("the keeper's room in the lighthouse", Size::Person, point(LIGHT.x, self.gl + 0.25, LIGHT.z + 0.7)),
                Place::new("the top of the sea stack", Size::Giant, point(STACK.x, self.stack_top(STACK.x, STACK.z), STACK.z)),
            ],
            borders,
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let solid = self.ground(p, view).or(self.outcrop(p, view));
        let solid = self.tunnels(solid, p, view);
        let solid = solid.or(self.stack(p, view)).or(self.boulders(p, view)).or(self.lighthouse(p, view));
        let water = Answer::new(p.y - LEVEL, Material::Water).cut(solid.distance);
        solid.or(water)
    }
}

area!(Headland);

impl Headland {
    fn mouth(&self) -> Point {
        point(MX, 0.0, self.oz + 0.8)
    }

    /// The cliff's line: land lies south of it (z greater).
    fn cline(&self, x: f32) -> f32 {
        24.0 + 1.5 * sin(0.23 * x + 0.7) + 0.6 * noise(point(x / 5.0, 0.0, 7.5), self.seed ^ 0x5a)
    }

    /// The headland's ground before it is levelled anywhere. A ridge rises gently north to the
    /// cliff. It widens to the south edge, and its flanks fall away to the sea.
    fn raw_land(&self, x: f32, z: f32) -> f32 {
        let plateau = 3.0 + 3.2 * between(z, 62.0, 26.0) + 0.45 * noise(point(x / 11.0, 0.0, z / 11.0), self.seed);
        let w = 8.5 + 22.0 * between(z, 34.0, 64.0) + noise(point(x / 12.0, 0.0, z / 12.0), self.seed ^ 0x21);
        let o = abs(x - 32.0) - w;
        plateau - 0.25 * (o + (o * o + 4.0).sqrt())
    }

    /// The ground, levelled about the lighthouse and about the crack's mouth.
    fn height(&self, x: f32, z: f32) -> f32 {
        let mut h = self.raw_land(x, z);
        h = mix(self.gl, h, between(across(x, z, LIGHT), 4.5, 9.0));
        h = mix(self.g0, h, between(across(x, z, self.mouth()), 1.5, 3.5));
        h
    }

    fn sea_h(&self, x: f32, z: f32) -> f32 {
        SEA_FLOOR + 0.45 * noise(point(x / 7.0, 0.0, z / 7.0), self.seed ^ 0x33) * between(across(x, z, STACK), 7.0, 15.0)
    }

    fn surface_h(&self, x: f32, z: f32) -> f32 {
        let sea = self.sea_h(x, z);
        if z < self.cline(x) { sea } else { max(self.height(x, z), sea) }
    }

    fn trodden(&self, x: f32, z: f32) -> bool {
        let wobble = 0.3 * noise(point(x / 2.0, 0.0, z / 2.0), self.seed ^ 0x71);
        let to_light = to_segment(x, z, (32.0, 63.0), (LIGHT.x, LIGHT.z + 3.9));
        let to_rock = to_segment(x, z, (LIGHT.x + 3.5, LIGHT.z - 1.5), (MX - 0.8, self.oz + 1.6));
        to_light < 0.7 + wobble || to_rock < 0.35 + wobble * 0.5 || across(x, z, LIGHT) < 4.8
    }

    /// The land and the sea floor: heath, sand by the water, the cliff of layered stone, and
    /// pebbles for the tiny.
    fn ground(&self, p: Point, view: &View) -> Answer {
        let h = self.height(p.x, p.z);
        let dm = across(p.x, p.z, self.mouth());
        let dl = across(p.x, p.z, LIGHT);
        let fade = between(dm, 0.4, 1.2) * between(dl, 4.5, 6.0);
        let near = abs(p.y - h) < 0.6;
        let surface = if near && fade > 0.0 {
            h + fade * detail(point(p.x, 0.0, p.z), view, 0.8, 0.035, self.seed ^ 0x9e37)
        } else {
            h
        };
        // Steep flanks and lumps together: the distance made much shorter.
        let land_hf = if near || fade == 0.0 {
            (p.y - surface) * 0.55
        } else if p.y > h {
            (p.y - h - 0.07) * 0.55
        } else {
            (p.y - h + 0.07) * 0.55
        };

        // The cliff: land only south of its line, its face banded with strata.
        let off = self.cline(p.x) - p.z;
        let face = if abs(off) < 1.5 {
            let strata = 0.03 * view.sees(0.3) * sin(p.y * 9.0 + 2.0 * noise(p / 3.0, self.seed ^ 0x5b));
            (off * 0.8 + strata + detail(p, view, 1.6, 0.03, self.seed ^ 0x5c)) * 0.5
        } else {
            (off * 0.8 - 0.15) * 0.5
        };
        let land = max(land_hf, face);
        let land_mat = if face > land_hf {
            if p.y > LEVEL - 0.7 && p.y < LEVEL + 0.5 { Material::Moss } else { Material::Stone }
        } else if h < 0.5 + 0.4 * noise(point(p.x / 3.0, 0.0, p.z / 3.0), self.seed ^ 0x3c) {
            Material::Sand
        } else if self.trodden(p.x, p.z) {
            Material::Soil
        } else if h > 2.0 && noise(point(p.x / 2.5, 0.0, p.z / 2.5), self.seed ^ 0x6d) > 0.4 {
            Material::Moss
        } else {
            Material::Grass
        };

        // The sea floor: rock about the stack, sand elsewhere.
        let sh = self.sea_h(p.x, p.z);
        let sea_d = if abs(p.y - sh) < 0.4 {
            (p.y - sh - detail(point(p.x, 0.0, p.z), view, 1.0, 0.03, self.seed ^ 0x31)) * 0.75
        } else if p.y > sh {
            (p.y - sh - 0.07) * 0.75
        } else {
            (p.y - sh + 0.07) * 0.75
        };
        let sea_mat = if across(p.x, p.z, STACK) < 13.0 { Material::Stone } else { Material::Sand };
        let d = smooth_union(land, sea_d, 1.2);
        let ground = Answer::new(d, if land < sea_d { land_mat } else { sea_mat });

        // Pebbles, half sunk in the heath: boulders to the tiny. None by the crack or the lighthouse.
        let seen = view.sees(0.015);
        if seen == 0.0 || abs(p.y - surface) > 0.03 {
            return ground;
        }
        let keep = between(dm, 0.5, 1.0) * between(dl, 4.2, 5.0);
        if keep == 0.0 {
            return ground;
        }
        let pebbles = scatter(p, 0.06, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.012) * seen * keep;
            sphere(point(q.x, p.y - surface + r * 0.4, q.z), r)
        });
        ground.or(Answer::new(max(pebbles * 0.6, face), Material::Stone))
    }

    /// The smugglers' rock: a granite outcrop near the cliff's edge, lichen on its top.
    fn outcrop(&self, p: Point, view: &View) -> Answer {
        let q = p - point(MX, self.g0 + 0.45, self.oz);
        let bound = cuboid(q, point(1.7, 1.3, 1.25));
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let base = rounded_cuboid(q, point(1.3, 0.85, 0.8), 0.35) + 0.12 * noise(p / 0.9, self.seed ^ 0x0c);
        let d = if base < 0.25 { (base + detail(p, view, 0.5, 0.04, self.seed ^ 0x0d)) * 0.6 } else { (base - 0.05) * 0.6 };
        let lichen = q.y > 0.55 + 0.2 * noise(p / 0.5, self.seed ^ 0x0e);
        Answer::new(d, if lichen { Material::Moss } else { Material::Stone })
    }

    /// The smugglers' tunnels, for those small enough to see them. The crack at the rock's foot
    /// leads to a tunnel sloping down, then running level into the hold under the rock. A second
    /// tunnel runs from the hold to a spy-hole in the cliff face.
    fn tunnels(&self, solid: Answer, p: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        if seen == 0.0 {
            return solid;
        }
        let (g0, oz) = (self.g0, self.oz);
        let z0 = self.wc - 0.45;
        let z1 = oz + 1.2;
        let near = cuboid(p - point(MX, g0 - 0.1, (z0 + z1) * 0.5), point(0.3, 0.4, (z1 - z0) * 0.5));
        if near > 0.05 {
            return solid;
        }
        let hf = g0 - 0.251;
        let hz = oz - 0.4;
        let cx = MX + 0.03 * sin((p.z - oz) * 7.0);
        // The crack: 2.2 cm wide at its floor, narrowing upward, from outside the rock into it.
        let up = clamp((p.y - g0) / 0.25, 0.0, 1.0);
        let slot = max(max(abs(p.x - cx) - 0.011 * (1.0 - 0.6 * up), abs(p.y - (g0 + 0.1245)) - 0.1255), abs(p.z - (oz + 0.875)) - 0.325);
        // The tunnel's floor: level at the crack, sloping down, then level at the hold's floor well
        // before the hold, so it runs into the hold without a drop, and on level to the cliff.
        let floor = g0 - 0.001 - 0.25 * clamp((oz + 0.6 - p.z) / 0.75, 0.0, 1.0);
        let half = (oz + 0.65 - z0) * 0.5;
        let tunnel = rounded_cuboid(point(p.x - cx, p.y - floor - 0.02, p.z - (z0 + half)), point(0.012, 0.02, half), 0.008);
        let hold = rounded_cuboid(p - point(MX, hf + 0.07, hz), point(0.2, 0.07, 0.16), 0.05);
        let hollow = (min(min(slot, tunnel), hold) * 0.8 + detail(p, view, 0.02, 0.04, self.seed ^ 0x4c)) * 0.8;
        let mut out = solid.cut(hollow);
        if hollow < 0.02 {
            out.material = if p.y > floor + 0.006 { Material::Stone } else { Material::Soil };
        }
        let mixed = Answer::new(mix(solid.distance, out.distance, seen), out.material);
        // The smugglers' goods: two casks and a crate in the hold.
        let cask = |x: f32, z: f32| cylinder(p - point(MX + x, hf + 0.011, hz + z), 0.011, 0.013);
        let crate_d = rounded_cuboid(p - point(MX + 0.12, hf + 0.012, hz - 0.05), point(0.016, 0.014, 0.016), 0.002);
        let goods = min(min(cask(-0.12, -0.06), cask(-0.13, 0.05)), crate_d);
        mixed.or(Answer::new(goods, Material::Wood))
    }

    fn stack_top(&self, x: f32, z: f32) -> f32 {
        0.35 + 0.08 * noise(point(x / 3.0, 0.0, z / 3.0), self.seed ^ 0x57) - 0.012 * across(x, z, STACK)
    }

    /// The sea stack: sheer sides from the sea floor, turf on top, weed at the waterline.
    fn stack(&self, p: Point, view: &View) -> Answer {
        let q = p - STACK;
        let bound = cylinder(q - point(0.0, -4.5, 0.0), 5.3, 5.3);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let rad = 4.4 + 0.35 * noise(point(p.x / 2.2, 0.0, p.z / 2.2), self.seed ^ 0x58);
        let side = (q.flat_length() - rad) * 0.65;
        let top = (p.y - self.stack_top(p.x, p.z)) * 0.9;
        let body = max(max(side, top), -(p.y + 9.0));
        let d = if body < 0.3 { (body + detail(p, view, 0.8, 0.03, self.seed ^ 0x59)) * 0.8 } else { (body - 0.05) * 0.8 };
        let material = if top > side {
            Material::Grass
        } else if p.y > LEVEL - 0.7 && p.y < LEVEL + 0.5 {
            Material::Moss
        } else {
            Material::Stone
        };
        Answer::new(d, material)
    }

    /// Fallen rocks at the cliff's foot, some breaking the water, none near the stack.
    fn boulders(&self, p: Point, view: &View) -> Answer {
        let bound = max(p.y + 0.6, p.z - 27.5);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let d = scatter(p, 7.0, self.seed ^ 0xb01d, |q, s| {
            let r = s.next(1.2, 2.6);
            let keep = s.next(0.0, 1.0);
            let at = p - q;
            if keep > 0.6 || at.z > self.cline(at.x) - 2.8 || (at - STACK).flat_length() < 12.0 {
                return 30.0;
            }
            sphere(point(q.x, (q.y - (SEA_FLOOR + r * 0.55)) * 1.4, q.z), r) / 1.4
        });
        let d = if d < 0.3 { (d + detail(p, view, 0.6, 0.04, self.seed ^ 0xb0)) * 0.8 } else { (d - 0.05) * 0.8 };
        let material = if p.y > LEVEL - 0.7 { Material::Moss } else { Material::Stone };
        Answer::new(d, material)
    }

    /// The lighthouse. A tapering tower of stone courses stands on a round plinth. A door 1 m wide
    /// leads to the keeper's room. Above are slit windows with shutters, a gallery, and the lantern
    /// on its posts under a wooden dome.
    fn lighthouse(&self, p: Point, view: &View) -> Answer {
        let q = p - point(LIGHT.x, self.gl, LIGHT.z);
        let bound = cylinder(q - point(0.0, 10.0, 0.0), 4.4, 11.8);
        if bound > 0.5 {
            return Answer::new(bound, Material::Stone);
        }
        let r = q.flat_length();
        let plinth = cylinder(q - point(0.0, -0.5, 0.0), 3.8, 0.75);
        let mut body = max(r - (3.0 - 0.04 * q.y), abs(q.y - 7.5) - 8.5);
        // The blocks' bevelled joints, seen from a person's size down. The courses are half a block
        // along on every other course, 28 blocks round.
        let seen = view.sees(0.4);
        if seen > 0.0 && body < 0.1 {
            let ch = 0.45;
            let course = (q.y / ch).floor();
            let bed = abs(q.y - (q.y / ch + 0.5).floor() * ch);
            let shift = if (course as i32) & 1 == 1 { 0.5 } else { 0.0 };
            let t = atan2(q.z, q.x) / TAU * 28.0 + shift;
            let joint = abs(t - (t + 0.5).floor()) * (TAU * 3.0 / 28.0);
            let m = min(bed, joint);
            let caps = between(q.y, 0.25, 0.45) * between(q.y, 16.0, 15.8);
            body += seen * caps * 0.025 * (1.0 - between(m, 0.0, 0.12));
        }
        let gallery = cylinder(q - point(0.0, 16.1, 0.0), 3.4, 0.2);
        let parapet = max(abs(r - 3.25) - 0.13, abs(q.y - 16.8) - 0.55);
        let sector = TAU / 8.0;
        let local = q.turned((atan2(q.z, q.x) / sector + 0.5).floor() * sector);
        let posts = capsule(local, point(1.55, 16.25, 0.0), point(1.55, 18.6, 0.0), 0.12);
        let lamp = min(sphere(q - point(0.0, 17.35, 0.0), 0.55), cylinder(q - point(0.0, 16.7, 0.0), 0.22, 0.45));
        let mut stone = min(min(min(plinth, body), min(gallery, parapet)), min(posts, lamp));
        // The keeper's room, and the door to it, facing the path.
        let room = cylinder(q - point(0.0, 1.625, 0.0), 2.0, 1.375);
        let door = min(cuboid(q - point(0.0, 1.25, 2.6), point(0.5, 1.0, 1.0)), cylinder(point(q.x, q.z - 2.6, q.y - 2.25), 0.5, 1.0));
        // Shuttered slit windows up the south face.
        let k = clamp(((q.y - 5.5) / 4.0 + 0.5).floor(), 0.0, 2.0);
        let wy = 5.5 + 4.0 * k;
        let window = rounded_cuboid(point(q.x, q.y - wy, q.z - (3.0 - 0.04 * wy)), point(0.3, 0.6, 0.3), 0.06);
        stone = cut(cut(stone, min(room, door)), window);
        stone = if stone < 0.05 { stone + detail(p, view, 0.1, 0.03, self.seed ^ 0x1f) } else { stone - 0.007 };
        let stone = Answer::new(stone * 0.65, if window < 0.03 { Material::Wood } else { Material::Stone });

        let roof = max(sphere(q - point(0.0, 18.4, 0.0), 1.95), 18.5 - q.y);
        let spire = min(capsule(q, point(0.0, 20.0, 0.0), point(0.0, 21.2, 0.0), 0.1), sphere(q - point(0.0, 21.3, 0.0), 0.2));
        let chest = rounded_cuboid(q - point(1.1, 0.51, -0.9), point(0.42, 0.28, 0.28), 0.04);
        let top = cuboid(q - point(-0.85, 0.79, -0.7), point(0.55, 0.09, 0.38));
        let legs = cuboid(point(abs(q.x + 0.85) - 0.42, q.y - 0.47, abs(q.z + 0.7) - 0.26), point(0.09, 0.26, 0.09));
        let mut wood = min(min(roof, spire), min(chest, min(top, legs)));
        wood = if wood < 0.03 { wood + detail(point(q.x * 0.25, q.y, q.z), view, 0.02, 0.05, self.seed ^ 0x3d) } else { wood - 0.002 };
        stone.or(Answer::new(wood * 0.8, Material::Wood))
    }
}

/// How far `(x, z)` is from `c`, across the ground.
fn across(x: f32, z: f32, c: Point) -> f32 {
    let (dx, dz) = (x - c.x, z - c.z);
    (dx * dx + dz * dz).sqrt()
}

/// How far `(x, z)` is from the segment from `a` to `b`, across the ground.
fn to_segment(x: f32, z: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (px, pz, dx, dz) = (x - a.0, z - a.1, b.0 - a.0, b.1 - a.1);
    let t = clamp((px * dx + pz * dz) / (dx * dx + dz * dz), 0.0, 1.0);
    let (ex, ez) = (px - dx * t, pz - dz * t);
    (ex * ex + ez * ez).sqrt()
}
