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
use plateau_engine::compose::{
    CompiledElement, Composed, ComposedLog, ElementCatalog, ElementRuntime, PROMOTED_SDF_MAX_SIZE_RATIO, PlacedElement, PlacementTransform, SdfElementEntry,
    euler_quat,
};
use plateau_engine::cut::{ClipPlane, ObjectPhysics, check_carry, check_cut, choose_cut_plane, clip_scene, toppled_transform, world_edit_to_local};
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
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 1,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
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
            rot: None,
            scale,
            stamp_size: player_size,
            after_edits: 0,
            promoted: None,
            clips: Vec::new(),
            local_edits: Vec::new(),
        author: None,
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
        rot: None,
        scale: 0.01,
        stamp_size: SIZES[0].1,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
    };
    let pl2 = PlacedElement {
        id: 2,
        module: lantern.module.clone(),
        seed: 202,
        pos: [40.0, 0.0, 40.0],
        yaw: std::f32::consts::FRAC_PI_2,
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
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
        PlacedElement { id: 1, module: hill.module.clone(), seed: 55, pos: [32.0, 0.0, 48.0], yaw: 0.75, rot: None, scale: 1.15, stamp_size: 1.7, after_edits: 1, promoted: None, clips: Vec::new(), local_edits: Vec::new(), author: None },
        PlacedElement { id: 2, module: tree.module.clone(), seed: 77, pos: [25.0, 0.0, 42.0], yaw: 1.2, rot: None, scale: 0.85, stamp_size: 1.7, after_edits: 2, promoted: Some(2), clips: Vec::new(), local_edits: Vec::new(), author: None },
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
    let pl = PlacedElement { id: 1, module: hill.module.clone(), seed: 11, pos: [32.0, 0.0, 48.0], yaw: 0.3, rot: None, scale: 1.0, stamp_size: 1.7, after_edits: 0, promoted: None, clips: Vec::new(), local_edits: Vec::new(), author: None };
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

    let pl = PlacedElement { id: 1, module: hill.module.clone(), seed: 7, pos: [32.0, 0.0, 48.0], yaw: 0.2, rot: None, scale: 1.0, stamp_size: 1.7, after_edits: 0, promoted: None, clips: Vec::new(), local_edits: Vec::new(), author: None };
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
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
    };
    runtime.spawn(pl.clone(), &tree, &mut world).unwrap();
    // Both trunk (id=1, solid) and canopy (id=2, non-solid) live in SceneWorld initially.
    assert!(world.dynamic.contains_key(&(1, 1)), "uncarved trunk should be in SceneWorld");
    assert!(world.dynamic.contains_key(&(1, 2)), "canopy should be in SceneWorld");

    // 2. Player begins carving a tiny room inside the trunk -> lazily promote solid trunk to MeshSdf!
    pl.promoted = Some(0);
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips).unwrap().expect("tree has solid trunk");
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
    let mesh_sdf_reloaded = catalog
        .get_or_build_mesh_sdf(&reloaded.elements[0].module, reloaded.elements[0].seed, &reloaded.elements[0].clips)
        .unwrap()
        .unwrap();
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
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&bridge.module, 9, &[]).unwrap().expect("bridge has solid deck");

    let pl = PlacedElement {
        id: 1,
        module: bridge.module.clone(),
        seed: 9,
        pos: [32.0, 2.5, 45.0],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: Some(0),
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
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

#[test]
fn chop_oak_tree_splits_into_watertight_stump_and_felled_log_and_second_cut_trims_crown() {
    let tree = load_element("tree");
    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();

    let standing = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 0.0, 48.0],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
    };
    runtime.spawn(standing.clone(), &tree, &mut world).unwrap();

    // Exact surface raycast hits the oak trunk at y ≈ 0.8 m.
    let (hit_id, _, hit_world) = world
        .raycast_placed_surface([32.0, 0.8, 52.0], [0.0, 0.0, -1.0], 10.0)
        .expect("ray must hit oak trunk");
    assert_eq!(hit_id, 1);
    assert!((hit_world[1] - 0.8).abs() < 0.05, "expected hit near y=0.8, got {:?}", hit_world);

    let phys = runtime.physics_of(1).unwrap();
    assert!(phys.rooted, "standing oak tree must be rooted");
    assert!(phys.local_volume_m3 > 0.3, "expected substantial oak trunk volume, got {}", phys.local_volume_m3);

    let hit_local = standing.transform().world_to_local_point(hit_world);
    let cut_plane = choose_cut_plane(&phys, hit_local).expect("cut plane chosen");
    assert_eq!(cut_plane.normal, [0.0, 1.0, 0.0], "oak tree longest axis should cut horizontally (+Y)");
    assert!((cut_plane.offset - 0.8).abs() < 0.1, "cut offset should match aimed y: {}", cut_plane.offset);

    // Split into stump (lower) + felled tree (upper).
    let mut stump = standing.clone();
    stump.clips.push(cut_plane.flipped());

    let mut felled = standing.clone();
    felled.id = 2;
    felled.clips.push(cut_plane);
    let upper_sc = clip_scene(runtime.local_scene_of(1).unwrap(), &[cut_plane]);
    let upper_phys = ObjectPhysics::compute(&upper_sc, 1.0);
    let toppled_tf = toppled_transform(standing.transform(), &upper_phys, [0.0, -1.0], 0.0);
    felled.set_transform(toppled_tf);

    runtime.remove(1, &mut world).unwrap();
    runtime.spawn(stump.clone(), &tree, &mut world).unwrap();
    runtime.spawn(felled.clone(), &tree, &mut world).unwrap();

    // Stump keeps solid trunk base (id 1) and drops canopy (id 2), remaining rooted.
    assert!(world.dynamic.contains_key(&(1, 1)), "stump must keep solid wood base");
    assert!(!world.dynamic.contains_key(&(1, 2)), "stump must not keep leafy canopy");
    let stump_phys = runtime.physics_of(1).unwrap();
    assert!(stump_phys.rooted, "stump remains rooted in the ground");
    assert!(stump_phys.solid_triangles > 10 && stump_phys.local_volume_m3 > 0.05);

    // Felled tree keeps both trunk & canopy, is no longer rooted, and tips horizontally.
    assert!(world.dynamic.contains_key(&(2, 1)), "felled tree keeps solid trunk");
    assert!(world.dynamic.contains_key(&(2, 2)), "felled tree keeps canopy until trimmed");
    let felled_phys = runtime.physics_of(2).unwrap();
    assert!(!felled_phys.rooted, "felled upper trunk must not be rooted");
    assert!(felled_phys.local_lo[1] >= cut_plane.offset - 0.02);

    // Second cut at local y = 2.6 m trims off upper boughs & leafy canopy into a clean timber log!
    let trim_plane = ClipPlane::new([0.0, -1.0, 0.0], -2.6);
    let mut clean_log = felled.clone();
    clean_log.id = 3;
    clean_log.clips.push(trim_plane);
    runtime.spawn(clean_log, &tree, &mut world).unwrap();
    assert!(world.dynamic.contains_key(&(3, 1)), "trimmed log must keep solid trunk");
    assert!(!world.dynamic.contains_key(&(3, 2)), "trimmed log must drop upper leafy canopy");
    let log_phys = runtime.physics_of(3).unwrap();
    assert!(log_phys.local_hi[1] <= 2.62 && log_phys.local_lo[1] >= cut_plane.offset - 0.02);
    assert!(log_phys.world_mass_kg < felled_phys.world_mass_kg, "trimmed log must weigh less than full felled tree");
}

