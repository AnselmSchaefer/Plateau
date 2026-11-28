//! Phase E (`WORLD_PLAN.md`): every change goes through the engine. A world opened without a window
//! is placed in, sculpted (on the terrain and on a placed tree), refused by the rule book and the
//! tool, undone and redone, saved; and its action log, replayed onto the save it began from, gives
//! the same world, bit for bit.

use std::path::PathBuf;

use plateau_engine::body::Body;
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::live::{Action, ActionLog, Actor, Outcome, SculptTarget, World, replay};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::Tool;
use plateau_engine::rules::AreaPolicy;
use plateau_engine::{Limits, Material, Region};

const PERSON: f32 = 1.7;
const TINY: f32 = 0.017;
const TREE: &str = "elements/modules/tree.wasm";
/// A spot on the garden's open lawn.
const LAWN: [f32; 3] = [38.0, 0.0, 50.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("world").join(name);
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

/// What the world's SDF answers about `at`, at `size`, as a hash.
fn sampled(world: &mut World, at: [f32; 3], size: f32) -> u64 {
    let points = plateau_engine::points_near(at, 1.5, 3000, 11);
    let mut out = Vec::new();
    world.combined().sample(&points, size, &mut out).unwrap();
    plateau_engine::hash(&out)
}

fn place_tree(world: &mut World, body: &mut Body) -> u32 {
    match act(world, body, Action::Place { module: TREE.into(), pos: LAWN, yaw: 0.0, rot: None, scale: 1.0, fitted: false, opening: None, seed: None, params: Vec::new() }) {
        Outcome::Done(s) => s.added[0],
        other => panic!("the tree was not placed: {other:?}"),
    }
}

/// A tiny's carve into the tree's trunk, a little above the ground.
fn trunk_carve(world: &World, id: u32) -> (Edit, [f32; 3]) {
    let pos = world.find(id).unwrap().pos;
    let at = [pos[0], pos[1] + 0.3, pos[2]];
    (Edit::carve(Shape::Sphere { radius: 0.02 }, at, 0.007, None, TINY), at)
}

/// The showcase's opening moves, as a player makes them: a tree placed, a hill raised beside it, a
/// tiny home carved into the trunk, a cave into the hill, an undo and a redo.
fn showcase(world: &mut World, body: &mut Body) -> Vec<Outcome> {
    let mut outcomes = Vec::new();
    let tree = place_tree(world, body);
    let hill = Edit::add(Shape::Sphere { radius: 1.2 }, [LAWN[0] + 4.0, -0.4, LAWN[2]], 0.3, Material::Soil, PERSON);
    outcomes.push(act(world, body, Action::Sculpt { edit: hill, hit: [LAWN[0] + 4.0, 0.0, LAWN[2]], tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }));
    let (home, hit) = trunk_carve(world, tree);
    outcomes.push(act(world, body, Action::Sculpt { edit: home, hit, tool: Tool::Saw, target: SculptTarget::Element(tree) }));
    let cave = Edit::carve(Shape::Sphere { radius: 0.3 }, [LAWN[0] + 4.0, 0.5, LAWN[2]], 0.1, None, PERSON);
    outcomes.push(act(world, body, Action::Sculpt { edit: cave, hit: [LAWN[0] + 4.0, 0.7, LAWN[2]], tool: Tool::Shovel, target: SculptTarget::Terrain { aimed: None } }));
    outcomes.push(act(world, body, Action::Undo));
    outcomes.push(act(world, body, Action::Redo));
    outcomes
}

#[test]
fn a_world_is_placed_in_sculpted_undone_and_saved_through_apply_alone() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let before = sampled(&mut world, LAWN, PERSON);

    let outcomes = showcase(&mut world, &mut body);
    for o in &outcomes {
        assert!(o.is_done(), "{o:?}");
    }
    assert_eq!(world.placed.len(), 1);
    assert_eq!(world.edits.len(), 2, "a hill and a cave on the terrain");
    let tree = &world.placed[0];
    assert!(tree.is_promoted(), "carving into the trunk promoted it");
    assert_eq!(tree.local_edits.len(), 1, "the tiny home travels with the tree");
    assert_ne!(sampled(&mut world, LAWN, PERSON), before);
    assert!(world.dirty);

    // The window learns what to show.
    let changes = world.take_changes();
    assert!(!changes.composed.is_empty() && !changes.layers.is_empty());
    assert!(world.take_changes().is_empty());

    // Undoing everything gives the lawn back.
    for _ in 0..4 {
        assert!(act(&mut world, &mut body, Action::Undo).is_done());
    }
    assert_eq!(act(&mut world, &mut body, Action::Undo), Outcome::Unchanged("nothing to undo".into()));
    assert!(world.placed.is_empty() && world.edits.is_empty());
    assert_eq!(sampled(&mut world, LAWN, PERSON), before);
}

