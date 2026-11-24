//! The heron's pond: a wide pond in a meadow, lily pads strung across it, a mossy rock in its
//! north-west corner, and a heron's nest on a dead snag to the north-east; the heron itself stands
//! in the shallows.
//!
//! - **The tiny** cross by the lily pads to a shelf of stone at the mossy rock's foot, where a
//!   crack under a mossy arch leads into a grotto with a still pool.
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
const NEST_H: f32 = 4.0;
const WILLOW: Point = point(52.5, 0.0, 45.0);
const HERON: Point = point(41.0, LEVEL, 26.4);

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
    (37.4, -0.75, 29.6, 1.3, 0.9, 1.0),
    (38.7, -0.8, 30.7, 0.7, 0.6, 0.7),
    (35.2, -0.7, 21.4, 0.9, 0.6, 1.1),
    (15.4, -0.6, 28.4, 0.8, 0.6, 0.7),
];

/// The meadow's trees: where, how turned, how large.
const TREES: [(f32, f32, f32, f32); 7] = [
    (8.0, 9.0, 0.4, 1.25),
    (34.0, 5.5, 1.9, 1.0),
    (59.0, 6.0, 2.7, 1.2),
    (59.5, 31.0, 0.9, 0.9),
    (6.0, 51.0, 3.5, 1.15),
    (15.0, 57.0, 5.1, 0.85),
    (4.6, 30.0, 4.2, 0.95),
];

/// Bushes in the meadow: where, and how large.
const BUSHES: [(f32, f32, f32); 13] = [
    (9.5, 47.0, 0.9),
    (10.6, 48.2, 0.7),
    (23.0, 8.0, 1.0),
    (24.4, 8.6, 0.7),
    (47.0, 58.0, 1.1),
    (48.6, 57.2, 0.8),
    (60.0, 45.0, 1.0),
    (58.5, 53.0, 0.9),
    (3.4, 41.0, 1.0),
    (19.5, 59.5, 0.8),
    (43.0, 7.6, 1.0),
    (44.4, 6.9, 0.7),
    (4.0, 19.0, 1.1),
];

struct Pad {
    x: f32,
    z: f32,
    r: f32,
    c: f32,
    s: f32,
    flower: bool,
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
    clump_spots: Vec<(Point, f32, u32)>,
    sticks: Vec<Stick>,
    /// The nest's great branches, which a giant sees too.
    boughs: Vec<Stick>,
    spokes: Vec<(f32, f32)>,
    ring: Vec<(f32, f32)>,
    nest_ground: f32,
    willow_ground: f32,
    dandelions: [(f32, f32, f32); 3],
    // Mutable runtime plugin state (`tick` / `interact`).
    raft_t: f32,
    raft_dir: f32,
    raft_moving: bool,
    willow_lit: bool,
    grotto_lit: bool,
}

const ID_REED_BASE: u32 = 1;
const ID_RAFT: u32 = 100;
const ID_DRAGONFLY: u32 = 101;
const ID_WILLOW_LANTERN: u32 = 200;
const ID_GROTTO_LANTERN: u32 = 201;

const LIGHT_WILLOW: u32 = 1;
const LIGHT_GROTTO: u32 = 2;

const MESH_REED_0: u32 = 0;
const MESH_RAFT: u32 = 3;
const MESH_DRAGONFLY: u32 = 4;

