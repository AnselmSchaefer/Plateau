//! Walking the walled garden at three sizes (POC_PLAN.md, step 2): a body as tall as its size,
//! colliding with the garden asked at that size. Each size finds its own garden: a person comes in
//! at the gate and is stopped by the wall; a giant steps over it and sits on the bench, whose seat a
//! person cannot reach; the tiny go in at the crack to the cave, which a person cannot; the tiny
//! stand on the pond while a person wades to its floor; a giant sinks into the mud.

mod played;

use played::Native;
use plateau_engine::body::{Body, Intent, sinks};
use plateau_engine::{Material, Region, START, SIZES};

const SEED: u64 = 7;
const TINY: f32 = SIZES[0].1;
const PERSON: f32 = SIZES[1].1;
const GIANT: f32 = SIZES[2].1;
/// Steps of the engine's physics, a second.
const HZ: f32 = 120.0;

/// Yaw turning a body to face east (+x), south (+z), west, north.
const EAST: f32 = -std::f32::consts::FRAC_PI_2;
const NORTH: f32 = 0.0;

fn garden() -> Native {
    played::area("garden", SEED)
}

/// A body standing on the ground under `at`, dropped from `size` above where the garden's ground
/// is about (y = 0), so under the wall, not on it.
fn place(garden: &mut Native, at: [f64; 2], size: f32, yaw: f32) -> Body {
    let mut body = Body::standing(garden, [at[0], 0.1 + size as f64 * 0.5, at[1]], size).unwrap();
    body.yaw = yaw;
    body
}

/// Walks `body` for `seconds`, as `intent` asks.
fn walk(garden: &mut Native, body: &mut Body, intent: Intent, seconds: f32) {
    for _ in 0..(seconds * HZ) as usize {
        body.step(garden, intent, 1.0 / HZ).unwrap();
    }
}

/// Walks `body` towards each point in turn (across the ground), turning to face it, until within
/// `near` metres of the last, or `seconds` have gone by. Whether it got there.
fn walk_to(garden: &mut Native, body: &mut Body, points: &[[f64; 2]], near: f64, seconds: f32) -> bool {
    let mut next = 0;
    for _ in 0..(seconds * HZ) as usize {
        let [x, z] = points[next];
        let (dx, dz) = (x - body.feet[0], z - body.feet[2]);
        let far = (dx * dx + dz * dz).sqrt();
        if far < near {
            if next + 1 == points.len() {
                return true;
            }
            next += 1;
            continue;
        }
        body.yaw = (-dx).atan2(-dz) as f32;
        body.step(garden, Intent { forward: 1.0, ..Intent::default() }, 1.0 / HZ).unwrap();
    }
    false
}

const FORWARD: Intent = Intent { forward: 1.0, right: 0.0, run: false, jump: false };
const RUNNING: Intent = Intent { forward: 1.0, right: 0.0, run: true, jump: false };

#[test]
fn a_person_comes_in_at_the_start_on_the_grass() {
    let mut garden = garden();
    let body = place(&mut garden, START, PERSON, NORTH);
    assert_eq!(body.on, Some(Material::Grass));
    assert!(body.feet[1].abs() < 1.5, "the start's ground is at {}", body.feet[1]);
}

#[test]
fn a_person_walks_in_at_the_gate_and_is_stopped_by_the_wall() {
    let mut garden = garden();
    // From the start, straight north through the gate (the wall's south side is at z = 54).
    let mut body = place(&mut garden, START, PERSON, NORTH);
    walk(&mut garden, &mut body, RUNNING, 4.0);
    assert!(body.feet[2] < 48.0, "a person got only to z = {} through the gate", body.feet[2]);
    assert_eq!(body.on, Some(Material::Grass));
    // A little to the west, the wall.
    let mut body = place(&mut garden, [20.0, 60.0], PERSON, NORTH);
    walk(&mut garden, &mut body, RUNNING, 4.0);
    assert!(body.feet[2] > 54.3 + body.radius() as f64 * 0.9, "a person went into the wall, to z = {}", body.feet[2]);
    assert!((body.feet[0] - 20.0).abs() < 0.5, "a person slid along the wall to x = {}", body.feet[0]);
}

