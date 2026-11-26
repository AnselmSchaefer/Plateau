//! Phase F (`WORLD_PLAN.md`): a world graph. Objects know what they come from, rest on, and belong
//! to: chopping gives two pieces linked to the original; a log laid on a bridge rests on it;
//! ownership survives a chop by another player; every law of the graph holds on the showcase world;
//! saves and undo keep the graph exactly.

use std::f32::consts::FRAC_PI_2;
use std::path::PathBuf;

use plateau_engine::body::{self, Body};
use plateau_engine::compose::{euler_quat, quat_rotate};
use plateau_engine::cut::placement_pos_for_aim;
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::graph::{Kind, Node, Relation};
use plateau_engine::laws;
use plateau_engine::live::{Action, ActionLog, Actor, Footing, Outcome, SculptTarget, Summary, World, replay};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::Tool;
use plateau_engine::rules::{AreaPolicy, CutCarryPolicy};
use plateau_engine::{Limits, Material};

const PERSON: f32 = 1.7;
const TINY: f32 = 0.017;
const TREE: &str = "elements/modules/tree.wasm";
const BRIDGE: &str = "elements/modules/bridge.wasm";
/// A spot on the garden's open lawn.
const LAWN: [f32; 3] = [38.0, 0.0, 50.0];
/// Where the bridge stands, west of the tree.
const SPAN: [f32; 3] = [30.0, 0.0, 50.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("graph").join(name);
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

fn act_as(world: &mut World, who: &str, body: &mut Body, action: Action) -> Outcome {
    world.apply(Actor { id: who, body }, action)
}

fn done(outcome: Outcome) -> Summary {
    match outcome {
        Outcome::Done(s) => s,
        other => panic!("not done: {other:?}"),
    }
}

fn place(world: &mut World, who: &str, body: &mut Body, module: &str, pos: [f32; 3]) -> u32 {
    done(act_as(world, who, body, Action::Place { module: module.into(), pos, yaw: 0.0, rot: None, scale: 1.0, fitted: false })).added[0]
}

/// Chops the element `id` a metre up with a saw, the top toppling east: (stump, trunk).
fn chop(world: &mut World, who: &str, body: &mut Body, id: u32) -> (u32, u32) {
    let pos = world.find(id).unwrap().pos;
    let s = done(act_as(world, who, body, Action::Cut { id, hit: [pos[0], pos[1] + 0.9, pos[2]], dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: Some(Material::Wood) }));
    (s.added[0], s.added[1])
}

/// Where the top of what is solid below `(x, z)` is, looking down.
fn top_at(world: &mut World, x: f32, z: f32) -> [f32; 3] {
    let hit = body::look(&mut world.combined(), [x as f64, 40.0, z as f64], [0.0, -1.0, 0.0], PERSON, 60.0).unwrap().unwrap();
    hit.map(|c| c as f32)
}

/// Picks the trunk up and lays it flat along the bridge, its middle on the deck's crown.
fn lay_on_bridge(world: &mut World, who: &str, body: &mut Body, trunk: u32) {
    done(act_as(world, who, body, Action::Grab { id: trunk }));
    let rot = euler_quat(0.0, FRAC_PI_2, 0.0);
    let aim = top_at(world, SPAN[0], SPAN[2]);
    let phys = world.physics_of(trunk).unwrap();
    let at = placement_pos_for_aim(&phys, aim, rot, 1.0);
    done(act_as(world, who, body, Action::Drop { pos: at, rot }));
    let up = quat_rotate(world.find(trunk).unwrap().transform().rot, [0.0, 1.0, 0.0]);
    assert!(up[1].abs() < 1e-3, "laid flat: its up is {up:?}");
}

fn graph_holds(world: &mut World) {
    if let Err(e) = laws::world_graph(world).unwrap() {
        panic!("the world graph breaks a law: {e}\n{}", world.to_save().to_text());
    }
}

#[test]
fn chopping_gives_two_pieces_linked_to_the_original() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let tree = place(&mut world, "local", &mut body, TREE, LAWN);
    assert_eq!(world.owner_of(tree), Some("local"));
    assert_eq!(world.footing(tree), Some(Footing::Rooted), "a tree stands on its roots");
    assert!(world.graph.supports_of(tree).is_empty());
    graph_holds(&mut world);

    let pos = world.find(tree).unwrap().pos;
    let cut = done(act_as(&mut world, "local", &mut body, Action::Cut { id: tree, hit: [pos[0], pos[1] + 0.9, pos[2]], dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: Some(Material::Wood) }));
    let (stump, trunk) = (cut.added[0], cut.added[1]);
    assert_eq!(cut.removed, vec![tree]);
    assert!(world.find(tree).is_none());

    // Both pieces come from the tree, each along its own side of the one cut.
    let (from_a, cut_a) = world.graph.derived_from(stump).expect("the stump is derived from the tree");
    let (from_b, cut_b) = world.graph.derived_from(trunk).expect("the trunk is derived from the tree");
    assert_eq!((from_a, from_b), (tree, tree));
    assert_eq!(cut_a, cut_b.flipped());
    assert_eq!(world.find(stump).unwrap().clips.last(), Some(&cut_a));
    assert_eq!(world.find(trunk).unwrap().clips.last(), Some(&cut_b));
    assert_eq!(world.graph.pieces_of(tree), vec![stump, trunk]);

    // The stump keeps its roots; the trunk lies on the lawn where its fall ends.
    assert_eq!(world.footing(stump), Some(Footing::Rooted));
    assert_eq!(world.footing(trunk), Some(Footing::Loose));
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Terrain]);
    graph_holds(&mut world);

    // The cut's outcome says what it did to the graph, and undoing it takes it all back.
    assert!(cut.graph.added.contains(&Relation::rests_on(trunk, Node::Terrain)));
    assert!(cut.graph.removed.contains(&Relation::owned_by(tree, "local")));
    let before_cut: Vec<Relation> = {
        let mut w = open(garden_save());
        let mut b = person(&mut w);
        place(&mut w, "local", &mut b, TREE, LAWN);
        w.graph.relations().collect()
    };
    let undo = done(act_as(&mut world, "local", &mut body, Action::Undo));
    assert_eq!(undo.graph.added.len(), cut.graph.removed.len());
    assert_eq!(world.graph.relations().collect::<Vec<_>>(), before_cut);
    let redo = done(act_as(&mut world, "local", &mut body, Action::Redo));
    assert_eq!(redo.graph.added.len(), cut.graph.added.len());
    assert_eq!(world.graph.derived_from(trunk).map(|d| d.0), Some(tree));
    graph_holds(&mut world);
}