const RAFT_A: Point = point(33.2, LEVEL + 0.04, 41.6);
const RAFT_B: Point = point(36.8, LEVEL + 0.04, 30.8);

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
            clump_spots: clump_spots(seed),
            sticks: sticks(seed, 37, 44, (-0.6, 0.26), (0.65, 1.15), (0.075, 0.13)),
            boughs: sticks(seed, 41, 10, (-0.45, 0.1), (0.95, 1.3), (0.2, 0.26)),
            spokes,
            ring,
            nest_ground: 0.0,
            willow_ground: 0.0,
            dandelions: [(28.0, 0.0, 49.2), (35.8, 0.0, 50.6), (44.0, 0.0, 46.8)],
            raft_t: 0.0,
            raft_dir: 1.0,
            raft_moving: false,
            willow_lit: false,
            grotto_lit: true,
        };
        pond.nest_ground = pond.lie(NEST.x, NEST.z).height;
        pond.willow_ground = pond.lie(WILLOW.x, WILLOW.z).height;
        for i in 0..3 {
            let (dx, _, dz) = pond.dandelions[i];
            pond.dandelions[i].1 = pond.lie(dx, dz).height;
        }
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
        let solid = solid.or(self.bushes(p, view, &lie)).or(self.heron(p, view));
        let water = Answer::new(max(p.y - LEVEL, lie.raw - 3.0), Material::Water).cut(solid.distance);
        solid.or(water).or(self.lilies(p, view, lie.raw))
    }

    fn scene(&self) -> Scene {
        let mut s = SceneBuilder::new();

        // 0..3: Three organic curved reed-clump variations (`sweep_tube` stalks + `ribbon` leaf blades + cattails).
        for variant in 0..3u32 {
            let mesh_id = s.add_mesh(build_reed_clump(self.seed as u64, variant));
            debug_assert_eq!(mesh_id, MESH_REED_0 + variant);
        }
        // 3: Solid wooden pond raft (`solid: true`, players stand on and ride it!).
        let m_raft = s.add_mesh(build_raft());
        debug_assert_eq!(m_raft, MESH_RAFT);
        // 4: Dynamic dragonfly (`sweep_tube` body + 4 `ribbon` wings).
        let m_dragonfly = s.add_mesh(build_dragonfly());
        debug_assert_eq!(m_dragonfly, MESH_DRAGONFLY);

        // 5: Curved multi-petal water-lily lotus flower (`ribbon` petals + golden center).
        let m_lotus = s.add_mesh(build_lotus());
        // 6: Meadow grass tussock (`ellipsoid` mound + curved `ribbon` grass blades).
        let m_tussock = s.add_mesh(build_tussock());
        // 7: Meadow wildflower (`sweep_tube` curved stem + 5 `ribbon` petals).
        let m_flower = s.add_mesh(build_wildflower());
        // 8: Dandelion tuft (`sweep_tube` stem + fluffy `ellipsoid` seed head + basal `ribbon` leaves).
        let m_dandelion = s.add_mesh(build_dandelion());
        // 9: Hanging hollow-willow cage lantern (`lathe` cap + struts + warm core).
        let m_willow_lantern = s.add_mesh(build_willow_lantern());
        // 10: Tiny glowing grotto lantern-mushroom (`lathe` stem + bell cap).
        let m_grotto_lantern = s.add_mesh(build_grotto_lantern());
        // 11: Area-specific static details (heron legs/beak/crest, nest rim sticks, grotto stalagmites/mushrooms).
        let m_details = s.add_mesh(self.build_static_details());

        // Dynamic swaying reed clumps along the pond shore (`id = 1..=24`).
        for (i, &(pos, yaw, variant)) in self.clump_spots.iter().enumerate() {
            s.spawn(ID_REED_BASE + i as u32, MESH_REED_0 + variant, pos, Quat::from_y(yaw), point(1.0, 1.0, 1.0), false);
        }

        // Dynamic solid wooden raft moored off the south beach (`id = 100`, `solid = true`).
        s.spawn(ID_RAFT, MESH_RAFT, RAFT_A, Quat::from_y(-0.2), point(1.0, 1.0, 1.0), true);

        // Dynamic dragonfly patrolling over the water lilies (`id = 101`).
        s.spawn(ID_DRAGONFLY, MESH_DRAGONFLY, point(29.0, LEVEL + 0.55, 34.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), false);

        // Dynamic interactive lanterns: hollow willow + tiny grotto.
        let willow_lantern_pos = point(WILLOW.x - 0.1, self.willow_ground + 2.1, WILLOW.z - 0.1);
        s.spawn(ID_WILLOW_LANTERN, m_willow_lantern, willow_lantern_pos, Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        let grotto_lantern_pos = ROOM + point(0.06, 0.0, -0.05);
        s.spawn(ID_GROTTO_LANTERN, m_grotto_lantern, grotto_lantern_pos, Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        // Grotto lantern starts warmly lit so the tiny cavern glows right away.
        s.light(LIGHT_GROTTO, grotto_lantern_pos + point(0.0, 0.045, 0.0), [1.0, 0.85, 0.55], 32.0, 0.95);

        // Static instanced lotus flowers on flowering lily pads.
        for (i, k) in self.pads.iter().enumerate() {
            if !k.flower {
                continue;
            }
            let sc = 0.85 + 0.3 * ((hash(i as i32, 0, 0, self.seed ^ 0x51) & 0xffff) as f32 / 65535.0);
            s.place(m_lotus, point(k.x, LEVEL + 0.006, k.z), Quat::from_y(i as f32 * 1.9), point(sc, sc, sc), false);
        }

        // Static instanced meadow tussocks & curved wildflowers across the 64 m meadow.
        let mut tseeds = Seeds::of(self.seed as u64, 71);
        for iz in 0..29 {
            for ix in 0..29 {
                let cx = (ix as f32 + 0.5) * 2.2;
                let cz = (iz as f32 + 0.5) * 2.2;
                let (x, z) = (cx + tseeds.next(-0.7, 0.7), cz + tseeds.next(-0.7, 0.7));
                if x < 1.5 || x > 62.5 || z < 1.5 || z > 62.5 {
                    continue;
                }
                let lie = self.lie(x, z);
                let dw = (point(x, 0.0, z) - WILLOW).flat_length();
                let from_path = abs(x - (31.6 + 0.7 * sin(z * 0.45)));
                let south = between(z, 44.0, 46.0);
                let here = between(lie.s, 1.0, 4.0) * (1.0 - between(from_path, 3.2, 0.8) * south) * between(dw, 3.0, 6.0);
                let r_mound = tseeds.next(0.75, 1.35) * here;
                let has_mound = tseeds.next(0.0, 1.0) <= 0.6;
                let has_flower = tseeds.next(0.0, 1.0) < 0.5;
                let yaw = tseeds.next(0.0, TAU);
                if here < 0.25 || !has_mound {
                    continue;
                }
                s.place(m_tussock, point(x, lie.height, z), Quat::from_y(yaw), point(r_mound, r_mound, r_mound), false);
                if has_flower {
                    let fh = 0.8 + 0.45 * here;
                    s.place(m_flower, point(x, lie.height, z), Quat::from_y(yaw + 0.9), point(fh, fh, fh), false);
                }
            }
        }

        // Static meadow dandelions (interactive updraft spots!).
        for (i, &(dx, dy, dz)) in self.dandelions.iter().enumerate() {
            s.place(m_dandelion, point(dx, dy, dz), Quat::from_y(i as f32 * 2.1), point(1.0, 1.0, 1.0), false);
        }

        // Static world-space details (heron legs/beak/crest, nest twigs, grotto stalagmites & lathe mushrooms).
        s.place(m_details, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), false);

        s.build()
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Vec<Command> {
        let mut cmds = Vec::with_capacity(32);
        let t = player.time;

        // 1. Gentle breeze swaying all 24 shoreline reed clumps.
        for (i, &(pos, yaw, variant)) in self.clump_spots.iter().enumerate() {
            let phase = t * 1.45 + pos.x * 0.23 + pos.z * 0.19;
            let gust = 0.045 * sin(phase) + 0.02 * sin(phase * 2.3 + 0.7);
            let tilt = 0.025 * cos(phase * 0.85);
            let rot = Quat::from_euler(yaw, tilt, gust);
            cmds.push(Command::SetTransform(instance(
                ID_REED_BASE + i as u32,
                MESH_REED_0 + variant,
                pos,
                rot,
                point(1.0, 1.0, 1.0),
                false,
            )));
        }

        // 2. Dragonfly hovering and darting in a lazy figure-8 above the lily pads.
        let a = t * 0.55;
        let dx = 28.5 + 4.8 * sin(a);
        let dz = 33.5 + 3.2 * sin(a * 2.0);
        let dy = LEVEL + 0.58 + 0.14 * sin(t * 2.4);
        let vx = 4.8 * 0.55 * cos(a);
        let vz = 3.2 * 1.1 * cos(a * 2.0);
        let heading = atan2(vx, vz);
        let bank = 0.25 * cos(a * 2.0);
        cmds.push(Command::SetTransform(instance(
            ID_DRAGONFLY,
            MESH_DRAGONFLY,
            point(dx, dy, dz),
            Quat::from_euler(heading, 0.06 * sin(t * 5.0), bank),
            point(1.0, 1.0, 1.0),
            false,
        )));

        // 3. Solid wooden raft bobbing on the water (and gliding smoothly when triggered).
        if self.raft_moving {
            self.raft_t += self.raft_dir * dt * 0.14;
            if self.raft_t >= 1.0 {
                self.raft_t = 1.0;
                self.raft_dir = -1.0;
                self.raft_moving = false;
            } else if self.raft_t <= 0.0 {
                self.raft_t = 0.0;
                self.raft_dir = 1.0;
                self.raft_moving = false;
            }
        }
        let u = ease(self.raft_t);
        let raft_pos = point(
            mix(RAFT_A.x, RAFT_B.x, u),
            RAFT_A.y + 0.015 * sin(t * 1.8),
            mix(RAFT_A.z, RAFT_B.z, u),
        );
        let raft_rot = Quat::from_euler(-0.2 + 0.22 * u, 0.014 * sin(t * 1.5), 0.016 * cos(t * 1.2));
        cmds.push(Command::SetTransform(instance(
            ID_RAFT,
            MESH_RAFT,
            raft_pos,
            raft_rot,
            point(1.0, 1.0, 1.0),
            true,
        )));

        cmds
    }

    fn interact(&mut self, player: &PlayerState) -> Vec<Command> {
        let mut cmds = Vec::new();
        let reach = max(player.size * 3.0, 0.4);

        // 1. Hollow willow lantern toggle.
        let willow_pos = point(WILLOW.x - 0.1, self.willow_ground + 2.1, WILLOW.z - 0.1);
        if player.target == ID_WILLOW_LANTERN || (player.eye - willow_pos).length() < max(reach, 3.2) {
            self.willow_lit = !self.willow_lit;
            let intensity = if self.willow_lit { 900.0 } else { 0.0 };
            cmds.push(Command::SetLight(light(
                LIGHT_WILLOW,
                willow_pos - point(0.0, 0.12, 0.0),
                [1.0, 0.78, 0.42],
                intensity,
                8.0,
            )));
            cmds.push(Command::Say(if self.willow_lit {
                "the willow lantern glows warmly".into()
            } else {
                "you snuff the willow lantern".into()
            }));
            return cmds;
        }

        // 2. Tiny grotto glowing mushroom lantern toggle.
        let grotto_pos = ROOM + point(0.06, 0.045, -0.05);
        if player.target == ID_GROTTO_LANTERN || (player.eye - grotto_pos).length() < 0.45 {
            self.grotto_lit = !self.grotto_lit;
            let intensity = if self.grotto_lit { 32.0 } else { 0.0 };
            cmds.push(Command::SetLight(light(
                LIGHT_GROTTO,
                grotto_pos,
                [1.0, 0.85, 0.55],
                intensity,
                0.95,
            )));
            cmds.push(Command::Say(if self.grotto_lit {
                "the tiny grotto mushroom shines".into()
            } else {
                "the grotto settles into cool shadow".into()
            }));
            return cmds;
        }

        // 3. Wooden pond raft: push off or recall across the pond!
        let u = ease(self.raft_t);
        let raft_pos = point(mix(RAFT_A.x, RAFT_B.x, u), RAFT_A.y, mix(RAFT_A.z, RAFT_B.z, u));
        if player.target == ID_RAFT || (player.feet - raft_pos).flat_length() < max(reach, 3.2) {
            self.raft_moving = !self.raft_moving;
            cmds.push(Command::Say(if self.raft_moving {
                if self.raft_dir > 0.0 {
                    "the wooden raft drifts out toward the heron".into()
                } else {
                    "the wooden raft glides back to the beach".into()
                }
            } else {
                "you steady the wooden raft".into()
            }));
            return cmds;
        }

        // 4. Meadow dandelion breeze updraft!
        for &(dx, dy, dz) in &self.dandelions {
            let dp = point(dx, dy + 0.35, dz);
            if (player.feet - dp).flat_length() < max(reach, 2.0) {
                let lift = clamp(player.size * 3.8, 0.35, 16.0);
                let push = clamp(player.size * 1.4, 0.12, 6.0);
                cmds.push(Command::ApplyImpulse(point(player.look.x * push, lift, player.look.z * push)));
                cmds.push(Command::Say("a puff of dandelion breeze lifts you up!".into()));
                return cmds;
            }
        }

        cmds.push(Command::Say("nothing nearby to interact with (try the raft, lanterns, or meadow dandelions)".into()));
        cmds
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
    for _ in 0..66 {
        let a = seeds.next(0.0, TAU);
        let k = seeds.next(0.15, 1.0);
        let (x, z) = (29.0 + 10.0 * k * cos(a), 33.5 + 8.5 * k * sin(a));
        let r = seeds.next(0.22, 0.56);
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

/// Clump anchor spots along the shore, away from the rock and the beach.
fn clump_spots(seed: u64) -> Vec<(Point, f32, u32)> {
    let mut seeds = Seeds::of(seed, 23);
    let edges = [0usize, 0, 1, 1, 6, 7, 7, 8, 9, 9, 10, 10, 11, 14];
    let mut spots = Vec::with_capacity(24);
    for k in 0..24 {
        let e = edges[k % edges.len()];
        let ((ax, az), (bx, bz)) = (OUTLINE[e], OUTLINE[(e + 1) % OUTLINE.len()]);
        let t = seeds.next(0.12, 0.9);
        let (x, z) = (mix(ax, bx, t), mix(az, bz, t));
        let (dx, dz) = (POND.0 - x, POND.1 - z);
        let l = (dx * dx + dz * dz).sqrt();
        let inward = seeds.next(0.2, 1.1);
        let (x, z) = (x + dx / l * inward, z + dz / l * inward);
        let yaw = seeds.next(0.0, TAU);
        spots.push((point(x, LEVEL, z), yaw, (k % 3) as u32));
    }
    spots
}

/// Builds one organic reed clump: curved `sweep_tube` stalks, arching `ribbon` leaf blades, and
/// velvety cattail heads.
fn build_reed_clump(seed: u64, variant: u32) -> MeshData {
    let mut m = MeshBuilder::new();
    let mut s = Seeds::of(seed, 101 + variant as usize);
    for _ in 0..7 {
        let a = s.next(0.0, TAU);
        let rad = s.next(0.05, 0.55);
        let (bx, bz) = (rad * cos(a), rad * sin(a));
        let h = s.next(1.05, 1.85);
        let lean_a = s.next(0.0, TAU);
        let lean = s.next(0.12, 0.34);
        let (lx, lz) = (cos(lean_a) * lean, sin(lean_a) * lean);
        let knots = [
            (point(bx, -0.85, bz), 0.017),
            (point(bx + lx * 0.12, h * 0.25, bz + lz * 0.12), 0.015),
            (point(bx + lx * 0.45, h * 0.65, bz + lz * 0.45), 0.012),
            (point(bx + lx, h, bz + lz), 0.006),
        ];
        m.tint(210, 235, 195);
        m.sweep_tube(&knots, 6, true, Material::Leaf);
        if s.next(0.0, 1.0) < 0.52 {
            let p0 = point(bx + lx * 0.72, h * 0.80, bz + lz * 0.72);
            let p1 = point(bx + lx * 0.90, h * 0.94, bz + lz * 0.90);
            m.tint(150, 115, 85);
            m.capsule(p0, p1, 0.032, 7, Material::Wood);
        }
        // Two arching ribbon leaf blades rising from the stalk base.
        m.tint(225, 250, 200);
        for _ in 0..2 {
            let ba = s.next(0.0, TAU);
            let (bc, bs) = (cos(ba), sin(ba));
            let bh = s.next(0.65, 1.25);
            let reach = s.next(0.28, 0.55);
            let n_hint = point(bc, 0.35, bs);
            let blade = [
                (point(bx, -0.15, bz), 0.022, n_hint),
                (point(bx + bc * reach * 0.28, bh * 0.45, bz + bs * reach * 0.28), 0.024, n_hint),
                (point(bx + bc * reach * 0.72, bh * 0.85, bz + bs * reach * 0.72), 0.015, n_hint),
                (point(bx + bc * reach, bh * 0.72, bz + bs * reach), 0.002, n_hint),
            ];
            m.ribbon(&blade, Material::Grass);
        }
    }
    m.build()
}

/// Curved multi-petal water-lily lotus flower (`ribbon` petals + golden stamen cup).
fn build_lotus() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(255, 225, 115);
    m.ellipsoid(point(0.0, 0.045, 0.0), point(0.04, 0.038, 0.04), 8, 4, Material::Sand);
    for ring in 0..2 {
        let count = if ring == 0 { 8 } else { 6 };
        let offset = if ring == 0 { 0.0 } else { PI / 6.0 };
        let reach = if ring == 0 { 0.14 } else { 0.10 };
        let lift = if ring == 0 { 0.07 } else { 0.105 };
        if ring == 0 {
            m.tint(255, 238, 232);
        } else {
            m.tint(255, 218, 222);
        }
        for i in 0..count {
            let a = i as f32 * TAU / count as f32 + offset;
            let (c, s) = (cos(a), sin(a));
            let n = point(-c * 0.35, 1.0, -s * 0.35);
            let knots = [
                (point(c * 0.02, 0.01, s * 0.02), 0.018, n),
                (point(c * reach * 0.55, lift * 0.45, s * reach * 0.55), 0.032, n),
                (point(c * reach, lift, s * reach), 0.004, n),
            ];
            m.ribbon(&knots, Material::Sand);
        }
    }
    m.build()
}

/// Meadow grass tussock (`ellipsoid` mound + 9 arching `ribbon` grass blades).
fn build_tussock() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(225, 240, 210);
    m.ellipsoid(point(0.0, -0.10, 0.0), point(0.24, 0.19, 0.24), 8, 4, Material::Grass);
    m.tint(240, 255, 220);
    for i in 0..9 {
        let a = i as f32 * TAU / 9.0 + 0.17 * (i % 3) as f32;
        let (c, s) = (cos(a), sin(a));
        let h = 0.20 + 0.08 * ((i % 3) as f32);
        let r = 0.15 + 0.06 * ((i % 2) as f32);
        let n = point(c, 0.5, s);
        m.ribbon(
            &[
                (point(c * 0.05, 0.0, s * 0.05), 0.018, n),
                (point(c * r * 0.65, h * 0.72, s * r * 0.65), 0.014, n),
                (point(c * r, h * 0.88, s * r), 0.002, n),
            ],
            Material::Grass,
        );
    }
    m.build()
}

/// Meadow wildflower (`sweep_tube` curved stem + 5 curved golden petals).
fn build_wildflower() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(220, 245, 200);
    m.sweep_tube(
        &[
            (point(0.0, -0.05, 0.0), 0.010),
            (point(0.02, 0.14, 0.01), 0.008),
            (point(0.05, 0.29, 0.02), 0.006),
        ],
        5,
        false,
        Material::Grass,
    );
    let head = point(0.05, 0.30, 0.02);
    m.tint(245, 195, 75);
    m.ellipsoid(head, point(0.022, 0.018, 0.022), 6, 4, Material::Sand);
    m.tint(255, 232, 130);
    for i in 0..5 {
        let a = i as f32 * TAU / 5.0;
        let (c, s) = (cos(a), sin(a));
        let n = point(-c * 0.2, 1.0, -s * 0.2);
        m.ribbon(
            &[
                (head + point(c * 0.015, 0.0, s * 0.015), 0.010, n),
                (head + point(c * 0.045, 0.012, s * 0.045), 0.018, n),
                (head + point(c * 0.068, 0.006, s * 0.068), 0.004, n),
            ],
            Material::Sand,
        );
    }
    m.build()
}

/// Tall interactive meadow dandelion tuft (`sweep_tube` curved stems + fluffy white seed puffs).
fn build_dandelion() -> MeshData {
    let mut m = MeshBuilder::new();
    // Basal rosette of curved leaves.
    m.tint(200, 235, 185);
    for i in 0..6 {
        let a = i as f32 * TAU / 6.0;
        let (c, s) = (cos(a), sin(a));
        let n = point(c * 0.3, 1.0, s * 0.3);
        m.ribbon(
            &[
                (point(c * 0.03, 0.02, s * 0.03), 0.03, n),
                (point(c * 0.20, 0.09, s * 0.20), 0.045, n),
                (point(c * 0.34, 0.04, s * 0.34), 0.008, n),
            ],
            Material::Leaf,
        );
    }
    // Three tall stalks with bright fluffy seed heads so they stand out in the meadow.
    for &(ox, oz, h, puff) in &[(0.0, 0.0, 0.72, 0.11), (0.14, 0.09, 0.54, 0.085), (-0.12, 0.11, 0.60, 0.095)] {
        m.tint(215, 240, 190);
        m.sweep_tube(
            &[
                (point(ox * 0.3, -0.04, oz * 0.3), 0.016),
                (point(ox * 0.7, h * 0.55, oz * 0.7), 0.013),
                (point(ox, h, oz), 0.010),
            ],
            6,
            false,
            Material::Grass,
        );
        m.tint(255, 255, 250);
        m.ellipsoid(point(ox, h + puff * 0.65, oz), point(puff, puff, puff), 10, 6, Material::Sand);
    }
    m.build()
}

/// Solid wooden pond raft (`cuboid` weathered planks + cross beams + mooring post).
fn build_raft() -> MeshData {
    let mut m = MeshBuilder::new();
    // Two underside buoyancy logs.
    m.tint(190, 175, 155);
    m.cuboid(point(-0.68, -0.10, 0.0), point(0.14, 0.11, 1.25), Material::Wood);
    m.cuboid(point(0.68, -0.10, 0.0), point(0.14, 0.11, 1.25), Material::Wood);
    // Six sturdy deck planks (top surface at y = +0.08).
    for i in 0..6 {
        let z = -1.05 + i as f32 * 0.42;
        let shade = if i % 2 == 0 { 235 } else { 215 };
        m.tint(shade, shade - 10, shade - 25);
        m.cuboid(point(0.0, 0.03, z), point(0.95, 0.05, 0.18), Material::Wood);
    }
    // Small corner mooring post so you can spot the raft from across the meadow.
    m.tint(205, 185, 160);
    m.capsule(point(0.75, 0.08, -0.95), point(0.75, 0.68, -0.95), 0.06, 8, Material::Wood);
    m.tint(255, 235, 185);
    m.ellipsoid(point(0.75, 0.72, -0.95), point(0.07, 0.04, 0.07), 8, 4, Material::Sand);
    m.build()
}

/// Graceful dragonfly patrolling over the pond (`sweep_tube` body + 4 `ribbon` wings).
fn build_dragonfly() -> MeshData {
    let mut m = MeshBuilder::new();
    // Slender segmented body along +Z.
    m.tint(130, 210, 235);
    m.sweep_tube(
        &[
            (point(0.0, 0.0, -0.22), 0.008),
            (point(0.0, 0.005, -0.08), 0.012),
            (point(0.0, 0.012, 0.04), 0.022),
            (point(0.0, 0.010, 0.10), 0.020),
        ],
        6,
        true,
        Material::Water,
    );
    // Four iridescent wings (`ribbon`).
    m.tint(245, 252, 255);
    for &(z_off, span, tilt) in &[(0.045, 0.24, 0.12), (0.01, 0.21, -0.08)] {
        for sgn in [-1.0f32, 1.0] {
            let n = point(0.0, 1.0, 0.0);
            m.ribbon(
                &[
                    (point(sgn * 0.02, 0.02, z_off), 0.014, n),
                    (point(sgn * span * 0.6, 0.035, z_off + tilt * 0.5), 0.028, n),
                    (point(sgn * span, 0.025, z_off + tilt), 0.008, n),
                ],
                Material::Sand,
            );
        }
    }
    m.build()
}

/// Hanging lantern for the hollow willow (`lathe` roof + 4 wood struts + warm glowing core).
fn build_willow_lantern() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(175, 160, 145);
    m.lathe(
        point(0.0, 0.0, 0.0),
        &[(0.02, 0.26), (0.16, 0.14), (0.20, 0.10), (0.14, 0.09)],
        10,
        Material::Wood,
    );
    m.lathe(
        point(0.0, 0.0, 0.0),
        &[(0.14, -0.22), (0.17, -0.25), (0.02, -0.28)],
        10,
        Material::Wood,
    );
    for i in 0..4 {
        let a = i as f32 * TAU / 4.0;
        let p = point(0.12 * cos(a), 0.0, 0.12 * sin(a));
        m.cuboid(p + point(0.0, -0.06, 0.0), point(0.015, 0.16, 0.015), Material::Wood);
    }
    m.tint(255, 225, 145);
    m.ellipsoid(point(0.0, -0.06, 0.0), point(0.08, 0.11, 0.08), 8, 5, Material::Sand);
    m.build()
}

