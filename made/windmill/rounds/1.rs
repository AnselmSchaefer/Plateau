//! The windy hilltop: a wooden mill turning on a green hill above a brook, a mossy stone bridge,
//! lanterns along the path, and a terraced cliff with a giant's lookout stone.
//!
//! - **The tiny** find a cleft in the bedstone of the mill, marked by moss and mushrooms: it leads
//!   into a glowing hollow under the millstone.
//! - **A person** crosses the bridge, lights the lanterns (F), walks into the mill, works its brake.
//! - **A giant** steps up the cliff's terraces, three metres each, to the lookout stone.

use plateau_sdk::*;

const LANTERNS: usize = 8;
const BOULDERS: usize = 7;
const LEAVES: u32 = 10;

struct Hilltop {
    seed: u32,
    /// The ground's height at the bridge's north and south ends.
    north: f32,
    south: f32,
    boulders: [(Point, f32); BOULDERS],
    lit: [bool; LANTERNS],
    angle: f32,
    speed: f32,
    brake: bool,
    said_hill: bool,
    said_hollow: bool,
    said_lookout: bool,
}

/// The mill's middle, on the ground; its floor is a step above it.
const MILL: Point = point(24.0, 4.4, 22.0);
const STEP: f32 = 0.15;
const FLOOR: f32 = 4.55;
const HUB: Point = point(24.0, 12.4, 25.7);

/// The brook's water.
const LEVEL: f32 = -0.6;
const BED: f32 = -1.1;

const BRIDGE: Point = point(32.0, 0.0, 44.0);

/// The cliff's terraces.
const T1: f32 = 7.2;
const T2: f32 = 10.2;
const T3: f32 = 13.2;
const LOOKOUT: Point = point(51.0, 15.2, 13.5);

/// The path, and its branch to the cliff.
const PATH: [(f32, f32, f32, f32); 8] = [
    (32.0, 66.0, 32.0, 57.0),
    (32.0, 57.0, 32.0, 50.0),
    (32.0, 50.0, 32.0, 38.0),
    (32.0, 38.0, 29.5, 33.0),
    (29.5, 33.0, 25.5, 28.5),
    (25.5, 28.5, 24.0, 25.8),
    (29.5, 33.0, 36.0, 28.5),
    (36.0, 28.5, 40.5, 24.5),
];

/// The lanterns' posts, and which way their arms point; the last hangs in the mill.
const POSTS: [(f32, f32, f32); LANTERNS - 1] = [
    (33.7, 56.5, PI),
    (30.2, 51.0, 0.0),
    (33.9, 37.8, PI),
    (27.0, 33.0, 0.0),
    (27.4, 28.2, PI),
    (35.2, 31.8, PI),
    (39.8, 27.2, PI),
];

// Meshes, in the order the scene adds them.
const SAILS: u32 = 0;
const SPINDLE: u32 = 1;
const POST: u32 = 2;
const DARK: u32 = 3;
const LIT: u32 = 4;
const LEVER: u32 = 5;
const LEAF: u32 = 6;
const BIRD: u32 = 7;

// Dynamic instances and lights.
const ID_SAILS: u32 = 1;
const ID_SPINDLE: u32 = 2;
const ID_LEVER: u32 = 3;
const ID_LANTERN: u32 = 10;
const ID_LEAF: u32 = 30;
const ID_BIRD: u32 = 60;
const LIGHT_LANTERN: u32 = 110;
const LIGHT_HOLLOW: u32 = 130;

impl Area for Hilltop {
    fn new(seed: u64) -> Hilltop {
        let seed = (seed ^ (seed >> 32)) as u32;
        let mut a = Hilltop {
            seed,
            north: 0.0,
            south: 0.0,
            boulders: [(point(0.0, 0.0, 0.0), 1.0); BOULDERS],
            lit: [true, false, true, false, false, true, false, true],
            angle: 0.3,
            speed: 0.6,
            brake: false,
            said_hill: false,
            said_hollow: false,
            said_lookout: false,
        };
        a.north = a.height(BRIDGE.x, BRIDGE.z - 5.0);
        a.south = a.height(BRIDGE.x, BRIDGE.z + 5.0);
        let spots: [(f32, f32, f32); BOULDERS] = [
            (44.0, 35.0, 1.4),
            (16.0, 33.0, 1.6),
            (11.0, 17.0, 1.3),
            (29.0, 11.0, 1.5),
            (19.0, 53.0, 1.1),
            (47.0, 54.0, 1.5),
            (38.0, 44.6, 0.8),
        ];
        for i in 0..BOULDERS {
            let (x, z, r) = spots[i];
            a.boulders[i] = (point(x, a.height(x, z) + 0.1 * r, z), r);
        }
        a
    }

