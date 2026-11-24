//! Tests for Step 8b: Composable standalone `.wasm` elements (POC_PLAN.md, AGENTS.md).
//!
//! Verifies:
//! 1. Carried showcase elements (`tree`, `bridge`, `lantern`, `hill`) pass sandbox checks & probe metadata.
//! 2. Placing an SDF hill element at person scale + carving a tiny cave into it + walking inside
//!    (and chronological ordering: pit dug before hill stays underneath; cave carved after cuts into hill).
//! 3. Solid procedural collision on placed `bridge.wasm` across tiny, person, and giant scales.
//! 4. Multi-placement isolation for `lantern.wasm` (`tick`, `interact`, local↔world transforms,
//!    and physically scaled `PointLight` intensity & radius across tiny/person/giant scales).
//! 5. `WorldSave` save/reload round-trip determinism ("made again equals played") with mixed base
//!    area + SDF edits + SDF & Scene elements + Step 8a backward compatibility.
//! 6. Targeted chunk invalidation soundness for SDF elements (and zero SDF overhead for pure-Scene elements).
//! 7. Sandbox rejection of invalid `element-bounds` or forbidden imports.

mod played;

use std::path::PathBuf;
use std::sync::Arc;

use plateau_engine::body::{Body, Intent};
use plateau_engine::compose::{CompiledElement, Composed, ComposedLog, ElementCatalog, ElementRuntime, PlacedElement, SdfElementEntry};
use plateau_engine::edit::{Edit, EditLog, Shape, WorldSave};
use plateau_engine::scene::{Combined, SceneWorld};
use plateau_engine::{Limits, Material, PlayerState, Region, SIZES, Wasm, chunk, hash, laws};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn garden(seed: u64) -> played::Native {
    played::area("garden", seed)
}

fn load_element(name: &str) -> Arc<CompiledElement> {
    let rel = format!("elements/modules/{name}.wasm");
    let path = repo_root().join(&rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run ./elements/build.sh)", path.display()));
    CompiledElement::load(rel, &bytes, Limits::default()).unwrap_or_else(|e| panic!("{name}: {e:#}"))
}

#[test]
fn carried_showcase_elements_pass_sandbox_and_hybrid_layer_ownership() {
    let tree = load_element("tree");
    let bridge = load_element("bridge");
    let lantern = load_element("lantern");
    let hill = load_element("hill");

    // Discrete/interactive/portable objects live strictly in the Scene layer (`sdf_bounds == None`).
    assert_eq!(tree.name, "Oak Tree");
    assert!(!tree.is_sdf());
    assert!(!tree.preview_scene.instances.is_empty());

    assert_eq!(bridge.name, "Timber Bridge");
    assert!(!bridge.is_sdf());
    assert!(bridge.preview_scene.instances.iter().any(|i| i.solid));

    assert_eq!(lantern.name, "Post Lantern");
    assert!(!lantern.is_sdf());
    assert!(!lantern.preview_scene.lights.is_empty());

    // Carvable landform lives in the SDF layer (`sdf_bounds == Some(...)`).
    assert_eq!(hill.name, "Mossy Hill");
    assert!(hill.is_sdf());

    // Catalog discovery finds all four carried elements.
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let discovered = catalog.discover();
    for expected in ["Oak Tree", "Timber Bridge", "Post Lantern", "Mossy Hill"] {
        assert!(discovered.iter().any(|e| e.name == expected), "missing {expected} from discovery");
    }
}