#[test]
fn a_giant_steps_over_the_wall() {
    let mut garden = garden();
    let mut body = place(&mut garden, [20.0, 62.0], GIANT, NORTH);
    walk(&mut garden, &mut body, FORWARD, 2.0);
    assert!(body.feet[2] < 50.0, "a giant got only to z = {}", body.feet[2]);
    assert!(body.feet[1] < 1.0, "a giant is on top of something, at {}", body.feet[1]);
}

/// The cave behind the crack, and the ground's height there.
fn cave(garden: &mut Native) -> [f64; 3] {
    let place = garden.describe().unwrap().places.into_iter().find(|p| p.name.contains("cave")).unwrap();
    place.at.map(|c| c as f64)
}

#[test]
fn the_tiny_go_in_at_the_crack_to_the_cave_and_a_person_cannot() {
    let mut garden = garden();
    let cave = cave(&mut garden);
    // At the foot of the east wall, by the crack (z = 30, the wall's inner face at x = 53.7).
    let mut tiny = place(&mut garden, [53.6, 30.0], TINY, EAST);
    // Into the crack, along it (it wiggles a few millimetres about z = 30), and into the room.
    let way = [[53.66, 30.0], [53.95, 30.0], [cave[0], cave[2]]];
    let there = walk_to(&mut garden, &mut tiny, &way, 0.01, 40.0);
    assert!(there, "the tiny got only to {:?}, the cave is at {cave:?}", tiny.feet);
    assert!((tiny.feet[1] - cave[1]).abs() < 0.01, "the tiny are at {} in the cave, its floor at {}", tiny.feet[1], cave[1]);
    // A person walking at the crack stays outside the wall.
    let mut person = place(&mut garden, [52.5, 30.0], PERSON, EAST);
    walk(&mut garden, &mut person, FORWARD, 4.0);
    assert!(person.feet[0] < 53.7 - person.radius() as f64 * 0.9, "a person went into the wall, to x = {}", person.feet[0]);
}

#[test]
fn the_tiny_in_the_cave_cannot_grow_and_a_person_under_the_bench_cannot_grow_to_a_giant() {
    let mut garden = garden();
    let cave = cave(&mut garden);
    let mut tiny = place(&mut garden, [cave[0], cave[2]], TINY, EAST);
    tiny.grow_to(PERSON);
    walk(&mut garden, &mut tiny, Intent::default(), 1.5);
    // The cave is 24 cm high: they grow until their head is at its roof.
    assert!(tiny.blocked && tiny.size < 0.27, "the tiny grew to {} m in the cave", tiny.size);
    // Under the bench's seat (its underside 3.6 m up).
    let mut person = place(&mut garden, [40.0, 17.5], PERSON, NORTH);
    person.grow_to(GIANT);
    walk(&mut garden, &mut person, Intent::default(), 1.5);
    assert!(person.blocked && person.size < 4.5, "a person grew to {} m under the bench", person.size);
    // In the open, they grow.
    let mut person = place(&mut garden, START, PERSON, NORTH);
    person.grow_to(GIANT);
    walk(&mut garden, &mut person, Intent::default(), 1.5);
    assert!(!person.blocked && person.size == GIANT, "a person in the open grew to {} m", person.size);
}

#[test]
fn a_giant_sits_on_the_bench_and_a_person_cannot() {
    let mut garden = garden();
    // In front of the bench (its seat 4.5 m up, from z = 16 to 20, the backrest to the north).
    let mut giant = place(&mut garden, [40.0, 23.5], GIANT, NORTH);
    assert!(giant.sit(&mut garden).unwrap(), "a giant found no seat at the bench");
    let seat = giant.seat.unwrap();
    assert!((seat.at[1] - 4.5).abs() < 0.3, "a giant sat at {}", seat.at[1]);
    assert!((15.9..20.1).contains(&seat.at[2]), "a giant sat at z = {}", seat.at[2]);
    // Sitting, the giant's eyes are above the seat; walking on, it stands where it was.
    assert!(giant.eye()[1] > 4.5 + 8.0);
    walk(&mut garden, &mut giant, FORWARD, 0.1);
    assert!(giant.seat.is_none() && giant.feet[2] > 22.0);
    let mut person = place(&mut garden, [40.0, 21.0], PERSON, NORTH);
    assert!(!person.sit(&mut garden).unwrap(), "a person sat at {:?}", person.seat);
}