/// Tiny glowing mushroom lantern inside the grotto (`lathe` curved stem + bell cap).
fn build_grotto_lantern() -> MeshData {
    let mut m = MeshBuilder::new();
    m.tint(255, 240, 195);
    m.lathe(
        point(0.0, 0.0, 0.0),
        &[(0.008, -0.005), (0.005, 0.02), (0.004, 0.045)],
        8,
        Material::Sand,
    );
    m.tint(255, 215, 120);
    m.lathe(
        point(0.0, 0.042, 0.0),
        &[(0.026, -0.004), (0.028, 0.006), (0.018, 0.018), (0.001, 0.024)],
        10,
        Material::Sand,
    );
    m.build()
}

/// The nest's sticks, laid round its rim, about the middle of the snag's foot: `n` of them, their
/// heights about the nest's floor, half lengths and radii within what is given.
fn sticks(seed: u64, k: usize, n: u32, up: (f32, f32), half: (f32, f32), thick: (f32, f32)) -> Vec<Stick> {
    let mut seeds = Seeds::of(seed, k);
    let mut sticks = Vec::new();
    for i in 0..n {
        let a = i as f32 * TAU / n as f32 + seeds.next(-0.1, 0.1);
        let (c, s) = (cos(a), sin(a));
        let r = seeds.next(2.4, 2.95);
        let at = point(r * c, NEST_H + seeds.next(up.0, up.1), r * s);
        let lean = seeds.next(-0.6, 0.6);
        let (lc, ls) = (cos(lean), sin(lean));
        let way = point(-s * lc + c * ls, seeds.next(-0.14, 0.14), c * lc + s * ls);
        let half = seeds.next(half.0, half.1);
        sticks.push(Stick { a: at - way * half, b: at + way * half, r: seeds.next(thick.0, thick.1) });
    }
    sticks
}

