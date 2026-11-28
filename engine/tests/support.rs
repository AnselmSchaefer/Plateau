//! Phase G (`WORLD_PLAN.md`): support and falling. What no longer holds together comes apart, and
//! what nothing reaching the ground holds up falls: a trunk carved through drops its canopy; a notch
//! half through leaves the tree standing and a deeper one fells it; a felled tree lands on a bridge
//! beside it rather than through it; a log falls when its bridge is taken away, and into a hole dug
//! under it; two pieces resting only on each other are not held up; a cascade stops at its limit;
//! and every fall is undone with what set it off and replays bit for bit.

mod played;

#[path = "../../elements/bridge/element.rs"]
mod native_bridge;
#[path = "../../elements/pillar/element.rs"]
mod native_pillar;
#[path = "../../elements/tree/element.rs"]
mod native_tree;

use std::f32::consts::FRAC_PI_2;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use plateau_engine::atlas::Atlas;
use plateau_engine::body::{self, Body};
use plateau_engine::compose::{Composed, ComposedLog, SdfElementEntry, euler_quat, quat_rotate};
use plateau_engine::cut::{clip_scene, cull_non_solid_by_mask, placement_pos_for_aim};
use plateau_engine::edit::{Edit, EditLog, SavedTile, Shape, WorldSave};
use plateau_engine::graph::{Data, Node, Relation};
use plateau_engine::laws;
use plateau_engine::live::{Action, ActionLog, Actor, Footing, Outcome, SculptTarget, Summary, World, replay};
use plateau_engine::mesh_sdf::MeshSdf;
use plateau_engine::modules::{ModuleCheck, sha256_hex};
use plateau_engine::physics::Tool;
use plateau_engine::scene::{Combined, SceneWorld};
use plateau_engine::water::WaterLog;
use plateau_engine::{Limits, Material, Region, Wasm, chunk, hash, points_near};

const TINY: f32 = 0.017;
const PERSON: f32 = 1.7;
const TREE: &str = "elements/modules/tree.wasm";
const BRIDGE: &str = "elements/modules/bridge.wasm";
const PILLAR: &str = "elements/modules/pillar.wasm";
/// A spot on the garden's open lawn.
const LAWN: [f32; 3] = [38.0, 0.0, 50.0];
/// Where a bridge stands, west of the lawn.
const SPAN: [f32; 3] = [30.0, 0.0, 50.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("support").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn garden_save() -> WorldSave {
    let mut save = WorldSave::new("The walled garden", 7, Vec::new());
    save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];
    save
}

fn open(save: WorldSave) -> World {
    World::open(save, None, &repo_root(), Limits::default(), ModuleCheck::Strict, PERSON).unwrap().0
}

fn person(world: &mut World) -> Body {
    Body::standing(&mut world.combined(), [LAWN[0] as f64 - 3.0, 40.0, LAWN[2] as f64], PERSON).unwrap()
}

fn act(world: &mut World, body: &mut Body, action: Action) -> Outcome {
    world.apply(Actor { id: "local", body }, action)
}

fn done(outcome: Outcome) -> Summary {
    match outcome {
        Outcome::Done(s) => s,
        other => panic!("not done: {other:?}"),
    }
}

fn place(world: &mut World, body: &mut Body, module: &str, pos: [f32; 3], yaw: f32) -> u32 {
    done(act(world, body, Action::Place { module: module.into(), pos, yaw, rot: None, scale: 1.0, fitted: false, opening: None, seed: Some(7), params: Vec::new() })).added[0]
}

fn laws_hold(world: &mut World) {
    if let Err(e) = laws::world_graph(world).unwrap() {
        panic!("the world graph breaks a law: {e}\n{}", world.to_save().to_text());
    }
}

/// Where the trunk's bark is at height `y`, west and east, along `z` (looked for from either side).
fn trunk_at(world: &mut World, y: f32, z: f32) -> (f32, f32) {
    let mut c = world.combined();
    let west = body::look(&mut c, [LAWN[0] as f64 - 2.0, y as f64, z as f64], [1.0, 0.0, 0.0], PERSON, 4.0).unwrap().unwrap()[0] as f32;
    let east = body::look(&mut c, [LAWN[0] as f64 + 2.0, y as f64, z as f64], [-1.0, 0.0, 0.0], PERSON, 4.0).unwrap().unwrap()[0] as f32;
    (west, east)
}

/// Where the top of what is solid below `(x, z)` is, looking down.
fn top_at(world: &mut World, x: f32, z: f32) -> [f32; 3] {
    body::look(&mut world.combined(), [x as f64, 40.0, z as f64], [0.0, -1.0, 0.0], PERSON, 60.0).unwrap().unwrap().map(|c| c as f32)
}

/// How far from upright the element `id` stands: its own up's height (1 upright, 0 on its side).
fn upness(world: &World, id: u32) -> f32 {
    quat_rotate(world.find(id).unwrap().transform().rot, [0.0, 1.0, 0.0])[1]
}

/// A carve into the tree `tree` from the terrain's brush, aimed at it, with a saw.
fn carve(world: &mut World, body: &mut Body, tree: u32, edit: Edit, hit: [f32; 3]) -> Summary {
    done(act(world, body, Action::Sculpt { edit, hit, tool: Tool::Saw, target: SculptTarget::Terrain { aimed: Some(tree) } }))
}

/// A notch cut with a box from the east, `share` of the way through the trunk a metre up.
fn notch(world: &mut World, body: &mut Body, tree: u32, share: f32) -> Summary {
    let (west, east) = trunk_at(world, 1.0, LAWN[2]);
    let to = east - share * (east - west);
    let edit = Edit::carve(Shape::Box { half: [0.5, 0.06, 0.6], round: 0.0 }, [to + 0.5, 1.0, LAWN[2]], 0.01, None, PERSON);
    carve(world, body, tree, edit, [east, 1.0, LAWN[2]])
}

/// The tree chopped a metre up with a saw, pushed over east: (stump, trunk).
fn chop(world: &mut World, body: &mut Body, id: u32) -> (u32, u32, Summary) {
    let pos = world.find(id).unwrap().pos;
    let s = done(act(world, body, Action::Cut { id, hit: [pos[0], pos[1] + 0.9, pos[2]], dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: Some(Material::Wood) }));
    (s.added[0], s.added[1], s)
}

/// The element `id` picked up and set down flat (along z) on the lawn at `(x, z)`.
fn lay_flat(world: &mut World, body: &mut Body, id: u32, x: f32, z: f32) {
    done(act(world, body, Action::Grab { id }));
    let rot = euler_quat(0.0, FRAC_PI_2, 0.0);
    let aim = top_at(world, x, z);
    let at = placement_pos_for_aim(&world.physics_of(id).unwrap(), aim, rot, 1.0);
    done(act(world, body, Action::Drop { pos: at, rot }));
}