/// A point on the pond with no lily pad on it: where the tiny land on water.
fn open_water(garden: &mut Native) -> Body {
    for k in 0..40 {
        let at = [22.0 + (k % 7) as f64 * 0.37, 36.0 + (k / 7) as f64 * 0.41];
        let tiny = place(garden, at, TINY, NORTH);
        if tiny.on == Some(Material::Water) {
            return tiny;
        }
    }
    panic!("nowhere on the pond is open water");
}

#[test]
fn the_tiny_stand_on_the_pond_and_a_person_wades_to_its_floor() {
    let mut garden = garden();
    let mut tiny = open_water(&mut garden);
    let at = [tiny.feet[0], tiny.feet[2]];
    assert!((tiny.feet[1] + 0.3).abs() < 0.001, "the tiny stand at {} on water at -0.3", tiny.feet[1]);
    // And stay on it, walking.
    walk(&mut garden, &mut tiny, FORWARD, 1.0);
    assert!((tiny.feet[1] + 0.3).abs() < 0.001 && tiny.on.is_some(), "the tiny went to {} walking on water", tiny.feet[1]);
    let mut person = place(&mut garden, at, PERSON, NORTH);
    assert_eq!(person.on, Some(Material::Mud));
    assert!(person.feet[1] < -0.8, "a person stands at {} in the pond", person.feet[1]);
    // Walking out of it, north, to the shore and the grass.
    walk(&mut garden, &mut person, RUNNING, 4.0);
    assert!(person.feet[1] > -0.3 && person.on == Some(Material::Grass), "a person walking out of the pond is at {:?} on {:?}", person.feet, person.on);
}

#[test]
fn a_giant_sinks_into_the_mud() {
    let mut garden = garden();
    let at = {
        let tiny = open_water(&mut garden);
        [tiny.feet[0], tiny.feet[2]]
    };
    let person = place(&mut garden, at, PERSON, NORTH);
    let giant = place(&mut garden, at, GIANT, NORTH);
    assert_eq!(giant.on, Some(Material::Mud));
    let deeper = person.feet[1] - giant.feet[1];
    let expected = (sinks(Material::Mud, GIANT) - sinks(Material::Mud, PERSON)) as f64;
    assert!(expected > 0.9 && (deeper - expected).abs() < 0.1, "a giant stands {deeper} m deeper in the mud than a person, not {expected}");
    // On the lawn, a giant sinks a little; on stone (the wall's top), not at all.
    assert!(sinks(Material::Grass, GIANT) < 0.3 && sinks(Material::Stone, GIANT) == 0.0 && sinks(Material::Mud, TINY) == 0.0);
}

#[test]
fn a_body_jumps_its_proportion_and_lands() {
    let mut garden = garden();
    for size in [TINY, PERSON, GIANT] {
        let mut body = place(&mut garden, [30.0, 30.0], size, NORTH);
        let ground = body.feet[1];
        body.step(&mut garden, Intent { jump: true, ..Intent::default() }, 1.0 / HZ).unwrap();
        let mut highest = ground;
        for _ in 0..(2.0 * HZ) as usize {
            body.step(&mut garden, Intent::default(), 1.0 / HZ).unwrap();
            highest = highest.max(body.feet[1]);
        }
        let jumped = (highest - ground) / size as f64;
        assert!((0.3..0.4).contains(&jumped), "a body {size} m tall jumped {jumped} of its height");
        assert!(body.on.is_some() && (body.feet[1] - ground).abs() < 0.02 * size as f64, "a body {size} m tall landed at {}", body.feet[1]);
    }
}