/// A ball stretched to reach `r` along each axis: a distance never too long.
fn ellipsoid(p: Point, r: Point) -> f32 {
    let q = point(p.x / r.x, p.y / r.y, p.z / r.z);
    max((q.length() - 1.0) * min(r.x, min(r.y, r.z)), cuboid(p, r))
}

/// Only where both are, the edge between them rounded.
fn smooth_both(a: f32, b: f32, k: f32) -> f32 {
    -smooth_union(-a, -b, k)
}

impl Pond {
    /// Area-specific world-space static procedural details (heron legs/beak/crest, nest rim
    /// sticks, and grotto stalagmites/stalactites/columns/lathe mushrooms).
    fn build_static_details(&self) -> MeshData {
        let mut m = MeshBuilder::new();
        // 1. Heron's thin legs, S-curved neck/head, crest, and sharp beak (crisp at 40 m).
        let (cy, sy) = (cos(-2.498), sin(-2.498));
        let heron_world = |lp: Point| point(HERON.x + lp.x * cy - lp.z * sy, HERON.y + lp.y, HERON.z + lp.x * sy + lp.z * cy);
        m.tint(210, 185, 145);
        for sgn in [-1.0f32, 1.0] {
            m.sweep_tube(
                &[
                    (heron_world(point(-0.1, -1.5, sgn * 0.28)), 0.068),
                    (heron_world(point(0.05, 0.9, sgn * 0.27)), 0.072),
                    (heron_world(point(-0.1, 2.0, sgn * 0.25)), 0.088),
                ],
                8,
                true,
                Material::Wood,
            );
        }
        m.tint(218, 224, 232);
        m.sweep_tube(
            &[
                (heron_world(point(0.65, 2.52, 0.0)), 0.22),
                (heron_world(point(1.22, 3.25, 0.0)), 0.16),
                (heron_world(point(0.96, 4.00, 0.0)), 0.145),
                (heron_world(point(1.20, 4.50, 0.0)), 0.175),
                (heron_world(point(1.50, 4.48, 0.0)), 0.16),
            ],
            8,
            true,
            Material::Stone,
        );
        m.tint(185, 195, 205);
        m.sweep_tube(
            &[
                (heron_world(point(1.2, 4.66, 0.0)), 0.068),
                (heron_world(point(0.8, 4.58, 0.0)), 0.048),
                (heron_world(point(0.42, 4.48, 0.0)), 0.015),
            ],
            6,
            true,
            Material::Soil,
        );
        m.tint(240, 205, 115);
        m.sweep_tube(
            &[
                (heron_world(point(1.48, 4.46, 0.0)), 0.082),
                (heron_world(point(2.00, 4.38, 0.0)), 0.042),
                (heron_world(point(2.48, 4.28, 0.0)), 0.008),
            ],
            8,
            true,
            Material::Wood,
        );

        // 2. Heron's nest rim sticks.
        m.clear_tint();
        let nest_base = point(NEST.x, self.nest_ground, NEST.z);
        for k in &self.sticks {
            m.capsule(nest_base + k.a, nest_base + k.b, k.r, 6, Material::Wood);
        }

        // 3. Grotto stalagmites, stalactites, columns, and smooth `lathe` mushrooms.
        for &(x, z, h, r) in &[
            (0.34, -0.22, 0.12, 0.014),
            (0.42, -0.04, 0.16, 0.018),
            (0.37, 0.06, 0.07, 0.008),
            (-0.37, -0.22, 0.1, 0.012),
            (-0.2, -0.27, 0.14, 0.016),
            (-0.02, -0.23, 0.06, 0.007),
            (0.27, -0.28, 0.07, 0.008),
        ] {
            m.cone(ROOM + point(x, 0.0, z), h, r, 8, Material::Stone);
            m.cone(ROOM + point(x - 0.01, 0.35, z), -(0.03 + h * 0.7), r, 8, Material::Stone);
        }
        for &(x, z, h, r) in &[(0.1, 0.14, 0.17, 0.02), (0.03, -0.1, 0.11, 0.013), (-0.22, 0.12, 0.2, 0.02), (-0.12, 0.2, 0.09, 0.01), (0.2, 0.0, 0.1, 0.012)] {
            m.cone(ROOM + point(x, 0.35, z), -h, r, 8, Material::Stone);
        }
        for &(x, z, r) in &[(0.16, -0.19, 0.026), (-0.34, -0.05, 0.02)] {
            m.capsule(ROOM + point(x, 0.0, z), ROOM + point(x, 0.35, z), r, 8, Material::Stone);
        }
        for &(x, z, h) in &[(0.21, 0.1, 0.05), (0.26, 0.13, 0.032), (0.17, 0.15, 0.026), (-0.43, -0.15, 0.045), (-0.1, -0.25, 0.04)] {
            let at = ROOM + point(x, 0.0, z);
            m.tint(240, 232, 210);
            m.lathe(at, &[(0.004 + h * 0.04, -0.008), (0.003 + h * 0.025, h * 0.88)], 8, Material::Sand);
            m.tint(205, 145, 115);
            m.lathe(
                at + point(0.0, h * 0.82, 0.0),
                &[(h * 0.42, 0.0), (h * 0.44, h * 0.08), (h * 0.28, h * 0.20), (0.001, h * 0.24)],
                10,
                Material::Wood,
            );
        }
        m.build()
    }

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