#[test]
fn a_trunk_carved_through_drops_its_canopy() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    laws_hold(&mut world);
    let whole = world.to_save().to_text();

    // A ball carved right through the trunk at a metre and a half.
    let (west, east) = trunk_at(&mut world, 1.5, LAWN[2]);
    let ball = Edit::carve(Shape::Sphere { radius: 0.45 }, [0.5 * (west + east), 1.5, LAWN[2]], 0.05, None, PERSON);
    let s = carve(&mut world, &mut body, tree, ball, [west, 1.5, LAWN[2]]);
    assert_eq!(s.promoted, vec![tree]);
    assert_eq!(s.removed, vec![tree], "the tree is two pieces now");
    let (stump, canopy) = (s.added[0], s.added[1]);
    assert_eq!(s.fell, vec![canopy]);
    assert!(s.said.as_deref().is_some_and(|w| w.ends_with("Oak Tree fell")), "{:?}", s.said);
    assert_eq!(world.graph.derived_from(stump).map(|d| d.0), Some(tree));
    assert_eq!(world.graph.derived_from(canopy).map(|d| d.0), Some(tree));
    assert_eq!(world.footing(stump), Some(Footing::Rooted), "the stump keeps its roots");

    // The canopy dropped onto what is left of the trunk, as far as the gap was (some 0.76 m), and
    // took the carve with it.
    let fell = -world.find(canopy).unwrap().pos[1];
    assert!((0.6..0.9).contains(&fell), "it fell {fell} m");
    assert_eq!(world.graph.supports_of(canopy), vec![Node::Element(stump)]);
    assert_eq!(world.find(canopy).unwrap().local_edits.len(), 1, "the carve went with it");
    laws_hold(&mut world);

    // Undone, the tree is whole and stands; redone, it is carved and fallen again, exactly.
    let fallen = world.to_save().to_text();
    done(act(&mut world, &mut body, Action::Undo));
    assert_eq!(world.to_save().to_text(), whole);
    laws_hold(&mut world);
    done(act(&mut world, &mut body, Action::Redo));
    assert_eq!(world.to_save().to_text(), fallen);
    laws_hold(&mut world);
}

#[test]
fn a_notch_half_through_leaves_the_tree_standing_and_a_deeper_one_fells_it() {
    // Half through: it stands, one tree, carved.
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let s = notch(&mut world, &mut body, tree, 0.5);
    assert!(s.added.is_empty() && s.fell.is_empty(), "{s:?}");
    assert!(world.find(tree).is_some_and(|pl| pl.is_promoted()));
    assert_eq!(upness(&world, tree), 1.0);
    laws_hold(&mut world);

    // Four-fifths through: what is left cannot hold the crown over it. It breaks at the notch, the
    // notch closes, and the top goes over the way the notch faces (east), onto the lawn.
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let standing = world.to_save().to_text();
    let s = notch(&mut world, &mut body, tree, 0.8);
    assert_eq!(s.removed, vec![tree]);
    let (stump, top) = (s.added[0], s.added[1]);
    assert_eq!(s.fell, vec![top]);
    assert_eq!(world.footing(stump), Some(Footing::Rooted));
    assert!(upness(&world, top) < 0.5, "felled: its up is {}", upness(&world, top));
    let lean = quat_rotate(world.find(top).unwrap().transform().rot, [0.0, 1.0, 0.0]);
    assert!(lean[0] > 0.8, "it fell east, towards the notch: {lean:?}");
    assert!(world.graph.supports_of(top).contains(&Node::Terrain), "{:?}", world.graph.supports_of(top));
    laws_hold(&mut world);
    done(act(&mut world, &mut body, Action::Undo));
    assert_eq!(world.to_save().to_text(), standing, "undone, it stands uncarved");
}

#[test]
fn a_felled_tree_lands_on_a_neighbouring_bridge_rather_than_through_it() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    // A bridge east of the tree, spanning east to west, where the tree will fall.
    let bridge = place(&mut world, &mut body, BRIDGE, [LAWN[0] + 3.6, 0.0, LAWN[2]], FRAC_PI_2);
    let deck = top_at(&mut world, LAWN[0] + 3.6, LAWN[2])[1];
    let (stump, trunk, s) = chop(&mut world, &mut body, tree);
    assert_eq!(s.fell, vec![trunk]);
    assert_eq!(world.falling.len(), 1, "the fall is shown, from where it began");
    let on = world.graph.supports_of(trunk);
    assert!(on.contains(&Node::Element(bridge)), "it rests on the bridge: {on:?}");
    assert!(on.contains(&Node::Element(stump)), "and on its stump: {on:?}");
    // Held up by the deck: it lies less flat than on the lawn, and over the deck's middle what is
    // seen first from above is the trunk, on the deck, not under it.
    assert!(upness(&world, trunk) > 0.2, "{}", upness(&world, trunk));
    world.finish_all_falls(None);
    let over = top_at(&mut world, LAWN[0] + 3.6, LAWN[2])[1];
    assert!(over > deck, "the trunk lies on the deck ({over} over {deck})");
    laws_hold(&mut world);
}

#[test]
fn removing_a_bridge_drops_the_log_lying_on_it_and_the_fall_is_undone_with_it() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let bridge = place(&mut world, &mut body, BRIDGE, SPAN, 0.0);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let (_, trunk, _) = chop(&mut world, &mut body, tree);
    // Along the bridge, its middle on the deck's crown.
    done(act(&mut world, &mut body, Action::Grab { id: trunk }));
    let rot = euler_quat(0.0, FRAC_PI_2, 0.0);
    let aim = top_at(&mut world, SPAN[0], SPAN[2]);
    let at = placement_pos_for_aim(&world.physics_of(trunk).unwrap(), aim, rot, 1.0);
    done(act(&mut world, &mut body, Action::Drop { pos: at, rot }));
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Element(bridge)]);
    let laid = world.to_save().to_text();

    let s = done(act(&mut world, &mut body, Action::Remove { id: bridge }));
    assert_eq!(s.fell, vec![trunk]);
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Terrain]);
    assert!(world.find(trunk).unwrap().pos[1] < at[1] - 0.2, "it fell to the ground");
    laws_hold(&mut world);
    let fallen = world.to_save().to_text();
    done(act(&mut world, &mut body, Action::Undo));
    assert_eq!(world.to_save().to_text(), laid, "the bridge back, the log on it");
    done(act(&mut world, &mut body, Action::Redo));
    assert_eq!(world.to_save().to_text(), fallen);
}

#[test]
fn terrain_dug_from_under_a_log_lets_it_fall_into_the_hole() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let (_, trunk, _) = chop(&mut world, &mut body, tree);
    let (x, z) = (LAWN[0] - 8.0, LAWN[2] - 4.0);
    lay_flat(&mut world, &mut body, trunk, x, z);
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Terrain]);
    let before = world.find(trunk).unwrap().pos;

    // A pit dug under the whole of it, wider than it is long.
    let ground = top_at(&mut world, x, z)[1];
    let pit = Edit::carve(Shape::Sphere { radius: 3.6 }, [x, ground - 2.0, z], 0.2, None, PERSON);
    let s = done(act(&mut world, &mut body, Action::Sculpt { edit: pit, hit: [x, ground, z], tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }));
    assert_eq!(s.fell, vec![trunk]);
    let after = world.find(trunk).unwrap().pos;
    assert!(after[1] < before[1] - 1.0, "it fell into the pit: {before:?} to {after:?}");
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Terrain], "it rests on the pit's floor");
    laws_hold(&mut world);
}

