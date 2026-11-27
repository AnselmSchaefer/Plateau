//! Phase H (`WORLD_PLAN.md`): materials below the surface, whole-brush tool checks, recorded
//! `Edit::tool`, `world` vs `workshop` mode, and the material economy (volume × density yield,
//! consumption on `Add`, carrying weight, undo/redo, save/reload, and replay tool verification).

mod played;

use std::path::PathBuf;

use plateau_engine::body::{self, Body};
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::live::{Action, ActionLog, Actor, Outcome, SculptTarget, World, replay, verify_save_edits};
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::{Tool, WorldMode, material_props};
use plateau_engine::{Limits, Material, Region, SIZES, hash, laws, points_near};

const TINY: f32 = SIZES[0].1;
const PERSON: f32 = SIZES[1].1;
const TREE: &str = "elements/modules/tree.wasm";
const HILL: &str = "elements/modules/hill.wasm";
const LAWN: [f32; 3] = [38.0, 0.0, 50.0];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tools").join(name);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn garden_save(mode: WorldMode) -> WorldSave {
    let mut save = WorldSave::new("The walled garden", 7, Vec::new());
    save.mode = mode;
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

fn sampled(world: &mut World, at: [f32; 3], size: f32) -> u64 {
    let pts = points_near(at, 1.5, 2000, 31);
    let mut out = Vec::new();
    world.combined().sample(&pts, size, &mut out).unwrap();
    hash(&out)
}

#[test]
fn strata_helper_layers_deterministically_by_depth() {
    use plateau_sdk::{Material as SdkMat, strata};

    let layers = [(SdkMat::Grass, 0.08), (SdkMat::Soil, 0.7), (SdkMat::Stone, 0.0)];
    assert_eq!(strata(-1.0, &layers), SdkMat::Grass);
    assert_eq!(strata(0.0, &layers), SdkMat::Grass);
    assert_eq!(strata(0.05, &layers), SdkMat::Grass);
    assert_eq!(strata(0.08, &layers), SdkMat::Soil);
    assert_eq!(strata(0.50, &layers), SdkMat::Soil);
    assert_eq!(strata(0.78, &layers), SdkMat::Stone);
    assert_eq!(strata(100.0, &layers), SdkMat::Stone);
    assert_eq!(strata(f32::NAN, &layers), SdkMat::Grass);
    assert_eq!(strata(0.5, &[]), SdkMat::Stone);
    assert_eq!(strata(0.5, &[(SdkMat::Wood, 0.0)]), SdkMat::Wood);
}

#[test]
fn every_carried_area_keeps_materials_go_down() {
    for (name, _) in areas_played::AREAS {
        let mut native = played::area(name, 7);
        let d = native.describe().unwrap();
        assert_eq!(laws::materials_go_down(&mut native, &d, 1, 400).unwrap(), Ok(()), "{name} (native)");

        let path = repo_root().join(format!("areas/modules/{name}.wasm"));
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let mut wasm = plateau_engine::Wasm::new(&bytes, 7, Limits::default()).unwrap();
        let wd = wasm.describe().unwrap();
        assert_eq!(laws::materials_go_down(&mut wasm, &wd, 1, 400).unwrap(), Ok(()), "{name} (wasm)");
    }
}