#[test]
fn scale_aware_carry_and_cut_limits_across_tiny_person_and_giant() {
    let tree = load_element("tree");
    let lantern = load_element("lantern");
    let [tiny, person, giant] = [SIZES[0].1, SIZES[1].1, SIZES[2].1];

    let tree_standing = ObjectPhysics::compute(&tree.preview_scene, 1.0);
    let cut = ClipPlane::new([0.0, 1.0, 0.0], 0.7);
    let tree_felled_sc = clip_scene(&tree.preview_scene, &[cut]);
    let tree_felled = ObjectPhysics::compute(&tree_felled_sc, 1.0);
    let tiny_felled = ObjectPhysics::compute(&tree_felled_sc, 0.01);
    let lantern_phys = ObjectPhysics::compute(&lantern.preview_scene, 1.0);

    // Tiny (1.7 cm) cannot chop or carry a person-scale tree.
    assert!(check_cut(&tree_standing, false, tiny).is_err(), "tiny must not chop a 5m tree");
    assert!(!check_carry(&tree_felled, false, tiny).is_allowed(), "tiny must not carry a person-scale log");

    // Tiny CAN chop & carry a tiny-scale (0.01x) tree (exact scale-invariant physics!).
    let tiny_standing = ObjectPhysics::compute(&tree.preview_scene, 0.01);
    assert!(check_cut(&tiny_standing, false, tiny).is_ok(), "tiny must chop a tiny tree");
    assert!(check_carry(&tiny_felled, false, tiny).is_allowed(), "tiny must carry a tiny felled log");

    // Person (1.7 m) must chop a standing person-scale oak tree before carrying it, then carries with heft.
    assert!(check_cut(&tree_standing, false, person).is_ok(), "person must chop standing tree");
    assert!(!check_carry(&tree_standing, false, person).is_allowed(), "person must chop rooted oak tree first");
    let carry_log = check_carry(&tree_felled, false, person);
    assert!(carry_log.is_allowed(), "person must carry felled oak tree: {carry_log:?}");
    assert!(carry_log.speed_factor() < 0.98 && carry_log.speed_factor() >= 0.60, "felled oak should feel weighty: {}", carry_log.speed_factor());

    // Light PostLantern can be picked up directly by a person without chopping!
    assert!(check_carry(&lantern_phys, false, person).is_allowed(), "person should lift post lantern directly");

    // Giant (17 m) can pluck a whole uncut person-scale oak tree straight out of the ground effortlessly!
    let giant_carry = check_carry(&tree_standing, false, giant);
    assert!(giant_carry.is_allowed(), "giant must uproot standing person-scale tree: {giant_carry:?}");
    assert!(giant_carry.speed_factor() > 0.99, "person-scale tree should feel feather-light to a giant");

    // Native SDF landforms (`is_sdf == true`) are part of terrain and cannot be carried or chopped.
    assert!(!check_carry(&tree_felled, true, giant).is_allowed());
    assert!(check_cut(&tree_felled, true, person).is_err());
}