#[test]
fn two_pieces_resting_only_on_each_other_are_not_held_up() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let (_, trunk, _) = chop(&mut world, &mut body, tree);
    lay_flat(&mut world, &mut body, trunk, LAWN[0] - 8.0, LAWN[2] - 4.0);
    // Cut in two where it lies: both halves lie on the lawn, neither held up by the other alone.
    let middle = {
        let pl = world.find(trunk).unwrap();
        pl.transform().local_to_world_point(world.physics_of(trunk).unwrap().local_center)
    };
    let s = done(act(&mut world, &mut body, Action::Cut { id: trunk, hit: middle, dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: Some(Material::Wood) }));
    let (a, b) = (s.added[0], s.added[1]);
    for piece in [a, b] {
        assert!(world.graph.supports_of(piece).contains(&Node::Terrain), "#{piece} rests on {:?}", world.graph.supports_of(piece));
    }
    laws_hold(&mut world);

    // Written behind `apply`'s back as resting only on each other, the law sees through it...
    for (piece, other) in [(a, b), (b, a)] {
        world.unrelate(&Relation::rests_on(piece, Node::Terrain));
        world.relate(Relation::rests_on(piece, Node::Element(other)));
    }
    let cycle = laws::none_unsupported(&world);
    assert!(cycle.as_ref().is_err_and(|e| e.contains("nothing it rests on reaches the ground")), "{cycle:?}");
    // ...and the next action finds what they really rest on: the lawn, so neither falls.
    let s = done(act(&mut world, &mut body, Action::Interact { at: middle }));
    assert!(s.fell.is_empty());
    for piece in [a, b] {
        assert!(world.graph.supports_of(piece).contains(&Node::Terrain));
    }
    laws_hold(&mut world);
}

#[test]
fn a_cascade_stops_at_its_limit_and_goes_on_with_the_next_action() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let bridge = place(&mut world, &mut body, BRIDGE, SPAN, 0.0);
    // Three stone pillars stood on the deck.
    let mut pillars = Vec::new();
    for dz in [-1.5, 0.0, 1.5] {
        let on = top_at(&mut world, SPAN[0], SPAN[2] + dz);
        let p = place(&mut world, &mut body, PILLAR, on, 0.0);
        assert_eq!(world.graph.supports_of(p), vec![Node::Element(bridge)], "pillar #{p} stands on the bridge");
        pillars.push(p);
    }
    laws_hold(&mut world);

    world.fall_limit = 2;
    let s = done(act(&mut world, &mut body, Action::Remove { id: bridge }));
    assert_eq!(s.fell.len(), 2, "two fall with the bridge: {:?}", s.fell);
    let left = laws::none_unsupported(&world);
    assert!(left.as_ref().is_err_and(|e| e.contains("rests on nothing")), "one is left standing on nothing: {left:?}");
    // The next action, whatever it is, lets the last one fall.
    let s = done(act(&mut world, &mut body, Action::Interact { at: SPAN }));
    assert_eq!(s.fell.len(), 1);
    for p in &pillars {
        assert_eq!(world.graph.supports_of(*p), vec![Node::Terrain], "pillar #{p}");
    }
    laws_hold(&mut world);
}

/// A session of falls (a deep notch, a tree felled onto a bridge, the bridge taken away) written to
/// the action log and replayed onto the save it began from gives the same world, every outcome as
/// it was, its graph deltas and falls too.
#[test]
fn a_session_of_falls_replays_bit_for_bit() {
    let dir = scratch("replay");
    let log_path = dir.join("session.jsonl");
    let start = garden_save();
    let mut world = open(start.clone());
    world.set_log(Some(ActionLog::create(&log_path).unwrap()));
    let mut body = person(&mut world);

    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    assert!(!notch(&mut world, &mut body, tree, 0.8).fell.is_empty());
    let other = place(&mut world, &mut body, TREE, [LAWN[0], 0.0, LAWN[2] + 8.0], 0.0);
    let bridge = place(&mut world, &mut body, BRIDGE, [LAWN[0] + 3.6, 0.0, LAWN[2] + 8.0], FRAC_PI_2);
    let (_, trunk, s) = chop(&mut world, &mut body, other);
    assert_eq!(s.fell, vec![trunk]);
    assert!(world.graph.supports_of(trunk).contains(&Node::Element(bridge)));
    let s = done(act(&mut world, &mut body, Action::Remove { id: bridge }));
    assert_eq!(s.fell, vec![trunk], "taken away, the bridge lets the trunk down");
    done(act(&mut world, &mut body, Action::Undo));
    done(act(&mut world, &mut body, Action::Redo));
    laws_hold(&mut world);
    world.set_log(None);

    let log = ActionLog::read(&log_path).unwrap();
    assert!(log.iter().filter(|l| matches!(&l.outcome, Outcome::Done(s) if !s.fell.is_empty())).count() >= 3);
    let mut replayed = open(start);
    replay(&mut replayed, &log).unwrap();
    assert_eq!(replayed.to_save().to_text(), world.to_save().to_text());
    assert_eq!(replayed.graph, world.graph);
}

/// Played by hand (2026-11-27): bowls dug with a person's brush round an oak's foot carved its base
/// and roots away (a terrain carve cuts the elements it reaches), yet it stood on, rooted by the
/// shape of its meshes, and nothing carved into it afterwards could fell it. Its roots holding in
/// nothing, it is `uprooted` and falls; its pieces have no roots either; undone, it stands again.
#[test]
fn a_tree_whose_foot_is_dug_away_falls_and_stays_uprooted() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let standing = world.to_save().to_text();
    let mut fell_at = None;
    let rim = [(1.3f32, 0.0f32), (-1.3, 0.0), (0.0, 1.3), (0.0, -1.3), (0.9, 0.9), (-0.9, -0.9), (0.9, -0.9), (-0.9, 0.9)];
    for (k, (dx, dz)) in rim.into_iter().enumerate() {
        let at = [LAWN[0] + dx, 0.0, LAWN[2] + dz];
        let bowl = Edit::carve(Shape::Sphere { radius: 1.28 }, at, 0.45, None, PERSON);
        let s = done(act(&mut world, &mut body, Action::Sculpt { edit: bowl, hit: at, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }));
        if s.fell.contains(&tree) && fell_at.is_none() {
            assert!(world.graph.contains(&Relation::uprooted(tree)), "it fell because its roots hold in nothing");
            assert_eq!(world.footing(tree), Some(Footing::Loose));
            fell_at = Some(k);
        }
        laws_hold(&mut world);
    }
    assert!(fell_at.is_some(), "dug round, the tree fell");
    for pl in world.placed.clone() {
        assert_eq!(world.footing(pl.id), Some(Footing::Loose), "#{} is no longer rooted", pl.id);
        assert!(world.graph.contains(&Relation::uprooted(pl.id)), "#{} has no roots", pl.id);
    }
    assert!(world.to_save().to_text().contains("rel uprooted "), "saved, it stays uprooted");
    for _ in rim {
        done(act(&mut world, &mut body, Action::Undo));
    }
    assert_eq!(world.to_save().to_text(), standing, "undone, it stands rooted again");
    laws_hold(&mut world);
}