        // The muddy margin, wide in places, in others grass to the water's edge.
        let beach = between(abs(p.x - 31.5), 7.0, 4.0) * between(p.z, 42.0, 44.5);
        let bays = 0.7 * noise(point(p.x / 4.0, 0.5, p.z / 4.0), self.seed ^ 0x3a) + 0.3 * noise(point(p.x / 1.3, 0.5, p.z / 1.3), self.seed ^ 0x3c);
        let wide = 0.75 + bays + 1.6 * beach;
        let from_path = abs(p.x - (31.6 + 0.7 * sin(p.z * 0.45)));
        let south = between(p.z, 44.0, 46.0);
        let worn = between(from_path + 0.3 * bays, 1.5, 0.8) * south;
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

        // (Tussocks & meadow flowers are now emitted as crisp GPU props in `props()`!)
        // Pebbles where there is no turf, half sunk: boulders to the tiny.
        let pebbly = view.sees(0.015) * (1.0 - turf);
        if pebbly > 0.0 && abs(above) < 0.03 {
            let pebbles = scatter(p, 0.07, self.seed ^ 0x77, |q, s| {
                let k = s.next(0.0, 1.0);
                let r = (0.003 + 0.011 * k * k) * pebbly;
                let (a, b) = (s.next(0.8, 1.3), s.next(0.8, 1.3));
                ellipsoid(point(q.x, above + r * 0.3, q.z), point(r * a + 0.0001, r * 0.8 + 0.0001, r * b + 0.0001))
            });
            ground = ground.or(Answer::new(pebbles * 0.65, Material::Stone));
        }
        // Fallen leaves and twigs on bare earth.
        let litter = view.sees(0.006) * (1.0 - turf) * between(lie.s, wide, wide + 0.6);
        if litter > 0.0 && abs(above) < 0.03 {
            let sunk = above + 0.008 * (1.0 - litter);
            let leaves = scatter(p, 0.21, self.seed ^ 0x79, |q, s| {
                let q = point(q.x, sunk, q.z).turned(s.next(0.0, TAU));
                let k = s.next(0.6, 1.0);
                ellipsoid(q, point(0.034 * k, 0.003, 0.016 * k))
            });
            let twigs = scatter(p, 0.27, self.seed ^ 0x7b, |q, s| {
                let a = point(s.next(-0.06, 0.06), 0.0, s.next(0.02, 0.06));
                capsule(point(q.x, sunk - 0.001, q.z), a, point(-a.x, 0.0, -a.z), 0.003)
            });
            ground = ground.or(Answer::new(leaves * 0.65, Material::Leaf)).or(Answer::new(twigs * 0.65, Material::Wood));
        }
        // Blades of grass, a forest to the tiny, sunk out of sight where the turf ends; a few
        // islands of them on the path.
        let blady = view.sees(0.005);
        if blady > 0.0 && above < 0.16 && above > -0.02 {
            let island = 0.7 * between(noise(point(p.x / 1.5, 0.5, p.z / 1.5), self.seed ^ 0x7d), 0.1, 0.7) * between(lie.s, wide, wide + 0.6);
            let blady = blady * max(clamp(turf * 2.0 - 1.0, 0.0, 1.0), island);
            if blady > 0.0 {
                let blades = scatter(p, 0.05, self.seed ^ 0x83, |q, s| {
                    let top = -0.01 + s.next(0.025, 0.053) * blady;
                    let lean = point(s.next(-0.012, 0.012) * blady, top, s.next(-0.012, 0.012) * blady);
                    capsule(point(q.x, above, q.z), point(0.0, -0.03, 0.0), lean, 0.0026)
                });
                ground = ground.or(Answer::new(blades * 0.6, Material::Grass));
            }
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
        let green = noise(p / 0.5, self.seed ^ 0x93) + 0.4 * noise(p / 0.2, self.seed ^ 0x95);
        let material = if p.y > LEVEL + 0.2 && green > 0.25 { Material::Moss } else { Material::Stone };
        Answer::new((d + rough) * 0.65, material)
    }

