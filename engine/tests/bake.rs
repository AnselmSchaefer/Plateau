//! Phase J, step 3 (`WORLD_PLAN.md`): a world's edits, past a few hundred, are baked (`EditLog::
//! rebaked`, `Bake`): what an action costs near a much-dug place stops growing with how much was
//! dug. A bake is of the edit list alone, so a world played, opened again, undone to or redone to
//! is the same to the bit; and it is near what every edit, asked exactly, gives.

use std::path::PathBuf;

use plateau_engine::body::Body;
use plateau_engine::compose::{Composed, ComposedLog};
use plateau_engine::edit::{BAKE_KEEP, BAKE_STEP, Edit, EditLog, SavedTile, Shape, WorldSave};
use plateau_engine::live::{Action, Actor, SculptTarget, World};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::{Tool, WorldMode};
use plateau_engine::{Answer, Limits, Material, Region, points_near};
use std::sync::Arc;

const DIG: [f32; 3] = [38.0, 0.0, 50.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn garden() -> WorldSave {
    let mut save = WorldSave::new("The walled garden", 7, Vec::new());
    save.mode = WorldMode::Workshop;
    save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];
    save
}

fn open(save: WorldSave) -> World {
    World::open(save, None, &repo_root(), Limits::default(), ModuleCheck::Strict, 1.7).unwrap().0
}

/// The `k`th stroke of a long dig in one place: mostly carves, a third mounds, scattered over four
/// metres a side, as a person makes them.
fn stroke(k: u32) -> Edit {
    let h = k.wrapping_mul(2_654_435_761);
    let dx = ((h & 1023) as f32 / 1023.0 - 0.5) * 4.0;
    let dz = (((h >> 10) & 1023) as f32 / 1023.0 - 0.5) * 4.0;
    let dy = (((h >> 20) & 255) as f32 / 255.0 - 0.7) * 1.0;
    let at = [DIG[0] + dx, DIG[1] + dy, DIG[2] + dz];
    if k % 3 == 2 {
        Edit::add(Shape::Sphere { radius: 0.3 }, at, 0.05, Material::Soil, 1.7)
    } else {
        Edit::carve(Shape::Sphere { radius: 0.35 }, at, 0.05, None, 1.7)
    }
}

fn dig(world: &mut World, body: &mut Body, from: u32, to: u32) {
    for k in from..to {
        let e = stroke(k);
        world.apply(Actor { id: "local", body: &mut *body }, Action::Sculpt { edit: e, hit: e.center, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } });
    }
}

fn sampled(world: &mut World, pts: &[[f32; 3]], size: f32) -> Vec<Answer> {
    let mut out = Vec::new();
    world.combined().sample(pts, size, &mut out).unwrap();
    out
}

fn same(a: &[Answer], b: &[Answer], what: &str) {
    for (i, (x, y)) in a.iter().zip(b).enumerate() {
        assert!(x.distance.to_bits() == y.distance.to_bits() && x.material == y.material, "{what}: point {i}: {x:?} against {y:?}");
    }
}

#[test]
fn a_long_dig_is_baked_the_same_however_the_world_came_to_it() {
    let n = (BAKE_KEEP + 4 * BAKE_STEP + 1) as u32;
    let mut world = open(garden());
    let mut body = Body::of_size(1.7);
    body.feet = [30.0, 0.0, 58.0];
    dig(&mut world, &mut body, 0, n);
    let target = EditLog::bake_target(n as usize);
    assert_eq!(target, 4 * BAKE_STEP);
    let bake = world.edits.bake().cloned().expect("a long dig is baked");
    assert_eq!(bake.upto, target);
    assert!(bake.bricks().count() > 100, "{} bricks", bake.bricks().count());

    let pts = points_near(DIG, 2.8, 3000, 11);
    let played = sampled(&mut world, &pts, 1.7);
    let tiny_pts = points_near([DIG[0], 0.0, DIG[2]], 0.3, 500, 12);
    let played_tiny = sampled(&mut world, &tiny_pts, 0.017);

    // Opened from its save, baked from the start: the same to the bit, at a person's size and a tiny's.
    let saved = world.to_save();
    let mut opened = open(WorldSave::from_text(&saved.to_text()).unwrap());
    assert_eq!(opened.edits.bake().map(|b| b.upto), Some(target));
    same(&sampled(&mut opened, &pts, 1.7), &played, "opened");
    same(&sampled(&mut opened, &tiny_pts, 0.017), &played_tiny, "opened, as a tiny asks");

    // Undone across a step of the bake: as a world opened with those edits; redone, as played.
    for _ in 0..2 {
        world.apply(Actor { id: "local", body: &mut body }, Action::Undo);
    }
    assert_eq!(world.edits.len(), n as usize - 2);
    assert_eq!(world.edits.bake().map(|b| b.upto), Some(EditLog::bake_target(n as usize - 2)));
    assert!(EditLog::bake_target(n as usize - 2) < target, "the undo crosses a step");
    let mut fewer = garden();
    fewer.edits = world.edits.edits().to_vec();
    let mut opened_fewer = open(fewer);
    same(&sampled(&mut world, &pts, 1.7), &sampled(&mut opened_fewer, &pts, 1.7), "undone");
    for _ in 0..2 {
        world.apply(Actor { id: "local", body: &mut body }, Action::Redo);
    }
    same(&sampled(&mut world, &pts, 1.7), &played, "redone");
}

#[test]
fn a_baked_dig_is_near_what_every_edit_gives() {
    let n = (BAKE_KEEP + 3 * BAKE_STEP + 10) as u32;
    let mut world = open(garden());
    let mut body = Body::of_size(1.7);
    body.feet = [30.0, 0.0, 58.0];
    dig(&mut world, &mut body, 0, n);
    assert!(world.edits.bake().is_some());
    // The same edits, unbaked: every one asked exactly.
    let exact_log = Arc::new(ComposedLog::with_edits(Arc::new(EditLog::from_edits(world.edits.edits().to_vec()).unwrap())));
    let pts = points_near(DIG, 2.5, 4000, 21);
    let baked = sampled(&mut world, &pts, 1.7);
    let mut exact = Vec::new();
    Composed::new(&mut world.region.region, exact_log).sample(&pts, 1.7, &mut exact).unwrap();
    let cell = 0.0625;
    let (mut near, mut worst) = (0, 0.0f32);
    for (b, e) in baked.iter().zip(&exact) {
        if e.distance.abs() < 0.3 {
            near += 1;
            worst = worst.max((b.distance - e.distance).abs());
        }
        if e.distance.abs() > cell {
            assert_eq!(b.distance > 0.0, e.distance > 0.0, "inside or out, as exactly: {b:?} against {e:?}");
        }
    }
    println!("{near} points near the surface: baked within {worst:.4} m of exact (cell {cell} m)");
    assert!(near > 300);
    assert!(worst < 0.5 * cell, "baked {worst} m from exact near the surface");
}