/// Carving a ring of four trenches around the middle of a bridge deck leaves a central plank
/// island surrounded on all four sides at the same height, so no single plane can part it from the
/// outer bridge frame. The component mask (`support::mask_boxes`) splits them cleanly, the island
/// drops straight through the hole onto the lawn, decorative rails stay only on the frame, and
/// save/reload and undo/redo round-trip bit for bit.
#[test]
fn a_ring_cut_island_in_a_bridge_deck_splits_by_component_mask_and_drops_through() {
    let dir = scratch("ring_bridge");
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let bridge = place(&mut world, &mut body, BRIDGE, SPAN, 0.0);
    let deck_y = top_at(&mut world, SPAN[0], SPAN[2])[1];
    assert!(deck_y > 0.35, "bridge deck at {deck_y}");
    laws_hold(&mut world);

    // Four trenches framing a central plank island (west, east, north, south).
    let trenches = [
        Edit::carve(Shape::Box { half: [0.09, 0.25, 0.62], round: 0.01 }, [SPAN[0] - 0.42, 0.42, SPAN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.09, 0.25, 0.62], round: 0.01 }, [SPAN[0] + 0.42, 0.42, SPAN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.51, 0.25, 0.09], round: 0.01 }, [SPAN[0], 0.42, SPAN[2] - 0.58], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.51, 0.25, 0.09], round: 0.01 }, [SPAN[0], 0.42, SPAN[2] + 0.58], 0.01, None, PERSON),
    ];
    for (k, edit) in trenches[..3].iter().enumerate() {
        let s = done(act(&mut world, &mut body, Action::Sculpt { edit: *edit, hit: edit.center, tool: Tool::Saw, target: SculptTarget::Terrain { aimed: Some(bridge) } }));
        assert!(s.removed.is_empty() && s.fell.is_empty(), "trench {k} leaves the island joined: {s:?}");
        laws_hold(&mut world);
    }
    let before_close = world.to_save().to_text();

    // Closing the fourth trench parts the island with no separating plane: component mask splits it.
    let fourth = trenches[3];
    let s = done(act(&mut world, &mut body, Action::Sculpt { edit: fourth, hit: fourth.center, tool: Tool::Saw, target: SculptTarget::Terrain { aimed: Some(bridge) } }));
    assert_eq!(s.removed, vec![bridge], "the bridge is two pieces now");
    assert_eq!(s.added.len(), 2);
    let (frame, island) = (s.added[0], s.added[1]);
    assert_eq!(s.fell, vec![island], "the surrounded island dropped");
    assert_eq!(world.graph.parent_of(frame), Some((bridge, Data::Split)));
    assert_eq!(world.graph.parent_of(island), Some((bridge, Data::Split)));
    assert!(world.find(frame).unwrap().clips.is_empty() && world.find(island).unwrap().clips.is_empty(), "non-planar mask split, no clip planes");
    assert!(!world.find(frame).unwrap().local_edits.is_empty() && !world.find(island).unwrap().local_edits.is_empty());
    assert_eq!(world.footing(frame), Some(Footing::Rooted), "the bridge frame stays rooted");
    assert_eq!(world.footing(island), Some(Footing::Loose));
    assert!(world.graph.contains(&Relation::uprooted(island)));
    assert_eq!(world.graph.supports_of(island), vec![Node::Terrain], "the island landed on the lawn beneath");
    assert!(world.find(island).unwrap().pos[1] < -0.20, "island dropped: {:?}", world.find(island).unwrap().pos);

    // Through the hole at the bridge's middle, the top surface is now the fallen island near the
    // lawn, while the side deck beam still stands at deck height.
    let hole_y = top_at(&mut world, SPAN[0], SPAN[2])[1];
    let side_y = top_at(&mut world, SPAN[0] + 0.68, SPAN[2])[1];
    assert!(hole_y < deck_y - 0.20, "hole dropped from {deck_y} to {hole_y}");
    assert!((side_y - deck_y).abs() < 0.12, "side beam still stands at {side_y} (deck was {deck_y})");

    // Decorative rails stayed on the outer frame and were culled from the inner island.
    let frame_trim = world.runtime.local_scene_of(frame).unwrap().instances.iter().filter(|i| !i.solid).count();
    let island_trim = world.runtime.local_scene_of(island).unwrap().instances.iter().filter(|i| !i.solid).count();
    assert_eq!(frame_trim, 1, "outer frame keeps its handrails");
    assert_eq!(island_trim, 0, "inner island carries no phantom handrails");
    laws_hold(&mut world);

    // Undo and redo round-trip bit-exactly (checked before `save` seals `world.modules`).
    let split_text = world.to_save().to_text();
    assert!(split_text.contains(&format!("rel derived_from {frame} {bridge} split")));
    assert!(split_text.contains(&format!("rel derived_from {island} {bridge} split")));
    done(act(&mut world, &mut body, Action::Undo));
    assert_eq!(world.to_save().to_text(), before_close);
    laws_hold(&mut world);
    done(act(&mut world, &mut body, Action::Redo));
    assert_eq!(world.to_save().to_text(), split_text);
    laws_hold(&mut world);

    // Save/reload round-trip preserves the mask, graph, and SDF samples bit-exactly.
    let save_path = dir.join("ring_bridge.plateau");
    world.save(&save_path).unwrap();
    let mut reopened = open(WorldSave::load(&save_path).unwrap());
    assert_eq!(reopened.graph, world.graph);
    assert_eq!(reopened.placed, world.placed);
    let probe = points_near([SPAN[0], 0.2, SPAN[2]], 1.2, 2000, 19);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    world.combined().sample(&probe, PERSON, &mut a).unwrap();
    reopened.combined().sample(&probe, PERSON, &mut b).unwrap();
    assert_eq!(hash(&a), hash(&b));
    laws_hold(&mut reopened);
}

