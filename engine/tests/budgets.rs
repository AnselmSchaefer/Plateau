//! Phase J, step 6 (`WORLD_PLAN.md`): budgets are laws. Sessions of heavy play, kept as action logs
//! (`tests/corpus/`) and made here, are played again, and what an action costs is held to budgets
//! counted, not timed (the same on every machine, in a debug build as in a release one):
//!
//! - every carved element is drawn within `MAX_CARVED_TRIANGLES`, and all of them within a whole;
//! - settling asks no more than its budget allows an action (`SETTLE_BUDGET`, and one step past it);
//! - a long dig in one place asks no more of its edits, near the surface, late than early (`Bake`).

use std::path::PathBuf;

use plateau_engine::body::Body;
use plateau_engine::carved::MAX_CARVED_TRIANGLES;
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave, edits_asked};
use plateau_engine::live::{Action, ActionLog, Actor, Outcome, SETTLE_BUDGET, SculptTarget, World};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::{Tool, WorldMode};
use plateau_engine::{Limits, Material, Region, Scene, points_near};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn garden(mode: WorldMode) -> World {
    let mut save = WorldSave::new("The walled garden", 7, Vec::new());
    save.mode = mode;
    save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];
    World::open(save, None, &repo_root(), Limits::default(), ModuleCheck::Strict, 1.7).unwrap().0
}

/// The triangles of a scene's solid meshes, as drawn.
fn solid_triangles(scene: &Scene) -> usize {
    scene.instances.iter().filter(|i| i.solid).filter_map(|i| scene.meshes.get(i.mesh as usize)).map(|m| m.indices.len() / 3).sum()
}

/// Played by hand 2026-11-28 (`tests/corpus/carved-bridge-cut.jsonl`, 655 actions): a person dug
/// round a timber bridge with a pickaxe and a shovel, then cut it and its pieces a dozen times. Its
/// carved pieces were drawn with 4.5 million triangles, until the graphics card ran out of memory,
/// and a cut took up to three seconds. Played again as the window plays it (each action, then a
/// settle each frame while any is left), every budget holds at every action.
#[test]
fn the_carved_bridge_session_keeps_every_budget() {
    let log = ActionLog::read(&repo_root().join("engine/tests/corpus/carved-bridge-cut.jsonl")).unwrap();
    let mut world = garden(WorldMode::World);
    let mut body = Body::of_size(1.7);
    let (mut most_drawn, mut most_total, mut most_settle, mut settles) = (0usize, 0usize, 0usize, 0usize);
    for line in &log {
        body.feet = line.player.feet;
        body.size = line.player.size;
        body.yaw = line.player.yaw;
        world.apply(Actor { id: &line.player.id, body: &mut body }, line.action.clone());
        most_settle = most_settle.max(world.settle_work());
        let mut frames = 0;
        while world.unsettled() {
            world.apply(Actor { id: &line.player.id, body: &mut body }, Action::Settle);
            most_settle = most_settle.max(world.settle_work());
            frames += 1;
            settles += 1;
            assert!(frames < 200, "action {}: settling ends", line.seq);
        }
        if matches!(line.action, Action::Cut { .. } | Action::Sculpt { .. }) {
            let drawn: Vec<usize> = world.placed.iter().filter_map(|p| world.runtime.drawn_scene_of(p.id)).map(solid_triangles).collect();
            let (most, total) = (drawn.iter().copied().max().unwrap_or(0), drawn.iter().sum::<usize>());
            assert!(most <= MAX_CARVED_TRIANGLES, "action {}: an element drawn with {most} triangles", line.seq);
            assert!(total <= 600_000, "action {}: all elements drawn with {total} triangles", line.seq);
            most_drawn = most_drawn.max(most);
            most_total = most_total.max(total);
        }
    }
    println!("most triangles of one element {most_drawn}, of all {most_total}; most settling an action {most_settle} points; {settles} settles");
    // Settling stops at its budget between steps of a fall: one step (a body's points) past it at most.
    assert!(most_settle <= SETTLE_BUDGET + 4 * 512, "settling asked {most_settle} points in one action");
}

/// The `k`th stroke of a long dig in four metres square: carves, and a third of them mounds.
fn stroke(k: u32) -> Edit {
    let h = k.wrapping_mul(2_654_435_761);
    let dx = ((h & 1023) as f32 / 1023.0 - 0.5) * 4.0;
    let dz = (((h >> 10) & 1023) as f32 / 1023.0 - 0.5) * 4.0;
    let dy = (((h >> 20) & 255) as f32 / 255.0 - 0.7) * 1.0;
    let at = [38.0 + dx, dy, 50.0 + dz];
    if k % 3 == 2 { Edit::add(Shape::Sphere { radius: 0.3 }, at, 0.05, Material::Soil, 1.7) } else { Edit::carve(Shape::Sphere { radius: 0.35 }, at, 0.05, None, 1.7) }
}

/// Asking the ground near the surface of a much-dug place costs no more after 2,000 strokes than
/// after 500: what was dug long ago is baked. (Before, the edits asked grew with every stroke.)
#[test]
fn a_long_dig_asks_no_more_of_its_edits_late_than_early() {
    let mut world = garden(WorldMode::Workshop);
    let mut body = Body::of_size(1.7);
    body.feet = [30.0, 0.0, 58.0];
    let mut asked_at = Vec::new();
    for k in 0..2000u32 {
        let e = stroke(k);
        let outcome = world.apply(Actor { id: "local", body: &mut body }, Action::Sculpt { edit: e, hit: e.center, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } });
        assert!(matches!(outcome, Outcome::Done(_)), "{outcome:?}");
        if k + 1 == 500 || k + 1 == 2000 {
            // The points of the dug place within half a metre of its surface.
            let pts = points_near([38.0, 0.0, 50.0], 2.0, 6000, 5);
            let mut out = Vec::new();
            world.combined().sample(&pts, 1.7, &mut out).unwrap();
            let near: Vec<[f32; 3]> = pts.iter().zip(&out).filter(|(_, a)| a.distance.abs() < 0.5).map(|(p, _)| *p).collect();
            let before = edits_asked();
            world.combined().sample(&near, 1.7, &mut out).unwrap();
            let per_point = (edits_asked() - before) as f64 / near.len() as f64;
            println!("after {} strokes: {:.1} edits asked a point near the surface ({} points)", k + 1, per_point, near.len());
            asked_at.push(per_point);
        }
    }
    assert!(asked_at[1] <= 1.5 * asked_at[0], "asked {:.1} a point after 2,000 strokes, {:.1} after 500", asked_at[1], asked_at[0]);
}