    fn describe(&self) -> Description {
        let mut borders = Borders::of_ground(|x, z| self.height(x, z));
        borders.east.water = Some(LEVEL);
        borders.west.water = Some(LEVEL);
        let (hx, hz) = (MILL.x, MILL.z + 0.3);
        Description {
            name: "The windy hilltop".into(),
            places: vec![
                Place::new("the glowing hollow under the millstone", Size::Tiny, point(hx, FLOOR + self.bump(hx, hz), hz)),
                Place::new("the mill room", Size::Person, point(MILL.x + 1.9, FLOOR, MILL.z + 0.8)),
                Place::new("the giant's lookout stone", Size::Giant, LOOKOUT),
            ],
            borders,
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let solid = self
            .ground(p, view)
            .or(self.bridge(p, view))
            .or(self.cliff(p, view))
            .or(self.mill(p, view))
            .or(self.rocks(p, view));
        let water = max(p.y - LEVEL, (abs(p.z - brook(p.x)) - 2.7) * 0.9);
        solid.or(Answer::new(water, Material::Water).cut(solid.distance))
    }

    fn scene(&self) -> Scene {
        let mut s = SceneBuilder::new();
        s.add_mesh(sails_mesh());
        s.add_mesh(spindle_mesh());
        s.add_mesh(post_mesh());
        s.add_mesh(lantern_mesh(false));
        s.add_mesh(lantern_mesh(true));
        s.add_mesh(lever_mesh());
        s.add_mesh(leaf_mesh());
        s.add_mesh(bird_mesh());
        let grass = s.add_mesh(grass_mesh());
        let flower_a = s.add_mesh(flower_mesh(255, 225, 90));
        let flower_b = s.add_mesh(flower_mesh(190, 150, 255));
        let reed = s.add_mesh(reed_mesh());
        let tree = s.add_mesh(tree_mesh());
        let shroom = s.add_mesh(mushroom_mesh());
        let crystal = s.add_mesh(crystal_mesh());
        let trim = s.add_mesh(trim_mesh());
        let sack = s.add_mesh(sack_mesh());

        let one = point(1.0, 1.0, 1.0);
        let none = Quat::IDENTITY;

        // The mill's moving parts and its fittings.
        s.spawn(ID_SAILS, SAILS, HUB, Quat::from_axis_angle(point(0.0, 0.0, 1.0), -self.angle), one, false);
        s.spawn(ID_SPINDLE, SPINDLE, MILL, Quat::from_y(self.angle), one, false);
        s.spawn(ID_LEVER, LEVER, self.lever(), Quat::from_axis_angle(point(0.0, 0.0, 1.0), -0.45), one, false);
        s.place(trim, MILL, none, one, false);
        for &(x, z, k) in &[(-1.9f32, -0.9f32, 1.0f32), (-1.5, -1.5, 0.9), (-2.1, -0.2, 0.8)] {
            s.place(sack, point(MILL.x + x, FLOOR, MILL.z + z), Quat::from_y(x * 3.0), one * k, true);
        }

        // The lanterns.
        for &(x, z, yaw) in &POSTS {
            s.place(POST, point(x, self.height(x, z) - 0.1, z), Quat::from_y(yaw), one, true);
        }
        for i in 0..LANTERNS {
            let hook = self.hook(i);
            s.spawn(ID_LANTERN + i as u32, if self.lit[i] { LIT } else { DARK }, hook, none, one, false);
            let strength = if self.lit[i] { 3.0 } else { 0.0 };
            s.light(LIGHT_LANTERN + i as u32, hook - point(0.0, 0.4, 0.0), [1.0, 0.72, 0.36], strength, 8.0);
        }

        // The hollow: mushrooms and crystals in a ring about the spindle's foot.
        let mut r = Seeds::of(self.seed as u64, 71);
        for k in 0..26 {
            let a = k as f32 * (TAU / 26.0) + r.next(-0.08, 0.08);
            let d = r.next(0.25, 0.5);
            let (x, z) = (d * cos(a), d * sin(a));
            // The way in from the south, and the place, are left clear.
            if abs(x) < 0.07 && z > 0.0 {
                continue;
            }
            let h = r.next(0.02, 0.07);
            let mesh = if k % 4 == 1 { crystal } else { shroom };
            let at = point(MILL.x + x, FLOOR - 0.001, MILL.z + z);
            s.place(mesh, at, Quat::from_euler(r.next(0.0, TAU), r.next(-0.15, 0.15), 0.0), one * h, false);
        }
        // And a few outside, where a person may notice them.
        for &(x, z, h) in &[(-0.11f32, 1.05f32, 0.07f32), (-0.17, 1.02, 0.045), (0.1, 1.06, 0.055), (0.16, 1.03, 0.08), (0.22, 1.0, 0.04)] {
            s.place(shroom, point(MILL.x + x, FLOOR - 0.001, MILL.z + z), Quat::from_y(x * 20.0), one * h, false);
        }
        s.light(LIGHT_HOLLOW, MILL + point(0.24, STEP + 0.15, 0.2), [0.3, 1.0, 0.8], 1.2, 1.0);
        s.light(LIGHT_HOLLOW + 1, MILL + point(-0.26, STEP + 0.15, -0.15), [0.6, 0.45, 1.0], 1.0, 1.0);
        s.light(LIGHT_HOLLOW + 2, MILL + point(0.0, STEP + 0.12, 1.16), [0.3, 1.0, 0.8], 0.4, 0.9);

        // Grass leaning with the wind, and flowers in it.
        let mut r = Seeds::of(self.seed as u64, 5);
        for k in 0..640 {
            let (x, z) = (r.next(1.0, 63.0), r.next(1.0, 63.0));
            let (yaw, size, kind) = (r.next(-0.6, 0.6), r.next(0.6, 1.4), r.below(2));
            if !self.meadow(x, z) {
                continue;
            }
            let at = point(x, self.height(x, z) - 0.04, z);
            if k % 6 == 0 {
                s.place(if kind == 0 { flower_a } else { flower_b }, at, Quat::from_y(yaw), one * size, false);
            } else {
                s.place(grass, at, Quat::from_y(yaw), one * size, false);
            }
        }
        // Reeds at the brook's edge.
        for k in 0..60 {
            let x = r.next(1.0, 63.0);
            let side = if k % 2 == 0 { 1.0 } else { -1.0 };
            let z = brook(x) + side * r.next(1.7, 2.3);
            let size = r.next(0.7, 1.2);
            if abs(x - BRIDGE.x) < 2.4 {
                continue;
            }
            s.place(reed, point(x, self.height(x, z) - 0.05, z), Quat::from_y(r.next(-0.5, 0.5)), one * size, false);
        }
        // Trees bent by the wind.
        for &(x, z, k) in &[(9.0f32, 27.0f32, 1.1f32), (54.0, 37.0, 0.9), (12.0, 56.0, 1.2), (56.0, 52.0, 1.0), (18.0, 8.0, 0.85)] {
            s.place(tree, point(x, self.height(x, z) - 0.15, z), Quat::from_y(0.25 * sin(x)), one * k, true);
        }

        for i in 0..LEAVES {
            s.spawn(ID_LEAF + i, LEAF, self.leaf_at(i, 0.0), none, one, false);
        }
        s.spawn(ID_BIRD, BIRD, point(58.0, 21.0, 15.0), none, one, false);
        s.build()
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Vec<Command> {
        let mut out = Vec::new();
        let dt = clamp(dt, 0.0, 0.1);
        let t = player.time;
        let one = point(1.0, 1.0, 1.0);
        let along_z = point(0.0, 0.0, 1.0);
        let gust = between(sin(t * 0.23) + 0.5 * sin(t * 0.61 + 1.0), 0.2, 1.2);

        // The sails, and what they drive.
        let wanted = if self.brake { 0.0 } else { 0.5 + 0.6 * gust };
        self.speed += (wanted - self.speed) * clamp(dt * 0.7, 0.0, 1.0);
        self.angle += self.speed * dt;
        if self.angle > TAU {
            self.angle -= TAU;
        }
        out.push(Command::SetTransform(instance(ID_SAILS, SAILS, HUB, Quat::from_axis_angle(along_z, -self.angle), one, false)));
        out.push(Command::SetTransform(instance(ID_SPINDLE, SPINDLE, MILL, Quat::from_y(self.angle * 2.0), one, false)));

        // The lanterns swing and flicker.
        for i in 0..LANTERNS {
            let k = i as f32;
            let indoors = i == LANTERNS - 1;
            let swing = if indoors { 0.02 * sin(t * 1.1) } else { (0.05 + 0.12 * gust) * sin(t * 2.1 + k * 1.7) + 0.1 * gust };
            let hook = self.hook(i);
            let turn = Quat::from_axis_angle(along_z, swing);
            let mesh = if self.lit[i] { LIT } else { DARK };
            out.push(Command::SetTransform(instance(ID_LANTERN + i as u32, mesh, hook, turn, one, false)));
            if self.lit[i] {
                let flicker = 3.0 + 0.25 * sin(t * 9.0 + k * 2.3) + 0.15 * sin(t * 17.0 + k);
                let at = hook + turn.rotate(point(0.0, -0.4, 0.0));
                out.push(Command::SetLight(light(LIGHT_LANTERN + i as u32, at, [1.0, 0.72, 0.36], flicker, 8.0)));
            }
        }

        // The hollow breathes.
        let breath = 0.5 + 0.5 * sin(t * 0.9);
        out.push(Command::SetLight(light(LIGHT_HOLLOW, MILL + point(0.24, STEP + 0.15, 0.2), [0.3, 1.0, 0.8], 0.8 + 0.6 * breath, 1.0)));
        out.push(Command::SetLight(light(LIGHT_HOLLOW + 1, MILL + point(-0.26, STEP + 0.15, -0.15), [0.6, 0.45, 1.0], 1.3 - 0.5 * breath, 1.0)));

        // Leaves on the wind, and a kestrel about the lookout.
        for i in 0..LEAVES {
            let k = i as f32;
            let turn = Quat::from_euler(t * 1.3 + k, t * 2.1 + k * 2.0, t * 0.7);
            out.push(Command::SetTransform(instance(ID_LEAF + i, LEAF, self.leaf_at(i, t), turn, one, false)));
        }
        let a = t * 0.35;
        let at = point(49.0 + 9.0 * cos(a), 21.5 + 1.2 * sin(t * 0.5), 15.0 + 9.0 * sin(a));
        out.push(Command::SetTransform(instance(ID_BIRD, BIRD, at, Quat::from_euler(-a, 0.0, 0.25 + 0.05 * sin(t * 3.0)), one, false)));

        // Gusts carry whoever leaps on the hilltop.
        let feet = player.feet;
        let from_mill = (feet - MILL).flat_length();
        if !player.grounded && feet.y > 3.2 && from_mill > 4.2 && player.size < 4.0 && gust > 0.3 {
            out.push(Command::ApplyImpulse(point(1.1 * gust * dt * player.size, 0.0, 0.1 * gust * dt * player.size)));
        }

        if !self.said_hill && feet.y > 3.6 && from_mill > 4.5 && from_mill < 14.0 {
            self.said_hill = true;
            out.push(Command::Say("The wind leans on you. The sails creak round.".into()));
        }
        if !self.said_hollow && player.size < 0.1 && (feet - point(MILL.x, FLOOR, MILL.z)).length() < 0.55 {
            self.said_hollow = true;
            out.push(Command::Say("The great stone hums overhead, and the dark is full of light.".into()));
        }
        if !self.said_lookout && player.size > 8.0 && (feet - LOOKOUT).length() < 3.0 {
            self.said_lookout = true;
            out.push(Command::Say("From here the mill is a toy, and the wind has nothing left to climb.".into()));
        }
        out
    }

    fn interact(&mut self, player: &PlayerState) -> Vec<Command> {
        let mut out = Vec::new();
        let lever = self.lever();
        let near_lever = (player.feet - lever).length() < 2.0 && player.size < 4.0;
        let mut chosen: Option<usize> = None;
        if player.target >= ID_LANTERN && player.target < ID_LANTERN + LANTERNS as u32 {
            chosen = Some((player.target - ID_LANTERN) as usize);
        } else if player.target == ID_LEVER || player.target == ID_SAILS || near_lever {
            self.brake = !self.brake;
            let lean = if self.brake { 0.45 } else { -0.45 };
            let turn = Quat::from_axis_angle(point(0.0, 0.0, 1.0), lean);
            out.push(Command::SetTransform(instance(ID_LEVER, LEVER, lever, turn, point(1.0, 1.0, 1.0), false)));
            let words = if self.brake { "The brake bites. The sails slow to a stop." } else { "The brake lets go. The wind takes the sails." };
            out.push(Command::Say(words.into()));
            return out;
        } else {
            let mut best = if player.size > 4.0 { 7.0 } else { 3.2 };
            for i in 0..LANTERNS {
                let d = (self.hook(i) - player.feet).flat_length();
                if d < best && abs(self.hook(i).y - player.feet.y) < 6.0 {
                    best = d;
                    chosen = Some(i);
                }
            }
        }
        if let Some(i) = chosen {
            self.lit[i] = !self.lit[i];
            let on = self.lit[i];
            let hook = self.hook(i);
            let mesh = if on { LIT } else { DARK };
            out.push(Command::SetTransform(instance(ID_LANTERN + i as u32, mesh, hook, Quat::IDENTITY, point(1.0, 1.0, 1.0), false)));
            let strength = if on { 3.0 } else { 0.0 };
            out.push(Command::SetLight(light(LIGHT_LANTERN + i as u32, hook - point(0.0, 0.4, 0.0), [1.0, 0.72, 0.36], strength, 8.0)));
            let mut all = true;
            for l in self.lit {
                all = all && l;
            }
            let words = if all {
                "Every lantern burns. The whole path glows up to the mill."
            } else if on {
                "The wick catches, and holds against the wind."
            } else {
                "You pinch out the flame."
            };
            out.push(Command::Say(words.into()));
        }
        out
    }
}

area!(Hilltop);

impl Hilltop {
    /// The ground's height, without its roughness: a meadow, the hill, the mill's levelled yard,
    /// and the brook's bed.
    fn height(&self, x: f32, z: f32) -> f32 {
        let meadow = 0.18 * noise(point(x / 10.0, 0.0, z / 10.0), self.seed) + 0.1 * noise(point(x / 5.0, 0.0, z / 5.0), self.seed ^ 0x21);
        let (hx, hz) = ((x - 33.0) / 1.25, z - 20.0);
        let hill = 4.4 * between((hx * hx + hz * hz).sqrt(), 24.0, 9.0);
        let h = meadow + hill;
        let (mx, mz) = (x - MILL.x, z - MILL.z);
        let yard = between((mx * mx + mz * mz).sqrt(), 8.0, 4.8);
        let h = mix(h, MILL.y, yard);
        mix(h, BED, between(abs(z - brook(x)), 3.4, 0.6))
    }

    /// Open meadow: where grass and flowers grow.
    fn meadow(&self, x: f32, z: f32) -> bool {
        let (mx, mz) = (x - MILL.x, z - MILL.z);
        let (cx, cz) = (x - 48.5, z - 16.5);
        abs(z - brook(x)) > 2.9 && path(x, z) > 1.1 && mx * mx + mz * mz > 4.8 * 4.8 && cx * cx + cz * cz > 11.5 * 11.5
    }

    fn ground(&self, p: Point, view: &View) -> Answer {
        let h = self.height(p.x, p.z);
        let dy = p.y - h;
        if abs(dy) > 1.0 {
            let d = if dy > 0.0 { dy - 0.2 } else { dy + 0.2 };
            return Answer::new(d * 0.5, Material::Grass);
        }
        let from_mill = (p - MILL).flat_length();
        let open = between(from_mill, 4.2, 5.5);
        let rough = if open > 0.0 { open * detail(point(p.x, 0.0, p.z), view, 1.2, 0.04, self.seed ^ 0x9e37) } else { 0.0 };
        let surface = h + rough;
        // The bed of sand, mud at its banks, trodden earth along the path.
        let ragged = noise(point(p.x / 1.3, 0.0, p.z / 1.3), self.seed ^ 0x3c);
        let off = abs(p.z - brook(p.x));
        let material = if off < 1.3 + 0.3 * ragged {
            Material::Sand
        } else if off < 2.5 + 0.35 * ragged {
            Material::Mud
        } else if path(p.x, p.z) < 0.85 + 0.2 * ragged || from_mill < 4.3 + 0.4 * ragged {
            Material::Soil
        } else {
            Material::Grass
        };
        let ground = Answer::new((p.y - surface) * 0.5, material);
        let seen = view.sees(0.015) * open;
        if seen == 0.0 || abs(p.y - surface) > 0.03 {
            return ground;
        }
        // Pebbles: boulders to the tiny.
        let pebbles = scatter(p, 0.07, self.seed ^ 0x77, |q, s| {
            let r = s.next(0.004, 0.011) * seen;
            sphere(point(q.x, p.y - surface + r * 0.4, q.z), r)
        });
        ground.or(Answer::new(pebbles * 0.42, Material::Stone))
    }

    /// The bridge's walkway at `z`.
    fn deck(&self, z: f32) -> f32 {
        let u = clamp((z - BRIDGE.z) / 5.0, -1.2, 1.2);
        mix(self.north, self.south, (u + 1.0) * 0.5) + 0.55 * (1.0 - u * u) - 0.06
    }

    /// The mossy bridge: one arch, a humped walkway, low parapets.
    fn bridge(&self, p: Point, view: &View) -> Answer {
        let q = p - BRIDGE;
        let bound = cuboid(q - point(0.0, -0.3, 0.0), point(1.8, 3.0, 5.4));
        if bound > 0.6 {
            return Answer::new(bound, Material::Stone);
        }
        let top = self.deck(p.z);
        let body = max(max(abs(q.x) - 1.35, abs(q.z) - 5.0), max((p.y - top) * 0.9, -2.6 - p.y));
        // The parapets, too thin for a giant's eye.
        let raised = 0.55 * view.sees(0.5);
        let wall = max(max(abs(abs(q.x) - 1.15) - 0.2, abs(q.z) - 4.9), max((p.y - top - raised) * 0.9, -1.0 - p.y));
        let mut d = min(body, wall);
        // The arch, closed for a giant.
        let opened = view.sees(1.2);
        if opened > 0.0 {
            let arch = (q.z * q.z + (p.y + 1.85) * (p.y + 1.85)).sqrt() - 2.3;
            d = mix(d, max(d, -arch), opened);
        }
        let near = between(d, 0.3, 0.12);
        let rough = if near > 0.0 { detail(p, view, 0.5, 0.03, self.seed ^ 0x51f) } else { 0.0 };
        d = (d + mix(-0.04, rough, near)) * 0.75;
        // Joints between the stones, ravines to the tiny.
        let depth = 0.015 * view.sees(0.03);
        if depth > 0.0 && near > 0.0 {
            let t = p.z / 0.5;
            let joint = abs(t - (t + 0.5).floor()) * 0.5 - 0.01;
            d = max(d, min(-joint, d + depth * near));
        }
        let mut answer = Answer::new(d, Material::Stone);
        if near > 0.0 {
            let side = between(abs(q.x), 0.7, 1.2);
            let patch = noise(p / 0.7, self.seed ^ 0x6d) + 0.35 * side + 0.4 * between(p.y, 0.2, -0.5);
            let moss = between(patch, 0.1, 0.35);
            if moss > 0.0 {
                answer.material = Material::Moss;
                let seen = view.sees(0.006) * moss * near;
                if seen > 0.0 {
                    let tufts = seen * 0.003 * (0.6 + 0.4 * noise(p / 0.006, self.seed ^ 0x2b));
                    answer = Answer::new((answer.distance - tufts) * (1.0 - 0.5 * seen), Material::Moss);
                }
            }
        }
        answer
    }

    /// The cliff: three terraces, each too high for a person, sheer to the north-east; on top the
    /// giant's lookout stone and a standing stone at its back.
    fn cliff(&self, p: Point, view: &View) -> Answer {
        let bound = cylinder(p - point(49.0, 8.0, 16.0), 13.0, 12.5);
        if bound > 1.0 {
            return Answer::new(bound, Material::Stone);
        }
        let ribs = 0.55 * noise(point(p.x / 4.0, p.y / 9.0, p.z / 4.0), self.seed ^ 0xc1) + 0.14 * noise(point(p.x / 1.6, p.y / 8.0, p.z / 1.6), self.seed ^ 0xc2);
        let swell = 0.14 * noise(point(p.x / 3.0, 0.0, p.z / 3.0), self.seed ^ 0xc3);
        let tier = |x: f32, z: f32, r: f32, top: f32| {
            let (dx, dz) = (p.x - x, p.z - z);
            let side = ((dx * dx + dz * dz).sqrt() - r + ribs) * 0.5;
            let up = (p.y - top - swell) * 0.9;
            let d = max(-smooth_union(-side, -up, 0.4), -4.0 - p.y);
            let turf = p.y > top - 0.35 && noise(p / 1.8, self.seed ^ 0xc4) > -0.35;
            Answer::new(d, if turf { Material::Grass } else { Material::Stone })
        };
        let mut rock = tier(48.0, 17.0, 9.5, T1).or(tier(49.5, 15.0, 7.0, T2)).or(tier(51.0, 13.5, 5.0, T3));
        if rock.material == Material::Stone && noise(p / 2.2, self.seed ^ 0xc5) > 0.42 {
            rock.material = Material::Moss;
        }
        let slab = rounded_cuboid((p - point(LOOKOUT.x, T3 + 0.75, LOOKOUT.z)).turned(0.4), point(3.2, 1.25, 2.9), 0.4);
        let menhir = rounded_cuboid((p - point(LOOKOUT.x + 2.6, T3 + 2.2, LOOKOUT.z - 2.6)).turned(0.75), point(1.2, 3.0, 0.95), 0.4);
        let crown = Answer::new(min(slab, menhir), Material::Stone);
        let a = rock.or(crown);
        let near = between(a.distance, 0.6, 0.3);
        let rough = if near > 0.0 { detail(p, view, 1.6, 0.04, self.seed ^ 0xc6) } else { 0.0 };
        Answer::new((a.distance + mix(-0.14, rough, near)) * 0.6, a.material)
    }

    /// The mill: a tapered, boarded tower on a stone footing under an eight-sided cap; a room with
    /// the millstones in it, and under them the hollow.
    fn mill(&self, p: Point, view: &View) -> Answer {
        let q = p - MILL;
        let bound = cylinder(q - point(0.0, 6.0, 0.0), 4.5, 6.8);
        if bound > 0.6 {
            return Answer::new(bound, Material::Wood);
        }
        let o = octagon(q.x, q.z);
        // Weatherboards: each leans out to its foot.
        let t = q.y / 0.4;
        let f = t - t.floor();
        let lap = if f < 0.2 { f / 0.2 } else { (1.0 - f) / 0.8 };
        let boards = 0.035 * view.sees(0.4) * lap;
        let tower = max((o - (3.6 - 0.141 * q.y) - boards) * 0.88, abs(q.y - 4.1) - 4.6);
        let cap = max((o + (q.y - 8.4) * 0.9 - 3.1) * 0.74, 8.4 - q.y);
        let footing = max(o - 3.8, abs(q.y - 0.25) - 0.75);
        let mut a = Answer::new(min(tower, cap), Material::Wood).or(Answer::new(footing, Material::Stone));
        if a.material == Material::Stone && noise(p / 0.6, self.seed ^ 0xd1) > 0.2 {
            a.material = Material::Moss;
        }
        let near = between(a.distance, 0.12, 0.05);
        let grain = if near > 0.0 { detail(point(q.x, q.y * 0.25, q.z), view, 0.2, 0.03, self.seed ^ 0xd2) } else { 0.0 };
        a = Answer::new((a.distance + mix(-0.02, grain, near)) * 0.85, a.material);

        // The room, its door and windows: closed for a giant, who sees a solid tower.
        let opened = view.sees(1.0);
        if opened > 0.0 {
            let room = max(o - 2.8, abs(q.y - STEP - 1.5) - 1.5);
            let door = cuboid(q - point(0.0, STEP + 1.15, 3.4), point(0.7, 1.15, 1.2));
            let windows = cuboid(point(abs(q.x) - 3.2, q.y - 1.75, q.z), point(1.0, 0.3, 0.3));
            let hollow = min(room, min(door, windows));
            if hollow < 0.05 && o < 3.4 {
                a.material = if q.y > 0.2 && q.y < 1.0 { Material::Stone } else { Material::Wood };
            }
            a = Answer::new(mix(a.distance, cut(a.distance, hollow), opened), a.material);
        }

        // The bedstone and the runner stone on it.
        let bed = cylinder(q - point(0.0, 0.25, 0.0), 0.96, 0.26) - 0.04;
        let runner = cylinder(q - point(0.0, 0.72, 0.0), 0.81, 0.18) - 0.04;
        let stones = min(bed, runner);
        let near = between(stones, 0.12, 0.05);
        let rough = if near > 0.0 { detail(p, view, 0.25, 0.02, self.seed ^ 0xd3) } else { 0.0 };
        let mut a = a.or(Answer::new((stones + mix(-0.012, rough, near)) * 0.8, Material::Stone));

        // The mark of the way in: moss about a dark cleft.
        let mark = (p - point(MILL.x, FLOOR + 0.04, MILL.z + 1.0)).length();
        if mark < 0.5 {
            let edge = 0.06 * noise(p / 0.1, self.seed ^ 0xd4);
            if mark < 0.1 + edge * 0.3 {
                a.material = Material::Mud;
            } else if mark < 0.32 + edge {
                a.material = Material::Moss;
            }
        }
        self.hollow(a, p, q, view)
    }

    /// The hollow's floor, above the mill's: worn earth, a millimetre up and down.
    fn bump(&self, x: f32, z: f32) -> f32 {
        0.001 * noise(point(x / 0.03, 0.0, z / 0.03), self.seed ^ 0xe1)
    }

    /// The hollow in the bedstone and the cleft that leads to it, for the tiny alone.
    fn hollow(&self, solid: Answer, p: Point, q: Point, view: &View) -> Answer {
        let seen = view.sees(0.06);
        if seen == 0.0 || cylinder(q - point(0.0, 0.25, 0.0), 1.2, 0.3) > 0.05 {
            return solid;
        }
        let rough = detail(p, view, 0.06, 0.05, self.seed ^ 0xe2);
        let room = cylinder(q - point(0.0, STEP + 0.1, 0.0), 0.47, 0.08) - 0.08 + rough;
        // The spindle's foot, a pillar of stone flaring into floor and roof.
        let flare = 0.6 * max(abs(q.y - STEP - 0.12) - 0.05, 0.0);
        let pillar = (q.flat_length() - 0.06 - flare) * 0.8;
        let room = smooth_cut(room, pillar, 0.03) * 0.55;
        let wiggle = 0.004 * noise(point(q.z / 0.06, 0.0, 0.0), self.seed ^ 0xe3);
        let up = clamp((q.y - STEP) / 0.06, 0.0, 1.0);
        let slot = (abs(q.x - wiggle) - 0.013 * (1.0 - 0.45 * up)) * 0.9;
        let cleft = max(max(slot, abs(q.z - 0.78) - 0.3), q.y - STEP - 0.06);
        let void = min(room, cleft);
        let floor = (STEP + self.bump(p.x, p.z) - q.y) * 0.9;
        let void = max(void, floor);
        let mut hollowed = solid.cut(void);
        if void < 0.02 && q.flat_length() < 0.99 {
            hollowed.material = if q.y < STEP + 0.008 {
                if noise(p / 0.07, self.seed ^ 0xe4) > 0.0 { Material::Moss } else { Material::Soil }
            } else if noise(p / 0.09, self.seed ^ 0xe5) > 0.3 {
                Material::Moss
            } else {
                Material::Stone
            };
        }
        Answer::new(mix(solid.distance, hollowed.distance, seen), hollowed.material)
    }

    /// Boulders on the hill and one in the brook.
    fn rocks(&self, p: Point, view: &View) -> Answer {
        let mut d = 1000.0;
        for (c, r) in self.boulders {
            d = min(d, sphere(p - c, r));
        }
        if d > 1.0 {
            return Answer::new((d - 0.3) * 0.55, Material::Stone);
        }
        let near = between(d, 0.8, 0.4);
        let rough = if near > 0.0 {
            0.16 * noise(p / 0.9, self.seed ^ 0xf1) + detail(p, view, 0.5, 0.03, self.seed ^ 0xf2)
        } else {
            0.0
        };
        let material = if noise(p / 0.5, self.seed ^ 0xf3) > 0.25 { Material::Moss } else { Material::Stone };
        Answer::new((d + mix(-0.3, rough, near)) * 0.55, material)
    }

    /// Where the lantern `i` hangs from.
    fn hook(&self, i: usize) -> Point {
        if i < POSTS.len() {
            let (x, z, yaw) = POSTS[i];
            point(x, self.height(x, z) - 0.1, z) + Quat::from_y(yaw).rotate(point(0.5, 2.15, 0.0))
        } else {
            MILL + point(-1.5, STEP + 2.98, 1.0)
        }
    }

    /// The brake lever's foot, inside by the door.
    fn lever(&self) -> Point {
        MILL + point(0.98, STEP + 0.8, 2.7)
    }

    /// Where the drifting leaf `i` is at a time: blown east over the hilltop, again and again.
    fn leaf_at(&self, i: u32, t: f32) -> Point {
        let k = i as f32;
        let far = t * (2.2 + 0.23 * k) + k * 7.3;
        let x = 1.0 + far - (far / 36.0).floor() * 36.0;
        let z = 14.0 + k * 2.9 + 1.5 * sin(t * 0.5 + k);
        point(x, self.height(x, z) + 1.6 + 0.8 * sin(t * 0.9 + k * 1.9), z)
    }
}

/// The brook's middle at `x`.
fn brook(x: f32) -> f32 {
    44.0 + 1.2 * (1.0 - cos((x - 32.0) * 0.15))
}

/// How far it is across the ground to the path.
fn path(x: f32, z: f32) -> f32 {
    let mut best = 1.0e6;
    for (ax, az, bx, bz) in PATH {
        let (px, pz, dx, dz) = (x - ax, z - az, bx - ax, bz - az);
        let t = clamp((px * dx + pz * dz) / (dx * dx + dz * dz), 0.0, 1.0);
        let (ex, ez) = (px - dx * t, pz - dz * t);
        best = min(best, ex * ex + ez * ez);
    }
    best.sqrt()
}

/// How far out of the middle in an eight-sided plan.
fn octagon(x: f32, z: f32) -> f32 {
    let (ax, az) = (abs(x), abs(z));
    max(max(ax, az), (ax + az) * 0.707_106_77)
}

// The scene's meshes.

/// A triangle seen from both sides.
fn both(mb: &mut MeshBuilder, a: Point, b: Point, c: Point, m: Material) {
    mb.flat_tri(a, b, c, m);
    mb.flat_tri(a, c, b, m);
}

/// A spot on the sail `k`: `along` its stock, `across` from it.
fn on_sail(k: u32, along: f32, across: f32) -> (f32, f32) {
    match k {
        0 => (across, along),
        1 => (along, -across),
        2 => (-across, -along),
        _ => (-along, across),
    }
}

fn spar(mb: &mut MeshBuilder, k: u32, middle: (f32, f32, f32), half: (f32, f32, f32)) {
    let (x, y) = on_sail(k, middle.0, middle.1);
    let size = if k % 2 == 0 { point(half.1, half.0, half.2) } else { point(half.0, half.1, half.2) };
    mb.cuboid(point(x, y, middle.2), size, Material::Wood);
}

/// The axle, the hub and four sails of lattice and canvas, turning about z.
fn sails_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(190, 170, 150);
    mb.sweep_tube(&[(point(0.0, 0.0, -1.5), 0.24), (point(0.0, 0.0, 0.2), 0.24), (point(0.0, 0.0, 0.5), 0.12)], 10, true, Material::Wood);
    mb.cuboid(point(0.0, 0.0, 0.0), point(0.34, 0.34, 0.2), Material::Wood);
    for k in 0..4 {
        mb.tint(200, 180, 155);
        spar(&mut mb, k, (2.5, 0.0, 0.0), (2.5, 0.1, 0.08));
        spar(&mut mb, k, (2.95, 1.1, 0.0), (1.95, 0.04, 0.04));
        spar(&mut mb, k, (2.95, 0.6, 0.0), (1.95, 0.025, 0.03));
        for j in 0..9 {
            spar(&mut mb, k, (1.02 + 0.48 * j as f32, 0.58, 0.0), (0.03, 0.56, 0.03));
        }
        // The canvas, reefed a little short of the tip and bellied by the wind.
        mb.tint(255, 244, 220);
        for j in 0..6 {
            let (l0, l1) = (1.1 + 0.6 * j as f32, 1.7 + 0.6 * j as f32);
            for i in 0..3 {
                let (c0, c1) = (0.1 + 0.32 * i as f32, 0.42 + 0.32 * i as f32);
                let belly = |c: f32| 0.05 + 0.1 * (1.0 - abs(c - 0.58) * 2.0);
                let at = |l: f32, c: f32| {
                    let (x, y) = on_sail(k, l, c);
                    point(x, y, belly(c))
                };
                both(&mut mb, at(l0, c0), at(l0, c1), at(l1, c1), Material::Sand);
                both(&mut mb, at(l0, c0), at(l1, c1), at(l1, c0), Material::Sand);
            }
        }
    }
    mb.build()
}

/// The upright shaft over the millstones and its toothed wheel, about y, from the mill's ground.
fn spindle_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(180, 150, 120);
    mb.sweep_tube(&[(point(0.0, 0.85, 0.0), 0.16), (point(0.0, 1.15, 0.0), 0.1), (point(0.0, 3.2, 0.0), 0.1)], 10, true, Material::Wood);
    let mut rim = Vec::new();
    for s in 0..25 {
        let a = s as f32 * (TAU / 24.0);
        rim.push((point(0.85 * cos(a), 2.6, 0.85 * sin(a)), 0.07));
    }
    mb.sweep_tube(&rim, 6, false, Material::Wood);
    mb.cuboid(point(0.0, 2.6, 0.0), point(0.85, 0.05, 0.06), Material::Wood);
    mb.cuboid(point(0.0, 2.6, 0.0), point(0.06, 0.05, 0.85), Material::Wood);
    mb.tint(220, 195, 160);
    for s in 0..24 {
        let a = s as f32 * (TAU / 24.0);
        mb.capsule(point(0.85 * cos(a), 2.5, 0.85 * sin(a)), point(0.85 * cos(a), 2.72, 0.85 * sin(a)), 0.03, 5, Material::Wood);
    }
    // The damsel: four knockers at the stone's eye.
    for s in 0..4 {
        let a = s as f32 * (TAU / 4.0);
        mb.capsule(point(0.2 * cos(a), 0.95, 0.2 * sin(a)), point(0.12 * cos(a), 1.4, 0.12 * sin(a)), 0.03, 5, Material::Wood);
    }
    mb.build()
}

