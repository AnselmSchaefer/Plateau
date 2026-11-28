//! Tests for Step 8a: scale-stamped SDF sculpting & digging (`edit`), exact chronological spatial
//! indexing, targeted chunk invalidation, save/load determinism ("made again equals played"),
//! multi-scale walking inside dug caves & onto raised hills, and SDF distance laws.

mod played;

use std::sync::Arc;

use plateau_engine::body::{Body, Intent};
use plateau_engine::chunk::{self, Key, Making};
use plateau_engine::edit::{Edit, EditLog, Edited, Op, Shape, WorldSave, surface_normal};
use plateau_engine::{Answer, Material, Region, SIZES, hash, laws, points, points_near, rings};

fn garden() -> played::Native {
    played::area("garden", 7)
}

/// Chronological BVH pruning must produce answers bit-for-bit identical to naive sequential evaluation.
#[test]
fn bvh_matches_bruteforce_bit_for_bit() {
    let mut base = garden();
    let mut log = EditLog::new();
    let mut state = 0x1234_5678u32;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    };

    // Mix 60 edits across tiny, person, and giant scales, all 3 shapes, sharp & smooth blends.
    let sizes = [SIZES[0].1, SIZES[1].1, SIZES[2].1];
    for i in 0..60 {
        let stamp_size = sizes[i % 3];
        let cx = 20.0 + next() * 24.0;
        let cy = -0.5 + next() * 4.0;
        let cz = 20.0 + next() * 24.0;
        let r = (0.3 + next() * 0.9) * stamp_size;
        let shape = match i % 3 {
            0 => Shape::Sphere { radius: r },
            1 => Shape::Box { half: [r, r * 0.7, r * 1.2], round: if i % 2 == 0 { 0.0 } else { r * 0.15 } },
            _ => Shape::Cylinder { radius: r, half_height: r * 0.6, round: r * 0.1 },
        };
        let blend = if i % 4 == 0 { 0.0 } else { r * 0.3 };
        let edit = if i % 2 == 0 {
            Edit::add(shape, [cx, cy, cz], blend, Material::ALL[i % Material::ALL.len()], stamp_size)
        } else {
            let mat = if i % 6 == 1 { Some(Material::Stone) } else { None };
            Edit::carve(shape, [cx, cy, cz], blend, mat, stamp_size)
        };
        log.push(edit).unwrap();
    }

    let pts = points(4000, 12.0, 99);
    let mut brute = Vec::new();
    base.sample(&pts, SIZES[1].1, &mut brute).unwrap();
    for (&p, ans) in pts.iter().zip(brute.iter_mut()) {
        for e in log.edits() {
            *ans = e.apply(p, *ans);
        }
    }

    let mut edited = Edited::new(garden(), Arc::new(log));
    let mut accel = Vec::new();
    edited.sample(&pts, SIZES[1].1, &mut accel).unwrap();

    assert_eq!(brute.len(), accel.len());
    for (k, (b, a)) in brute.iter().zip(&accel).enumerate() {
        assert_eq!(
            (b.distance.to_bits(), b.material),
            (a.distance.to_bits(), a.material),
            "mismatch at point {k} {:?}: brute={b:?}, accel={a:?}",
            pts[k]
        );
    }
}

