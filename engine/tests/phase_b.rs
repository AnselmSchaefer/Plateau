//! Integration tests for Phase B:
//! 1. Material physics table & material-aware Tool interactions (`Saw` vs `Pickaxe` vs `Shovel` vs `Hands`),
//!    including mixed-material `SceneWorld::raycast_placed_surface_material` (TimberBridge wood deck vs stone abutment).
//! 2. Archimedes buoyancy on real carried `.wasm` elements (`boat.wasm`, felled `tree.wasm`, `pillar.wasm`)
//!    across empty, person-loaded, and giant-swamped weights.
//! 3. Navigating a floating `Wooden Rowboat` seamlessly from a base-area pond into a Phase A
//!    dynamically expanded pond channel, stopping cleanly against solid shores.
//! 4. Sandbox checks, watertight cutting, and `standing_on_placed` deck contact for the new carried elements.

mod played;

use std::path::PathBuf;
use std::sync::Arc;

use plateau_engine::body::Body;
use plateau_engine::compose::{CompiledElement, Composed, ComposedLog, ElementCatalog, ElementRuntime, PlacedElement, PlacementTransform, euler_quat};
use plateau_engine::cut::{ClipPlane, ObjectPhysics, check_cut_with_tool, choose_cut_plane, clip_scene};
use plateau_engine::edit::{Edit, EditLog, Shape};
use plateau_engine::physics::{BuoyancyState, Tool, compute_buoyancy, material_props, player_mass_kg, step_floating_move};
use plateau_engine::scene::{Combined, SceneWorld};
use plateau_engine::water::WaterLog;
use plateau_engine::{Limits, Material, SIZES};

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
fn tool_material_matrix_enforces_saw_vs_pickaxe_vs_shovel_on_real_elements() {
    let tree = load_element("tree");
    let bridge = load_element("bridge");
    let boat = load_element("boat");
    let pillar = load_element("pillar");

    assert_eq!(boat.name, "Wooden Rowboat");
    assert!(!boat.is_sdf());
    assert_eq!(pillar.name, "Stone Pillar");
    assert!(!pillar.is_sdf());

    let tree_phys = ObjectPhysics::compute(&tree.preview_scene, 1.0);
    let bridge_phys = ObjectPhysics::compute(&bridge.preview_scene, 1.0);
    let boat_phys = ObjectPhysics::compute(&boat.preview_scene, 1.0);
    let pillar_phys = ObjectPhysics::compute(&pillar.preview_scene, 1.0);

    assert_eq!(tree_phys.dominant_material, Material::Wood);
    assert_eq!(bridge_phys.dominant_material, Material::Wood);
    assert_eq!(boat_phys.dominant_material, Material::Wood);
    assert_eq!(pillar_phys.dominant_material, Material::Stone);
    assert!(!boat_phys.rooted, "boat must be unrooted so it can float and be carried");
    assert!(!pillar_phys.rooted, "stone pillar must be unrooted");

    let person = SIZES[1].1;

    // Saw cuts Wood (tree, boat, bridge planks) but REFUSES Stone (stone pillar & bridge stone abutment)!
    assert!(check_cut_with_tool(&tree_phys, false, person, Tool::Saw, None).is_ok());
    assert!(check_cut_with_tool(&boat_phys, false, person, Tool::Saw, None).is_ok());
    assert!(check_cut_with_tool(&bridge_phys, false, person, Tool::Saw, Some(Material::Wood)).is_ok());
    let saw_stone = check_cut_with_tool(&pillar_phys, false, person, Tool::Saw, None).unwrap_err();
    assert!(saw_stone.contains("stone") && saw_stone.contains("Pickaxe"), "unexpected saw denial: {saw_stone}");
    assert!(check_cut_with_tool(&bridge_phys, false, person, Tool::Saw, Some(Material::Stone)).is_err());

    // Pickaxe cuts Stone (pillar & stone abutment) and carves Stone/Soil, but REFUSES Wood!
    assert!(check_cut_with_tool(&pillar_phys, false, person, Tool::Pickaxe, None).is_ok());
    assert!(check_cut_with_tool(&bridge_phys, false, person, Tool::Pickaxe, Some(Material::Stone)).is_ok());
    assert!(check_cut_with_tool(&tree_phys, false, person, Tool::Pickaxe, None).is_err());
    assert!(Tool::Pickaxe.can_carve(Material::Stone).is_ok());
    assert!(Tool::Pickaxe.can_carve(Material::Soil).is_ok());
    assert!(Tool::Pickaxe.can_carve(Material::Wood).is_err());

    // Shovel digs earth (Soil/Mud/Sand/Grass/Moss), refuses Stone & Wood!
    assert!(Tool::Shovel.can_carve(Material::Soil).is_ok());
    assert!(Tool::Shovel.can_carve(Material::Mud).is_ok());
    assert!(Tool::Shovel.can_carve(Material::Sand).is_ok());
    assert!(Tool::Shovel.can_carve(Material::Stone).is_err());
    assert!(Tool::Shovel.can_carve(Material::Wood).is_err());
    assert!(check_cut_with_tool(&tree_phys, false, person, Tool::Shovel, None).is_err());

    // Hands cannot cut or carve; Workshop (All) cuts both wood and stone.
    assert!(check_cut_with_tool(&tree_phys, false, person, Tool::Hands, None).is_err());
    assert!(Tool::Hands.can_carve(Material::Soil).is_err());
    assert!(check_cut_with_tool(&pillar_phys, false, person, Tool::All, None).is_ok());
    assert!(check_cut_with_tool(&tree_phys, false, person, Tool::All, None).is_ok());

    // Per-triangle hit material on mixed-material TimberBridge distinguishes Wood deck vs Stone abutment!
    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();
    let pl_bridge = PlacedElement {
        id: 1,
        module: bridge.module.clone(),
        seed: 7,
        pos: [32.0, 1.0, 45.0],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        stamp_size: person,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
    };
    runtime.spawn(pl_bridge, &bridge, &mut world).unwrap();
    let (_, _, _, mid_mat) = world.raycast_placed_surface_material([32.0, 3.0, 45.0], [0.0, -1.0, 0.0], 5.0).expect("hit bridge deck");
    let (_, _, _, end_mat) = world.raycast_placed_surface_material([32.0, 3.0, 47.42], [0.0, -1.0, 0.0], 5.0).expect("hit stone abutment");
    assert_eq!(mid_mat, Material::Wood, "middle deck should hit Wood");
    assert_eq!(end_mat, Material::Stone, "bridge abutment should hit Stone");

    // Material physics table invariants.
    assert!(material_props(Material::Wood).buoyant);
    assert!(!material_props(Material::Stone).buoyant);
    assert!(material_props(Material::Stone).hardness > material_props(Material::Wood).hardness);
}