/// A lantern's post, its arm reaching along x.
fn post_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(175, 150, 125);
    mb.sweep_tube(&[(point(0.0, -0.3, 0.0), 0.09), (point(0.02, 1.0, 0.0), 0.075), (point(0.0, 2.35, 0.0), 0.06)], 8, true, Material::Wood);
    mb.cuboid(point(0.24, 2.2, 0.0), point(0.34, 0.035, 0.035), Material::Wood);
    mb.sweep_tube(&[(point(0.02, 1.75, 0.0), 0.025), (point(0.16, 2.0, 0.0), 0.028), (point(0.36, 2.18, 0.0), 0.025)], 6, true, Material::Wood);
    mb.tint(150, 150, 150);
    mb.ellipsoid(point(0.0, 0.05, 0.0), point(0.22, 0.14, 0.2), 8, 5, Material::Stone);
    mb.build()
}

/// A lantern hanging from the origin: a cage about a glass, dark or lit.
fn lantern_mesh(lit: bool) -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(80, 78, 76);
    mb.sweep_tube(&[(point(0.0, 0.03, 0.0), 0.012), (point(0.0, -0.16, 0.0), 0.012)], 5, true, Material::Stone);
    mb.cone(point(0.0, -0.26, 0.0), 0.13, 0.17, 8, Material::Stone);
    mb.cuboid(point(0.0, -0.27, 0.0), point(0.12, 0.015, 0.12), Material::Stone);
    mb.cuboid(point(0.0, -0.57, 0.0), point(0.12, 0.02, 0.12), Material::Stone);
    for &(x, z) in &[(-0.1f32, -0.1f32), (0.1, -0.1), (0.1, 0.1), (-0.1, 0.1)] {
        mb.cuboid(point(x, -0.42, z), point(0.012, 0.15, 0.012), Material::Stone);
    }
    if lit {
        mb.tint(255, 222, 130);
        mb.ellipsoid(point(0.0, -0.42, 0.0), point(0.08, 0.12, 0.08), 10, 8, Material::Sand);
        mb.tint(255, 255, 220);
        mb.cone(point(0.0, -0.5, 0.0), 0.2, 0.05, 8, Material::Sand);
    } else {
        mb.tint(90, 105, 120);
        mb.ellipsoid(point(0.0, -0.42, 0.0), point(0.08, 0.12, 0.08), 10, 8, Material::Stone);
    }
    mb.build()
}