#[test]
fn felled_tree_laid_horizontally_acts_as_walkable_solid_bridge_across_gap() {
    let tree = load_element("tree");
    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();

    // Chop oak tree into a clean 3.4m straight timber log (y in [0.5, 3.9]).
    let clips = vec![ClipPlane::new([0.0, 1.0, 0.0], 0.5), ClipPlane::new([0.0, -1.0, 0.0], -3.9)];
    // Rotate +90° around X so local +Y trunk lies flat horizontally along world +Z!
    // Place elevated at y = 2.5 m above the lawn so ONLY the felled trunk holds the player up!
    let rot = euler_quat(0.0, std::f32::consts::FRAC_PI_2, 0.0);
    let mut log = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 2.5, 44.0],
        yaw: 0.0,
        rot: Some(rot),
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: None,
        clips,
        local_edits: Vec::new(),
        author: None,
    };
    runtime.spawn(log.clone(), &tree, &mut world).unwrap();

    // While held in-hand, collision disappears; when placed back down, collision returns.
    runtime.set_held(1, true, &mut world).unwrap();
    assert!(!world.has_solids(), "held object must not collide while carried");
    let placed_tf = PlacementTransform::with_rot([32.0, 2.5, 44.0], rot, 1.0);
    runtime.relocate(1, placed_tf, &mut world).unwrap();
    runtime.set_held(1, false, &mut world).unwrap();
    assert!(world.has_solids(), "placed log must immediately be solid");
    log.set_transform(placed_tf);

    let mut composed = Composed::new(garden(7), Arc::new(ComposedLog::empty()));
    let mut combined = Combined { region: &mut composed, world: &world };

    // Local y in [0.5, 3.9] rotated +90° around X maps to world z in [44.5, 47.9] at y ≈ 2.5 + radius (~2.72 m).
    let mut person = Body::standing(&mut combined, [32.0, 3.8, 45.0], SIZES[1].1).unwrap();
    assert!(person.feet[1] > 2.55, "person should stand atop horizontal felled tree trunk (~2.7m), got {}", person.feet[1]);
    assert_eq!(person.on, Some(Material::Wood), "person should be standing on Wood");

    // Walk 2 metres along the horizontal log (+Z is yaw = PI).
    person.yaw = std::f32::consts::PI;
    for _ in 0..160 {
        person.step(&mut combined, Intent { forward: 1.0, ..Intent::default() }, 1.0 / 120.0).unwrap();
    }
    assert!(person.feet[2] > 46.2, "person should walk forward along the felled log bridge, z={}", person.feet[2]);
    assert!(person.feet[1] > 2.50, "person should remain elevated on the felled log bridge (not fall to lawn), y={}", person.feet[1]);
}