#[test]
fn a_log_laid_on_a_bridge_rests_on_it() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    let bridge = place(&mut world, "local", &mut body, BRIDGE, SPAN);
    assert_eq!(world.footing(bridge), Some(Footing::Rooted), "its abutments are set into the ground");
    let tree = place(&mut world, "local", &mut body, TREE, LAWN);
    let (_, trunk) = chop(&mut world, "local", &mut body, tree);
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Terrain]);

    lay_on_bridge(&mut world, "local", &mut body, trunk);
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Element(bridge)], "on the bridge, not the lawn");
    assert_eq!(world.graph.resting_on(bridge), vec![trunk]);
    graph_holds(&mut world);

    // Undoing the move puts it back on the lawn, in the graph too; redoing it, on the bridge.
    done(act_as(&mut world, "local", &mut body, Action::Undo));
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Terrain]);
    graph_holds(&mut world);
    done(act_as(&mut world, "local", &mut body, Action::Redo));
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Element(bridge)]);

    // The law looks: moved behind `apply`'s back, a metre up, the log touches nothing it rests on.
    let laid = world.find(trunk).unwrap().transform();
    let mut lifted = laid;
    lifted.pos[1] += 1.0;
    world.relocate_placed(trunk, lifted, None);
    let floating_free = laws::rests_are_contacts(&mut world);
    assert!(floating_free.as_ref().is_err_and(|e| e.contains("does not touch it")), "{floating_free:?}");
    world.relocate_placed(trunk, laid, None);
    graph_holds(&mut world);

    // Taking the bridge away takes what rests on it with it, and the law says the log is loose
    // (it falls in phase G).
    done(act_as(&mut world, "local", &mut body, Action::Remove { id: bridge }));
    assert!(world.graph.supports_of(trunk).is_empty());
    let loose = laws::none_unsupported(&world);
    assert!(loose.as_ref().is_err_and(|e| e.contains("rests on nothing")), "{loose:?}");
    done(act_as(&mut world, "local", &mut body, Action::Undo));
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Element(bridge)]);
    graph_holds(&mut world);
}