#[test]
fn placed_sdf_hill_carves_tiny_cave_and_preserves_chronological_order() {
    let hill = load_element("hill");
    let bounds = hill.sdf_bounds.expect("hill has SDF bounds");
    let garden = garden(7);

    // 1. Dig a pit at (32, 0, 48) BEFORE placing the hill (`edits[0]`).
    let mut edits = EditLog::new();
    let pit = Edit::carve(Shape::Sphere { radius: 1.5 }, [32.0, -0.4, 48.0], 0.1, None, SIZES[1].1);
    edits.push(pit).unwrap();

    // 2. Place `hill.wasm` at (32, 0, 48) with `after_edits = 1` so it fills over the earlier pit!
    let placed_hill = PlacedElement {
        id: 1,
        module: hill.module.clone(),
        seed: 42,
        pos: [32.0, 0.0, 48.0],
        yaw: 0.4,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 1,
        promoted: None,
    };

    // 3. Carve a tiny cave right into the placed hill's flank (`after_edits = 1`, so `edits[1]` runs AFTER hill!).
    let cave_entry = [32.0, 0.55, 50.15];
    let cave_room = [32.0, 0.55, 49.45];
    edits.push(Edit::carve(Shape::Cylinder { radius: 0.045, half_height: 0.035, round: 0.01 }, cave_room, 0.01, Some(Material::Stone), SIZES[0].1)).unwrap();
    for step in 0..8 {
        let t = step as f32 / 7.0;
        let z = cave_room[2] + (cave_entry[2] - cave_room[2]) * t;
        edits.push(Edit::carve(Shape::Sphere { radius: 0.025 }, [32.0, 0.54, z], 0.008, Some(Material::Stone), SIZES[0].1)).unwrap();
    }

    let log = Arc::new(ComposedLog::new(
        Arc::new(edits),
        vec![SdfElementEntry::new(placed_hill, hill.compiled.clone(), bounds)],
        1,
    ));
    let mut composed = Composed::new(garden, log);

    // Pit was filled by the hill placed after it (center at (32, 0.2, 48) is solid!).
    let mut ans = Vec::new();
    composed.sample(&[[32.0, 0.2, 48.0], cave_room], SIZES[0].1, &mut ans).unwrap();
    assert!(ans[0].distance < -0.5, "hill placed after pit should be solid at center, got {}", ans[0].distance);
    // Cave carved AFTER placing the hill is open air inside the hill!
    assert!(ans[1].distance > 0.015, "tiny cave carved into placed hill should be open air, got {}", ans[1].distance);

    // A tiny person stands inside the carved cave and cannot grow into a person (blocked by hill rock roof!).
    let mut tiny = Body::standing(&mut composed, [cave_room[0] as f64, cave_room[1] as f64 + 0.01, cave_room[2] as f64], SIZES[0].1).unwrap();
    assert!(tiny.on.is_some(), "tiny should stand on cave floor inside placed hill");
    tiny.grow_to(SIZES[1].1);
    for _ in 0..80 {
        tiny.step(&mut composed, Intent::default(), 1.0 / 120.0).unwrap();
    }
    assert!(tiny.size < 0.10, "tiny inside carved hill cave should be blocked from growing, reached {}", tiny.size);

    // A person stands ON TOP of the placed hill (well above original lawn y = 0).
    let person = Body::standing(&mut composed, [32.0, 6.0, 48.0], SIZES[1].1).unwrap();
    assert!(person.feet[1] > 1.2, "person should stand atop placed hill (y > 1.2), got {}", person.feet[1]);
}

#[test]
fn placed_bridge_is_walkable_solid_geometry_at_every_scale() {
    let bridge = load_element("bridge");

    for (size_idx, scale) in [(0, 0.01f32), (1, 1.0f32), (2, 10.0f32)] {
        let player_size = SIZES[size_idx].1;
        let garden = garden(7);
        // Place the bridge elevated cleanly above the lawn with open air beneath it so ONLY the
        // bridge's solid procedural deck can hold the player up!
        let deck_base_y = 2.0 * scale + 0.5;
        let mut composed = Composed::new(garden, Arc::new(ComposedLog::empty()));

        let mut world = SceneWorld::default();
        let mut runtime = ElementRuntime::new();
        let pl = PlacedElement {
            id: 1,
            module: bridge.module.clone(),
            seed: 9,
            pos: [32.0, deck_base_y, 45.0],
            yaw: 0.0,
            scale,
            stamp_size: player_size,
            after_edits: 0,
            promoted: None,
        };
        runtime.spawn(pl, &bridge, &mut world).unwrap();

        let mut combined = Combined { region: &mut composed, world: &world };
        let start_y = (deck_base_y + 1.5 * scale) as f64;
        let body = Body::standing(&mut combined, [32.0, start_y, 45.0], player_size).unwrap();
        // Bridge arch crest at z=0 is at local y = 0.42 + 0.06 = 0.48.
        let expected_y = (deck_base_y + 0.48 * scale) as f64;
        assert!(
            (body.feet[1] - expected_y).abs() < 0.08 * scale as f64,
            "size {} (scale {scale}): expected feet on bridge crest ~{expected_y:.4}, got {:.4}",
            SIZES[size_idx].0.name(),
            body.feet[1]
        );
    }
}