#[test]
fn shovel_aimed_at_moss_over_stone_core_is_refused_or_shrunk_naming_stone() {
    let mut world = open(garden_save(WorldMode::World));
    let mut body = person(&mut world);

    // 1. East garden wall at the mossy patch around the crack mouth (x = 53.7, z = 30.0):
    //    surface is Moss (~3 cm skin), core is Stone.
    let mut out = Vec::new();
    let wall_y = 0.08;
    let moss_hit = [53.70, wall_y, 30.0];
    let core_pt = [53.95, wall_y, 30.0];
    world.region.sample(&[moss_hit, core_pt], PERSON, &mut out).unwrap();
    assert_eq!(out[0].material, Material::Moss, "wall face around crack is mossy");
    assert_eq!(out[1].material, Material::Stone, "wall core behind moss is stone");
    assert!(out[1].distance < -0.1, "wall core is solid stone");

    // Person-scale shovel brush (r = 0.45 m) aimed straight at the mossy patch penetrates into stone
    // even at the smallest valid person-scale shrink -> refused, naming stone.
    let big_shovel = Edit::carve(Shape::Sphere { radius: 0.45 }, moss_hit, 0.1, None, PERSON);
    let res = act(&mut world, &mut body, Action::Sculpt {
        edit: big_shovel,
        hit: moss_hit,
        tool: Tool::Shovel,
        target: SculptTarget::Terrain { aimed: None },
    });
    match &res {
        Outcome::Denied(why) => {
            assert!(why.to_lowercase().contains("stone"), "denial must name stone: {why}");
            assert!(why.contains("Shovel") || why.contains("Pickaxe"), "denial names tool: {why}");
        }
        other => panic!("expected person shovel on thin moss over stone to be refused, got {other:?}"),
    }
    assert!(world.edits.is_empty(), "refused stroke adds no edit");

    // Tiny-scale shovel brush (r = 0.05 m) aimed at the mossy face reaches into stone at full size,
    // so the engine shrinks it until it fits inside the 3 cm moss skin (naming stone).
    let tiny_shovel = Edit::carve(Shape::Sphere { radius: 0.05 }, [53.695, wall_y, 30.0], 0.005, None, TINY);
    let mut tiny_body = Body::of_size(TINY);
    tiny_body.feet = [53.6, wall_y as f64, 30.0];
    let tiny_res = act(&mut world, &mut tiny_body, Action::Sculpt {
        edit: tiny_shovel,
        hit: [53.695, wall_y, 30.0],
        tool: Tool::Shovel,
        target: SculptTarget::Terrain { aimed: None },
    });
    match &tiny_res {
        Outcome::Done(summary) => {
            let said = summary.said.as_deref().unwrap_or("");
            assert!(said.to_lowercase().contains("stone") && said.contains("shrunk"), "expected shrunk message naming stone, got: {said}");
            let stored = world.edits.edits().last().unwrap();
            assert!(stored.shape.radius() < 0.05, "stored edit must be shrunk: {:?}", stored.shape);
            assert_eq!(stored.tool, Tool::Shovel);
        }
        other => panic!("expected tiny shovel on 3 cm moss over stone to shrink and succeed, got {other:?}"),
    }

    // 2. On the lawn (Grass 8 cm + Soil 70 cm over Stone bedrock), a deep 1.2 m shovel stroke
    //    hits bedrock and shrinks to stay above the stone (naming stone), whereas Pickaxe digs deep.
    let lawn_hit = [LAWN[0], 0.0, LAWN[2]];
    let deep_shovel = Edit::carve(Shape::Sphere { radius: 1.2 }, lawn_hit, 0.2, None, PERSON);
    let lawn_res = act(&mut world, &mut body, Action::Sculpt {
        edit: deep_shovel,
        hit: lawn_hit,
        tool: Tool::Shovel,
        target: SculptTarget::Terrain { aimed: None },
    });
    match &lawn_res {
        Outcome::Done(summary) => {
            let said = summary.said.as_deref().unwrap_or("");
            assert!(said.to_lowercase().contains("stone") && said.contains("shrunk"), "expected shrunk message naming stone: {said}");
            assert!(world.edits.edits().last().unwrap().shape.radius() < 1.2);
        }
        other => panic!("expected shovel on deep lawn to shrink above stone bedrock, got {other:?}"),
    }

    // Pickaxe digs straight through into the stone bedrock at full 1.2 m radius without shrinking.
    let pick_hit = [LAWN[0] - 6.0, 0.0, LAWN[2]];
    let deep_pick = Edit::carve(Shape::Sphere { radius: 1.2 }, pick_hit, 0.2, None, PERSON);
    let pick_res = act(&mut world, &mut body, Action::Sculpt {
        edit: deep_pick,
        hit: pick_hit,
        tool: Tool::Pickaxe,
        target: SculptTarget::Terrain { aimed: None },
    });
    assert!(pick_res.is_done(), "pickaxe digs soil+stone: {pick_res:?}");
    assert_eq!(world.edits.edits().last().unwrap().shape.radius(), 1.2);
    assert_eq!(world.edits.edits().last().unwrap().tool, Tool::Pickaxe);

    // 3. Also verify standalone Mossy Hill SDF element (mossy shoulder over stone core).
    let placed_hill = act(&mut world, &mut body, Action::Place {
        module: HILL.into(),
        pos: [28.0, 0.0, 50.0],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        fitted: false,
        opening: None,
        seed: Some(7),
    });
    assert!(placed_hill.is_done(), "{placed_hill:?}");
    let hill_hit = body::look(&mut world.combined(), [28.0, 0.5, 54.0], [0.0, 0.0, -1.0], PERSON, 6.0).unwrap().unwrap().map(|c| c as f32);
    let hill_shovel = Edit::carve(Shape::Sphere { radius: 0.5 }, hill_hit, 0.1, None, PERSON);
    let hill_res = act(&mut world, &mut body, Action::Sculpt {
        edit: hill_shovel,
        hit: hill_hit,
        tool: Tool::Shovel,
        target: SculptTarget::Terrain { aimed: None },
    });
    match &hill_res {
        Outcome::Denied(why) => assert!(why.to_lowercase().contains("stone"), "{why}"),
        Outcome::Done(s) => {
            let said = s.said.as_deref().unwrap_or("");
            assert!(said.to_lowercase().contains("stone") && said.contains("shrunk"), "{said}");
        }
        other => panic!("unexpected result on mossy hill: {other:?}"),
    }
}