/// Played by hand (2026-11-26): a channel dug from the pond floods, a bridge is laid across it, and
/// a log set down on the bridge sank through it to the water, which knew only the terrain. Resting
/// on the bridge, it stays on the bridge.
#[test]
fn a_log_laid_on_a_bridge_over_water_stays_on_the_bridge() {
    let mut world = open(garden_save());
    let mut body = person(&mut world);
    // The channel, as it was dug.
    let mound = Edit::add(Shape::Sphere { radius: 1.2750001 }, [27.022926, -0.29227033, 37.05045], 0.44625002, Material::Soil, PERSON);
    done(act_as(&mut world, "local", &mut body, Action::Sculpt { edit: mound, hit: mound.center, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }));
    let dig = Edit::carve(Shape::Sphere { radius: 1.2750001 }, [27.046272, 0.99110466, 37.08866], 0.44625002, None, PERSON);
    assert!(done(act_as(&mut world, "local", &mut body, Action::Sculpt { edit: dig, hit: dig.center, tool: Tool::Shovel, target: SculptTarget::Terrain { aimed: None } })).water, "the channel floods");
    let bridge = place(&mut world, "local", &mut body, BRIDGE, [27.27918, -0.28953496, 34.372276]);
    let tree = place(&mut world, "local", &mut body, TREE, LAWN);
    let (_, long) = chop(&mut world, "local", &mut body, tree);
    // A short log, cut from the middle of the trunk where it lies: short enough to lie over water
    // alone, beside the bridge.
    let middle = {
        let pl = world.find(long).unwrap();
        pl.transform().local_to_world_point(world.physics_of(long).unwrap().local_center)
    };
    let halves = done(act_as(&mut world, "local", &mut body, Action::Cut { id: long, hit: middle, dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: Some(Material::Wood) }));
    let trunk = halves.added[0];

    done(act_as(&mut world, "local", &mut body, Action::Grab { id: trunk }));
    let rot = euler_quat(0.0, FRAC_PI_2, 0.0);
    // Along the bridge, its middle on the deck's crown, water either side and below.
    let (x, z) = (27.27918, 34.372276);
    let aim = top_at(&mut world, x, z);
    let at = placement_pos_for_aim(&world.physics_of(trunk).unwrap(), aim, rot, 1.0);
    done(act_as(&mut world, "local", &mut body, Action::Drop { pos: at, rot }));

    // There is water under the deck, and the log is not in it.
    let mut out = Vec::new();
    plateau_engine::Region::sample(&mut world.region, &[[x, -0.45, z]], PERSON, &mut out).unwrap();
    assert_eq!(out[0].material, Material::Water, "water under the bridge");
    assert_eq!(world.find(trunk).unwrap().pos, at, "it stays where it was set down");
    assert_eq!(world.footing(trunk), Some(Footing::Loose), "not afloat");
    assert!(world.graph.supports_of(trunk).contains(&Node::Element(bridge)), "{:?}", world.graph.supports_of(trunk));
    // Stood on, it is not pressed down through the deck either (shown only, but it is what is seen).
    let mut on_it = Body::standing(&mut world.combined(), [x as f64, 40.0, z as f64], PERSON).unwrap();
    let feet = on_it.feet[1];
    for _ in 0..30 {
        world.animate(1.0 / 30.0, &mut on_it);
    }
    assert_eq!(on_it.feet[1], feet);
    graph_holds(&mut world);
}

#[test]
fn ownership_survives_a_chop_by_another_player() {
    let mut world = open(garden_save());
    let mut policy = AreaPolicy::owned([0, 0], "alice");
    policy.builders = vec!["bob".into()];
    world.rules.set_area(policy.clone());
    let mut body = person(&mut world);
    let tree = place(&mut world, "alice", &mut body, TREE, LAWN);
    assert_eq!(world.owner_of(tree), Some("alice"));

    // Bob may chop here; the pieces are still Alice's.
    let (stump, trunk) = chop(&mut world, "bob", &mut body, tree);
    assert_eq!(world.owner_of(stump), Some("alice"));
    assert_eq!(world.owner_of(trunk), Some("alice"));
    assert!(world.graph.relations().all(|r| r.to != Node::Player("bob".into())), "nothing is Bob's");

    // Where builders may move only what they own, Bob cannot carry the trunk off; Alice can.
    policy.builder_policy.cut_carry = CutCarryPolicy::OwnOnly;
    world.rules.set_area(policy);
    let denied = act_as(&mut world, "bob", &mut body, Action::Grab { id: trunk });
    assert!(matches!(&denied, Outcome::Denied(r) if r.contains("you own")), "{denied:?}");
    assert!(matches!(act_as(&mut world, "bob", &mut body, Action::Cut { id: stump, hit: LAWN, dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: None }), Outcome::Denied(_)));
    done(act_as(&mut world, "alice", &mut body, Action::Grab { id: trunk }));
    graph_holds(&mut world);
}