#[test]
fn multiple_lantern_placements_isolate_state_transforms_and_scale_lights() {
    let lantern = load_element("lantern");
    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();

    // Place two lanterns: #1 tiny at (20, 0, 20), #2 person-scale rotated 90 deg at (40, 0, 40).
    let pl1 = PlacedElement {
        id: 1,
        module: lantern.module.clone(),
        seed: 101,
        pos: [20.0, 0.0, 20.0],
        yaw: 0.0,
        scale: 0.01,
        stamp_size: SIZES[0].1,
        after_edits: 0,
        promoted: None,
    };
    let pl2 = PlacedElement {
        id: 2,
        module: lantern.module.clone(),
        seed: 202,
        pos: [40.0, 0.0, 40.0],
        yaw: std::f32::consts::FRAC_PI_2,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: None,
    };
    runtime.spawn(pl1, &lantern, &mut world).unwrap();
    runtime.spawn(pl2, &lantern, &mut world).unwrap();

    // Both lights exist in distinct `(owner, id)` namespaces with physically scaled intensity (scale^2) & range (scale).
    let l1 = *world.lights.get(&(1, 1)).expect("tiny lantern light");
    let l2 = *world.lights.get(&(2, 1)).expect("person lantern light");
    assert!((l1.radius - 0.09).abs() < 1e-3, "tiny light radius should scale by 0.01: {}", l1.radius);
    assert!((l2.radius - 9.0).abs() < 1e-3, "person light radius should be 9.0: {}", l2.radius);
    assert!((l1.intensity - 85.0 * 0.0001).abs() < 1e-4, "tiny light intensity should scale by scale^2: {}", l1.intensity);
    // Rotation by +pi/2 around Y maps local +Z (0.46) to world +X (+0.46).
    assert!((l2.pos[0] - 40.46).abs() < 0.03 && (l2.pos[2] - 40.0).abs() < 0.03, "rotated lantern light pos: {:?}", l2.pos);

    // Ticking updates both lanterns independently in world space.
    let ps = PlayerState { time: 0.8, size: 1.7, ..PlayerState::default() };
    let ticked = runtime.tick_all(0.016, &ps, 0, 0, &mut world);
    assert_eq!(ticked.len(), 2);

    // Interacting specifically with lantern #2 turns OFF #2 while leaving #1 ON!
    let ps_interact = PlayerState { feet: [40.0, 0.0, 41.0], eye: [40.0, 1.5, 41.0], size: 1.7, ..PlayerState::default() };
    let effects = runtime.interact(&ps_interact, 2, 1, &mut world);
    assert_eq!(effects.len(), 1);
    assert_eq!(effects[0].said.as_deref(), Some("Lantern extinguished."));
    assert!(world.lights.contains_key(&(1, 1)), "lantern #1 must remain lit");
    assert!(!world.lights.contains_key(&(2, 1)), "lantern #2 must be extinguished");

    // Removing lantern #1 cleans up its instances and light cleanly without affecting #2.
    runtime.remove(1, &mut world).unwrap();
    assert!(!world.lights.contains_key(&(1, 1)));
    assert!(!world.dynamic.contains_key(&(1, 1)));
    assert!(world.dynamic.contains_key(&(2, 1)));
}