#[test]
fn saw_cannot_raise_stone_and_world_mode_enforces_rules_and_economy() {
    let mut world = open(garden_save(WorldMode::World));
    let mut body = person(&mut world);

    assert_eq!(world.default_tool(), Tool::Hands);

    // 1. Workshop tool (`Tool::All`) is refused in `world` mode.
    let free_hill = Edit::add(Shape::Sphere { radius: 0.6 }, [LAWN[0], 0.3, LAWN[2]], 0.1, Material::Soil, PERSON);
    assert!(matches!(
        act(&mut world, &mut body, Action::Sculpt { edit: free_hill, hit: LAWN, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }),
        Outcome::Denied(r) if r.contains("workshop")
    ));

    // 2. Carving cannot repaint walls in `world` mode.
    let repaint = Edit::carve(Shape::Sphere { radius: 0.3 }, [LAWN[0], -0.1, LAWN[2]], 0.05, Some(Material::Wood), PERSON);
    assert!(matches!(
        act(&mut world, &mut body, Action::Sculpt { edit: repaint, hit: LAWN, tool: Tool::Shovel, target: SculptTarget::Terrain { aimed: None } }),
        Outcome::Denied(r) if r.contains("repaint")
    ));

    // 3. Digging stone with Pickaxe yields stone mass (volume × density) into inventory.
    let stone_hit = [53.7, 1.0, 26.0]; // bare stone east wall
    let quarry = Edit::carve(Shape::Sphere { radius: 0.5 }, [53.95, 1.0, 26.0], 0.05, None, PERSON);
    let dug = act(&mut world, &mut body, Action::Sculpt {
        edit: quarry,
        hit: stone_hit,
        tool: Tool::Pickaxe,
        target: SculptTarget::Terrain { aimed: None },
    });
    assert!(dug.is_done(), "pickaxe quarries stone: {dug:?}");
    let stone_kg = world.inventory("local").get(Material::Stone);
    let expected_max = quarry.shape.volume_m3() * material_props(Material::Stone).density_kg_m3;
    assert!(stone_kg > 50.0 && stone_kg <= expected_max + 1.0, "expected stone yield in (50..={expected_max}], got {stone_kg}");

    // 4. Even WITH stone in inventory, a Saw CANNOT raise stone in `world` mode!
    let stone_mound = Edit::add(Shape::Sphere { radius: 0.25 }, [LAWN[0], 0.2, LAWN[2]], 0.05, Material::Stone, PERSON);
    let saw_add = act(&mut world, &mut body, Action::Sculpt {
        edit: stone_mound,
        hit: LAWN,
        tool: Tool::Saw,
        target: SculptTarget::Terrain { aimed: None },
    });
    match &saw_add {
        Outcome::Denied(why) => assert!(why.contains("Saw") && why.to_lowercase().contains("stone"), "unexpected saw denial: {why}"),
        other => panic!("Saw must not raise stone in world mode: {other:?}"),
    }

    // 5. Bare hands cannot raise stone either.
    assert!(matches!(
        act(&mut world, &mut body, Action::Sculpt { edit: stone_mound, hit: LAWN, tool: Tool::Hands, target: SculptTarget::Terrain { aimed: None } }),
        Outcome::Denied(r) if r.contains("hands") || r.contains("Hands")
    ));

    // 6. Pickaxe CAN raise stone using carried stone — and consumes it from inventory!
    let add_ok = act(&mut world, &mut body, Action::Sculpt {
        edit: stone_mound,
        hit: LAWN,
        tool: Tool::Pickaxe,
        target: SculptTarget::Terrain { aimed: None },
    });
    assert!(add_ok.is_done(), "pickaxe raises stone from inventory: {add_ok:?}");
    let stone_after = world.inventory("local").get(Material::Stone);
    assert!(stone_after < stone_kg - 10.0, "adding stone must consume carried stone ({stone_kg} -> {stone_after})");

    // 7. Raising more stone than carried is refused naming stone.
    let huge_stone = Edit::add(Shape::Sphere { radius: 2.0 }, [LAWN[0] + 5.0, 1.0, LAWN[2]], 0.1, Material::Stone, PERSON);
    let no_mat = act(&mut world, &mut body, Action::Sculpt {
        edit: huge_stone,
        hit: [LAWN[0] + 5.0, 0.0, LAWN[2]],
        tool: Tool::Pickaxe,
        target: SculptTarget::Terrain { aimed: None },
    });
    assert!(matches!(&no_mat, Outcome::Denied(r) if r.contains("not enough stone")), "{no_mat:?}");

    // 8. Undo/redo restores inventory bit-for-bit.
    assert!(act(&mut world, &mut body, Action::Undo).is_done());
    assert!((world.inventory("local").get(Material::Stone) - stone_kg).abs() < 1e-4, "undo add restores stone");
    assert!(act(&mut world, &mut body, Action::Undo).is_done());
    assert_eq!(world.inventory("local").get(Material::Stone), 0.0, "undo carve removes stone");
    assert!(act(&mut world, &mut body, Action::Redo).is_done());
    assert!((world.inventory("local").get(Material::Stone) - stone_kg).abs() < 1e-4, "redo carve restores stone");
    assert!(act(&mut world, &mut body, Action::Redo).is_done());
    assert!((world.inventory("local").get(Material::Stone) - stone_after).abs() < 1e-4, "redo add consumes stone");

    // 9. Switching to `workshop` mode through `World::apply` allows wall repainting & free `Add`.
    assert!(act(&mut world, &mut body, Action::SetMode { mode: WorldMode::Workshop }).is_done());
    assert_eq!(world.default_tool(), Tool::All);
    assert!(act(&mut world, &mut body, Action::Sculpt { edit: repaint, hit: LAWN, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }).is_done());
    assert!(act(&mut world, &mut body, Action::Sculpt { edit: huge_stone, hit: [LAWN[0] + 5.0, 0.0, LAWN[2]], tool: Tool::All, target: SculptTarget::Terrain { aimed: None } }).is_done());
}