fn lever_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(150, 120, 95);
    mb.capsule(point(0.0, 0.0, 0.0), point(0.0, 0.7, 0.0), 0.035, 7, Material::Wood);
    mb.tint(220, 110, 80);
    mb.ellipsoid(point(0.0, 0.76, 0.0), point(0.06, 0.08, 0.06), 8, 6, Material::Wood);
    mb.tint(90, 90, 90);
    mb.ellipsoid(point(0.0, 0.0, 0.0), point(0.08, 0.08, 0.1), 8, 6, Material::Stone);
    mb.build()
}

fn leaf_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(235, 190, 90);
    let (a, b, c, d) = (point(-0.09, 0.0, 0.0), point(0.0, 0.02, 0.045), point(0.1, 0.01, 0.0), point(0.0, 0.02, -0.045));
    both(&mut mb, a, b, c, Material::Leaf);
    both(&mut mb, a, c, d, Material::Leaf);
    mb.build()
}

/// A kestrel, flying along z.
fn bird_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(170, 120, 85);
    mb.ellipsoid(point(0.0, 0.0, 0.0), point(0.11, 0.1, 0.3), 8, 6, Material::Wood);
    mb.ellipsoid(point(0.0, 0.05, 0.3), point(0.08, 0.08, 0.1), 8, 6, Material::Wood);
    for side in [-1.0f32, 1.0] {
        let (a, b) = (point(0.06 * side, 0.03, 0.16), point(0.06 * side, 0.03, -0.12));
        let (c, d) = (point(0.5 * side, 0.14, 0.14), point(0.45 * side, 0.12, -0.14));
        both(&mut mb, a, c, b, Material::Wood);
        both(&mut mb, b, c, d, Material::Wood);
        both(&mut mb, d, c, point(0.95 * side, 0.08, -0.16), Material::Wood);
    }
    both(&mut mb, point(0.0, 0.02, -0.2), point(0.14, 0.02, -0.55), point(-0.14, 0.02, -0.55), Material::Wood);
    mb.build()
}