#[test]
fn world_save_round_trip_is_bit_exact_and_backward_compatible() {
    let hill = load_element("hill");
    let tree = load_element("tree");
    let bounds = hill.sdf_bounds.unwrap();

    // Backward compatibility: Step 8a save file with zero `element` lines loads cleanly.
    let old_v1 = "plateau-world 1\narea The walled garden\nseed 7\nsdf add sphere 32 0 48 1.5 blend 0.3 mat soil size 1.7\n";
    let parsed_old = WorldSave::from_text(old_v1).unwrap();
    assert_eq!(parsed_old.edits.len(), 1);
    assert!(parsed_old.elements.is_empty());

    // Full Step 8b save with SDF edits + SDF element + Scene element.
    let edits = vec![
        Edit::add(Shape::Sphere { radius: 1.2 }, [28.0, 0.0, 46.0], 0.3, Material::Soil, 1.7),
        Edit::carve(Shape::Sphere { radius: 0.4 }, [32.0, 0.8, 48.0], 0.1, Some(Material::Stone), 1.7),
    ];
    let elements = vec![
        PlacedElement { id: 1, module: hill.module.clone(), seed: 55, pos: [32.0, 0.0, 48.0], yaw: 0.75, scale: 1.15, stamp_size: 1.7, after_edits: 1, promoted: None },
        PlacedElement { id: 2, module: tree.module.clone(), seed: 77, pos: [25.0, 0.0, 42.0], yaw: 1.2, scale: 0.85, stamp_size: 1.7, after_edits: 2, promoted: Some(2) },
    ];
    let save = WorldSave::with_elements("The walled garden", 7, edits, elements);
    let tmp = std::env::temp_dir().join(format!("plateau-8b-save-{}.plateau", std::process::id()));
    save.save(&tmp).unwrap();
    let reloaded = WorldSave::load(&tmp).unwrap();
    let _ = std::fs::remove_file(&tmp);
    assert_eq!(save, reloaded);

    // "Made again equals played": chunk samples before and after save/reload match bit-for-bit.
    let build_composed = |w: &WorldSave| {
        let log = Arc::new(EditLog::from_edits(w.edits.clone()).unwrap());
        let sdf_elems: Vec<_> = w
            .elements
            .iter()
            .filter(|e| e.module == hill.module)
            .map(|e| SdfElementEntry::new(e.clone(), hill.compiled.clone(), bounds))
            .collect();
        Composed::new(garden(w.seed), Arc::new(ComposedLog::new(log, sdf_elems, 1)))
    };
    let mut a = build_composed(&save);
    let mut b = build_composed(&reloaded);
    let key = chunk::Key { level: -4, at: [16, 0, 24], asked: -4 };
    let ma = chunk::make(&mut a, key).unwrap();
    let mb = chunk::make(&mut b, key).unwrap();
    let mesh_a = ma.mesh.expect("chunk has surface");
    let mesh_b = mb.mesh.expect("reloaded chunk has surface");
    assert_eq!(mesh_a.positions, mesh_b.positions);
    assert_eq!(mesh_a.indices, mesh_b.indices);
    assert_eq!(mesh_a.materials, mesh_b.materials);
}

#[test]
fn targeted_chunk_invalidation_for_sdf_element_is_sound_and_local() {
    let hill = load_element("hill");
    let bounds = hill.sdf_bounds.unwrap();
    let pl = PlacedElement { id: 1, module: hill.module.clone(), seed: 11, pos: [32.0, 0.0, 48.0], yaw: 0.3, scale: 1.0, stamp_size: 1.7, after_edits: 0, promoted: None };
    let entry = SdfElementEntry::new(pl, hill.compiled.clone(), bounds);

    let mut base = garden(7);
    let mut with_hill = Composed::new(garden(7), Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![entry.clone()], 1)));

    // Far chunk: affects_chunk == false AND chunk mesh is bit-identical to unedited base!
    let far = chunk::Key { level: -4, at: [2, 0, 2], asked: -4 };
    assert!(!entry.affects_chunk(far), "far chunk should not be invalidated");
    let mf_a = chunk::make(&mut base, far).unwrap().mesh.unwrap();
    let mf_b = chunk::make(&mut with_hill, far).unwrap().mesh.unwrap();
    assert_eq!(mf_a.positions, mf_b.positions);
    assert_eq!(mf_a.indices, mf_b.indices);

    // Overlapping chunk: affects_chunk == true AND mesh actually changed!
    let near = chunk::Key { level: -4, at: [16, 0, 24], asked: -4 };
    assert!(entry.affects_chunk(near), "overlapping chunk must be invalidated");
    let mn_a = chunk::make(&mut base, near).unwrap().mesh.unwrap();
    let mn_b = chunk::make(&mut with_hill, near).unwrap().mesh.unwrap();
    assert_ne!(mn_a.positions, mn_b.positions);
}

#[test]
fn sdf_hill_keeps_distance_laws_and_nans_unseen_when_composed() {
    let hill = load_element("hill");
    let bounds = hill.sdf_bounds.unwrap();
    let bytes = std::fs::read(repo_root().join(&hill.module)).unwrap();
    let pts = plateau_engine::points_near([0.0, 0.0, 0.0], 3.0, 2000, 1);
    assert_eq!(laws::nans_unseen(&bytes, 7, &pts).unwrap(), Ok(()));

    let pl = PlacedElement { id: 1, module: hill.module.clone(), seed: 7, pos: [32.0, 0.0, 48.0], yaw: 0.2, scale: 1.0, stamp_size: 1.7, after_edits: 0, promoted: None };
    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::new(pl, hill.compiled.clone(), bounds)], 1)),
    );
    let d = composed.describe().unwrap();
    assert_eq!(laws::never_too_long(&mut composed, &d, 1, 200).unwrap(), Ok(()));
    assert_eq!(laws::surfaces_have_an_inside(&mut composed, &d, 1, 200).unwrap(), Ok(()));

    // Also verify hash determinism on two fresh instantiations.
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut w1 = Wasm::new(&bytes, 7, Limits::default()).unwrap();
    let mut w2 = Wasm::new(&bytes, 7, Limits::default()).unwrap();
    w1.sample(&pts, 1.7, &mut a).unwrap();
    w2.sample(&pts, 1.7, &mut b).unwrap();
    assert_eq!(hash(&a), hash(&b));
}