#[test]
fn the_rule_book_and_the_tool_refuse_through_apply() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place_tree(&mut world, &mut body);

    // A shovel does not dig wood; a saw does.
    let (home, hit) = trunk_carve(&world, tree);
    let shovel = act(&mut world, &mut body, Action::Sculpt { edit: home, hit, tool: Tool::Shovel, target: SculptTarget::Element(tree) });
    assert!(matches!(&shovel, Outcome::Denied(r) if r.contains("Shovel")), "{shovel:?}");
    assert!(world.placed[0].local_edits.is_empty());
    assert!(act(&mut world, &mut body, Action::Sculpt { edit: home, hit, tool: Tool::Saw, target: SculptTarget::Element(tree) }).is_done());
    // The same stamp again changes nothing.
    assert!(matches!(act(&mut world, &mut body, Action::Sculpt { edit: home, hit, tool: Tool::Saw, target: SculptTarget::Element(tree) }), Outcome::Unchanged(_)));
    assert_eq!(world.placed[0].local_edits.len(), 1);

    // An area someone else owns is not someone else's to change.
    world.rules.set_area(AreaPolicy::owned([0, 0], "owner"));
    let hill = Edit::add(Shape::Sphere { radius: 1.0 }, [LAWN[0] + 4.0, -0.4, LAWN[2]], 0.3, Material::Soil, PERSON);
    let edits = world.edits.len();
    assert!(matches!(act(&mut world, &mut body, Action::Sculpt { edit: hill, hit: LAWN, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }), Outcome::Denied(_)));
    assert!(matches!(act(&mut world, &mut body, Action::Place { module: TREE.into(), pos: LAWN, yaw: 0.0, rot: None, scale: 1.0, fitted: false, opening: None, seed: None, params: Vec::new() }), Outcome::Denied(_)));
    assert!(matches!(act(&mut world, &mut body, Action::Remove { id: tree }), Outcome::Denied(_)));
    assert_eq!((world.edits.len(), world.placed.len()), (edits, 1));
    assert!(world.apply(Actor { id: "owner", body: &mut body }, Action::Remove { id: tree }).is_done());
}

#[test]
fn a_log_replayed_onto_the_save_it_began_from_gives_the_same_world() {
    let dir = scratch("replay");
    let start = garden_save();
    let log_path = dir.join("session.jsonl");

    let mut played = open(start.clone());
    played.set_log(Some(ActionLog::create(&log_path).unwrap()));
    let mut body = person(&mut played);
    showcase(&mut played, &mut body);
    // A refusal is logged too, and must be refused again.
    let tree = played.placed[0].id;
    let (home, hit) = trunk_carve(&played, tree);
    let moved = Edit { center: [home.center[0], home.center[1] + 0.05, home.center[2]], ..home };
    assert!(matches!(act(&mut played, &mut body, Action::Sculpt { edit: moved, hit, tool: Tool::Shovel, target: SculptTarget::Element(tree) }), Outcome::Denied(_)));
    assert!(act(&mut played, &mut body, Action::Remove { id: tree }).is_done());
    played.set_log(None);

    let log = ActionLog::read(&log_path).unwrap();
    assert_eq!(log.len(), 8);
    assert_eq!(log[0].action.name(), "place");
    let text = std::fs::read_to_string(&log_path).unwrap();
    assert!(text.starts_with("{\"log\":\"plateau-log 1\"}\n"), "{text}");

    let mut replayed = open(start);
    replay(&mut replayed, &log).unwrap();
    assert_eq!(replayed.to_save().to_text(), played.to_save().to_text());
    for size in [TINY, PERSON] {
        assert_eq!(sampled(&mut replayed, LAWN, size), sampled(&mut played, LAWN, size));
    }

    // A log that does not fit the world it is replayed onto says where.
    let mut elsewhere = open(garden_save());
    let mut wrong = log.clone();
    wrong.remove(0);
    let e = replay(&mut elsewhere, &wrong).expect_err("a sculpt on a tree that is not there");
    assert!(format!("{e:#}").contains("replaying action"), "{e:#}");
}