/// A tuft of grass combed east by the wind.
fn grass_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    for k in 0..8 {
        let a = k as f32 * 2.4;
        let d = 0.03 + 0.014 * k as f32;
        let h = 0.26 + 0.03 * ((k * 5) % 8) as f32;
        let (x, z) = (d * cos(a), d * sin(a));
        let shade = (185 + (k * 9) % 60) as u8;
        mb.tint(shade, 235, 150);
        let n = point(0.35, 0.4, 1.0);
        let (w, lean) = (0.014, 0.14 + 0.02 * k as f32);
        mb.ribbon(
            &[
                (point(x, 0.0, z), w, n),
                (point(x + lean * 0.25, h * 0.5, z), w * 0.85, n),
                (point(x + lean * 0.7, h * 0.85, z), w * 0.55, n),
                (point(x + lean * 1.25, h, z), w * 0.1, n),
            ],
            Material::Grass,
        );
    }
    mb.build()
}

fn flower_mesh(r: u8, g: u8, b: u8) -> MeshData {
    let mut mb = MeshBuilder::new();
    for k in 0..3 {
        let a = k as f32 * 2.2;
        let (x, z) = (0.1 * cos(a), 0.1 * sin(a));
        let h = 0.26 + 0.07 * k as f32;
        let top = point(x + 0.1, h, z);
        mb.tint(170, 225, 140);
        mb.sweep_tube(&[(point(x, 0.0, z), 0.006), (point(x + 0.03, h * 0.6, z), 0.005), (top, 0.004)], 4, false, Material::Leaf);
        mb.tint(r, g, b);
        for j in 0..6 {
            let (t0, t1) = (j as f32 * (TAU / 6.0) - 0.4, j as f32 * (TAU / 6.0) + 0.4);
            let (u, v) = (point(0.045 * cos(t0), 0.012, 0.045 * sin(t0)), point(0.045 * cos(t1), 0.012, 0.045 * sin(t1)));
            both(&mut mb, top, top + u, top + v, Material::Sand);
        }
        mb.tint(150, 95, 40);
        mb.ellipsoid(top + point(0.0, 0.005, 0.0), point(0.014, 0.012, 0.014), 6, 4, Material::Sand);
    }
    mb.build()
}