/// Raising a hill at person scale, digging a tiny 2 cm cave into it, and building a tiny stone
/// pillar inside it: coarse pass agrees with full grid, body walks onto the hill and into the cave,
/// and a person cannot fit/grow inside the tiny cave.
#[test]
fn raise_hill_and_dig_tiny_cave_and_walk() {
    let tiny = SIZES[0].1;
    let person = SIZES[1].1;
    let mut log = EditLog::new();

    // 1. Raise a 2.4 m tall grassy/soil hill on the open lawn at (32, 0, 50).
    let hill_center = [32.0, 0.0, 50.0];
    log.push(Edit::add(Shape::Sphere { radius: 2.5 }, hill_center, 0.8, Material::Grass, person)).unwrap();

    // 2. Dig a tiny cave (~4 cm tunnel + 5 cm chamber) into the south flank of the hill at tiny scale.
    // Hill surface on south flank at y = 0.25 is roughly z = 50 + sqrt(2.5^2 - 0.25^2) ≈ 52.49.
    let mut hill_only = Edited::new(garden(), Arc::new(log.clone()));
    let mut probe = Vec::new();
    // Find exact surface z along south flank at x=32, y=0.25.
    let mut surface_z = 52.5f32;
    for _ in 0..20 {
        hill_only.sample(&[[32.0, 0.25, surface_z]], tiny, &mut probe).unwrap();
        surface_z -= probe[0].distance;
    }
    assert!(surface_z > 52.0 && surface_z < 53.2, "hill flank at {surface_z}");

    // Carve an entrance tunnel going north (-z) from the hill flank into a tiny chamber.
    for step in 0..8 {
        let z = surface_z + 0.015 - step as f32 * 0.015;
        log.push(Edit::carve(Shape::Sphere { radius: 0.022 }, [32.0, 0.25, z], 0.004, Some(Material::Soil), tiny)).unwrap();
    }
    let chamber = [32.0, 0.25, surface_z - 0.12];
    log.push(Edit::carve(Shape::Cylinder { radius: 0.045, half_height: 0.03, round: 0.005 }, chamber, 0.005, Some(Material::Stone), tiny)).unwrap();

    // Place a tiny stone block (altar/stool) inside the carved cave.
    log.push(Edit::add(
        Shape::Box { half: [0.008, 0.006, 0.008], round: 0.001 },
        [chamber[0] + 0.02, chamber[1] - 0.024, chamber[2]],
        0.0,
        Material::Stone,
        tiny,
    ))
    .unwrap();

    let mut edited = Edited::new(garden(), Arc::new(log));

    // A person stands on top of the raised hill (~2.5 m above the lawn).
    let on_hill = Body::standing(&mut edited, [32.0, 10.0, 50.0], person).unwrap();
    assert!(on_hill.feet[1] > 2.2 && on_hill.feet[1] < 2.8, "person should stand atop hill, feet.y = {}", on_hill.feet[1]);

    // A tiny person stands inside the carved cave chamber and walks a few steps without falling out.
    let mut in_cave = Body::standing(&mut edited, [chamber[0] as f64, (chamber[1] + 0.01) as f64, chamber[2] as f64], tiny).unwrap();
    assert!(in_cave.on.is_some(), "tiny should stand on cave floor");
    assert!(
        (in_cave.feet[1] - (chamber[1] - 0.03) as f64).abs() < 0.015,
        "tiny feet should rest on cave floor, got {}",
        in_cave.feet[1]
    );

    // Growing to person size inside the tiny cave must be blocked by the cave roof!
    in_cave.grow_to(person);
    for _ in 0..60 {
        in_cave.step(&mut edited, Intent::default(), 1.0 / 60.0).unwrap();
    }
    assert!(in_cave.blocked || in_cave.size < 0.1, "tiny must not grow to person inside tiny cave, size={}", in_cave.size);

    // Coarse chunk pass must produce the exact same surface as full grid at the tiny cave's floor & altar!
    let level = rings::finest(tiny);
    let side = 2f64.powi(level) * chunk::CELLS as f64;
    let floor_pt = [chamber[0] as f64, (chamber[1] - 0.03) as f64, chamber[2] as f64];
    let key = Key::new(level, [0, 1, 2].map(|a| (floor_pt[a] / side).floor() as i32));
    let coarse = chunk::make_with(&mut edited, key, Making { coarse: true, skirts: false, shading: false }).unwrap();
    let full = chunk::make_with(&mut edited, key, Making { coarse: false, skirts: false, shading: false }).unwrap();
    assert!(coarse.mesh.is_some(), "tiny cave floor chunk must produce visible geometry");
    assert_eq!(coarse.mesh, full.mesh, "coarse pass must match full grid inside carved cave");

    // Surface normal on top of the hill points roughly +Y.
    let n = surface_normal(&mut edited, [32.0, on_hill.feet[1], 50.0], person).unwrap();
    assert!(n[1] > 0.9, "hilltop normal should point up, got {n:?}");
}