    /// The bushes in the meadow, each standing on the ground.
    fn bushes(&self, p: Point, view: &View, lie: &Lie) -> Answer {
        let above = p.y - lie.height;
        if above > 5.0 {
            return Answer::new((above - 1.8) * 0.75, Material::Leaf);
        }
        let mut d = 1000.0;
        for &(x, z, r) in BUSHES.iter() {
            let q = point(p.x - x, above, p.z - z);
            let body = sphere(q - point(0.0, 0.2 * r, 0.0), r);
            if body - r < d {
                let one = sphere(q - point(0.6 * r, 0.1 * r, 0.5 * r), 0.7 * r);
                let two = sphere(q - point(-0.7 * r, 0.0, 0.2 * r), 0.65 * r);
                d = min(d, smooth_union(smooth_union(body, one, 0.2), two, 0.2));
            }
        }
        if d > 0.6 {
            return Answer::new(d * 0.75 - 0.1, Material::Leaf);
        }
        Answer::new((d + detail(p, view, 0.6, 0.1, self.seed ^ 0x99)) * 0.55, Material::Leaf)
    }

    /// The mossy rock, and the stone shelf at its foot where the pads lead.
    fn rock(&self, p: Point, view: &View) -> Answer {
        let bound = cuboid(p - point(19.6, -0.4, 20.6), point(5.8, 3.5, 5.7));
        if bound > 3.0 {
            return Answer::new(bound, Material::Stone);
        }
        let c = point(19.3, -0.4, 19.7);
        let main = ellipsoid(p - c, point(4.8, 2.9, 3.0));
        let west = ellipsoid(p - point(17.2, -0.4, 20.6), point(2.6, 1.9, 2.6));
        let east = ellipsoid(p - point(21.8, -0.4, 18.8), point(2.9, 2.2, 2.6));
        let mut crag = smooth_union(smooth_union(main, west, 0.6), east, 0.6);
        // Broken faces: its east and west shoulders, its top, its back.
        let q = p - c;
        for &(x, y, z, d) in [(0.75, 0.6, 0.27, 3.4), (-0.7, 0.64, 0.3, 3.25), (0.15, 0.94, -0.27, 2.4), (0.0, 0.5, -0.86, 2.45), (-0.36, 0.9, 0.2, 2.4)].iter() {
            crag = smooth_both(crag, q.dot(point(x, y, z)) - d, 0.25);
        }
        let from_mouth = (p - MOUTH).flat_length();

        let crag = if crag > 0.5 {
            Answer::new(crag - 0.25, Material::Stone)
        } else {
            let rough = detail(p, view, 1.6, 0.05, self.seed ^ 0xa1);
            let strata = 0.02 * view.sees(0.5) * sin(p.y * 9.0 + 2.0 * noise(p / 2.5, self.seed ^ 0xa3));
            let mut d = (crag + rough + strata) * 0.6;
            // Two clefts across it, well away from the arch.
            let deep = 0.18 * view.sees(0.3);
            if deep > 0.0 {
                let one = abs((p - point(17.6, 0.0, 20.0)).dot(point(0.91, 0.12, 0.38)));
                let two = abs((p - point(22.6, 0.0, 19.0)).dot(point(0.89, -0.2, -0.39)));
                for &w in [one, two].iter() {
                    d = max(d, min((0.13 - w) * 0.6, d + deep * 0.6));
                }
            }
            let arch = between((p - point(MOUTH.x, FLOOR + 0.2, MOUTH.z)).length(), 0.95, 0.6);
            let ragged = noise(p / 1.3, self.seed ^ 0xa5) + 0.5 * noise(p / 0.4, self.seed ^ 0xab) + 0.2 * noise(p / 0.12, self.seed ^ 0xad);
            let patch = between(ragged + 0.2 * (p.y - 1.2), 0.2, 0.4) * between(p.y, LEVEL + 0.1, LEVEL + 0.3);
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
        let q = p - ROOM;
        // The floor: sand in faint ripples, dipping gently under the pool.
        let from_pool = point(q.x + 0.2, 0.0, q.z - 0.09).flat_length();
        let dip = 0.012 * between(from_pool, 0.15, 0.07);
        let ripple = 0.0007 * sin(p.x * 150.0 + 3.0 * noise(p / 0.12, self.seed ^ 0xb7));
        let floor = (0.25 * rough + ripple - dip - above) * 0.8;
        // The walls rise sheer from the floor, smooth at their foot and rough only above the
        // tiny's jump, so nobody climbs into a pocket of them; the roof is rounded.
        let wall_rough = rough * between(above, 0.015, 0.065);
        let bay = |x: f32, z: f32, r: f32| cylinder(q - point(x, 0.0, z), r - 0.13, 0.21) - 0.13;
        let vault = smooth_union(smooth_union(bay(-0.1, 0.0, 0.36), bay(0.26, -0.08, 0.3), 0.12), bay(-0.28, -0.16, 0.24), 0.12);
        let room = max((vault + wall_rough) * 0.6, floor);
        // A jagged crack, a little over two centimetres wide at its foot, narrowing upwards.
        let wiggle = 0.012 * noise(point(p.z / 0.06, p.y / 0.06, 0.5), self.seed ^ 0xb3);
        let up = clamp(above / 0.19, 0.0, 1.0);
        let slot = (abs(p.x - MOUTH.x - wiggle) - 0.011 * (1.0 - 0.6 * up)) / 1.5;
        let crack = max(max(slot, floor), max(above - 0.19, abs(p.z - 22.4) - 0.5));
        let hollow = min(room, crack);
        let mut inner = out.cut(hollow);
        if hollow < 0.02 && p.z < MOUTH.z - 0.2 {
            let green = noise(p / 0.12, self.seed ^ 0xb5) + 0.4 * noise(p / 0.03, self.seed ^ 0xb9);
            inner.material = if above < 0.008 {
                if green > 0.45 { Material::Moss } else { Material::Sand }
            } else if green > 0.2 {
                Material::Moss
            } else {
                Material::Stone
            };
        }
        if room < 0.02 {
            // (Stalagmites, stalactites, columns, and mushrooms are emitted as crisp GPU props!)
            let pool = max(above + 0.002, from_pool - 0.135);
            inner = inner.or(Answer::new(pool, Material::Water).cut(inner.distance));
        }
        Answer::new(mix(out.distance, inner.distance, seen), inner.material)
    }

    /// The heron's nest: a great bowl of sticks in the crook of a dead snag.
    fn nest(&self, p: Point, view: &View) -> Answer {
        let q = p - point(NEST.x, self.nest_ground, NEST.z);
        let bound = cuboid(q - point(0.0, 3.2, 0.0), point(4.6, 4.6, 4.6));
        if bound > 3.0 {
            return Answer::new(bound, Material::Wood);
        }
        let mut wood = capsule(q, point(0.0, -1.0, 0.0), point(0.0, 2.5, 0.0), 0.9);
        for &(c, s) in self.spokes.iter() {
            let root = capsule(q, point(c * 0.5, 0.45, s * 0.5), point(c * 2.2, -0.4, s * 2.2), 0.3);
            let limb = capsule(q, point(-c * 0.3, 1.9, -s * 0.3), point(-c * 2.6, 3.4, -s * 2.6), 0.34);
            wood = smooth_union(smooth_union(wood, root, 0.35), limb, 0.35);
        }
        // Two of its limbs go on up beside the nest, broken off.
        let (c, s) = self.spokes[2];
        wood = smooth_union(wood, capsule(q, point(-c * 2.4, 3.2, -s * 2.4), point(-c * 4.0, 6.9, -s * 4.0), 0.34), 0.3);
        wood = smooth_union(wood, capsule(q, point(-c * 3.5, 5.7, -s * 3.5), point(-c * 4.0 + s * 1.0, 6.4, -s * 4.0 - c * 1.0), 0.24), 0.2);
        let (c, s) = self.spokes[3];
        wood = smooth_union(wood, capsule(q, point(-c * 2.4, 3.2, -s * 2.4), point(-c * 4.0, 5.4, -s * 4.0), 0.32), 0.3);
        let wood = if wood > 0.5 {
            wood - 0.12
        } else {
            let furrows = 0.04 * view.sees(0.35) * abs(noise(point(q.x / 0.3, q.y / 2.0, q.z / 0.3), self.seed ^ 0xc5));
            (wood + furrows + detail(point(q.x, q.y * 0.2, q.z), view, 0.35, 0.06, self.seed ^ 0xc1)) * 0.7
        };
        // A bowl, round beneath, its rim uneven, its floor smooth for the giant.
        let brim = cylinder(q - point(0.0, NEST_H - 1.1, 0.0), 2.75, 1.1) - 0.3;
        let body = smooth_both(brim, sphere(q - point(0.0, NEST_H + 1.9, 0.0), 4.0), 0.3);
        let body = cut(body, sphere(q - point(0.0, NEST_H + 8.0, 0.0), 8.0));
        let body = if body > 0.5 {
            body - 0.2
        } else {
            let uneven = 0.1 * between(q.flat_length(), 1.2, 2.4) * noise(p / 0.8, self.seed ^ 0xc7);
            (body + uneven + detail(q, view, 0.4, 0.06, self.seed ^ 0xc3)) * 0.65
        };
        let lined = q.flat_length() < 2.1 && q.y > NEST_H - 0.3 && noise(p / 0.7, self.seed ^ 0xc9) + 0.5 * noise(p / 0.25, self.seed ^ 0xcb) > 0.1;
        let mut nest = Answer::new(wood, Material::Wood).or(Answer::new(body, if lined { Material::Moss } else { Material::Wood }));
        let mut d = 1000.0;
        for k in self.boughs.iter() {
            d = min(d, capsule(q, k.a, k.b, k.r));
        }
        nest = nest.or(Answer::new(d, Material::Wood));
        // Three eggs.
        let seen = view.sees(0.7);
        if seen > 0.0 {
            let mut d = 1000.0;
            for &(x, z, a) in [(1.0, 0.45, 0.3), (0.55, 1.15, 1.6), (1.35, 1.2, 2.5)].iter() {
                let e = (q - point(x, NEST_H + 0.34, z)).turned(a);
                d = min(d, ellipsoid(e, point(0.45, 0.35, 0.35) * seen));
            }
            nest = nest.or(Answer::new(d, Material::Stone));
        }
        nest
    }

    /// The heron, still as stone in the shallows below its nest, looking over the pond.
    fn heron(&self, p: Point, view: &View) -> Answer {
        let q = p - HERON;
        let bound = sphere(q - point(0.0, 1.9, 0.0), 3.9);
        if bound > 2.0 {
            return Answer::new(bound, Material::Stone);
        }
        let q = q.turned(2.498);
        let m = point(q.x, q.y, abs(q.z));
        let neck = max(0.16, view.finest * 0.15);
        let tilted = |x: f32, y: f32| {
            let (a, b) = (m.x - x, m.y - y);
            point(a * 0.95 + b * 0.3, b * 0.95 - a * 0.3, m.z)
        };
        let mut body = ellipsoid(tilted(-0.2, 2.3), point(1.25, 0.62, 0.56));
        body = smooth_union(body, capsule(m, point(-1.1, 2.0, 0.0), point(-1.9, 1.65, 0.0), max(0.18, neck)), 0.35);
        body = smooth_union(body, capsule(m, point(0.7, 2.55, 0.0), point(1.25, 3.25, 0.0), neck * 1.25), 0.4);
        body = smooth_union(body, capsule(m, point(1.25, 3.25, 0.0), point(0.95, 4.0, 0.0), neck), 0.15);
        body = smooth_union(body, capsule(m, point(0.95, 4.0, 0.0), point(1.2, 4.5, 0.0), neck), 0.15);
        body = smooth_union(body, capsule(m, point(1.15, 4.5, 0.0), point(1.5, 4.48, 0.0), neck * 1.25), 0.15);
        let wing = ellipsoid(tilted(-0.45, 2.36) - point(0.0, 0.0, 0.36), point(1.15, 0.4, 0.3));
        let feathers = if min(body, wing) < 0.3 { detail(q, view, 0.2, 0.05, self.seed ^ 0xf1) } else { -0.02 };
        Answer::new((body + feathers) * 0.75, Material::Stone)
            .or(Answer::new((wing + feathers) * 0.75, Material::Soil))
    }

    /// The old willow, its trunk hollow, with a door a person walks in by, a window over the water
    /// and a bench at the back.
    fn willow(&self, p: Point, view: &View) -> Answer {
        let q = p - point(WILLOW.x, self.willow_ground, WILLOW.z);
        let bound = cuboid(q - point(0.0, 6.0, 0.0), point(6.2, 8.0, 6.2));
        if bound > 4.0 {
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
        // The door looks north-west, over the water; the window south-west.
        let along = -0.8 * q.x - 0.6 * q.z;
        let across = 0.6 * q.x - 0.8 * q.z;
        let inside = cylinder(q - point(0.0, 1.1, 0.0), 1.1, 1.75) - 0.25;
        let door = rounded_cuboid(point(along - 2.4, q.y - 0.6, across), point(1.6, 1.55, 0.65), 0.35);
        let window = capsule(point(along, q.y - 1.6, across), point(0.0, 0.0, -0.9), point(0.0, 0.0, -3.2), 0.36);
        let hollow = smooth_union(smooth_union(inside, door, 0.15), window, 0.1);
        let wood = if wood > 0.6 {
            wood - 0.25
        } else {
            let furrows = 0.07 * view.sees(0.35) * abs(noise(point(q.x / 0.4, q.y / 3.0, q.z / 0.4), self.seed ^ 0xd7));
            let rough = furrows + detail(point(q.x, q.y * 0.15, q.z), view, 0.5, 0.05, self.seed ^ 0xd1);
            cut((wood + rough) * 0.65, (hollow - furrows * 0.5) * 0.65)
        };
        let bench = max(cylinder(q + point(0.0, 0.1, 0.0), 1.7, 0.5), along + 0.62);
        let wood = min(wood, bench);
        let green = noise(p / 0.8, self.seed ^ 0xd5) + 0.5 * noise(p / 0.25, self.seed ^ 0xd9) - 0.25 * q.y;
        let trunk = Answer::new(wood, if q.y < 2.0 && green > 0.1 && along > -0.5 { Material::Moss } else { Material::Wood });
        // Shelves of fungus, either side of the door.
        let mut shelves = 1000.0;
        for &(x, y, z, r) in [(-0.49, 1.5, -2.1, 0.45), (-0.3, 2.05, -2.15, 0.34), (-2.16, 1.8, 0.12, 0.42), (-2.1, 2.3, 0.4, 0.3)].iter() {
            shelves = min(shelves, ellipsoid(q - point(x, y, z), point(r, 0.14, r)));
        }

        let mut leaf = sphere(q - point(0.0, 10.0, 0.0), 3.2);
        for &(x, y, z, r) in [(3.0, 8.8, 0.8, 2.4), (-2.6, 9.0, 1.8, 2.5), (0.6, 8.9, -3.0, 2.4), (-1.9, 9.2, -1.8, 2.3), (1.2, 9.0, 2.8, 2.3)].iter() {
            leaf = smooth_union(leaf, sphere(q - point(x, y, z), r), 0.5);
        }
        // Its curtains of leaves, hanging to different heights.
        for (i, &(c, s)) in self.ring.iter().enumerate() {
            let low = 4.9 + 0.8 * sin(i as f32 * 2.3);
            leaf = smooth_union(leaf, capsule(q, point(c * 3.9, 8.4, s * 3.9), point(c * 4.4, low, s * 4.4), 0.85), 0.4);
            let (c, s) = (c * 0.9 - s * 0.43, s * 0.9 + c * 0.43);
            leaf = smooth_union(leaf, capsule(q, point(c * 3.7, 8.4, s * 3.7), point(c * 4.2, low + 1.0, s * 4.2), 0.7), 0.4);
        }
        let leaf = self.foliage(leaf, p, view);
        trunk.or(Answer::new(shelves, Material::Sand)).or(Answer::new(leaf, Material::Leaf))
    }

    /// Leaves in clumps, over a smooth shape: great clumps a giant sees, small ones for the rest.
    fn foliage(&self, leaf: f32, p: Point, view: &View) -> f32 {
        if leaf > 1.4 { leaf - 0.65 } else { (leaf + detail(p, view, 3.6, 0.08, self.seed ^ 0xd3)) * 0.53 }
    }

    /// The trees along the meadow's edges.
    fn trees(&self, p: Point, view: &View) -> Answer {
        let mut best = Answer::new(1000.0, Material::Leaf);
        for &(x, z, a, k) in TREES.iter() {
            let q = (p - point(x, 0.0, z)) / k;
            let bound = cuboid(q - point(0.0, 4.6, 0.0), point(4.2, 6.2, 4.2)) * k;
            if bound >= best.distance {
                continue;
            }
            if bound > 4.0 {
                best = Answer::new(bound, Material::Leaf);
                continue;
            }
            let (c, s) = (cos(a), sin(a));
            let mut wood = capsule(q, point(0.0, -1.0, 0.0), point(0.1 * c, 6.0, 0.1 * s), 0.38);
            wood = smooth_union(wood, sphere(q - point(0.0, -0.2, 0.0), 0.75), 0.6);
            wood = smooth_union(wood, capsule(q, point(0.0, 3.2, 0.0), point(1.5 * c, 5.4, 1.5 * s), 0.22), 0.3);
            wood = smooth_union(wood, capsule(q, point(0.0, 3.7, 0.0), point(-1.4 * s, 5.6, 1.4 * c), 0.2), 0.3);
            let wood = wood * k;
            let wood = if wood > 0.3 {
                wood - 0.05
            } else {
                (wood + detail(point(p.x, p.y * 0.2, p.z), view, 0.2, 0.06, self.seed ^ 0xe1)) * 0.75
            };
            let blob = |f: f32, t: f32, y: f32, r: f32| sphere(q - point(f * c - t * s, y, f * s + t * c), r);
            let low = smooth_union(smooth_union(blob(1.7, 0.1, 6.0, 1.45), blob(-1.4, -0.9, 6.3, 1.5), 0.4), smooth_union(blob(0.1, 1.6, 6.1, 1.4), blob(0.3, -1.6, 6.6, 1.3), 0.4), 0.4);
            let leaf = smooth_union(smooth_union(blob(0.0, 0.0, 7.4, 1.9), blob(0.5, 0.4, 8.6, 1.2), 0.4), low, 0.4) * k;
            best = best.or(Answer::new(wood, Material::Wood)).or(Answer::new(self.foliage(leaf, p, view), Material::Leaf));
        }
        best
    }

    /// A fallen trunk, its head in the water.
    fn log(&self, p: Point, view: &View) -> Answer {
        let (a, b) = (point(37.6, LEVEL + 0.4, 48.6), point(34.2, LEVEL + 0.02, 43.8));
        let log = capsule(p, a, b, 0.38);
        let stub = capsule(p, point(35.6, LEVEL + 0.2, 45.8), point(35.1, LEVEL + 0.9, 46.4), 0.15);
        let d = smooth_union(log, stub, 0.15);
        if d > 0.3 {
            return Answer::new(d - 0.06, Material::Wood);
        }
        let d = (d + detail(p, view, 0.2, 0.05, self.seed ^ 0xe5)) * 0.75;
        let moss = noise(p / 0.4, self.seed ^ 0xe7) + 0.4 * noise(p / 0.15, self.seed ^ 0xe9) > 0.25 && p.y > LEVEL + 0.3;
        Answer::new(d, if moss { Material::Moss } else { Material::Wood })
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
        }
        best
    }
}