fn reed_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    for k in 0..6 {
        let a = k as f32 * 2.4;
        let (x, z) = (0.14 * cos(a), 0.14 * sin(a));
        let h = 0.9 + 0.1 * ((k * 3) % 6) as f32;
        mb.tint(185, 220, 130);
        let top = point(x + 0.22, h, z);
        mb.sweep_tube(&[(point(x, 0.0, z), 0.012), (point(x + 0.06, h * 0.6, z), 0.01), (top, 0.006)], 5, false, Material::Leaf);
        if k % 2 == 0 {
            mb.tint(140, 95, 60);
            mb.capsule(top - point(0.03, 0.14, 0.0), top, 0.024, 6, Material::Wood);
        }
        let n = point(0.2, 0.3, 1.0);
        mb.tint(170, 225, 130);
        mb.ribbon(
            &[(point(x, 0.0, z), 0.016, n), (point(x + 0.1, h * 0.45, z + 0.04), 0.014, n), (point(x + 0.35, h * 0.62, z + 0.08), 0.002, n)],
            Material::Leaf,
        );
    }
    mb.build()
}

/// A tree grown slanting in the wind, its crown streaming east.
fn tree_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(180, 160, 140);
    mb.sweep_tube(
        &[
            (point(0.0, -0.3, 0.0), 0.34),
            (point(0.05, 0.4, 0.0), 0.23),
            (point(0.25, 1.5, 0.05), 0.18),
            (point(0.75, 2.6, 0.0), 0.14),
            (point(1.6, 3.4, 0.0), 0.07),
        ],
        9,
        true,
        Material::Wood,
    );
    mb.sweep_tube(&[(point(0.35, 1.8, 0.0), 0.1), (point(0.8, 2.3, 0.6), 0.07), (point(1.5, 2.7, 0.9), 0.04)], 7, true, Material::Wood);
    mb.sweep_tube(&[(point(0.5, 2.1, 0.0), 0.09), (point(1.0, 2.6, -0.5), 0.06), (point(1.7, 2.9, -0.9), 0.04)], 7, true, Material::Wood);
    let crowns = [
        (point(0.9, 3.2, 0.0), point(0.9, 0.5, 0.8), 225u8),
        (point(1.8, 3.5, 0.1), point(1.1, 0.5, 0.8), 245),
        (point(1.7, 2.9, 0.9), point(0.9, 0.38, 0.6), 205),
        (point(1.9, 3.05, -0.9), point(0.95, 0.4, 0.6), 215),
        (point(2.8, 3.4, 0.0), point(0.8, 0.32, 0.55), 235),
    ];
    for (c, r, shade) in crowns {
        mb.tint(shade, shade, 180);
        mb.ellipsoid(c, r, 10, 7, Material::Leaf);
    }
    mb.build()
}