/// An interlocking mortise-and-tenon cut across a stone pillar has no separating plane at any
/// angle: the component mask parts them, the upper piece settles into the lower socket, and a
/// subsequent planar cut on one of the masked pieces keeps every graph law.
#[test]
fn an_interlocking_mortise_and_tenon_cut_splits_by_mask_and_survives_a_further_cut() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let ground = top_at(&mut world, LAWN[0], LAWN[2]);
    let pillar = place(&mut world, &mut body, PILLAR, ground, 0.0);
    laws_hold(&mut world);

    // U-shaped mortise-and-tenon cut: shoulders at y = 1.22, deep pocket down to y = 0.62 in the
    // middle (x in [-0.14, 0.14]), connected by two vertical slots at x = +/- 0.22.
    let cuts = [
        Edit::carve(Shape::Box { half: [0.16, 0.07, 0.50], round: 0.01 }, [LAWN[0] - 0.34, ground[1] + 1.22, LAWN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.16, 0.07, 0.50], round: 0.01 }, [LAWN[0] + 0.34, ground[1] + 1.22, LAWN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.06, 0.35, 0.50], round: 0.01 }, [LAWN[0] - 0.20, ground[1] + 0.92, LAWN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.06, 0.35, 0.50], round: 0.01 }, [LAWN[0] + 0.20, ground[1] + 0.92, LAWN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.22, 0.07, 0.50], round: 0.01 }, [LAWN[0], ground[1] + 0.62, LAWN[2]], 0.01, None, PERSON),
    ];
    let mut split = None;
    for edit in cuts {
        let target = world.placed.last().unwrap().id;
        let s = done(act(&mut world, &mut body, Action::Sculpt { edit, hit: edit.center, tool: Tool::Pickaxe, target: SculptTarget::Terrain { aimed: Some(target) } }));
        if !s.added.is_empty() {
            split = Some(s);
            break;
        }
        laws_hold(&mut world);
    }
    let s = split.expect("the mortise-and-tenon cut parts the pillar");
    let (lower, upper) = (s.added[0], s.added[1]);
    assert_eq!(world.graph.parent_of(lower), Some((pillar, Data::Split)));
    assert_eq!(world.graph.parent_of(upper), Some((pillar, Data::Split)));
    assert!(s.fell.contains(&upper), "upper tenon settled onto lower socket: {s:?}");
    assert!(world.graph.supports_of(upper).contains(&Node::Element(lower)), "rests in socket: {:?}", world.graph.supports_of(upper));
    laws_hold(&mut world);

    // A further planar cut on the upper masked piece keeps all world-graph laws.
    let pos = world.find(upper).unwrap().pos;
    let s2 = done(act(&mut world, &mut body, Action::Cut {
        id: upper,
        hit: [pos[0], pos[1] + 1.55, pos[2]],
        dir: [1.0, 0.0, 0.0],
        tool: Tool::Pickaxe,
        material: Some(Material::Stone),
    }));
    assert_eq!(s2.added.len(), 2);
    laws_hold(&mut world);
    done(act(&mut world, &mut body, Action::Undo));
    done(act(&mut world, &mut body, Action::Undo));
    laws_hold(&mut world);
}

/// Builds `garden`, `tree`, `bridge`, and `pillar` into `into` at the given rustc opt-level (`"0"`
/// for `-O0`), mirroring `areas/build.sh` and `elements/build.sh`.
fn build_o0_world_root(root: &Path) {
    let repo = repo_root();
    let out = root.join("build");
    std::fs::create_dir_all(&out).unwrap();
    std::fs::create_dir_all(root.join("areas/modules")).unwrap();
    std::fs::create_dir_all(root.join("elements/modules")).unwrap();
    let fixed = [
        "--edition", "2024",
        "--target", "wasm32-unknown-unknown",
        "-C", "opt-level=0",
        "-C", "codegen-units=1",
        "-C", "panic=abort",
    ];
    let rlib = out.join("libplateau_sdk.rlib");
    let status = std::process::Command::new("rustc")
        .args(fixed)
        .args(["--crate-type", "rlib", "--crate-name", "plateau_sdk"])
        .arg(repo.join("sdk/src/lib.rs"))
        .arg("-o")
        .arg(&rlib)
        .status()
        .unwrap();
    assert!(status.success(), "building plateau_sdk at -O0 failed");

    let build_mod = |src: PathBuf, dest: PathBuf| {
        let status = std::process::Command::new("rustc")
            .args(fixed)
            .args(["--crate-name", "area", "--crate-type", "cdylib", "-C", "link-arg=--max-memory=67108864", "--extern"])
            .arg(format!("plateau_sdk={}", rlib.display()))
            .arg(&src)
            .arg("-o")
            .arg(&dest)
            .status()
            .unwrap();
        assert!(status.success(), "building {} at -O0 failed", src.display());
    };
    build_mod(repo.join("areas/garden/area.rs"), root.join("areas/modules/garden.wasm"));
    for name in ["tree", "bridge", "pillar"] {
        build_mod(repo.join(format!("elements/{name}/element.rs")), root.join(format!("elements/modules/{name}.wasm")));
    }
}

/// Runs the representative Phase G fall scenarios (carved trunk drop, notch break & tip, felled
/// tree landing on bridge, bridge removal cascade, and non-planar ring-cut mask split) against the
/// world root `root` with `limits`, returning the final save text, action log text, SDF sample
/// hashes at every fall site (at tiny and person scale), chunk hashes, and the live world.
fn run_fall_suite(root: &Path, limits: Limits, log_path: &Path) -> (String, String, Vec<u64>, Vec<u64>, World) {
    let mut world = World::open(garden_save(), None, root, limits, ModuleCheck::Strict, PERSON).unwrap().0;
    world.set_log(Some(ActionLog::create(log_path).unwrap()));
    let mut body = person(&mut world);

    // 1. Trunk carved through -> canopy drops onto stump.
    let t1 = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let (w1, e1) = trunk_at(&mut world, 1.5, LAWN[2]);
    let ball = Edit::carve(Shape::Sphere { radius: 0.45 }, [0.5 * (w1 + e1), 1.5, LAWN[2]], 0.05, None, PERSON);
    assert!(!carve(&mut world, &mut body, t1, ball, [w1, 1.5, LAWN[2]]).fell.is_empty());

    // 2. Deep notch -> breaks and tips east onto the lawn.
    let t2_pos = [LAWN[0], 0.0, LAWN[2] - 8.0];
    let t2 = place(&mut world, &mut body, TREE, t2_pos, 0.0);
    let (w2, e2) = trunk_at(&mut world, 1.0, t2_pos[2]);
    let to = e2 - 0.8 * (e2 - w2);
    let cut_notch = Edit::carve(Shape::Box { half: [0.5, 0.06, 0.6], round: 0.0 }, [to + 0.5, 1.0, t2_pos[2]], 0.01, None, PERSON);
    assert!(!carve(&mut world, &mut body, t2, cut_notch, [e2, 1.0, t2_pos[2]]).fell.is_empty());

    // 3. Tree chopped onto neighbouring bridge, then bridge removed so trunk falls to lawn.
    let t3_pos = [LAWN[0], 0.0, LAWN[2] + 8.0];
    let t3 = place(&mut world, &mut body, TREE, t3_pos, 0.0);
    let b3 = place(&mut world, &mut body, BRIDGE, [t3_pos[0] + 3.6, 0.0, t3_pos[2]], FRAC_PI_2);
    let (_, trunk3, s3) = chop(&mut world, &mut body, t3);
    assert_eq!(s3.fell, vec![trunk3]);
    let s_rem = done(act(&mut world, &mut body, Action::Remove { id: b3 }));
    assert_eq!(s_rem.fell, vec![trunk3]);

    // 4. Ring-cut bridge deck -> non-planar component mask split & drop through hole.
    let b4 = place(&mut world, &mut body, BRIDGE, SPAN, 0.0);
    for edit in [
        Edit::carve(Shape::Box { half: [0.09, 0.25, 0.62], round: 0.01 }, [SPAN[0] - 0.42, 0.42, SPAN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.09, 0.25, 0.62], round: 0.01 }, [SPAN[0] + 0.42, 0.42, SPAN[2]], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.51, 0.25, 0.09], round: 0.01 }, [SPAN[0], 0.42, SPAN[2] - 0.58], 0.01, None, PERSON),
        Edit::carve(Shape::Box { half: [0.51, 0.25, 0.09], round: 0.01 }, [SPAN[0], 0.42, SPAN[2] + 0.58], 0.01, None, PERSON),
    ] {
        done(act(&mut world, &mut body, Action::Sculpt { edit, hit: edit.center, tool: Tool::Saw, target: SculptTarget::Terrain { aimed: Some(b4) } }));
    }

    world.finish_all_falls(None);
    laws_hold(&mut world);
    world.set_log(None);

    let sites = [LAWN, t2_pos, t3_pos, SPAN];
    let mut sdf_hashes = Vec::new();
    let mut out = Vec::new();
    for (idx, &site) in sites.iter().enumerate() {
        let pts = points_near([site[0], 0.6, site[2]], 2.0, 1500, 100 + idx as u32);
        for size in [TINY, PERSON] {
            world.combined().sample(&pts, size, &mut out).unwrap();
            sdf_hashes.push(hash(&out));
        }
    }
    let chunk_keys = [
        chunk::Key::new(-3, [9, 0, 12]),  // around LAWN (38, 0, 50) at person level (-3 -> 4 m chunks)
        chunk::Key::new(-3, [7, 0, 12]),  // around SPAN (30, 0, 50)
    ];
    let chunk_hashes: Vec<u64> = laws::chunk_hashes(&mut world.region, &chunk_keys).unwrap().into_iter().map(|(_, h)| h).collect();
    let save_text = world.to_save().to_text();
    let log_text = std::fs::read_to_string(log_path).unwrap();
    (save_text, log_text, sdf_hashes, chunk_hashes, world)
}