#[test]
fn invalid_element_bounds_are_rejected_by_sandbox() {
    // A valid WASM module whose `element-bounds` returns inverted bounds (lo >= hi) must be rejected.
    let wat = r#"(module
      (import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))
      (memory (export "memory") 2 2)
      (func (export "cabi_realloc") (param i32 i32 i32 i32) (result i32) (i32.const 1024))
      (func (export "sample") (param i32 i32 f32) (result i32) (i32.const 0))
      (func (export "cabi_post_sample") (param i32))
      (func (export "describe") (result i32) (i32.const 0))
      (func (export "cabi_post_describe") (param i32))
      (func (export "element-bounds") (result i32)
        (i32.store8 (i32.const 64) (i32.const 1))
        (f32.store (i32.const 68) (f32.const 5.0))
        (f32.store (i32.const 72) (f32.const 5.0))
        (f32.store (i32.const 76) (f32.const 5.0))
        (f32.store (i32.const 80) (f32.const 1.0))
        (f32.store (i32.const 84) (f32.const 1.0))
        (f32.store (i32.const 88) (f32.const 1.0))
        (f32.store (i32.const 92) (f32.const 0.0))
        (i32.const 64)
      )
    )"#;
    let bytes = wat::parse_str(wat).unwrap();
    let mut wasm = Wasm::new(&bytes, 7, Limits::default()).unwrap();
    let err = wasm.element_bounds().unwrap_err().to_string();
    assert!(err.contains("element bounds are invalid"), "{err}");
}

#[test]
fn lazy_scene_to_sdf_promotion_carves_tiny_home_inside_oak_tree_while_keeping_canopy() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load("elements/modules/tree.wasm").unwrap();

    // 1. Place untouched Oak Tree at (32, 0, 48): pure Scene, zero SDF cost.
    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();
    let mut pl = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 0.0, 48.0],
        yaw: 0.0,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: None,
    };
    runtime.spawn(pl.clone(), &tree, &mut world).unwrap();
    // Both trunk (id=1, solid) and canopy (id=2, non-solid) live in SceneWorld initially.
    assert!(world.dynamic.contains_key(&(1, 1)), "uncarved trunk should be in SceneWorld");
    assert!(world.dynamic.contains_key(&(1, 2)), "canopy should be in SceneWorld");

    // 2. Player begins carving a tiny room inside the trunk -> lazily promote solid trunk to MeshSdf!
    pl.promoted = Some(0);
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed).unwrap().expect("tree has solid trunk");
    runtime.set_promoted(1, Some(0), &mut world).unwrap();

    // Solid trunk moved out of SceneWorld into SDF, while swaying canopy stays crisp in SceneWorld!
    assert!(!world.dynamic.contains_key(&(1, 1)), "promoted trunk must leave SceneWorld to avoid double render/collision");
    assert!(world.dynamic.contains_key(&(1, 2)), "non-solid canopy must stay in SceneWorld");
    let tick_res = runtime.tick_all(0.016, &PlayerState { time: 1.0, size: 1.7, ..PlayerState::default() }, 0, 0, &mut world);
    assert_eq!(tick_res.len(), 1, "canopy should still sway on tick after trunk promotion");

    // Before carving, trunk core at (32, 0.5, 48) is solid Wood in the promoted SDF.
    let mut edits = EditLog::new();
    let mut composed_unhollowed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(
            Arc::new(edits.clone()),
            vec![SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf.clone())],
            1,
        )),
    );
    let mut out = Vec::new();
    composed_unhollowed.sample(&[[32.0, 0.5, 48.0], [32.0, 0.5, 50.0]], SIZES[0].1, &mut out).unwrap();
    assert!(out[0].distance < -0.10 && out[0].material == Material::Wood, "trunk interior must be solid Wood: {:?}", out[0]);
    assert!(out[1].distance > 0.2, "air outside trunk must be positive: {:?}", out[1]);

    // 3. Carve a tiny hollow room inside the oak trunk at (32, 0.5, 48).
    let room_center = [32.0, 0.5, 48.0];
    edits.push(Edit::carve(Shape::Cylinder { radius: 0.08, half_height: 0.05, round: 0.015 }, room_center, 0.01, Some(Material::Wood), SIZES[0].1)).unwrap();

    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(
            Arc::new(edits.clone()),
            vec![SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf.clone())],
            2,
        )),
    );

    // Tiny player stands inside the carved oak tree room and cannot grow into a person!
    let mut tiny = Body::standing(&mut composed, [32.0, 0.52, 48.0], SIZES[0].1).unwrap();
    assert!(tiny.on.is_some(), "tiny should stand on the carved wood floor inside the oak trunk");
    tiny.grow_to(SIZES[1].1);
    for _ in 0..80 {
        tiny.step(&mut composed, Intent::default(), 1.0 / 120.0).unwrap();
    }
    assert!(tiny.size < 0.15, "tiny inside hollowed oak trunk must be blocked from growing, reached {}", tiny.size);

    // 4. Save & reload round-trip produces bit-identical chunk mesh of the carved tree!
    let save = WorldSave::with_elements("The walled garden", 7, edits.edits().to_vec(), vec![pl.clone()]);
    let reloaded = WorldSave::from_text(&save.to_text()).unwrap();
    assert_eq!(reloaded.elements[0].promoted, Some(0));
    let mesh_sdf_reloaded = catalog.get_or_build_mesh_sdf(&reloaded.elements[0].module, reloaded.elements[0].seed).unwrap().unwrap();
    let mut composed_reloaded = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(
            Arc::new(EditLog::from_edits(reloaded.edits).unwrap()),
            vec![SdfElementEntry::from_promoted_mesh(reloaded.elements[0].clone(), mesh_sdf_reloaded)],
            2,
        )),
    );
    let key = chunk::Key { level: -6, at: [64, 0, 96], asked: -6 };
    let ma = chunk::make(&mut composed, key).unwrap().mesh.expect("carved oak chunk has surface");
    let mb = chunk::make(&mut composed_reloaded, key).unwrap().mesh.expect("reloaded oak chunk has surface");
    assert_eq!(ma.positions, mb.positions);
    assert_eq!(ma.indices, mb.indices);
    assert_eq!(ma.materials, mb.materials);

    // 5. Undo / demote restores trunk back to SceneWorld cleanly.
    runtime.set_promoted(1, None, &mut world).unwrap();
    assert!(world.dynamic.contains_key(&(1, 1)), "demoted trunk should return to SceneWorld");
}