#[test]
fn clipped_horizontal_felled_tree_promotes_lazily_to_carvable_mesh_sdf() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load("elements/modules/tree.wasm").unwrap();

    // Chopped oak log (watertight capped at both ends!) laid horizontally along +Z at (32, 2.0, 44).
    let clips = vec![ClipPlane::new([0.0, 1.0, 0.0], 0.6), ClipPlane::new([0.0, -1.0, 0.0], -3.6)];
    let rot = euler_quat(0.0, std::f32::consts::FRAC_PI_2, 0.0);
    let pl = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 2.0, 44.0],
        yaw: 0.0,
        rot: Some(rot),
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: Some(0),
        clips: clips.clone(),
        local_edits: Vec::new(),
        author: None,
    };

    // Build MeshSdf of the CLIPPED oak log!
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &clips).unwrap().expect("clipped log has solid MeshSdf");
    assert!(mesh_sdf.lo()[1] >= 0.58 && mesh_sdf.hi()[1] <= 3.62, "MeshSdf bounds must match clipped log: {:?}..{:?}", mesh_sdf.lo(), mesh_sdf.hi());

    // Local point (0, 2.0, 0) maps under +90° X rotation to world (32, 2.0, 46.0) — must be solid Wood!
    // Meanwhile local y = 0.2 (stump, clipped away!) at world (32, 2.0, 44.2) must be open air!
    let mut uncarved = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf.clone())], 1)),
    );
    let mut ans = Vec::new();
    uncarved.sample(&[[32.0, 2.0, 46.0], [32.0, 2.0, 44.2]], SIZES[0].1, &mut ans).unwrap();
    assert!(ans[0].distance < -0.10 && ans[0].material == Material::Wood, "horizontal clipped log core must be solid Wood: {:?}", ans[0]);
    assert!(ans[1].distance > 0.25, "clipped-away stump region must be open air: {:?}", ans[1]);

    // Carve a hollow notch right into the horizontal felled log!
    let mut edits = EditLog::new();
    edits.push(Edit::carve(Shape::Sphere { radius: 0.45 }, [32.0, 2.0, 46.0], 0.02, Some(Material::Wood), SIZES[1].1)).unwrap();
    let mut carved = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(edits), vec![SdfElementEntry::from_promoted_mesh(pl, mesh_sdf)], 2)),
    );
    carved.sample(&[[32.0, 2.0, 46.0]], SIZES[0].1, &mut ans).unwrap();
    assert!(ans[0].distance > 0.20, "carved notch in felled log must be open air: {:?}", ans[0]);

    // Still obeys SDF laws (`never_too_long`, `surfaces_have_an_inside`) under full 3D quaternion + clipping!
    let d = carved.describe().unwrap();
    assert_eq!(laws::never_too_long(&mut carved, &d, 1, 200).unwrap(), Ok(()));
    assert_eq!(laws::surfaces_have_an_inside(&mut carved, &d, 1, 200).unwrap(), Ok(()));
}

#[test]
fn world_save_step_8c_round_trip_preserves_cuts_quaternions_and_backward_compat() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load("elements/modules/tree.wasm").unwrap();

    // Step 8b save format (without `rot` or `clip`) still parses cleanly.
    let old_8b = "plateau-world 1\narea The walled garden\nseed 7\n\
                  element 1 elements/modules/tree.wasm seed 7 pos 32 0 48 yaw 0.5 scale 1 size 1.7 after 0\n\
                  element 2 elements/modules/tree.wasm seed 9 pos 30 0 46 yaw 1.0 scale 1 size 1.7 after 0 promoted 1\n";
    let parsed_8b = WorldSave::from_text(old_8b).unwrap();
    assert_eq!(parsed_8b.elements.len(), 2);
    assert_eq!(parsed_8b.elements[0].rot, None);
    assert!(parsed_8b.elements[0].clips.is_empty());
    assert_eq!(parsed_8b.elements[1].promoted, Some(1));

    // Full Step 8c save: stump (1 clip) + horizontal felled log bridge (quaternion + 2 clips + promoted + carve).
    let stump = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [28.0, 0.0, 48.0],
        yaw: 0.3,
        rot: None,
        scale: 1.0,
        stamp_size: 1.7,
        after_edits: 0,
        promoted: None,
        clips: vec![ClipPlane::new([0.0, -1.0, 0.0], -0.7)],
        local_edits: Vec::new(),
        author: None,
    };
    let bridge_log = PlacedElement {
        id: 2,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 2.0, 44.0],
        yaw: 0.0,
        rot: Some(euler_quat(0.4, std::f32::consts::FRAC_PI_2, 0.0)),
        scale: 1.0,
        stamp_size: 1.7,
        after_edits: 0,
        promoted: Some(0),
        clips: vec![ClipPlane::new([0.0, 1.0, 0.0], 0.7), ClipPlane::new([0.0, -1.0, 0.0], -3.6)],
        local_edits: vec![Edit::carve(Shape::Sphere { radius: 0.12 }, [0.0, 1.8, 0.0], 0.02, Some(Material::Wood), 0.17)],
        author: None,
    };
    let edits = vec![Edit::carve(Shape::Sphere { radius: 0.35 }, [32.0, 2.0, 46.0], 0.02, Some(Material::Wood), 1.7)];
    let save = WorldSave::with_elements("The walled garden", 7, edits, vec![stump, bridge_log.clone()]);

    let text = save.to_text();
    let reloaded = WorldSave::from_text(&text).unwrap();
    assert_eq!(save, reloaded);

    // Chunk mesh of the promoted, clipped, rotated felled log matches bit-for-bit after save/reload!
    let sdf_a = catalog.get_or_build_mesh_sdf(&bridge_log.module, bridge_log.seed, &bridge_log.clips).unwrap().unwrap();
    let rlog = &reloaded.elements[1];
    let sdf_b = catalog.get_or_build_mesh_sdf(&rlog.module, rlog.seed, &rlog.clips).unwrap().unwrap();
    let mut ca = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::from_edits(save.edits.clone()).unwrap()), vec![SdfElementEntry::from_promoted_mesh(bridge_log, sdf_a)], 1)),
    );
    let mut cb = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::from_edits(reloaded.edits).unwrap()), vec![SdfElementEntry::from_promoted_mesh(rlog.clone(), sdf_b)], 1)),
    );
    let key = chunk::Key { level: -5, at: [32, 2, 44], asked: -5 };
    let ma = chunk::make(&mut ca, key).unwrap().mesh.expect("chunk has felled log surface");
    let mb = chunk::make(&mut cb, key).unwrap().mesh.expect("reloaded chunk has felled log surface");
    assert_eq!(ma.positions, mb.positions);
    assert_eq!(ma.indices, mb.indices);
    assert_eq!(ma.materials, mb.materials);
}

