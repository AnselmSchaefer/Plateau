//! Phase E (`WORLD_PLAN.md`): tracing times what the engine does and changes nothing. Alone in its
//! own test binary: tracing's interest in each span is kept process-wide, and tests on other
//! threads with no subscriber would set it first.

use std::path::PathBuf;

use plateau_engine::body::Body;
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::live::{Action, Actor, Outcome, SculptTarget, World};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::Tool;
use plateau_engine::{Limits, Material, Region};

const PERSON: f32 = 1.7;
const TINY: f32 = 0.017;
const LAWN: [f32; 3] = [38.0, 0.0, 50.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn open() -> World {
    let mut save = WorldSave::new("The walled garden", 7, Vec::new());
    save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];
    World::open(save, None, &repo_root(), Limits::default(), ModuleCheck::Strict, PERSON).unwrap().0
}

/// A tree placed, carved into, chopped and set down; a hill raised and a cave dug; undone, redone.
fn session() -> (String, [u64; 2]) {
    let mut world = open();
    let mut body = Body::standing(&mut world.combined(), [LAWN[0] as f64 - 3.0, 40.0, LAWN[2] as f64], PERSON).unwrap();
    let mut act = |world: &mut World, action| world.apply(Actor { id: "local", body: &mut body }, action);
    let tree = match act(&mut world, Action::Place { module: "elements/modules/tree.wasm".into(), pos: LAWN, yaw: 0.0, rot: None, scale: 1.0, fitted: false, opening: None, seed: None }) {
        Outcome::Done(s) => s.added[0],
        other => panic!("{other:?}"),
    };
    let at = [LAWN[0], LAWN[1] + 0.3, LAWN[2]];
    let home = Edit::carve(Shape::Sphere { radius: 0.02 }, at, 0.007, None, TINY);
    assert!(act(&mut world, Action::Sculpt { edit: home, hit: at, tool: Tool::Saw, target: SculptTarget::Element(tree) }).is_done());
    let hill = Edit::add(Shape::Sphere { radius: 1.2 }, [LAWN[0] + 4.0, -0.4, LAWN[2]], 0.3, Material::Soil, PERSON);
    assert!(act(&mut world, Action::Sculpt { edit: hill, hit: LAWN, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }).is_done());
    let cave = Edit::carve(Shape::Sphere { radius: 0.3 }, [LAWN[0] + 4.0, 0.5, LAWN[2]], 0.1, None, PERSON);
    assert!(act(&mut world, Action::Sculpt { edit: cave, hit: [LAWN[0] + 4.0, 0.7, LAWN[2]], tool: Tool::Shovel, target: SculptTarget::Terrain { aimed: None } }).is_done());
    let chop = Action::Cut { id: tree, hit: [LAWN[0], LAWN[1] + 0.9, LAWN[2]], dir: [1.0, 0.0, 0.0], tool: Tool::Saw, material: Some(Material::Wood) };
    assert!(act(&mut world, chop).is_done());
    assert!(act(&mut world, Action::Undo).is_done());
    assert!(act(&mut world, Action::Redo).is_done());
    let points = plateau_engine::points_near(LAWN, 3.0, 3000, 11);
    let hashes = [TINY, PERSON].map(|size| {
        let mut out = Vec::new();
        world.combined().sample(&points, size, &mut out).unwrap();
        plateau_engine::hash(&out)
    });
    (world.to_save().to_text(), hashes)
}

/// A subscriber that counts the spans entered, by name: enough to know they are there.
#[derive(Default)]
struct Counting(std::sync::Mutex<std::collections::BTreeMap<&'static str, usize>>, std::sync::atomic::AtomicU64);

impl tracing::Subscriber for Counting {
    fn enabled(&self, _: &tracing::Metadata<'_>) -> bool {
        true
    }
    fn new_span(&self, span: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        *self.0.lock().unwrap().entry(span.metadata().name()).or_default() += 1;
        tracing::span::Id::from_u64(1 + self.1.fetch_add(1, std::sync::atomic::Ordering::Relaxed))
    }
    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}
    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}
    fn event(&self, _: &tracing::Event<'_>) {}
    fn enter(&self, _: &tracing::span::Id) {}
    fn exit(&self, _: &tracing::span::Id) {}
}

/// Tracing times things; it changes nothing: the same session, traced and not, gives the same world.
#[test]
fn tracing_changes_nothing_and_spans_are_there() {
    let counting = std::sync::Arc::new(Counting::default());
    let traced = tracing::subscriber::with_default(counting.clone(), session);
    let plain = session();
    assert_eq!(traced, plain);
    let seen = counting.0.lock().unwrap().clone();
    for name in ["apply", "rules", "tool", "promote", "commit", "spawn_placed", "remove_placed"] {
        assert!(seen.get(name).copied().unwrap_or(0) > 0, "no `{name}` span among {seen:?}");
    }
}