#[test]
fn carrying_weight_counts_carried_material_pack() {
    let mut world = open(garden_save(WorldMode::World));
    let mut body = person(&mut world);

    // Place a tree and chop it so we have a carryable log (~400-700 kg, well below person's 1400 kg limit).
    let tree = match act(&mut world, &mut body, Action::Place {
        module: TREE.into(),
        pos: LAWN,
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        fitted: false,
        opening: None,
        seed: Some(11),
    }) {
        Outcome::Done(s) => s.added[0],
        other => panic!("place tree failed: {other:?}"),
    };
    let pos = world.find(tree).unwrap().pos;
    let trunk = match act(&mut world, &mut body, Action::Cut {
        id: tree,
        hit: [pos[0], pos[1] + 0.9, pos[2]],
        dir: [1.0, 0.0, 0.0],
        tool: Tool::Saw,
        material: Some(Material::Wood),
    }) {
        Outcome::Done(s) => s.added[1],
        other => panic!("cut failed: {other:?}"),
    };
    world.finish_all_falls(None);

    // Empty pack: full walk speed and log is easily carried.
    assert_eq!(world.carry_speed_factor("local", PERSON), 1.0);
    assert!(act(&mut world, &mut body, Action::Grab { id: trunk }).is_done());
    assert!(act(&mut world, &mut body, Action::Return).is_done());

    // Quarry heavy stone from the east wall until pack + log exceeds person carry capacity (1400 kg).
    for k in 0..4 {
        let z = 22.0 + k as f32 * 1.4;
        let quarry = Edit::carve(Shape::Sphere { radius: 0.55 }, [53.95, 1.0, z], 0.02, None, PERSON);
        assert!(act(&mut world, &mut body, Action::Sculpt {
            edit: quarry,
            hit: [53.7, 1.0, z],
            tool: Tool::Pickaxe,
            target: SculptTarget::Terrain { aimed: None },
        }).is_done());
    }
    let pack_kg = world.inventory_mass_kg("local");
    assert!(pack_kg > 900.0, "expected heavy stone pack (> 900 kg), got {pack_kg}");
    assert!(world.carry_speed_factor("local", PERSON) < 0.80, "heavy pack slows walking pace");

    // Trying to grab the log while laden with stone is refused due to combined weight!
    let grab_heavy = act(&mut world, &mut body, Action::Grab { id: trunk });
    match &grab_heavy {
        Outcome::Denied(why) => assert!(why.contains("pack") || why.contains("heavy"), "expected heavy pack denial: {why}"),
        other => panic!("expected heavy pack to block grabbing log, got {other:?}"),
    }
}