/// A mushroom a metre tall, to be scaled down: pale, glowing green-blue.
fn mushroom_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(215, 255, 240);
    mb.sweep_tube(&[(point(0.0, 0.0, 0.0), 0.1), (point(0.03, 0.4, 0.0), 0.075), (point(0.0, 0.8, 0.0), 0.06)], 8, false, Material::Sand);
    mb.tint(110, 255, 220);
    mb.ellipsoid(point(0.0, 0.8, 0.0), point(0.4, 0.2, 0.4), 12, 7, Material::Sand);
    mb.tint(240, 255, 250);
    for j in 0..5 {
        let a = j as f32 * (TAU / 5.0);
        mb.ellipsoid(point(0.24 * cos(a), 0.95, 0.24 * sin(a)), point(0.045, 0.03, 0.045), 6, 4, Material::Sand);
    }
    mb.build()
}

/// A cluster of crystals a metre tall, to be scaled down.
fn crystal_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(200, 170, 255);
    for &(x, z, h, r) in &[(0.0f32, 0.0f32, 1.0f32, 0.16f32), (0.2, 0.08, 0.65, 0.12), (-0.16, 0.12, 0.5, 0.11), (0.02, -0.2, 0.4, 0.1)] {
        mb.sweep_tube(
            &[(point(x, -0.05, z), r * 0.8), (point(x * 1.5, h * 0.7, z * 1.5), r), (point(x * 1.8, h, z * 1.8), r * 0.05)],
            6,
            true,
            Material::Sand,
        );
    }
    mb.build()
}