/// Made again equals played: saving and reloading a `WorldSave` produces bit-exact identical
/// answers and chunk meshes at tiny, person, and giant scales.
#[test]
fn made_again_equals_played_save_and_reload() {
    let [tiny, person, giant] = SIZES.map(|(_, h)| h);
    let mut log = EditLog::new();
    log.push(Edit::add(Shape::Sphere { radius: 3.1234567 }, [32.0, 0.2, 48.0], 0.75, Material::Soil, person)).unwrap();
    log.push(Edit::carve(Shape::Box { half: [0.03, 0.025, 0.05], round: 0.004 }, [32.0, 0.5, 51.0], 0.005, None, tiny)).unwrap();
    log.push(Edit::add(Shape::Cylinder { radius: 5.5, half_height: 4.0, round: 0.8 }, [18.0, 1.5, 44.0], 1.2, Material::Stone, giant)).unwrap();
    log.push(Edit::carve(Shape::Cylinder { radius: 1.5, half_height: 2.0, round: 0.2 }, [18.0, 2.0, 44.0], 0.3, Some(Material::Moss), person)).unwrap();

    let save = WorldSave::new("garden", 7, log.edits().to_vec());
    let text = save.to_text();
    let parsed = WorldSave::from_text(&text).unwrap();
    assert_eq!(save, parsed, "WorldSave text round-trip must be exact");

    // Also test disk round-trip.
    let path = std::env::temp_dir().join(format!("plateau_edit_test_{}.plateau", std::process::id()));
    save.save(&path).unwrap();
    let from_disk = WorldSave::load(&path).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(save, from_disk, "WorldSave disk round-trip must be exact");

    let mut played_area = Edited::new(garden(), Arc::new(log));
    let mut reloaded_area = Edited::new(garden(), Arc::new(EditLog::from_edits(from_disk.edits).unwrap()));

    for &size in &[tiny, person, giant] {
        let pts = points_near([32.0, 0.5, 51.0], size * 2.0, 2000, 42);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        played_area.sample(&pts, size, &mut a).unwrap();
        reloaded_area.sample(&pts, size, &mut b).unwrap();
        assert_eq!(hash(&a), hash(&b), "sample hash must match after reload at size {size}");

        let level = rings::finest(size);
        let side = 2f64.powi(level) * chunk::CELLS as f64;
        let key = Key::new(level, [32.0, 0.5, 51.0].map(|c| (c / side).floor() as i32));
        let m1 = chunk::make(&mut played_area, key).unwrap().mesh;
        let m2 = chunk::make(&mut reloaded_area, key).unwrap().mesh;
        assert_eq!(m1, m2, "chunk mesh must match identically after reload at size {size}");
    }
}

/// Targeted chunk invalidation: every chunk whose mesh changes is flagged by `affects_chunk`,
/// while distant chunks are not invalidated.
#[test]
fn targeted_chunk_invalidation_is_sound_and_local() {
    let person = SIZES[1].1;
    let edit = Edit::add(Shape::Sphere { radius: 0.9 }, [32.0, 0.3, 50.0], 0.25, Material::Stone, person);
    let mut log = EditLog::new();
    log.push(edit).unwrap();

    let mut before = garden();
    let mut after = Edited::new(garden(), Arc::new(log));

    let wanted = rings::wanted([32.0, 1.8, 52.0], person);
    let mut affected_count = 0usize;
    let mut unaffected_count = 0usize;
    for key in wanted {
        let affects = edit.affects_chunk(key);
        if affects {
            affected_count += 1;
        } else {
            unaffected_count += 1;
            // Any chunk NOT flagged by `affects_chunk` must produce the exact same mesh!
            let m0 = chunk::make(&mut before, key).unwrap().mesh;
            let m1 = chunk::make(&mut after, key).unwrap().mesh;
            assert_eq!(m0, m1, "chunk {key:?} changed despite affects_chunk == false!");
        }
    }
    assert!(affected_count > 0, "edit should affect at least one nearby chunk");
    assert!(unaffected_count > 0, "edit should spare distant chunks");
}

/// Edited worlds preserve Plateau's distance laws (`never_too_long`, `has_inside`).
#[test]
fn edited_area_keeps_distance_laws() {
    let [tiny, person, giant] = SIZES.map(|(_, h)| h);
    let mut log = EditLog::new();
    // Raise a hill, carve a cave into it, and build a stone block beside it.
    log.push(Edit::add(Shape::Sphere { radius: 2.5 }, [32.0, 0.0, 48.0], 0.6, Material::Grass, person)).unwrap();
    log.push(Edit::carve(Shape::Sphere { radius: 1.2 }, [32.0, 0.8, 49.8], 0.2, Some(Material::Stone), person)).unwrap();
    log.push(Edit::add(Shape::Box { half: [0.6, 0.8, 0.6], round: 0.05 }, [28.0, 0.7, 50.0], 0.0, Material::Wood, person)).unwrap();
    log.push(Edit::add(Shape::Cylinder { radius: 4.0, half_height: 2.5, round: 0.5 }, [16.0, 0.5, 42.0], 0.8, Material::Soil, giant)).unwrap();
    log.push(Edit::carve(Shape::Sphere { radius: 0.03 }, [32.0, 2.5, 48.0], 0.005, None, tiny)).unwrap();

    let mut edited = Edited::new(garden(), Arc::new(log));
    let description = edited.describe().unwrap();
    let too_long = laws::never_too_long(&mut edited, &description, 13, 400).unwrap();
    assert!(too_long.is_ok(), "edited area broke never_too_long: {:?}", too_long);
    let no_inside = laws::surfaces_have_an_inside(&mut edited, &description, 13, 400).unwrap();
    assert!(no_inside.is_ok(), "edited area broke surfaces_have_an_inside: {:?}", no_inside);
}