#[test]
fn archimedes_buoyancy_floats_boat_and_felled_log_sinks_stone_and_swamps_under_giant() {
    let boat = load_element("boat");
    let pillar = load_element("pillar");
    let tree = load_element("tree");
    let mut region = garden(7);

    // Center of garden pond: (22.0, -0.30, 36.0), depth ≈ 1.1 m (bed near y ≈ -1.4).
    let pond_tf = PlacementTransform::new([22.0, -0.30, 36.0], 0.0, 1.0);
    let boat_phys = ObjectPhysics::compute(&boat.preview_scene, 1.0);
    let pillar_phys = ObjectPhysics::compute(&pillar.preview_scene, 1.0);

    // 1. Empty Wooden Rowboat floats high on the pond!
    let empty_boat = compute_buoyancy(&boat_phys, pond_tf, &mut region, 0.0).unwrap();
    let (empty_y, empty_sub, water_y) = match empty_boat {
        BuoyancyState::Floating { water_y, target_y, submerged_fraction, .. } => (target_y, submerged_fraction, water_y),
        other => panic!("expected empty wooden rowboat to float, got {other:?}"),
    };
    assert!((water_y - (-0.30)).abs() < 0.03, "expected pond water level ~ -0.30, got {water_y}");
    assert!(empty_sub < 0.45, "empty hollow rowboat should float high (sub < 0.45), got {empty_sub}");

    // 2. Person (75 kg) aboard the rowboat: stays floating, drafts slightly deeper,
    //    AND interior cockpit deck (local y = 0.22 m) stays dry above water_y even at wave trough!
    let person_boat = compute_buoyancy(&boat_phys, pond_tf, &mut region, player_mass_kg(SIZES[1].1)).unwrap();
    match person_boat {
        BuoyancyState::Floating { water_y: wy, target_y, submerged_fraction, bob_amplitude, .. } => {
            assert!(submerged_fraction > empty_sub, "rider weight must increase draft: {submerged_fraction} vs {empty_sub}");
            assert!(target_y < empty_y, "rider weight must settle hull slightly lower: {target_y} vs {empty_y}");
            let deck_trough_y = target_y - bob_amplitude + 0.22;
            assert!(deck_trough_y > wy + 0.04, "boat cockpit deck must stay dry above water_y ({wy:.3}), trough={deck_trough_y:.3}");
        }
        other => panic!("rowboat should easily carry one person, got {other:?}"),
    }

    // 3. Giant (75,000 kg!) stepping onto a person-scale rowboat swamps it straight to the pond bed!
    let giant_boat = compute_buoyancy(&boat_phys, pond_tf, &mut region, player_mass_kg(SIZES[2].1)).unwrap();
    assert!(matches!(giant_boat, BuoyancyState::Sinking { .. }), "giant must swamp a person-scale boat, got {giant_boat:?}");

    // 4. Felled horizontal oak log (solid Wood, 600 kg/m³ < 1000 kg/m³) floats ~60% submerged!
    let log_sc = clip_scene(&tree.preview_scene, &[ClipPlane::new([0.0, 1.0, 0.0], 0.6), ClipPlane::new([0.0, -1.0, 0.0], -3.6)]);
    let log_phys = ObjectPhysics::compute(&log_sc, 1.0);
    let log_tf = PlacementTransform::with_rot([22.0, -0.30, 36.0], euler_quat(0.0, std::f32::consts::FRAC_PI_2, 0.0), 1.0);
    let log_buoy = compute_buoyancy(&log_phys, log_tf, &mut region, 0.0).unwrap();
    match log_buoy {
        BuoyancyState::Floating { submerged_fraction, .. } => {
            assert!((0.45..=0.75).contains(&submerged_fraction), "solid oak log should float ~60% submerged, got {submerged_fraction}");
        }
        other => panic!("felled oak log should float, got {other:?}"),
    }

    // 5. Stone Pillar (2200 kg/m³ > 1000 kg/m³) sinks straight to the pond bed!
    let stone_buoy = compute_buoyancy(&pillar_phys, pond_tf, &mut region, 0.0).unwrap();
    match stone_buoy {
        BuoyancyState::Sinking { water_y, bed_y, target_y } => {
            assert!(bed_y < water_y - 0.5, "pond bed should lie well below water surface");
            assert!(target_y < water_y - 0.5, "sinking stone pillar must settle on pond bed: {target_y}");
        }
        other => panic!("stone pillar must sink in pond, got {other:?}"),
    }

    // 6. Boat on dry lawn (46, 0, 48) is Dry (does not float on grass!).
    let dry_tf = PlacementTransform::new([46.0, 0.0, 48.0], 0.0, 1.0);
    let dry_buoy = compute_buoyancy(&boat_phys, dry_tf, &mut region, 0.0).unwrap();
    assert!(matches!(dry_buoy, BuoyancyState::Dry { .. }), "boat on dry lawn must be Dry, got {dry_buoy:?}");
}