/// The showcase (a tree placed, a tiny's home carved in it, a hill, chopped, the trunk carried and
/// laid on a bridge) keeps every law of the graph at every step; saved, it opens with the same
/// graph, and its log replays to the same graph.
#[test]
fn every_law_holds_on_the_showcase_and_saves_round_trip() {
    let dir = scratch("showcase");
    let log_path = dir.join("session.jsonl");
    let mut world = open(garden_save());
    world.set_log(Some(ActionLog::create(&log_path).unwrap()));
    let mut body = person(&mut world);

    let bridge = place(&mut world, "local", &mut body, BRIDGE, SPAN);
    let tree = place(&mut world, "local", &mut body, TREE, LAWN);
    graph_holds(&mut world);
    let pos = world.find(tree).unwrap().pos;
    let home = Edit::carve(Shape::Sphere { radius: 0.02 }, [pos[0], pos[1] + 0.3, pos[2]], 0.007, None, TINY);
    done(act_as(&mut world, "local", &mut body, Action::Sculpt { edit: home, hit: home.center, tool: Tool::Saw, target: SculptTarget::Element(tree) }));
    let hill = Edit::add(Shape::Sphere { radius: 1.2 }, [LAWN[0] + 4.0, -0.4, LAWN[2] + 6.0], 0.3, Material::Soil, PERSON);
    done(act_as(&mut world, "local", &mut body, Action::Sculpt { edit: hill, hit: hill.center, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }));
    graph_holds(&mut world);
    let (stump, trunk) = chop(&mut world, "local", &mut body, tree);
    graph_holds(&mut world);
    assert_eq!(world.find(trunk).unwrap().local_edits.len(), 1, "the tiny's home went with the trunk");
    lay_on_bridge(&mut world, "local", &mut body, trunk);
    graph_holds(&mut world);
    world.set_log(None);

    let kinds = |w: &World, k: Kind| w.graph.relations().filter(|r| r.kind == k).count();
    assert_eq!(kinds(&world, Kind::DerivedFrom), 2);
    assert_eq!(kinds(&world, Kind::OwnedBy), 3, "the bridge, the stump and the trunk");
    assert_eq!(world.graph.supports_of(trunk), vec![Node::Element(bridge)]);
    assert!(world.graph.supports_of(stump).is_empty());

    // Saved and opened again: the same graph, the same text.
    let path = dir.join("garden.plateau");
    world.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains(&format!("rel rests_on {trunk} {bridge}\n")), "{text}");
    assert!(text.contains(&format!("rel owned_by {trunk} @local\n")), "{text}");
    assert!(text.contains(&format!("rel derived_from {trunk} {tree} clip ")), "{text}");
    let mut again = open(WorldSave::load(&path).unwrap());
    assert_eq!(again.graph, world.graph);
    assert_eq!(again.to_save().to_text(), world.to_save().to_text());
    graph_holds(&mut again);
    // A piece's original is gone, but its id is not given out again.
    let next = place(&mut again, "local", &mut body, TREE, [LAWN[0], 0.0, LAWN[2] + 8.0]);
    assert!(next > trunk && next != tree);

    // The log replays to the same graph, every action's graph delta as it was.
    let log = ActionLog::read(&log_path).unwrap();
    assert!(log.iter().any(|l| matches!(&l.outcome, Outcome::Done(s) if !s.graph.is_empty())));
    let mut replayed = open(garden_save());
    replay(&mut replayed, &log).unwrap();
    assert_eq!(replayed.graph, world.graph);

    // A save from before the graph gets one when it opens: owners from who placed each element.
    let mut old = world.to_save();
    old.relations.clear();
    let mut derived = open(old);
    assert_eq!(derived.owner_of(trunk), Some("local"));
    assert_eq!(derived.graph.supports_of(trunk), vec![Node::Element(bridge)]);
    assert!(laws::rests_are_contacts(&mut derived).is_ok());
}

#[test]
fn a_save_naming_what_is_not_there_is_refused() {
    let mut save = garden_save();
    save.relations = vec![Relation::rests_on(4, Node::Terrain)];
    let e = WorldSave::from_text(&save.to_text()).expect_err("no element 4");
    assert!(format!("{e:#}").contains("element 4"), "{e:#}");
    let twice = "plateau-world 2\nseed 1\nelement 1 m seed 1 pos 0 0 0 yaw 0 scale 1 size 1.7 after 0\nrel owned_by 1 @a\nrel owned_by 1 @a\n";
    assert!(WorldSave::from_text(twice).is_err());
    let old = "plateau-world 1\nseed 1\nrel owned_by 1 @a\n";
    assert!(WorldSave::from_text(old).is_err(), "a version 1 save has no graph");
}