/// Undo (`pop`) restores exact prior state and increments revision; invalid edits are rejected.
#[test]
fn undo_and_validation() {
    let mut log = EditLog::new();
    assert_eq!(log.revision(), 0);

    // Invalid edits rejected.
    assert!(log.push(Edit::add(Shape::Sphere { radius: 0.0 }, [0.0; 3], 0.0, Material::Soil, 1.7)).is_err());
    assert!(log.push(Edit::add(Shape::Sphere { radius: f32::NAN }, [0.0; 3], 0.0, Material::Soil, 1.7)).is_err());
    assert!(log.push(Edit { op: Op::Add, shape: Shape::Sphere { radius: 1.0 }, center: [0.0; 3], blend: 0.0, material: None, stamp_size: 1.7, tool: plateau_engine::Tool::All, mode: plateau_engine::WorldMode::Workshop }).is_err());
    assert!(log.push(Edit::carve(Shape::Sphere { radius: 1.0 }, [0.0; 3], -0.1, None, 1.7)).is_err());
    assert_eq!(log.revision(), 0);

    let pts = points_near([32.0, 0.0, 50.0], 4.0, 500, 5);
    let mut base = garden();
    let mut orig = Vec::new();
    base.sample(&pts, 1.7, &mut orig).unwrap();

    let rev1 = log.push(Edit::add(Shape::Sphere { radius: 1.5 }, [32.0, 0.0, 50.0], 0.3, Material::Stone, 1.7)).unwrap();
    assert_eq!(rev1, 1);
    let mut after_add = Vec::new();
    Edited::new(garden(), Arc::new(log.clone())).sample(&pts, 1.7, &mut after_add).unwrap();
    assert_ne!(hash(&orig), hash(&after_add));

    let popped = log.pop();
    assert!(popped.is_some());
    assert_eq!(log.revision(), 2);
    let mut after_undo = Vec::new();
    Edited::new(garden(), Arc::new(log)).sample(&pts, 1.7, &mut after_undo).unwrap();
    assert_eq!(hash(&orig), hash(&after_undo));

    // Also verify `Answer` unchanged when log is empty.
    let mut single = vec![Answer { distance: 1.25, material: Material::Grass }];
    EditLog::new().apply_batch(&[[0.0, 0.0, 0.0]], &mut single);
    assert_eq!(single[0], Answer { distance: 1.25, material: Material::Grass });
}

/// Raising a hill directly intersecting the player lifts them cleanly onto the new hill instead
/// of catapulting them away.
#[test]
fn raising_a_hill_on_the_player_lifts_smoothly_without_catapulting() {
    let person = SIZES[1].1;
    let mut base = garden();
    let mut body = Body::standing(&mut base, [32.0, 10.0, 50.0], person).unwrap();
    let start_xz = [body.feet[0], body.feet[2]];

    // Stamp a 1.5 m hill right where the player is standing (slightly offset so it hits the flank).
    let mut log = EditLog::new();
    let hill = Edit::add(Shape::Sphere { radius: 1.5 }, [32.3, 0.0, 50.2], 0.45, Material::Soil, person);
    log.push(hill).unwrap();
    let mut edited = Edited::new(garden(), Arc::new(log));

    let (_, hi) = hill.influence_bounds();
    let max_lift = (hi[1] as f64 - body.feet[1]).max(0.0) + person as f64 * 2.0;
    let lifted = body.surface_from_solid(&mut edited, max_lift).unwrap();
    assert!(lifted, "body buried by new hill should be lifted onto it");

    for _ in 0..60 {
        body.step(&mut edited, Intent::default(), 1.0 / 60.0).unwrap();
    }
    let drift = ((body.feet[0] - start_xz[0]).powi(2) + (body.feet[2] - start_xz[1]).powi(2)).sqrt();
    let speed = body.velocity.iter().map(|v| v * v).sum::<f32>().sqrt();
    assert!(drift < 0.05, "player should stay on the hill, not be catapulted (drift = {drift:.3} m)");
    assert!(body.feet[1] > 1.0, "player should stand atop the raised hill, feet.y = {:.3}", body.feet[1]);
    assert!(body.on.is_some(), "player should be grounded on the new hill");
    assert!(speed < 0.1, "player should not have launch velocity, speed = {speed:.3}");
}
