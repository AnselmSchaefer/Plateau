//! The laws (POC_PLAN.md, step 3): every area of `areas/` keeps them all, played natively (and as
//! its module, for the laws about the module); and a small yard made wrong on purpose, one way for
//! each law, breaks that law and no other, saying where and why.

mod played;

use plateau_engine::reach::{Reach, World};
use plateau_engine::{Description, Region, Size, laws};
use plateau_sdk::{Answer, Area, Material, Place, Point, View, cuboid, cut, point, union};
use played::Native;

const SEED: u64 = 7;
/// The searches' seed, and how many looks each makes: what `cargo test` runs every time.
const LOOKS: u32 = 1;
const RAYS: usize = 600;

fn module(name: &str) -> Vec<u8> {
    let path = format!("{}/../areas/modules/{name}.wasm", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}: run areas/build.sh"))
}

fn kept(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../areas/{name}/hashes.txt"))
}

/// The laws that need no module, over an area played natively: each law's verdict, by name.
fn verdicts(area: &mut dyn Region) -> Vec<(&'static str, laws::Verdict)> {
    let d = area.describe().unwrap();
    let sizes = laws::sizes_agree(area, &d, LOOKS, RAYS).unwrap();
    let long = laws::never_too_long(area, &d, LOOKS, RAYS).unwrap();
    let inside = laws::surfaces_have_an_inside(area, &d, LOOKS, RAYS).unwrap();
    let mut world = World::new(area, &d.places);
    let all: Reach = world.reach(&Size::ALL).unwrap();
    let gates = laws::gates_are_real(&mut world, &all, &d).unwrap();
    let traps = laws::no_traps(&world, &all).unwrap();
    vec![
        ("sizes agree", sizes),
        ("never too long", long),
        ("surfaces have an inside", inside),
        ("gates are real", gates),
        ("no traps", traps),
    ]
}