#[test]
fn a_saved_world_opens_again_as_it_was() {
    let dir = scratch("save");
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    showcase(&mut world, &mut body);
    let path = dir.join("garden.plateau");
    world.save(&path).unwrap();
    assert!(!world.dirty);

    let mut again = open(WorldSave::load(&path).unwrap());
    assert_eq!(again.placed, world.placed);
    for size in [TINY, PERSON] {
        assert_eq!(sampled(&mut again, LAWN, size), sampled(&mut world, LAWN, size));
    }
    // Reloading over a changed world brings the save back.
    assert!(act(&mut world, &mut body, Action::Undo).is_done());
    let (edits, elements) = world.reload(&path).unwrap();
    assert_eq!((edits, elements), (2, 1));
    assert_eq!(sampled(&mut world, LAWN, PERSON), sampled(&mut again, LAWN, PERSON));
}

/// The showcase's later half: the tree with a tiny's home in it is chopped, the trunk carried and
/// laid flat as a bridge, stood on; moving it is undone and redone; and the whole session's log
/// replays onto where it began.
#[test]
fn a_chopped_trunk_is_carried_and_laid_as_a_bridge_and_the_session_replays() {
    use plateau_engine::compose::euler_quat;
    use plateau_engine::cut::placement_pos_for_aim;

    let dir = scratch("bridge");
    let log_path = dir.join("session.jsonl");
    let start = garden_save();
    let mut world = open(start.clone());
    world.set_log(Some(ActionLog::create(&log_path).unwrap()));
    let mut body = person(&mut world);

    let tree = place_tree(&mut world, &mut body);
    let (home, hit) = trunk_carve(&world, tree);
    assert!(act(&mut world, &mut body, Action::Sculpt { edit: home, hit, tool: Tool::Saw, target: SculptTarget::Element(tree) }).is_done());

    // A shovel does not chop a tree; a saw does, and the top topples east.
    let pos = world.find(tree).unwrap().pos;
    let chop_at = [pos[0], pos[1] + 0.9, pos[2]];
    let cut = |tool| Action::Cut { id: tree, hit: chop_at, dir: [1.0, 0.0, 0.0], tool, material: Some(Material::Wood) };
    assert!(matches!(act(&mut world, &mut body, cut(Tool::Shovel)), Outcome::Denied(_)));
    let (stump, trunk) = match act(&mut world, &mut body, cut(Tool::Saw)) {
        Outcome::Done(s) => (s.added[0], s.added[1]),
        other => panic!("not chopped: {other:?}"),
    };
    assert_eq!(world.placed.len(), 2);
    assert!(world.find(tree).is_none(), "the tree is two pieces now");
    assert_eq!(world.falling.len(), 1, "the top is toppling");
    let stump = world.find(stump).unwrap().clone();
    assert_eq!(stump.clips.len(), 1);

    // Picked up (the fall ends first), carried, and laid flat across the lawn to the west.
    assert!(act(&mut world, &mut body, Action::Grab { id: trunk }).is_done(), "a person carries the trunk");
    assert!(world.falling.is_empty());
    assert_eq!(world.held.map(|h| h.id), Some(trunk));
    let picked_from = world.held.unwrap().from;
    let phys = world.physics_of(trunk).unwrap();
    let rot = euler_quat(0.0, std::f32::consts::FRAC_PI_2, 0.0);
    let lawn_y = Body::standing(&mut world.combined(), [LAWN[0] as f64 - 8.0, 40.0, LAWN[2] as f64 - 4.0], PERSON).unwrap().feet[1];
    let aim = [LAWN[0] - 8.0, lawn_y as f32, LAWN[2] - 4.0];
    let at = placement_pos_for_aim(&phys, aim, rot, 1.0);
    assert!(act(&mut world, &mut body, Action::Drop { pos: at, rot }).is_done());
    assert!(world.held.is_none());
    let laid = world.find(trunk).unwrap().transform();
    let up = plateau_engine::compose::quat_rotate(laid.rot, [0.0, 1.0, 0.0]);
    assert!(up[1].abs() < 1e-3, "laid flat: its up is {up:?}");

    // Stood on: it holds a person up off the lawn, and the tiny's home went with it.
    let on_it = Body::standing(&mut world.combined(), [aim[0] as f64, 40.0, aim[2] as f64], PERSON).unwrap();
    assert!(on_it.feet[1] > lawn_y + 0.05, "standing on the laid trunk: {} over a lawn at {lawn_y}", on_it.feet[1]);
    assert_eq!(world.find(trunk).unwrap().local_edits.len(), 1);

    // Moving it is undone and redone.
    assert!(act(&mut world, &mut body, Action::Undo).is_done());
    assert_eq!(world.find(trunk).unwrap().transform(), picked_from);
    assert!(act(&mut world, &mut body, Action::Redo).is_done());
    assert_eq!(world.find(trunk).unwrap().transform(), laid);
    world.set_log(None);

    let log = ActionLog::read(&log_path).unwrap();
    let names: Vec<&str> = log.iter().map(|l| l.action.name()).collect();
    assert_eq!(names, ["place", "sculpt", "cut", "cut", "grab", "drop", "undo", "redo"]);
    let mut replayed = open(start);
    replay(&mut replayed, &log).unwrap();
    assert_eq!(replayed.to_save().to_text(), world.to_save().to_text());
    for size in [TINY, PERSON] {
        assert_eq!(sampled(&mut replayed, aim, size), sampled(&mut world, aim, size));
    }
}