#[test]
fn carved_trunk_stays_solid_at_giant_scale_and_tunnel_travels_when_chopped_and_relocated() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load("elements/modules/tree.wasm").unwrap();

    // 1. Place standing OakTree at (32, 0, 48) and carve a tunnel through its trunk at y = 1.6 m.
    let standing_tf = PlacementTransform::new([32.0, 0.0, 48.0], 0.0, 1.0);
    let world_tunnel = Edit::carve(Shape::Sphere { radius: 0.22 }, [32.0, 1.6, 48.0], 0.02, Some(Material::Wood), SIZES[1].1);
    let local_tunnel = world_edit_to_local(&world_tunnel, standing_tf);
    let mut pl = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 0.0, 48.0],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: Some(0),
        clips: Vec::new(),
        local_edits: vec![local_tunnel],
        author: None,
    };

    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();
    runtime.spawn_for_size(pl.clone(), &tree, &mut world, SIZES[1].1).unwrap();

    // At Person scale (1.7 m <= 1.0 * PROMOTED_SDF_MAX_SIZE_RATIO), the solid trunk collides via
    // the SDF (skip_solid == true) so the carved tunnel is open (it is drawn carved, `carved`).
    assert!(SIZES[1].1 <= pl.scale * PROMOTED_SDF_MAX_SIZE_RATIO);
    assert!(runtime.should_skip_solid(1));
    assert!(!world.dynamic.contains_key(&(1, 1)), "at person scale, promoted trunk lives in SDF");

    // Switching to Giant scale (17.0 m > 1.0 * PROMOTED_SDF_MAX_SIZE_RATIO):
    // - Macro proxy automatically activates in SceneWorld so the trunk stays 100% solid & clickable!
    // - Coarse giant SDF chunks skip the sub-voxel PromotedMesh so zero broken giant-voxel slivers appear!
    let flipped = runtime.sync_macro_proxies(SIZES[2].1, &mut world);
    assert_eq!(flipped.len(), 1);
    assert!(!flipped[0].2, "skip_solid must flip to false at giant scale");
    assert!(world.dynamic.contains_key(&(1, 1)), "giant macro proxy must restore crisp trunk in SceneWorld");
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips).unwrap().unwrap();
    let mut composed_standing = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf.clone())], 1)),
    );
    let mut out = Vec::new();
    // At giant chunk size (10 m > 6.5 m), SDF skips the sub-voxel trunk cleanly (air at (32, 2.2, 48))!
    composed_standing.sample(&[[32.0, 2.2, 48.0]], 10.0, &mut out).unwrap();
    assert!(out[0].distance > 1.0, "giant SDF must skip sub-voxel promoted trunk: {:?}", out[0]);
    // At person scale (1.7 m), SDF has solid trunk at y=2.2 and open carved tunnel at y=1.6!
    composed_standing.sample(&[[32.0, 2.2, 48.0], [32.0, 1.6, 48.0]], SIZES[1].1, &mut out).unwrap();
    assert!(out[0].distance < -0.10, "person SDF must show solid wood at y=2.2: {:?}", out[0]);
    assert!(out[1].distance > 0.15, "person SDF must show carved tunnel at y=1.6: {:?}", out[1]);

    // 2. Now chop the tunneled tree at y = 0.7 m, relocate the felled upper trunk horizontally to
    // (20, 2.0, 40) rotated +90° about X! The carved tunnel (local y = 1.6) must travel AND rotate
    // to world (20, 2.0, 41.6)!
    let cut = ClipPlane::new([0.0, 1.0, 0.0], 0.7);
    pl.clips.push(cut);
    let rot = euler_quat(0.0, std::f32::consts::FRAC_PI_2, 0.0);
    pl.set_transform(PlacementTransform::with_rot([20.0, 2.0, 40.0], rot, 1.0));
    let felled_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips).unwrap().unwrap();
    let mut composed_moved = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::from_promoted_mesh(pl.clone(), felled_sdf)], 2)),
    );
    // World (20, 2.0, 42.2) is solid wood (local y = 2.2); world (20, 2.0, 41.6) is the moved tunnel (local y = 1.6)!
    composed_moved.sample(&[[20.0, 2.0, 42.2], [20.0, 2.0, 41.6]], SIZES[1].1, &mut out).unwrap();
    assert!(out[0].distance < -0.10 && out[0].material == Material::Wood, "relocated felled trunk must be solid Wood: {:?}", out[0]);
    assert!(out[1].distance > 0.15, "carved tunnel must travel & rotate with felled trunk: {:?}", out[1]);

    // Shrinking back to person scale hides the macro proxy again cleanly.
    let back = runtime.sync_macro_proxies(SIZES[1].1, &mut world);
    assert_eq!(back.len(), 1);
    assert!(back[0].2, "skip_solid must flip back to true at person scale");
}