#[test]
fn boat_sails_from_base_pond_into_phase_a_expanded_channel_and_stops_at_shore() {
    let boat = load_element("boat");
    let boat_phys = ObjectPhysics::compute(&boat.preview_scene, 1.0);

    // 1. Expand the garden pond eastward with a carved channel from x = 28.5 to x = 33.0 (Phase A!).
    let mut edits = EditLog::new();
    let mut water = WaterLog::new();
    for cx in [29.4f32, 31.4f32] {
        let carve = Edit::carve(Shape::Sphere { radius: 2.0 }, [cx, -0.55, 36.0], 0.15, None, SIZES[1].1);
        let mut before = Composed::new(
            garden(7),
            Arc::new(ComposedLog::new_with_water(Arc::new(edits.clone()), Vec::new(), Arc::new(water.clone()), 1)),
        );
        let wb = water.expand_from_carve(&mut before, &edits, &carve, None).unwrap();
        assert!(wb.is_some(), "channel carve at x={cx} must expand pond water");
        edits.push(carve).unwrap();
    }

    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits), Vec::new(), Arc::new(water), 10)),
    );

    // 2. Launch the Wooden Rowboat at (25.5, -0.3, 36.0) facing east (+X: yaw = -PI/2 maps local -Z to +X).
    let mut tf = PlacementTransform::new([25.5, -0.3, 36.0], -std::f32::consts::FRAC_PI_2, 1.0);
    let rider = player_mass_kg(SIZES[1].1);
    let dt = 1.0 / 30.0;
    let mut t = 0.0f32;
    let mut last_state = BuoyancyState::Dry { floor_y: 0.0 };

    // Sail east for 3.5 seconds at 2.6 m/s (total attempt ~9 m — enough to enter the expanded channel
    // and reach the solid earth wall at x ≈ 33.4).
    for _ in 0..105 {
        t += dt;
        let (next_tf, buoy) = step_floating_move(&boat_phys, tf, 2.6, 0.0, dt, &mut composed, rider, t).unwrap();
        tf = next_tf;
        last_state = buoy;
    }

    assert!(last_state.is_floating(), "boat must remain floating in the expanded channel: {last_state:?}");
    // Original pond shore ended around x ≈ 28.8; the boat must have sailed cleanly PAST the old shore
    // into the Phase A expanded channel (x > 30.0) while stopping before clipping into solid land (x < 33.2)!
    assert!(tf.pos[0] > 30.0, "boat should sail into the expanded pond channel (x > 30.0), got x={:.2}", tf.pos[0]);
    assert!(tf.pos[0] < 33.2, "boat must stop against the solid end bank (x < 33.2), got x={:.2}", tf.pos[0]);
}