fn native_element_scene(module: &str, seed: u64) -> plateau_engine::Scene {
    match module {
        TREE => played::Native(native_tree::played(seed)).scene().unwrap(),
        BRIDGE => played::Native(native_bridge::played(seed)).scene().unwrap(),
        PILLAR => played::Native(native_pillar::played(seed)).scene().unwrap(),
        other => panic!("unexpected element module {other}"),
    }
}

/// Phase G determinism done-when: the fall scenarios produce bit-identical saves, action logs,
/// graph deltas, SDF samples, and chunk hashes:
/// 1. across `-O` and `-O0` `.wasm` modules and canonical NaNs vs native NaNs;
/// 2. between `.wasm` modules and the area & elements played natively;
/// 3. against kept cross-build hashes (verified under both `-O` release and `-O0` debug builds of
///    the engine).
#[test]
fn falls_and_hashes_match_natively_as_modules_and_across_o_and_o0() {
    let dir = scratch("determinism");
    let o0_root = dir.join("o0_root");
    build_o0_world_root(&o0_root);

    // 1. Every module used in falls matches its native source bit for bit, at both -O and -O0.
    let lawn_pts = points_near([LAWN[0], 0.5, LAWN[2]], 8.0, 2000, 77);
    let mut native_garden = played::area("garden", 7);
    let mut g_native = Vec::new();
    for size in [TINY, PERSON] {
        native_garden.sample(&lawn_pts, size, &mut g_native).unwrap();
        let h_nat = hash(&g_native);
        for (label, path) in [("-O", repo_root().join("areas/modules/garden.wasm")), ("-O0", o0_root.join("areas/modules/garden.wasm"))] {
            let mut w = Wasm::new(&std::fs::read(&path).unwrap(), 7, Limits::default()).unwrap();
            let mut out = Vec::new();
            w.sample(&lawn_pts, size, &mut out).unwrap();
            assert_eq!(hash(&out), h_nat, "garden {label} at {size} m");
        }
    }
    for module in [TREE, BRIDGE, PILLAR] {
        let nat_scene = native_element_scene(module, 7);
        for (label, root) in [("-O", repo_root()), ("-O0", o0_root.clone())] {
            let mut w = Wasm::new(&std::fs::read(root.join(module)).unwrap(), 7, Limits::default()).unwrap();
            assert_eq!(w.scene().unwrap(), nat_scene, "{module} {label} scene matches native bit for bit");
        }
    }

    // 2. Full fall suite: -O modules == canonical NaNs == -O0 modules.
    let (save_o, log_o, sdf_o, chunks_o, world_o) = run_fall_suite(&repo_root(), Limits::default(), &dir.join("log_o.jsonl"));
    let (save_nan, log_nan, sdf_nan, chunks_nan, _) =
        run_fall_suite(&repo_root(), Limits { canonical_nans: true, ..Limits::default() }, &dir.join("log_nan.jsonl"));
    let (save_o0, log_o0, sdf_o0, chunks_o0, _) = run_fall_suite(&o0_root, Limits::default(), &dir.join("log_o0.jsonl"));

    assert_eq!(save_o, save_nan, "canonical NaNs match default Limits");
    assert_eq!(log_o, log_nan);
    assert_eq!(sdf_o, sdf_nan);
    assert_eq!(chunks_o, chunks_nan);

    assert_eq!(save_o, save_o0, "-O and -O0 .wasm modules give identical saves");
    assert_eq!(log_o, log_o0, "-O and -O0 .wasm modules give identical action logs");
    assert_eq!(sdf_o, sdf_o0, "-O and -O0 .wasm modules give identical SDF hashes");
    assert_eq!(chunks_o, chunks_o0, "-O and -O0 .wasm modules give identical chunk hashes");

    // 3. Native world (native garden + native element scenes / MeshSdfs) matches WASM world
    //    bit for bit across all fall sites and chunk hashes.
    let mut native_sdf_entries = Vec::new();
    let mut native_scene_world = SceneWorld::default();
    for pl in &world_o.placed {
        let raw = native_element_scene(&pl.module, pl.seed);
        let clipped = clip_scene(&raw, &pl.clips);
        let phys = plateau_engine::cut::ObjectPhysics::compute(&clipped, pl.scale);
        let local_scene = if pl.is_promoted() && !pl.local_edits.is_empty() {
            cull_non_solid_by_mask(&clipped, &pl.local_edits, phys.local_lo, phys.local_hi)
        } else {
            clipped.clone()
        };
        native_scene_world.add_layer_filtered(pl.id, &local_scene, Some(pl.transform()), pl.is_promoted());
        if pl.is_promoted()
            && let Some(msdf) = MeshSdf::from_solid_scene(&clipped)
        {
            native_sdf_entries.push(SdfElementEntry::from_promoted_mesh(pl.clone(), Arc::new(msdf)));
        }
    }
    let native_log = Arc::new(ComposedLog::new_with_water(
        Arc::new(EditLog::from_edits(world_o.edits.edits().to_vec()).unwrap()),
        native_sdf_entries,
        Arc::new(WaterLog::from_bodies(world_o.water.bodies().to_vec()).unwrap()),
        0,
    ));
    let mut native_composed = Composed::new(Atlas::single(played::area("garden", 7)), native_log);
    let sites = [LAWN, [LAWN[0], 0.0, LAWN[2] - 8.0], [LAWN[0], 0.0, LAWN[2] + 8.0], SPAN];
    let mut native_sdf_hashes = Vec::new();
    let mut out = Vec::new();
    for (idx, &site) in sites.iter().enumerate() {
        let pts = points_near([site[0], 0.6, site[2]], 2.0, 1500, 100 + idx as u32);
        for size in [TINY, PERSON] {
            Combined { region: &mut native_composed, world: &native_scene_world }.sample(&pts, size, &mut out).unwrap();
            native_sdf_hashes.push(hash(&out));
        }
    }
    assert_eq!(native_sdf_hashes, sdf_o, "native garden + native elements match WASM world bit for bit");
    let chunk_keys = [chunk::Key::new(-3, [9, 0, 12]), chunk::Key::new(-3, [7, 0, 12])];
    let native_chunks: Vec<u64> = laws::chunk_hashes(&mut native_composed, &chunk_keys).unwrap().into_iter().map(|(_, h)| h).collect();
    assert_eq!(native_chunks, chunks_o, "native chunk hashes match WASM chunk hashes bit for bit");

    // 4. Cross-build golden digest: verified bit-identical whether `plateau-engine` itself is
    //    built with `-O` (`--release`), dev (`opt-level=1`), or `-O0` (`-C opt-level=0`).
    const KEPT_SAVE_SHA: &str = "ba3ab7c7bfd27f7b9b517847f37b7fa2efdd4beaa75ce5f54f054206bd1b59b9";
    const KEPT_LOG_SHA: &str = "24f8c8ea62febe425f1f2d49d531ecb5f96554c79e9bd9b42922a3295375b911";
    const KEPT_SDF: [u64; 8] = [
        0xa4ec_3407_8b19_e715,
        0xf95f_54d6_4d05_8f32,
        0x3fd8_a32e_0aba_484c,
        0x59bb_1e64_c28b_f175,
        0xe85c_710d_081c_0e6b,
        0x5e53_5fc4_d142_a1a2,
        0xf0c2_409f_d742_3532,
        0x49bf_f506_4429_177a,
    ];
    const KEPT_CHUNKS: [u64; 2] = [0x76ae_0037_39d4_e29a, 0xcea0_89ce_53c6_447e];

    let save_sha = sha256_hex(save_o.as_bytes());
    let log_sha = sha256_hex(log_o.as_bytes());
    if std::env::var_os("PLATEAU_KEEP").is_some() {
        eprintln!("FALL_SUITE_SAVE_SHA = {save_sha}");
        eprintln!("FALL_SUITE_LOG_SHA  = {log_sha}");
        eprintln!("FALL_SUITE_SDF      = {sdf_o:016x?}");
        eprintln!("FALL_SUITE_CHUNKS   = {chunks_o:016x?}");
    } else {
        assert_eq!(save_sha, KEPT_SAVE_SHA, "fall suite save hash changed across build");
        assert_eq!(log_sha, KEPT_LOG_SHA, "fall suite action log hash changed across build");
        assert_eq!(sdf_o, KEPT_SDF, "fall suite SDF hashes changed across build");
        assert_eq!(chunks_o, KEPT_CHUNKS, "fall suite chunk hashes changed across build");
    }
}

