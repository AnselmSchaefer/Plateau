//! Phase G (`WORLD_PLAN.md`): support and falling. What no longer holds together comes apart, and
//! what nothing reaching the ground holds up falls: a trunk carved through drops its canopy; a notch
//! half through leaves the tree standing and a deeper one fells it; a felled tree lands on a bridge
//! beside it rather than through it; a log falls when its bridge is taken away, and into a hole dug
//! under it; two pieces resting only on each other are not held up; a cascade stops at its limit;
//! and every fall is undone with what set it off and replays bit for bit.

use std::f32::consts::FRAC_PI_2;
use std::path::PathBuf;

use plateau_engine::body::{self, Body};
use plateau_engine::compose::{euler_quat, quat_rotate};
use plateau_engine::cut::placement_pos_for_aim;
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::graph::{Node, Relation};
use plateau_engine::laws;
use plateau_engine::live::{Action, ActionLog, Actor, Footing, Outcome, SculptTarget, Summary, World, replay};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::Tool;
use plateau_engine::{Limits, Material};

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
    done(act(world, body, Action::Place { module: module.into(), pos, yaw, rot: None, scale: 1.0, fitted: false, opening: None, seed: Some(7) })).added[0]
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