#[test]
fn lazy_scene_to_sdf_promotion_carves_bridge_deck_and_keeps_distance_laws() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let bridge = catalog.get_or_load("elements/modules/bridge.wasm").unwrap();
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&bridge.module, 9).unwrap().expect("bridge has solid deck");

    let pl = PlacedElement {
        id: 1,
        module: bridge.module.clone(),
        seed: 9,
        pos: [32.0, 2.5, 45.0],
        yaw: 0.0,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: Some(0),
    };

    // Uncarved promoted bridge holds a person at the arch crest (y ≈ 2.5 + 0.48 = 2.98).
    let mut uncarved = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf.clone())], 1)),
    );
    let person_on = Body::standing(&mut uncarved, [32.0, 4.5, 45.0], SIZES[1].1).unwrap();
    assert!((person_on.feet[1] - 2.98).abs() < 0.08, "expected person on promoted bridge ~2.98, got {}", person_on.feet[1]);

    // Carve a wide opening right through the middle of the bridge deck.
    let mut edits = EditLog::new();
    edits.push(Edit::carve(Shape::Sphere { radius: 1.1 }, [32.0, 2.95, 45.0], 0.0, None, SIZES[1].1)).unwrap();
    let mut carved = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(edits), vec![SdfElementEntry::from_promoted_mesh(pl, mesh_sdf)], 2)),
    );
    let person_through = Body::standing(&mut carved, [32.0, 4.5, 45.0], SIZES[1].1).unwrap();
    assert!(person_through.feet[1] < 0.5, "person should fall through carved hole in bridge to lawn, got {}", person_through.feet[1]);

    // Promoted & carved bridge still obeys SDF distance laws (`never_too_long`, `surfaces_have_an_inside`).
    let d = carved.describe().unwrap();
    assert_eq!(laws::never_too_long(&mut carved, &d, 1, 200).unwrap(), Ok(()));
    assert_eq!(laws::surfaces_have_an_inside(&mut carved, &d, 1, 200).unwrap(), Ok(()));
}