#[test]
fn giant_tree_carved_by_person_has_no_coarse_pass_or_winding_distortions() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load("elements/modules/tree.wasm").unwrap();

    // Giant-scale OakTree (10x, ~50 m tall, ~6 m trunk diameter) placed at (32, 0, 48).
    let tf = PlacementTransform::new([32.0, 0.0, 48.0], 0.0, 10.0);
    let carve = Edit::carve(Shape::Sphere { radius: 1.2 }, [32.0, 1.5, 50.0], 0.3, Some(Material::Wood), SIZES[1].1);
    let local_carve = world_edit_to_local(&carve, tf);
    let pl = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 0.0, 48.0],
        yaw: 0.0,
        rot: None,
        scale: 10.0,
        stamp_size: SIZES[2].1,
        after_edits: 0,
        promoted: Some(0),
        clips: Vec::new(),
        local_edits: vec![local_carve],
        author: None,
    };
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &[]).unwrap().unwrap();
    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::from_promoted_mesh(pl, mesh_sdf.clone())], 1)),
    );

    // 1. Check MeshSdf inside/outside consistency AND absence of buried internal zero-crossings
    // where OakTree's 4 buttress roots penetrate inside the main trunk!
    let mut bad_inside = 0usize;
    let mut buried_seams = 0usize;
    let mut grazing_flips = 0usize;
    for ix in -16..=16 {
        for iy in 0..=20 {
            for iz in -16..=16 {
                let p = [ix as f32 * 0.04, iy as f32 * 0.04, iz as f32 * 0.04];
                // Main trunk cylinder has radius >= 0.25 for y in [0.0, 0.8], centered near (0, y, 0).
                let r = (p[0] * p[0] + p[2] * p[2]).sqrt();
                let ans = mesh_sdf.sample(p, f32::INFINITY).unwrap();
                if r < 0.20 && p[1] >= 0.02 && p[1] <= 0.78 && ans.distance >= 0.0 {
                    if bad_inside < 10 {
                        eprintln!("BAD SIGN inside core trunk at {p:?}: {ans:?}");
                    }
                    bad_inside += 1;
                }
                // Deep inside the trunk (r <= 0.16, y in [0.08, 0.50] — right where buttress roots overlap!),
                // true union SDF must remain deeply negative (< -0.05) with zero buried root-wall seams!
                if r <= 0.16 && p[1] >= 0.08 && p[1] <= 0.50 && ans.distance > -0.05 {
                    if buried_seams < 10 {
                        eprintln!("BURIED SEAM inside trunk at {p:?}: d={:.5}", ans.distance);
                    }
                    buried_seams += 1;
                }
                // Check sharp sign/distance jump vs tiny neighbor (2mm away)
                let p2 = [p[0] + 0.002, p[1], p[2]];
                let ans2 = mesh_sdf.sample(p2, f32::INFINITY).unwrap();
                if (ans.distance - ans2.distance).abs() > 0.01 {
                    if grazing_flips < 10 {
                        eprintln!("DISCONTINUITY at {p:?}: d1={:.4} d2={:.4}", ans.distance, ans2.distance);
                    }
                    grazing_flips += 1;
                }
            }
        }
    }
    eprintln!("bad_inside = {bad_inside}, buried_seams = {buried_seams}, grazing_flips = {grazing_flips}");

    // 2. Compare coarse=true vs coarse=false chunks around the carved giant trunk!
    let mut coarse_diffs = 0usize;
    for cx in 14..=18 {
        for cy in 0..=3 {
            for cz in 22..=26 {
                let key = chunk::Key { level: -4, at: [cx, cy, cz], asked: -4 };
                let a = chunk::make_with(&mut composed, key, chunk::Making { coarse: true, ..Default::default() }).unwrap();
                let b = chunk::make_with(&mut composed, key, chunk::Making { coarse: false, ..Default::default() }).unwrap();
                if a.mesh != b.mesh {
                    let na = a.mesh.as_ref().map_or(0, |m| m.indices.len() / 3);
                    let nb = b.mesh.as_ref().map_or(0, |m| m.indices.len() / 3);
                    eprintln!("COARSE DIFF at chunk {cx},{cy},{cz}: coarse_tris={na} exact_tris={nb}");
                    coarse_diffs += 1;
                }
            }
        }
    }
    eprintln!("coarse_diffs = {coarse_diffs}");

    // 3. Check SDF distance laws on the carved giant trunk!
    let d = composed.describe().unwrap();
    let ntl = laws::never_too_long(&mut composed, &d, 1, 400).unwrap();
    eprintln!("never_too_long = {ntl:?}");
    let shai = laws::surfaces_have_an_inside(&mut composed, &d, 1, 400).unwrap();
    eprintln!("surfaces_have_an_inside = {shai:?}");

    assert_eq!(bad_inside, 0, "trunk core had positive (outside) points");
    assert_eq!(buried_seams, 0, "overlapping buttress roots created internal zero-crossings inside trunk");
    assert_eq!(grazing_flips, 0, "MeshSdf had sign discontinuities");
    assert_eq!(coarse_diffs, 0, "coarse pass diverged from exact meshing");
    assert_eq!(ntl, Ok(()));
    assert_eq!(shai, Ok(()));
}