#[test]
fn save_and_action_log_replay_check_every_edits_tool_and_round_trip_deterministically() {
    let dir = scratch("replay");
    let save_path = dir.join("world.plateau");
    let log_path = dir.join("session.jsonl");

    let start = garden_save(WorldMode::World);
    let mut world = open(start.clone());
    world.set_log(Some(ActionLog::create(&log_path).unwrap()));
    let mut body = person(&mut world);

    // 1. Dig soil on the lawn with Shovel.
    let dig_soil = Edit::carve(Shape::Sphere { radius: 0.35 }, [LAWN[0], -0.05, LAWN[2]], 0.05, None, PERSON);
    assert!(act(&mut world, &mut body, Action::Sculpt {
        edit: dig_soil,
        hit: LAWN,
        tool: Tool::Shovel,
        target: SculptTarget::Terrain { aimed: None },
    }).is_done());

    // 2. Raise a small soil mound with Shovel using the dug soil.
    let add_soil = Edit::add(Shape::Sphere { radius: 0.25 }, [LAWN[0] + 2.0, 0.15, LAWN[2]], 0.05, Material::Soil, PERSON);
    assert!(act(&mut world, &mut body, Action::Sculpt {
        edit: add_soil,
        hit: [LAWN[0] + 2.0, 0.0, LAWN[2]],
        tool: Tool::Shovel,
        target: SculptTarget::Terrain { aimed: None },
    }).is_done());

    // 3. Quarry stone from the east wall with Pickaxe.
    let quarry = Edit::carve(Shape::Sphere { radius: 0.4 }, [53.95, 1.1, 24.0], 0.05, None, PERSON);
    assert!(act(&mut world, &mut body, Action::Sculpt {
        edit: quarry,
        hit: [53.7, 1.1, 24.0],
        tool: Tool::Pickaxe,
        target: SculptTarget::Terrain { aimed: None },
    }).is_done());

    // 4. Raise a small stone block with Pickaxe.
    let add_stone = Edit::add(Shape::Box { half: [0.15, 0.15, 0.15], round: 0.02 }, [LAWN[0] + 4.0, 0.15, LAWN[2]], 0.0, Material::Stone, PERSON);
    assert!(act(&mut world, &mut body, Action::Sculpt {
        edit: add_stone,
        hit: [LAWN[0] + 4.0, 0.0, LAWN[2]],
        tool: Tool::Pickaxe,
        target: SculptTarget::Terrain { aimed: None },
    }).is_done());

    // 5. Place a tree and carve a tiny home into it with Saw.
    let tree = match act(&mut world, &mut body, Action::Place {
        module: TREE.into(),
        pos: [LAWN[0] - 4.0, 0.0, LAWN[2]],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        fitted: false,
        opening: None,
        seed: Some(7),
    }) {
        Outcome::Done(s) => s.added[0],
        other => panic!("place tree failed: {other:?}"),
    };
    let tpos = world.find(tree).unwrap().pos;
    let home = Edit::carve(Shape::Sphere { radius: 0.02 }, [tpos[0], tpos[1] + 0.3, tpos[2]], 0.005, None, TINY);
    assert!(act(&mut world, &mut body, Action::Sculpt {
        edit: home,
        hit: home.center,
        tool: Tool::Saw,
        target: SculptTarget::Element(tree),
    }).is_done());

    // 6. Save and verify bit-exact text round-trip (mode, tools, inventories).
    world.set_log(None);
    world.save(&save_path).unwrap();
    let saved_text = std::fs::read_to_string(&save_path).unwrap();
    assert!(saved_text.contains("mode world\n"), "{saved_text}");
    assert!(saved_text.contains("tool shovel"), "{saved_text}");
    assert!(saved_text.contains("tool pickaxe"), "{saved_text}");
    assert!(saved_text.contains("tool saw"), "{saved_text}");
    assert!(saved_text.contains("inv local "), "{saved_text}");

    let loaded = WorldSave::load(&save_path).unwrap();
    assert_eq!(loaded.to_text(), saved_text, "save round-trips bit-exactly");
    assert_eq!(loaded.mode, WorldMode::World);
    assert_eq!(loaded.edits[0].tool, Tool::Shovel);
    assert_eq!(loaded.edits[1].tool, Tool::Shovel);
    assert_eq!(loaded.edits[2].tool, Tool::Pickaxe);
    assert_eq!(loaded.edits[3].tool, Tool::Pickaxe);
    assert_eq!(loaded.elements[0].local_edits[0].tool, Tool::Saw);

    // 7. Replaying the save verifies every edit's tool cleanly.
    assert_eq!(
        verify_save_edits(&loaded, None, &repo_root(), Limits::default(), ModuleCheck::Strict).unwrap(),
        Ok(())
    );

    // 8. Tampering with recorded tools in the save is caught by `verify_save_edits`!
    // (a) Forge stone quarry as `tool shovel` -> caught naming edit #2 and stone.
    let bad_carve_text = saved_text.replacen("tool pickaxe", "tool shovel", 1);
    let bad_carve = WorldSave::from_text(&bad_carve_text).unwrap();
    let err = verify_save_edits(&bad_carve, None, &repo_root(), Limits::default(), ModuleCheck::Strict)
        .unwrap()
        .expect_err("forged shovel on stone quarry must be refused");
    assert!(err.contains("edit #2") && err.to_lowercase().contains("stone"), "{err}");

    // (b) Forge stone add as `tool saw` -> caught naming edit #3, Saw, and stone.
    let mut bad_add = loaded.clone();
    bad_add.edits[3].tool = Tool::Saw;
    let err = verify_save_edits(&bad_add, None, &repo_root(), Limits::default(), ModuleCheck::Strict)
        .unwrap()
        .expect_err("forged saw raising stone must be refused");
    assert!(err.contains("edit #3") && err.contains("Saw") && err.to_lowercase().contains("stone"), "{err}");

    // (c) Forge wall repaint on carve in `world` mode -> caught.
    let mut bad_paint = loaded.clone();
    bad_paint.edits[0].material = Some(Material::Wood);
    let err = verify_save_edits(&bad_paint, None, &repo_root(), Limits::default(), ModuleCheck::Strict)
        .unwrap()
        .expect_err("forged wall repaint in world mode must be refused");
    assert!(err.contains("edit #0") && err.contains("repaint"), "{err}");

    // (d) Forge `tool all` in `world` mode -> caught.
    let mut bad_all = loaded.clone();
    bad_all.edits[0].tool = Tool::All;
    let err = verify_save_edits(&bad_all, None, &repo_root(), Limits::default(), ModuleCheck::Strict)
        .unwrap()
        .expect_err("forged workshop tool in world mode must be refused");
    assert!(err.contains("edit #0") && err.contains("workshop"), "{err}");

    // (e) Forge element local edit on wooden tree as `tool shovel` -> caught.
    let mut bad_el = loaded.clone();
    bad_el.elements[0].local_edits[0].tool = Tool::Shovel;
    let err = verify_save_edits(&bad_el, None, &repo_root(), Limits::default(), ModuleCheck::Strict)
        .unwrap()
        .expect_err("forged shovel on wooden tree must be refused");
    assert!(err.contains("element #") && err.to_lowercase().contains("wood"), "{err}");

    // 9. Action log replay produces bit-identical save, inventories, and SDF hashes —
    //    and matches canonical-NaN execution bit-for-bit.
    let log = ActionLog::read(&log_path).unwrap();
    let mut replayed = open(start.clone());
    replay(&mut replayed, &log).unwrap();
    let mut replayed_save = replayed.to_save();
    replayed_save.seal(&repo_root()).unwrap();
    assert_eq!(replayed_save.to_text(), world.to_save().to_text());
    assert_eq!(replayed.inventories, world.inventories);
    for size in [TINY, PERSON] {
        assert_eq!(sampled(&mut replayed, LAWN, size), sampled(&mut world, LAWN, size));
    }

    let mut canon = World::open(
        start,
        None,
        &repo_root(),
        Limits { canonical_nans: true, ..Limits::default() },
        ModuleCheck::Strict,
        PERSON,
    )
    .unwrap()
    .0;
    replay(&mut canon, &log).unwrap();
    let mut canon_save = canon.to_save();
    canon_save.seal(&repo_root()).unwrap();
    assert_eq!(canon_save.to_text(), world.to_save().to_text(), "canonical NaNs match bit-for-bit");
    assert_eq!(sampled(&mut canon, LAWN, PERSON), sampled(&mut world, LAWN, PERSON));
}

#[test]
fn legacy_saves_without_tool_mode_or_inv_still_load_cleanly() {
    let legacy_v1 = "plateau-world 1\narea The walled garden\nseed 7\n\
                     sdf add sphere 32 0 48 1.5 blend 0.3 mat soil size 1.7\n\
                     sdf carve sphere 32 0.2 48 0.4 blend 0.1 mat none size 1.7 author local\n";
    let save = WorldSave::from_text(legacy_v1).unwrap();
    assert_eq!(save.version, 1);
    assert_eq!(save.mode, WorldMode::Workshop);
    assert!(save.inventories.is_empty());
    assert_eq!(save.edits.len(), 2);
    assert_eq!(save.edits[0].tool, Tool::All);
    assert_eq!(save.edits[1].tool, Tool::All);
    assert_eq!(save.edit_authors, vec![None, Some("local".into())]);

    // Written back as v2 with explicit `tool all`, which round-trips cleanly.
    let v2_text = save.to_text();
    assert!(v2_text.starts_with("plateau-world 2\n"));
    assert!(v2_text.contains("tool all"));
    let again = WorldSave::from_text(&v2_text).unwrap();
    assert_eq!(again.to_text(), v2_text);
}