#[test]
fn boat_and_pillar_are_walkable_cuttable_and_discovered_by_catalog() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let discovered = catalog.discover();
    assert!(discovered.iter().any(|e| e.name == "Wooden Rowboat"), "boat.wasm must be discovered");
    assert!(discovered.iter().any(|e| e.name == "Stone Pillar"), "pillar.wasm must be discovered");

    let boat = load_element("boat");
    let pillar = load_element("pillar");

    // 1. Person stands on the floating Wooden Rowboat's deck and `standing_on_placed` identifies it!
    let mut world = SceneWorld::default();
    let mut runtime = ElementRuntime::new();
    let mut region = garden(7);
    let boat_phys = ObjectPhysics::compute(&boat.preview_scene, 1.0);
    let buoy = compute_buoyancy(&boat_phys, PlacementTransform::new([22.0, -0.3, 36.0], 0.0, 1.0), &mut region, player_mass_kg(SIZES[1].1)).unwrap();
    let target_y = match buoy {
        BuoyancyState::Floating { target_y, .. } => target_y,
        other => panic!("expected floating boat, got {other:?}"),
    };
    let pl_boat = PlacedElement {
        id: 7,
        module: boat.module.clone(),
        seed: 1,
        pos: [22.0, target_y, 36.0],
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
    runtime.spawn(pl_boat, &boat, &mut world).unwrap();

    let mut combined = Combined { region: &mut region, world: &world };
    let person = Body::standing(&mut combined, [22.0, 1.5, 36.0], SIZES[1].1).unwrap();
    assert_eq!(person.on, Some(Material::Wood), "person should stand on Wooden Rowboat deck");
    assert_eq!(world.standing_on_placed(person.feet, person.size), Some(7), "standing_on_placed must detect boat id #7");

    // 2. Cutting Stone Pillar with Pickaxe splits it into two watertight stone halves!
    let pillar_phys = ObjectPhysics::compute(&pillar.preview_scene, 1.0);
    let plane = choose_cut_plane(&pillar_phys, [0.0, 1.0, 0.0]).expect("cut plane for pillar");
    let lower = clip_scene(&pillar.preview_scene, &[plane.flipped()]);
    let upper = clip_scene(&pillar.preview_scene, &[plane]);
    assert!(!lower.instances.is_empty() && !upper.instances.is_empty());
    let lower_phys = ObjectPhysics::compute(&lower, 1.0);
    let upper_phys = ObjectPhysics::compute(&upper, 1.0);
    assert_eq!(lower_phys.dominant_material, Material::Stone);
    assert_eq!(upper_phys.dominant_material, Material::Stone);
    assert!(lower_phys.local_volume_m3 > 0.1 && upper_phys.local_volume_m3 > 0.1);
}