#[test]
fn tiny_carving_person_scale_tree_at_grass_level_has_no_chunk_holes() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let tree = catalog.get_or_load("elements/modules/tree.wasm").unwrap();

    // Person-scale OakTree (scale = 1.0) at (32, 0, 48), carved by a TINY player (0.017 m) right at grass level!
    let tf = PlacementTransform::new([32.0, 0.0, 48.0], 0.0, 1.0);
    // Find exact grass height right outside trunk at (32.0, ?, 48.35).
    let mut base = garden(7);
    let mut ans = Vec::new();
    base.sample(&[[32.0, 0.0, 48.35]], SIZES[0].1, &mut ans).unwrap();
    let grass_y = -ans[0].distance;

    let r = 0.75 * SIZES[0].1; // default brush radius for tiny = 0.01275 m
    let carve1 = Edit::carve(Shape::Sphere { radius: r }, [32.0, grass_y + 0.01, 48.33], r * 0.35, None, SIZES[0].1);
    let carve2 = Edit::carve(Shape::Sphere { radius: r }, [32.0, grass_y + 0.01, 48.315], r * 0.35, None, SIZES[0].1);
    let local_edits = vec![world_edit_to_local(&carve1, tf), world_edit_to_local(&carve2, tf)];

    let pl = PlacedElement {
        id: 1,
        module: tree.module.clone(),
        seed: 7,
        pos: [32.0, 0.0, 48.0],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        stamp_size: SIZES[1].1,
        after_edits: 0,
        promoted: Some(0),
        clips: Vec::new(),
        local_edits,
        author: None,
    };
    let mesh_sdf = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &[]).unwrap().unwrap();
    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new(Arc::new(EditLog::new()), vec![SdfElementEntry::from_promoted_mesh(pl, mesh_sdf.clone())], 1)),
    );

    // 1. Check coarse=true vs coarse=false across tiny/fine levels (-11, -10, -9) around the carved
    // tree-grass seam `(32.0, grass_y, 48.33)`.
    let mut diffs = 0usize;
    for level in [-11, -10, -9] {
        let s = chunk::Key::new(level, [0, 0, 0]).side();
        let cx = (32.0 / s).floor() as i32;
        let cy = (grass_y as f64 / s).floor() as i32;
        let cz = (48.33 / s).floor() as i32;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    let key = chunk::Key { level, at: [cx + dx, cy + dy, cz + dz], asked: level };
                    let a = chunk::make_with(&mut composed, key, chunk::Making { coarse: true, ..Default::default() }).unwrap();
                    let b = chunk::make_with(&mut composed, key, chunk::Making { coarse: false, ..Default::default() }).unwrap();
                    if a.mesh != b.mesh {
                        diffs += 1;
                    }
                }
            }
        }
    }

    // 2. Verify zero Lipschitz / sign-flip spikes on MeshSdf across the entire grass-to-trunk zone.
    let mut max_mesh_grad = 0.0f32;
    let eps = 0.002f32;
    for ix in -10..=10 {
        for iy in -2..=12 {
            for iz in 0..=12 {
                let p0 = [32.0 + ix as f32 * 0.015, grass_y + iy as f32 * 0.01, 48.28 + iz as f32 * 0.015];
                let l0 = tf.world_to_local_point(p0);
                let d0 = mesh_sdf.sample(l0, f32::INFINITY).unwrap().distance;
                for axis in 0..3 {
                    let mut l1 = l0;
                    l1[axis] += eps;
                    let d1 = mesh_sdf.sample(l1, f32::INFINITY).unwrap().distance;
                    max_mesh_grad = max_mesh_grad.max((d1 - d0).abs() / eps);
                }
            }
        }
    }

    // 3. Cast camera rays from a tiny player standing outside the trunk (`open == 0`) looking at the
    // carved tree-grass seam against all multi-LOD `rings::wanted_looking` chunks to verify zero
    // sky-blue holes leak through!
    let mut sky_leaks = 0usize;
    let eye = [32.0, grass_y as f64 + SIZES[0].1 as f64 * 0.9, 48.72];
    let cam = [eye[0], eye[1] + SIZES[0].1 as f64 * 0.6, eye[2] + SIZES[0].1 as f64 * 3.5];
    let open = plateau_engine::rings::open(&mut composed, eye, SIZES[0].1).unwrap();
    assert_eq!(open, 0, "tiny player standing outside roots should have open == COARSEST");
    let mut rings = plateau_engine::rings::Rings::default();
    let wanted = rings.wanted_looking(eye, [0.0, -1.0], SIZES[0].1, open);
    let mut chunk_meshes: Vec<(chunk::Key, [f64; 3], chunk::Mesh)> = Vec::new();
    for &k in &wanted {
        let o = k.origin();
        let s = k.side();
        if o[0] + s < 31.4 || o[0] > 32.6 || o[2] + s < 47.5 || o[2] > 48.9 || o[1] + s < -0.4 || o[1] > 0.4 {
            continue;
        }
        if let Some(m) = chunk::make(&mut composed, k).unwrap().mesh {
            chunk_meshes.push((k, o, m));
        }
    }
    let ray_tri = |ro: [f64; 3], rd: [f64; 3], a: [f64; 3], b: [f64; 3], c: [f64; 3]| -> Option<f64> {
        let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let h = [rd[1] * e2[2] - rd[2] * e2[1], rd[2] * e2[0] - rd[0] * e2[2], rd[0] * e2[1] - rd[1] * e2[0]];
        let det = e1[0] * h[0] + e1[1] * h[1] + e1[2] * h[2];
        if det.abs() < 1e-12 {
            return None;
        }
        let inv = 1.0 / det;
        let s = [ro[0] - a[0], ro[1] - a[1], ro[2] - a[2]];
        let u = (s[0] * h[0] + s[1] * h[1] + s[2] * h[2]) * inv;
        if !(0.0..=1.0).contains(&u) {
            return None;
        }
        let q = [s[1] * e1[2] - s[2] * e1[1], s[2] * e1[0] - s[0] * e1[2], s[0] * e1[1] - s[1] * e1[0]];
        let v = (rd[0] * q[0] + rd[1] * q[1] + rd[2] * q[2]) * inv;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        let t = (e2[0] * q[0] + e2[1] * q[1] + e2[2] * q[2]) * inv;
        (t > 1e-5).then_some(t)
    };
    for iy in 0..24 {
        let ty = grass_y as f64 - 0.015 + (iy as f64 / 23.0) * 0.065;
        for ix in 0..32 {
            let tx = 31.82 + (ix as f64 / 31.0) * 0.36;
            let d = [tx - cam[0], ty - cam[1], 48.33 - cam[2]];
            let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let rd = [d[0] / len, d[1] / len, d[2] / len];
            let mut hit_t = f64::INFINITY;
            for (k, o, m) in &chunk_meshes {
                let s = k.side() + 4.0 * k.voxel();
                let lo = [o[0] - 2.0 * k.voxel(), o[1] - 2.0 * k.voxel(), o[2] - 2.0 * k.voxel()];
                let hi = [lo[0] + s, lo[1] + s, lo[2] + s];
                let mut tmin = 0.0f64;
                let mut tmax = hit_t;
                let mut aabb_hit = true;
                for a in 0..3 {
                    if rd[a].abs() < 1e-12 {
                        if cam[a] < lo[a] || cam[a] > hi[a] {
                            aabb_hit = false;
                            break;
                        }
                    } else {
                        let t0 = (lo[a] - cam[a]) / rd[a];
                        let t1 = (hi[a] - cam[a]) / rd[a];
                        tmin = tmin.max(t0.min(t1));
                        tmax = tmax.min(t0.max(t1));
                        if tmin > tmax {
                            aabb_hit = false;
                            break;
                        }
                    }
                }
                if !aabb_hit {
                    continue;
                }
                for tri in m.indices.as_chunks::<3>().0 {
                    let p = |idx: u32| {
                        let q = m.positions[idx as usize];
                        [o[0] + q[0] as f64, o[1] + q[1] as f64, o[2] + q[2] as f64]
                    };
                    if let Some(t) = ray_tri(cam, rd, p(tri[0]), p(tri[1]), p(tri[2]))
                        && t < hit_t
                    {
                        hit_t = t;
                    }
                }
            }
            if !hit_t.is_finite() {
                sky_leaks += 1;
            }
        }
    }
    assert_eq!(diffs, 0, "coarse pass diverged from exact meshing at tiny tree-grass seam");
    assert!(max_mesh_grad <= 1.05, "MeshSdf gradient spike: {max_mesh_grad:.3}");
    assert_eq!(sky_leaks, 0, "multi-LOD chunks leaked sky blue through grass-tree seam!");
}