/// A boat floats where it is put on the pond, is paddled east, let go, and the paddling replays.
#[test]
fn a_boat_is_paddled_across_the_pond_and_the_paddling_replays() {
    let dir = scratch("boat");
    let log_path = dir.join("session.jsonl");
    let start = garden_save();
    let mut world = open(start.clone());
    world.set_log(Some(ActionLog::create(&log_path).unwrap()));
    let mut body = person(&mut world);

    let boat = match act(&mut world, &mut body, Action::Place {
        module: "elements/modules/boat.wasm".into(),
        pos: [24.0, -0.3, 36.0],
        yaw: -std::f32::consts::FRAC_PI_2,
        rot: None,
        scale: 1.0,
        fitted: false,
        opening: None,
        seed: None,
        params: Vec::new(),
    }) {
        Outcome::Done(s) => s.added[0],
        other => panic!("no boat: {other:?}"),
    };
    assert!(world.floating.contains_key(&boat), "the boat floats");
    let rest = world.find(boat).unwrap().pos;

    // Bobbing is only shown: the boat's saved place does not move.
    for _ in 0..30 {
        world.animate(1.0 / 30.0, &mut body);
    }
    assert_eq!(world.find(boat).unwrap().pos, rest);

    assert!(matches!(act(&mut world, &mut body, Action::Steer { forward: 1.0, turn: 0.0, dt: 0.1 }), Outcome::Denied(_)), "not at the helm yet");
    assert!(act(&mut world, &mut body, Action::Pilot { id: Some(boat) }).is_done());
    for _ in 0..15 {
        act(&mut world, &mut body, Action::Steer { forward: 1.0, turn: 0.2, dt: 1.0 / 30.0 });
    }
    assert!(act(&mut world, &mut body, Action::Pilot { id: None }).is_done());
    let moved = world.find(boat).unwrap().pos;
    assert!(moved[0] > rest[0] + 0.5, "paddled east: {rest:?} → {moved:?}");
    world.set_log(None);

    let log = ActionLog::read(&log_path).unwrap();
    let mut replayed = open(start);
    replay(&mut replayed, &log).unwrap();
    assert_eq!(replayed.to_save().to_text(), world.to_save().to_text());
}