/// Played by hand (2026-11-27): after chopping an oak, sculpting at the stump's root or the grass
/// right beside it previously hit the fallen trunk's wider AABB along the camera ray and promoted
/// the fallen trunk instead of sculpting the stump or terrain. `scene_element_at` asks the exact
/// surface at the aimed point, and `promote_overlapping_scene_elements` checks exact solid
/// distance, so the fallen trunk is never hijacked.
#[test]
fn aiming_at_a_stumps_root_or_the_grass_beside_it_never_targets_the_fallen_trunk() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, &mut body, TREE, LAWN, 0.0);
    let (stump, trunk, _) = chop(&mut world, &mut body, tree);
    world.finish_all_falls(None);
    laws_hold(&mut world);

    // Aim directly at the stump's west side near the base: must resolve to the stump, not the fallen trunk.
    let (west, _) = trunk_at(&mut world, 0.35, LAWN[2]);
    let stump_hit = [west, 0.35, LAWN[2]];
    assert_eq!(world.scene_element_at(stump_hit, PERSON), Some(stump));

    // Aim at the grass 1.0 m west of the stump (clear of the roots and of the east-fallen trunk):
    // must resolve to terrain (`None`), and carving there promotes only the stump (which the
    // 1.28 m sphere reaches), leaving the fallen trunk untouched.
    let grass_hit = top_at(&mut world, LAWN[0] - 1.1, LAWN[2]);
    assert_eq!(world.scene_element_at(grass_hit, PERSON), None);
    let bowl = Edit::carve(Shape::Sphere { radius: 1.0 }, grass_hit, 0.25, None, PERSON);
    let s = done(act(&mut world, &mut body, Action::Sculpt {
        edit: bowl,
        hit: grass_hit,
        tool: Tool::All,
        target: SculptTarget::Terrain { aimed: None },
    }));
    assert_eq!(s.promoted, vec![stump], "only the stump is touched by the carve near its root");
    assert!(!world.find(trunk).unwrap().is_promoted(), "the fallen trunk behind the stump stays untouched");
    // Once promoted, aiming back at the stump still resolves to the stump.
    let stump_east = body::look(&mut world.combined(), [LAWN[0] as f64 + 1.2, 0.45, LAWN[2] as f64], [-1.0, 0.0, 0.0], PERSON, 2.0).unwrap().unwrap().map(|c| c as f32);
    assert_eq!(world.scene_element_at(stump_east, PERSON), Some(stump));
    laws_hold(&mut world);
}