fn only_broken(verdicts: &[(&'static str, laws::Verdict)]) -> Vec<&'static str> {
    verdicts.iter().filter(|(_, v)| v.is_err()).map(|(n, _)| *n).collect()
}

#[test]
fn every_area_keeps_the_laws() {
    for (name, _) in areas_played::AREAS {
        for (law, verdict) in verdicts(&mut played::area(name, SEED)) {
            assert_eq!(verdict, Ok(()), "{name}: {law}");
        }
    }
}

/// The hashes are kept in `areas/<name>/hashes.txt`; `PLATEAU_KEEP=1 cargo test` writes them again,
/// for a change to the world made on purpose.
#[test]
fn every_area_s_module_is_the_same_world_as_kept() {
    for (name, _) in areas_played::AREAS {
        let mut wasm = plateau_engine::Wasm::new(&module(name), SEED, plateau_engine::Limits::default()).unwrap();
        let d = wasm.describe().unwrap();
        if std::env::var_os("PLATEAU_KEEP").is_some() {
            let hashes = laws::chunk_hashes(&mut wasm, &laws::kept_chunks(&d)).unwrap();
            let head = format!(
                "# The hashes of the answers of chunks of areas/modules/{name}.wasm, seed {SEED}: the law \"the same everywhere\"\n\
                 # (POC_PLAN.md, step 3). Each chunk's level, its place on the level's grid, its hash. Written by\n\
                 # `PLATEAU_KEEP=1 cargo test -p plateau-engine --test laws`, when the world is changed on purpose.\n"
            );
            std::fs::write(kept(name), head + &laws::hashes_text(&hashes)).unwrap();
        }
        let kept = std::fs::read_to_string(kept(name)).expect("hashes kept: PLATEAU_KEEP=1 cargo test");
        assert_eq!(laws::same_everywhere(&mut wasm, &kept).unwrap(), Ok(()), "{name}");
    }
}

#[test]
fn another_world_is_not_the_same_world() {
    let kept = std::fs::read_to_string(kept("garden")).unwrap();
    let verdict = laws::same_everywhere(&mut played::area("garden", SEED + 1), &kept).unwrap();
    let why = verdict.expect_err("another seed is another world");
    assert!(why.contains("not the same") && why.contains("not ") && why.contains("as kept"), "{why}");
}

// The yard: flat grass, a stone block with a room inside, and a slot through the block's west face
// into the room that only the tiny fit through. Made wrong in one way at a time.

const RIGHT: u8 = 0;
/// The slot as wide and tall as a door: a person walks in.
const WIDE: u8 = 1;
/// No slot: nobody gets in, the tiny neither.
const SEALED: u8 = 2;
/// A pit out on the grass, too deep to climb out of and too narrow to grow in.
const PIT: u8 = 3;
/// The block half a metre further east for whoever is smaller than half a metre.
const SLIDING: u8 = 4;
/// The ground's distance half as long again as it is.
const LONG: u8 = 5;
/// A hollow by the start cut out exactly down from the ground's surface: across its mouth the
/// distance is nought, a surface with nothing inside it.
const LID: u8 = 6;

struct Yard<const KIND: u8>;

impl<const KIND: u8> Area for Yard<KIND> {
    fn new(_: u64) -> Self {
        Yard
    }

    fn describe(&self) -> plateau_sdk::Description {
        plateau_sdk::Description {
            name: "The yard".into(),
            places: vec![Place::new("the room in the block", plateau_sdk::Size::Tiny, point(41.0, 0.0, 32.0))],
            borders: plateau_sdk::Borders::of_ground(|_, _| 0.0),
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let ground = if KIND == LONG { p.y * 1.5 } else { p.y };
        // Cut out from below the ground to above it, so the pit's mouth is open (`LID`).
        let ground = if KIND == PIT { cut(ground, cuboid(p - point(20.0, -3.5, 20.0), point(0.6, 4.5, 0.6))) } else { ground };
        let ground = if KIND == LID { cut(ground, cuboid(p - point(32.0, -0.15, 56.0), point(2.0, 0.15, 2.0))) } else { ground };
        let east = if KIND == SLIDING && view.size < 0.5 { 0.5 } else { 0.0 };
        let block = cuboid(p - point(41.0 + east, 1.5, 32.0), point(1.0, 1.5, 3.0));
        let room = cuboid(p - point(41.0 + east, 1.1, 32.0), point(0.4, 1.1, 0.8));
        let (wide, high) = match KIND {
            WIDE => (0.5, 2.0),
            SEALED => (0.0, 0.0),
            _ => (0.006, 0.03),
        };
        let slot = if wide > 0.0 { cuboid(p - point(40.3 + east, high * 0.5, 32.0), point(0.35, high * 0.5, wide)) } else { 1000.0 };
        let stone = cut(block, union(room, slot));
        let ground = cut(ground, union(room, slot));
        Answer::new(ground, Material::Grass).or(Answer::new(stone, Material::Stone))
    }
}

fn yard<const KIND: u8>() -> Native {
    Native(Box::new(Yard::<KIND>))
}

fn broken<const KIND: u8>() -> (Vec<&'static str>, Vec<(&'static str, laws::Verdict)>) {
    let v = verdicts(&mut yard::<KIND>());
    (only_broken(&v), v)
}

fn why<'a>(verdicts: &'a [(&'static str, laws::Verdict)], law: &str) -> &'a str {
    verdicts.iter().find(|(n, _)| *n == law).unwrap().1.as_ref().unwrap_err()
}

#[test]
fn the_yard_made_right_keeps_every_law() {
    let (broken, v) = broken::<RIGHT>();
    assert!(broken.is_empty(), "{v:#?}");
}

#[test]
fn a_slot_wide_enough_for_a_person_breaks_gates_are_real_saying_who_walks_in_and_how() {
    let (broken, v) = broken::<WIDE>();
    assert_eq!(broken, ["gates are real"], "{v:#?}");
    let why = why(&v, "gates are real");
    assert!(why.contains("\"the room in the block\"") && why.contains("is reached by a person who is never tiny"), "{why}");
    // The way: from the start, at last as a person, into the room.
    assert!(why.contains(": as a giant (32.") && why.contains("as a person (") && why.contains(", (40.9"), "{why}");
}

#[test]
fn a_room_nobody_gets_into_breaks_gates_are_real_saying_how_near_the_tiny_come() {
    let (broken, v) = broken::<SEALED>();
    assert_eq!(broken, ["gates are real"], "{v:#?}");
    let why = why(&v, "gates are real");
    assert!(why.contains("is not reached by them") && why.contains("the way in is shut"), "{why}");
}

#[test]
fn a_pit_nobody_climbs_out_of_breaks_no_traps_saying_where_it_is() {
    let (broken, v) = broken::<PIT>();
    assert_eq!(broken, ["no traps"], "{v:#?}");
    let why = why(&v, "no traps");
    assert!(why.contains("a person who comes to (") && why.contains(", -8.00, ") && why.contains("cannot get back"), "{why}");
}

#[test]
fn a_block_that_moves_as_the_viewer_shrinks_breaks_sizes_agree() {
    let (broken, v) = broken::<SLIDING>();
    assert!(broken.contains(&"sizes agree"), "{v:#?}");
    let why = why(&v, "sizes agree");
    assert!(why.contains("17.0 cm") && why.contains("54.0 cm"), "{why}");
}

#[test]
fn a_ground_whose_distance_is_too_long_breaks_never_too_long_saying_by_how_much() {
    let (broken, v) = broken::<LONG>();
    assert_eq!(broken, ["never too long"], "{v:#?}");
    let why = why(&v, "never too long");
    assert!(why.contains("1.50 times too long") || why.contains("1.49 times too long"), "{why}");
}

#[test]
fn a_hollow_cut_down_from_the_ground_s_very_surface_breaks_surfaces_have_an_inside() {
    let (broken, v) = broken::<LID>();
    assert_eq!(broken, ["surfaces have an inside"], "{v:#?}");
    let why = why(&v, "surfaces have an inside");
    assert!(why.contains("comes to nought") && why.contains(", 0.0"), "{why}");
}

/// For looking at what the laws take and find over the garden: `cargo test --release -p
/// plateau-engine --test laws -- --ignored --nocapture explore`.
#[test]
#[ignore]
fn explore() {
    let timed = |what: &str, started: std::time::Instant| eprintln!("{what}: {:.2} s", started.elapsed().as_secs_f32());
    let rays: usize = std::env::var("RAYS").ok().and_then(|r| r.parse().ok()).unwrap_or(RAYS);
    let mut garden = played::area("garden", SEED);
    let d: Description = garden.describe().unwrap();
    for seed in 1..=std::env::var("SEEDS").ok().and_then(|r| r.parse().ok()).unwrap_or(1) {
        let t = std::time::Instant::now();
        eprintln!("{:?}", laws::sizes_agree(&mut garden, &d, seed, rays).unwrap());
        timed("sizes agree", t);
        let t = std::time::Instant::now();
        eprintln!("{:?}", laws::never_too_long(&mut garden, &d, seed, rays).unwrap());
        timed("never too long", t);
        let t = std::time::Instant::now();
        eprintln!("{:?}", laws::surfaces_have_an_inside(&mut garden, &d, seed, rays).unwrap());
        timed("surfaces have an inside", t);
    }
    let mut world = World::new(&mut garden, &d.places);
    let t = std::time::Instant::now();
    let all = world.reach(&Size::ALL).unwrap();
    timed("reach", t);
    for s in Size::ALL {
        eprintln!("{}: {} columns, {} stands reached", s.name(), world.columns(s).count(), all.stands.iter().filter(|t| t.size == s).count());
    }
    let t = std::time::Instant::now();
    eprintln!("{:?}", laws::gates_are_real(&mut world, &all, &d).unwrap());
    eprintln!("{:?}", laws::no_traps(&world, &all).unwrap());
    timed("gates and traps", t);
}