/// The mill's door frame and its open door, window sills, and the finial on its cap.
fn trim_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(150, 120, 95);
    for side in [-1.0f32, 1.0] {
        mb.cuboid(point(0.8 * side, 1.3, 3.8), point(0.1, 1.25, 0.12), Material::Wood);
    }
    mb.cuboid(point(0.0, 2.55, 3.8), point(1.0, 0.12, 0.16), Material::Wood);
    // The door, folded back against the footing.
    mb.tint(195, 120, 90);
    mb.cuboid(point(-1.5, 1.3, 3.9), point(0.62, 1.12, 0.05), Material::Wood);
    mb.tint(110, 85, 70);
    mb.cuboid(point(-1.5, 0.75, 3.96), point(0.6, 0.07, 0.03), Material::Wood);
    mb.cuboid(point(-1.5, 1.95, 3.96), point(0.6, 0.07, 0.03), Material::Wood);
    for side in [-1.0f32, 1.0] {
        mb.cuboid(point(3.42 * side, 1.42, 0.0), point(0.14, 0.05, 0.42), Material::Wood);
        mb.cuboid(point(3.4 * side, 2.1, 0.0), point(0.12, 0.05, 0.42), Material::Wood);
    }
    mb.tint(170, 150, 130);
    mb.sweep_tube(&[(point(0.0, 11.5, 0.0), 0.16), (point(0.0, 12.3, 0.0), 0.1), (point(0.0, 12.9, 0.0), 0.02)], 8, true, Material::Wood);
    mb.ellipsoid(point(0.0, 12.2, 0.0), point(0.22, 0.2, 0.22), 10, 7, Material::Wood);
    // A pennant streaming from it.
    mb.tint(255, 130, 95);
    both(&mut mb, point(0.0, 12.85, 0.0), point(0.0, 12.45, 0.0), point(1.3, 12.6, 0.15), Material::Sand);
    mb.build()
}

/// A sack of flour.
fn sack_mesh() -> MeshData {
    let mut mb = MeshBuilder::new();
    mb.tint(245, 232, 205);
    mb.ellipsoid(point(0.0, 0.3, 0.0), point(0.3, 0.34, 0.26), 12, 8, Material::Sand);
    mb.tint(225, 208, 175);
    mb.cone(point(0.0, 0.76, 0.0), -0.2, 0.13, 8, Material::Sand);
    mb.build()
}