#[test]
fn clipped_element_meshes_stay_watertight_across_single_and_chained_cuts() {
    use plateau_engine::cut::{ClipPlane, clip_scene};
    use plateau_engine::mesh_sdf::MeshSdf;

    let count_boundary = |sc: &plateau_engine::Scene| -> (usize, usize) {
        let quant = |p: [f32; 3]| -> [i64; 3] {
            [
                (p[0] as f64 * 10_000.0).round() as i64,
                (p[1] as f64 * 10_000.0).round() as i64,
                (p[2] as f64 * 10_000.0).round() as i64,
            ]
        };
        let mut open = 0usize;
        let mut nonman = 0usize;
        for inst in &sc.instances {
            if !inst.solid {
                continue;
            }
            let mesh = &sc.meshes[inst.mesh as usize];
            let mut edges: std::collections::BTreeMap<([i64; 3], [i64; 3]), usize> = std::collections::BTreeMap::new();
            for tri in mesh.indices.as_chunks::<3>().0 {
                let ps = [
                    quant(mesh.vertices[tri[0] as usize].pos),
                    quant(mesh.vertices[tri[1] as usize].pos),
                    quant(mesh.vertices[tri[2] as usize].pos),
                ];
                if ps[0] == ps[1] || ps[1] == ps[2] || ps[2] == ps[0] {
                    continue;
                }
                for (u, v) in [(ps[0], ps[1]), (ps[1], ps[2]), (ps[2], ps[0])] {
                    let key = if u < v { (u, v) } else { (v, u) };
                    *edges.entry(key).or_default() += 1;
                }
            }
            open += edges.values().filter(|&&c| c == 1).count();
            nonman += edges.values().filter(|&&c| c > 2).count();
        }
        (open, nonman)
    };

    let mut catalog = plateau_engine::compose::ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load(TREE).unwrap();
    let raw = tree.compiled.instantiate(218).unwrap().scene().unwrap();
    assert_eq!(count_boundary(&raw), (0, 0), "unclipped tree must be watertight");

    // Exact single-cut and double-cut clip planes from live play-testing (including short ~3 mm
    // quad-diagonal cut segments and multi-tube bough+trunk intersections).
    let y_lo = ClipPlane::new([0.0, -1.0, 0.0], -1.3417867);
    let y_hi = ClipPlane::new([0.0, 1.0, 0.0], 1.3417867);
    let diag_a = ClipPlane::new([0.81644994, -0.321231, -0.47981268], -0.3965153);
    let diag_b = diag_a.flipped();
    let diag_c = ClipPlane::new([0.7221242, -0.66356444, -0.19549656], -1.6637183);

    for (label, planes) in [
        ("stump", vec![y_lo]),
        ("upper", vec![y_hi]),
        ("upper_a", vec![y_hi, diag_a]),
        ("upper_b", vec![y_hi, diag_b]),
        ("triple", vec![y_hi, diag_b, diag_c]),
    ] {
        let clipped = clip_scene(&raw, &planes);
        let (open, nonman) = count_boundary(&clipped);
        assert_eq!((open, nonman), (0, 0), "{label} must stay watertight (no open/non-manifold edges)");

        // Promoted MeshSdf must stay Lipschitz-safe (|grad d| <= 1.15) at TINY scale with no
        // open-cap inside/outside sign flips.
        let msdf = MeshSdf::from_solid_scene(&clipped).expect("clipped piece keeps solid mesh");
        let lo = msdf.lo();
        let hi = msdf.hi();
        let mid = [0, 1, 2].map(|a| 0.5 * (lo[a] + hi[a]));
        let rad = [0, 1, 2].map(|a| 0.6 * (hi[a] - lo[a])).into_iter().fold(0.0f32, f32::max);
        let eps = 0.001f32;
        let mut max_grad = 0.0f32;
        for p in points_near(mid, rad, 400, 17) {
            let d0 = msdf.sample(p, f32::INFINITY).unwrap().distance;
            let dx = (msdf.sample([p[0] + eps, p[1], p[2]], f32::INFINITY).unwrap().distance - d0) / eps;
            let dy = (msdf.sample([p[0], p[1] + eps, p[2]], f32::INFINITY).unwrap().distance - d0) / eps;
            let dz = (msdf.sample([p[0], p[1], p[2] + eps], f32::INFINITY).unwrap().distance - d0) / eps;
            max_grad = max_grad.max((dx * dx + dy * dy + dz * dz).sqrt());
        }
        assert!(max_grad <= 1.15, "{label} promoted MeshSdf must stay Lipschitz-safe, got |grad|={max_grad:.3}");
    }

    // Bridge deck & piers also stay watertight under tilted cuts.
    let bridge = catalog.get_or_load(BRIDGE).unwrap();
    let raw_bridge = bridge.compiled.instantiate(11).unwrap().scene().unwrap();
    for plane in [
        ClipPlane::new([1.0, 0.0, 0.0], 0.35),
        ClipPlane::new([0.0, 1.0, 0.0], 0.18),
        ClipPlane::new([0.6, 0.8, 0.0], 0.12),
    ] {
        let a = clip_scene(&raw_bridge, &[plane]);
        let b = clip_scene(&raw_bridge, &[plane.flipped()]);
        assert_eq!(count_boundary(&a), (0, 0), "bridge +half must stay watertight");
        assert_eq!(count_boundary(&b), (0, 0), "bridge -half must stay watertight");
    }
}

/// Played by hand (2026-11-27): chopping an oak left the fallen trunk resting on top of the stump;
/// digging a pit through the stump's lower trunk split the upper stump loose, yet `meets()` used
/// unsigned mesh proximity without checking motion direction, so the trunk resting *above* the
/// severed stump froze it floating in mid-air with no support. Stepping along the fall direction
/// lets the severed stump drop into the pit and the trunk follow it.
#[test]
fn a_severed_stump_under_a_fallen_trunk_drops_into_the_pit_instead_of_floating() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let site = [31.919285, 0.018911324, 45.120136];
    let tree = place(&mut world, &mut body, TREE, site, 0.0);
    let s = done(act(&mut world, &mut body, Action::Cut {
        id: tree,
        hit: [31.896683, 1.7236083, 45.340374],
        dir: [0.406281, -0.30029318, -0.86299455],
        tool: Tool::All,
        material: Some(Material::Wood),
    }));
    let (stump, trunk) = (s.added[0], s.added[1]);
    assert!(world.graph.supports_of(trunk).contains(&Node::Element(stump)), "trunk rests on stump");
    laws_hold(&mut world);

    // Dig a pit through the stump's lower trunk — splits off the upper stump right under `trunk`.
    let pit1 = Edit::carve(Shape::Sphere { radius: 1.2750001 }, [31.55847, -0.06082036, 45.93107], 0.44625002, None, PERSON);
    let s1 = done(act(&mut world, &mut body, Action::Sculpt {
        edit: pit1,
        hit: [31.55847, -0.06082036, 45.93107],
        tool: Tool::All,
        target: SculptTarget::Terrain { aimed: None },
    }));
    assert!(!s1.fell.is_empty(), "severed upper stump and trunk must fall");
    laws_hold(&mut world);

    // Dig out the remaining root beneath — everything loose must rest on solid support, nothing floating.
    let pit2 = Edit::carve(Shape::Sphere { radius: 1.2750001 }, [31.753262, -0.6693301, 44.828728], 0.44625002, None, PERSON);
    let _s2 = done(act(&mut world, &mut body, Action::Sculpt {
        edit: pit2,
        hit: [31.753262, -0.6693301, 44.828728],
        tool: Tool::All,
        target: SculptTarget::Terrain { aimed: None },
    }));
    for pl in &world.placed {
        if world.footing(pl.id) == Some(plateau_engine::live::Footing::Loose) {
            assert!(!world.graph.supports_of(pl.id).is_empty(), "loose #{} must not float unsupported", pl.id);
        }
    }
    laws_hold(&mut world);
}
