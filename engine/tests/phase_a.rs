//! Integration tests for Phase A:
//! 1. Multi-scale Ownership & Rule Book (`engine/src/rules.rs`)
//! 2. Engine-managed Hydrostatic Water Layer & Dynamic Pond Expansion (`engine/src/water.rs`)
//! 3. Unified `WorldSave` Manifest (`tile`, `policy`, `plot`, `water`, `water-basin`, `author`)
//!    with full bit-exact round-trip determinism & Step 8a/8b/8c backward compatibility.

mod played;

use std::sync::Arc;

use plateau_engine::body::{Body, Intent};
use plateau_engine::compose::{Composed, ComposedLog, PlacedElement};
use plateau_engine::edit::{Edit, EditLog, SavedTile, Shape, WorldSave};
use plateau_engine::rules::{Action, AreaPolicy, CutCarryPolicy, LOCAL_PLAYER, Plot, Role, RolePolicy, RuleBook};
use plateau_engine::water::{WaterBasin, WaterBody, WaterLog};
use plateau_engine::{Material, Region, SIZES, Size, chunk, laws};

fn garden(seed: u64) -> played::Native {
    played::area("garden", seed)
}

#[test]
fn rulebook_enforces_roles_scale_caps_guards_plots_and_object_ownership() {
    let mut rules = RuleBook::new();

    // 1. Unclaimed open tile [0, 0]: anyone (including local single-player) has full permissions.
    assert_eq!(AreaPolicy::open([0, 0]).role_of("alice"), Role::Owner);
    assert!(
        rules
            .check(
                LOCAL_PLAYER,
                &Action::Sculpt { lo: [30.0, -1.0, 30.0], hi: [34.0, 1.0, 34.0], player_size: SIZES[2].1, is_water: false },
            )
            .is_ok()
    );

    // 2. Claim tile [0, 0] for owner "alice", trusted builder "bob", tiny-friendly visitors,
    //    with 2.0m border_guard and 4.0m spawn_guard around (32, 60).
    let mut p0 = AreaPolicy::owned([0, 0], "alice");
    p0.builders.push("bob".into());
    p0.builder_policy = RolePolicy::builder();
    p0.visitor_policy = RolePolicy::tiny_visitor();
    p0.border_guard = 2.0;
    p0.spawn_guard = 4.0;
    rules.set_area(p0);

    // Claim neighbor tile [1, 0] (x in [64, 128)) as read-only for visitors, owned by "carol".
    let mut p1 = AreaPolicy::owned([1, 0], "carol");
    p1.visitor_policy = RolePolicy::read_only();
    rules.set_area(p1);

    // Role lookups
    let a0 = rules.area_policy([0, 0]).unwrap();
    assert_eq!(a0.role_of("alice"), Role::Owner);
    assert_eq!(a0.role_of("bob"), Role::Builder);
    assert_eq!(a0.role_of("eve"), Role::Visitor);

    // Owner "alice" can sculpt at Giant scale even right at the spawn and border of her own tile.
    assert!(
        rules
            .check(
                "alice",
                &Action::Sculpt { lo: [30.0, -1.0, 58.0], hi: [34.0, 1.0, 62.0], player_size: SIZES[2].1, is_water: false },
            )
            .is_ok()
    );

    // Builder "bob" can sculpt at Person scale in the middle, but NOT at Giant scale.
    assert!(
        rules
            .check(
                "bob",
                &Action::Sculpt { lo: [20.0, -1.0, 20.0], hi: [22.0, 1.0, 22.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_ok()
    );
    let err_giant = rules
        .check(
            "bob",
            &Action::Sculpt { lo: [20.0, -2.0, 20.0], hi: [28.0, 2.0, 28.0], player_size: SIZES[2].1, is_water: false },
        )
        .unwrap_err();
    assert!(err_giant.reason.contains("person"), "expected scale cap message, got: {}", err_giant.reason);

    // Non-owner "bob" is blocked from sculpting inside spawn_guard (within 4m of (32, 60)) or border_guard (x < 2).
    assert!(
        rules
            .check(
                "bob",
                &Action::Sculpt { lo: [31.0, -1.0, 59.0], hi: [33.0, 1.0, 61.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_err()
    );
    assert!(
        rules
            .check(
                "bob",
                &Action::Sculpt { lo: [0.5, -1.0, 30.0], hi: [1.5, 1.0, 32.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_err()
    );

    // Cross-border edit AABB touching tile [1, 0] (carol's read-only tile) is denied even for alice!
    assert!(
        rules
            .check(
                "alice",
                &Action::Sculpt { lo: [62.5, -1.0, 30.0], hi: [65.5, 1.0, 32.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_err()
    );

    // Tiny-friendly visitor "eve" can sculpt & place at Tiny scale, cannot sculpt at Person scale,
    // and cannot modify water (`water_max: None`).
    assert!(
        rules
            .check(
                "eve",
                &Action::Sculpt { lo: [20.0, 0.0, 20.0], hi: [20.05, 0.05, 20.05], player_size: SIZES[0].1, is_water: false },
            )
            .is_ok()
    );
    assert!(
        rules
            .check("eve", &Action::PlaceElement { at: [20.0, 0.0, 20.0], player_size: SIZES[0].1 })
            .is_ok()
    );
    assert!(
        rules
            .check(
                "eve",
                &Action::Sculpt { lo: [20.0, 0.0, 20.0], hi: [21.0, 1.0, 21.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_err()
    );
    assert!(
        rules
            .check(
                "eve",
                &Action::ModifyWater { lo: [20.0, -0.5, 20.0], hi: [20.1, 0.0, 20.1], player_size: SIZES[0].1 },
            )
            .is_err()
    );

    // CutCarryPolicy::OwnOnly: visitor "eve" can cut/carry/remove her own object, NOT alice's!
    assert!(
        rules
            .check("eve", &Action::CutOrCarry { at: [20.0, 0.0, 20.0], owner: Some("eve") })
            .is_ok()
    );
    assert!(
        rules
            .check("eve", &Action::CutOrCarry { at: [20.0, 0.0, 20.0], owner: Some("alice") })
            .is_err()
    );
    assert!(
        rules
            .check("eve", &Action::RemoveElement { at: [20.0, 0.0, 20.0], owner: Some("alice") })
            .is_err()
    );

    // 3. Sub-volume Plot: "carol" leases a tiny hobbit-cavity plot inside her read-only tile [1, 0] to "eve"!
    rules.set_plot(Plot {
        id: 1,
        name: "hobbit-hollow".into(),
        lo: [80.0, 0.0, 30.0],
        hi: [82.0, 2.0, 32.0],
        owners: vec!["eve".into()],
        builders: Vec::new(),
        builder_policy: RolePolicy::builder(),
        visitor_policy: Some(RolePolicy::read_only()),
    });

    // "eve" can sculpt and place freely INSIDE her leased plot (even at person scale),
    // but is still blocked outside the plot in carol's read-only tile!
    assert!(
        rules
            .check(
                "eve",
                &Action::Sculpt { lo: [80.5, 0.2, 30.5], hi: [81.5, 1.2, 31.5], player_size: SIZES[1].1, is_water: false },
            )
            .is_ok()
    );
    assert!(
        rules
            .check("eve", &Action::PlaceElement { at: [81.0, 0.5, 31.0], player_size: SIZES[0].1 })
            .is_ok()
    );
    assert!(
        rules
            .check(
                "eve",
                &Action::Sculpt { lo: [81.5, 0.2, 31.5], hi: [83.0, 1.2, 33.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_err(),
        "sculpt spilling outside leased plot into read-only tile must be denied"
    );

    // Outsider "dan" cannot edit inside eve's plot (plot's visitor_policy = read_only).
    assert!(
        rules
            .check(
                "dan",
                &Action::Sculpt { lo: [80.5, 0.2, 30.5], hi: [81.0, 0.8, 31.0], player_size: SIZES[0].1, is_water: false },
            )
            .is_err()
    );
    // Tile owner "carol" still retains override authority inside her own tile.
    assert!(
        rules
            .check(
                "carol",
                &Action::Sculpt { lo: [80.5, 0.2, 30.5], hi: [81.0, 0.8, 31.0], player_size: SIZES[1].1, is_water: false },
            )
            .is_ok()
    );
}

#[test]
fn pond_expansion_fills_carved_channel_supports_tiny_and_person_physics_and_respects_dams() {
    // In `areas/garden`: pond is centered at (22, 0, 36), water level = -0.3, shore radius ≈ 7.7m
    // (pond water ends near x ≈ 29.7 at z = 36).
    let mut base = garden(7);
    let mut edits = EditLog::new();
    let mut water = WaterLog::new();

    // Sanity check: (28.0, -0.42, 36.0) is inside the base pond water, while (30.5, -0.5, 36.0) is solid dirt.
    let mut probe = Vec::new();
    base.sample(&[[28.0, -0.42, 36.0], [30.5, -0.5, 36.0]], SIZES[1].1, &mut probe).unwrap();
    assert_eq!(probe[0].material, Material::Water);
    assert!(probe[0].distance < 0.0, "expected water inside base pond: {:?}", probe[0]);
    assert_ne!(probe[1].material, Material::Water);
    assert!(probe[1].distance < 0.0, "expected solid bank outside base pond: {:?}", probe[1]);

    // 1. Dig a dry pit FAR from the pond at (46, -0.8, 48): must stay completely dry!
    let dry_pit = Edit::carve(Shape::Sphere { radius: 1.2 }, [46.0, -0.7, 48.0], 0.1, None, SIZES[1].1);
    let dry_res = water.expand_from_carve(&mut base, &edits, &dry_pit, Some("alice".into())).unwrap();
    assert!(dry_res.is_none(), "carving far from pond must not create water");
    assert!(water.is_empty());
    edits.push(dry_pit).unwrap();

    // 2. Dig a pre-dug dry basin at (32.2, -0.6, 36.0) separated by a dirt wall from the pond:
    //    stays dry until the separating bank is breached!
    let pre_dug = Edit::carve(Shape::Sphere { radius: 1.4 }, [32.2, -0.6, 36.0], 0.1, None, SIZES[1].1);
    assert!(water.expand_from_carve(&mut base, &edits, &pre_dug, Some("alice".into())).unwrap().is_none());
    assert!(water.is_empty(), "isolated basin must stay dry before breach");
    edits.push(pre_dug).unwrap();

    // 3. Now carve through the pond bank at (29.7, -0.55, 36.0) connecting pond water to the pre-dug basin!
    let breach = Edit::carve(Shape::Sphere { radius: 1.8 }, [29.7, -0.55, 36.0], 0.1, None, SIZES[1].1);
    let wb = water.expand_from_carve(&mut base, &edits, &breach, Some("alice".into())).unwrap();
    assert!(wb.is_some(), "breaching the pond bank must trigger hydrostatic expansion");
    edits.push(breach).unwrap();

    // Both the breach AND the transitively connected pre-dug basin should now belong to one WaterBody at y ≈ -0.3!
    assert_eq!(water.bodies().len(), 1);
    let level = water.bodies()[0].level;
    assert!((level - (-0.3)).abs() < 0.05, "detected pond water level should be ~ -0.3, got {level}");
    assert_eq!(water.bodies()[0].basins.len(), 2, "breach + transitively connected pre-dug basin should both flood");

    // Compose the world with the expanded hydrostatic water layer!
    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits.clone()), Vec::new(), Arc::new(water.clone()), 1)),
    );

    // Check SDF samples in the newly flooded channel & pre-dug basin:
    // - (29.7, -0.55, 36.0) and (32.2, -0.55, 36.0): submerged -> negative Water!
    // - (32.2, -0.05, 36.0): air above water level -> positive!
    // - (32.2, -2.5, 36.0): solid floor underneath -> negative non-Water!
    // - (46.0, -0.7, 48.0): far dry pit -> positive air (still dry)!
    let mut ans = Vec::new();
    composed
        .sample(
            &[
                [29.7, -0.55, 36.0],
                [32.2, -0.55, 36.0],
                [32.2, -0.05, 36.0],
                [32.2, -2.5, 36.0],
                [46.0, -0.7, 48.0],
            ],
            SIZES[1].1,
            &mut ans,
        )
        .unwrap();
    assert!(ans[0].distance < -0.1 && ans[0].material == Material::Water, "breach should be filled with water: {:?}", ans[0]);
    assert!(ans[1].distance < -0.1 && ans[1].material == Material::Water, "transitively connected basin should flood: {:?}", ans[1]);
    assert!(ans[2].distance > 0.15, "space above water level must be open air: {:?}", ans[2]);
    assert!(ans[3].distance < -0.1 && ans[3].material != Material::Water, "solid dirt must stay beneath carved basin: {:?}", ans[3]);
    assert!(ans[4].distance > 0.2, "far dry pit must remain dry air: {:?}", ans[4]);

    // 4. Multi-scale physics on the expanded pond:
    //    - Tiny stands right on the newly expanded water surface at (32.2, ~ -0.3, 36.0)!
    //    - Person wades through the expanded water down to the carved dirt floor (~ -1.8 m)!
    let mut tiny = Body::standing(&mut composed, [32.2, 0.2, 36.0], SIZES[0].1).unwrap();
    assert_eq!(tiny.on, Some(Material::Water), "tiny must stand on expanded water surface");
    assert!((tiny.feet[1] - level as f64).abs() < 0.03, "tiny feet should sit on water level {level}, got {}", tiny.feet[1]);
    // Tiny walks from the expanded basin back into the original pond seamlessly!
    tiny.yaw = std::f32::consts::FRAC_PI_2; // west (-X)
    for _ in 0..80 {
        tiny.step(&mut composed, Intent { forward: 1.0, ..Intent::default() }, 1.0 / 60.0).unwrap();
    }
    assert_eq!(tiny.on, Some(Material::Water));

    let person = Body::standing(&mut composed, [32.2, 0.5, 36.0], SIZES[1].1).unwrap();
    assert_ne!(person.on, Some(Material::Water), "person must wade through water to solid floor");
    assert!(person.feet[1] < level as f64 - 0.8, "person feet should stand on carved pond bed below water, got {}", person.feet[1]);

    // 5. Adding a solid dirt dam (`Op::Add`) across the middle of the channel cuts cleanly through water!
    let mut edits_with_dam = edits.clone();
    let dam = Edit::add(Shape::Sphere { radius: 1.0 }, [31.0, -0.3, 36.0], 0.05, Material::Soil, SIZES[1].1);
    edits_with_dam.push(dam).unwrap();
    let mut dammed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits_with_dam), Vec::new(), Arc::new(water.clone()), 2)),
    );
    dammed.sample(&[[31.0, -0.45, 36.0]], SIZES[1].1, &mut ans).unwrap();
    assert!(ans[0].distance < -0.2 && ans[0].material == Material::Soil, "solid dam must cleanly cut water: {:?}", ans[0]);

    // 6. Undoing the breach edit (`WaterLog::recompute_from_edits`) recedes the water cleanly!
    let mut edits_undone = edits.clone();
    edits_undone.pop(); // remove breach
    let receded = WaterLog::recompute_from_edits(&mut base, &edits_undone).unwrap();
    assert!(receded.is_empty(), "undoing the pond breach must recede the expanded water");

    // 7. SDF distance laws still hold on the expanded pond!
    let d = composed.describe().unwrap();
    assert_eq!(laws::never_too_long(&mut composed, &d, 1, 200).unwrap(), Ok(()));
    assert_eq!(laws::surfaces_have_an_inside(&mut composed, &d, 1, 200).unwrap(), Ok(()));
}

#[test]
fn world_save_phase_a_manifest_round_trips_bit_exactly_and_preserves_legacy_saves() {
    // 1. Legacy Step 8a/8b/8c save still parses cleanly with default empty tiles/rules/water.
    let legacy = "plateau-world 1\narea The walled garden\nseed 7\n\
                  sdf add sphere 32 0 48 1.5 blend 0.3 mat soil size 1.7\n\
                  element 1 elements/modules/tree.wasm seed 7 pos 32 0 48 yaw 0.5 scale 1 size 1.7 after 1\n";
    let parsed_legacy = WorldSave::from_text(legacy).unwrap();
    assert_eq!(parsed_legacy.edits.len(), 1);
    assert_eq!(parsed_legacy.elements.len(), 1);
    assert!(parsed_legacy.tiles.is_empty());
    assert!(parsed_legacy.rules.is_empty());
    assert!(parsed_legacy.water.is_empty());
    assert!(parsed_legacy.edit_authors.is_empty());
    assert_eq!(parsed_legacy.elements[0].author, None);

    // 2. Full Phase A manifest: multi-tile atlas + rules (AreaPolicy + Plot) + WaterBody +
    //    authored edits + authored elements!
    let mut rules = RuleBook::new();
    let mut pol = AreaPolicy::owned([0, 0], "alice");
    pol.owners.push("bob".into());
    pol.builders = vec!["carol".into()];
    pol.builder_policy = RolePolicy::builder();
    pol.visitor_policy = RolePolicy::tiny_visitor();
    pol.border_guard = 2.0;
    pol.spawn_guard = 4.5;
    rules.set_area(pol);
    rules.set_plot(Plot {
        id: 1,
        name: "oak-door".into(),
        lo: [10.0, 0.0, 12.0],
        hi: [12.5, 2.5, 14.5],
        owners: vec!["eve".into()],
        builders: vec!["dan".into()],
        builder_policy: RolePolicy::tiny_visitor(),
        visitor_policy: Some(RolePolicy::read_only()),
    });

    let water_body = WaterBody {
        id: 1,
        level: -0.3,
        seed_point: [29.8, -0.3, 36.0],
        basins: vec![
            WaterBasin { shape: Shape::Sphere { radius: 1.6 }, center: [30.4, -0.55, 36.0], pad: 0.064 },
            WaterBasin { shape: Shape::Cylinder { radius: 1.2, half_height: 0.8, round: 0.1 }, center: [32.8, -0.6, 36.0], pad: 0.05 },
        ],
        author: Some("alice".into()),
    };

    let edits = vec![
        Edit::carve(Shape::Sphere { radius: 1.6 }, [30.4, -0.55, 36.0], 0.1, None, SIZES[1].1),
        Edit::add(Shape::Box { half: [0.5, 0.3, 0.4], round: 0.05 }, [20.0, 0.2, 20.0], 0.0, Material::Stone, SIZES[1].1),
    ];
    let elements = vec![PlacedElement {
        id: 1,
        module: "elements/modules/tree.wasm".into(),
        seed: 42,
        pos: [25.0, 0.0, 42.0],
        yaw: 0.75,
        rot: None,
        scale: 1.0,
        stamp_size: 1.7,
        after_edits: 1,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: Some("eve".into()),
    }];

    let mut save = WorldSave::with_elements("The walled garden", 7, edits, elements);
    save.tiles = vec![
        SavedTile { at: [0, 0], module: "made/pond-sketched/area.wasm".into(), seed: 7 },
        SavedTile { at: [1, 0], module: "made/forest-beside-pond/area.wasm".into(), seed: 11 },
    ];
    save.rules = rules;
    save.water = vec![water_body];
    save.edit_authors = vec![Some("alice".into()), Some("carol".into())];

    let tmp = std::env::temp_dir().join(format!("plateau-phase-a-{}.plateau", std::process::id()));
    save.save(&tmp).unwrap();
    let reloaded = WorldSave::load(&tmp).unwrap();
    let _ = std::fs::remove_file(&tmp);

    assert_eq!(save, reloaded);

    // Chunk mesh of the expanded pond before and after save/reload matches bit-for-bit!
    let build = |w: &WorldSave| {
        let log = Arc::new(EditLog::from_edits(w.edits.clone()).unwrap());
        let wl = Arc::new(WaterLog::from_bodies(w.water.clone()).unwrap());
        Composed::new(garden(w.seed), Arc::new(ComposedLog::new_with_water(log, Vec::new(), wl, 1)))
    };
    let mut a = build(&save);
    let mut b = build(&reloaded);
    let key = chunk::Key { level: -4, at: [15, -1, 18], asked: -4 };
    let ma = chunk::make(&mut a, key).unwrap().mesh.expect("pond chunk has mesh");
    let mb = chunk::make(&mut b, key).unwrap().mesh.expect("reloaded pond chunk has mesh");
    assert_eq!(ma.positions, mb.positions);
    assert_eq!(ma.indices, mb.indices);
    assert_eq!(ma.materials, mb.materials);
}

#[test]
fn role_policy_encode_decode_round_trips() {
    for p in [RolePolicy::full(), RolePolicy::builder(), RolePolicy::tiny_visitor(), RolePolicy::read_only()] {
        let enc = p.encode();
        let dec = RolePolicy::decode(&enc).unwrap();
        assert_eq!(p, dec);
    }
    let custom = RolePolicy {
        visit: true,
        interact: false,
        sculpt_max: Some(Size::Person),
        place_max: Some(Size::Tiny),
        water_max: Some(Size::Giant),
        cut_carry: CutCarryPolicy::OwnOnly,
    };
    assert_eq!(RolePolicy::decode(&custom.encode()).unwrap(), custom);
}

#[test]
fn expanded_water_meets_old_pond_seamlessly_with_smooth_carve_blend() {
    // Regression test for the curved outline/groove seam between old pond water and newly expanded
    // water when sculpting with a smooth `blend > 0` brush (like `HillSphere`, `blend = 0.35 * r`).
    let mut base = garden(7);
    let mut edits = EditLog::new();
    let mut water = WaterLog::new();

    // Smooth-blended carve right across the garden pond shoreline (pond water level = -0.3).
    // `r = 1.275` (default person brush), `blend = 0.35 * r ≈ 0.446 m`!
    let r = 1.275f32;
    let blend = r * 0.35;
    let breach = Edit::carve(Shape::Sphere { radius: r }, [29.2, -0.1, 36.0], blend, None, SIZES[1].1);
    let wb = water.expand_from_carve(&mut base, &edits, &breach, None).unwrap();
    assert!(wb.is_some(), "breach must expand pond");
    edits.push(breach).unwrap();

    // 1. Detected water level must match exact pond level (-0.3) to sub-millimetre precision!
    let level = water.bodies()[0].level;
    assert!((level - (-0.3)).abs() < 0.001, "water level drifted from -0.3: {level}");

    // 2. Sweep densely from deep inside the old pond (x = 26.0) all the way through the carve
    //    rim & blend ring (x = 27.0..29.5) right at `y = -0.3`: distance must stay identically 0.0
    //    and material must stay `Material::Water` with ZERO trench, ridge, or step!
    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits), Vec::new(), Arc::new(water), 1)),
    );
    let mut pts = Vec::new();
    for i in 0..=80 {
        let x = 26.0 + (i as f32 / 80.0) * 3.8; // crosses d_shape = +2*blend -> +blend -> 0 -> -r
        pts.push([x, -0.3, 36.0]);
    }
    let mut ans = Vec::new();
    composed.sample(&pts, SIZES[1].1, &mut ans).unwrap();
    for (p, a) in pts.iter().zip(&ans) {
        assert_eq!(a.material, Material::Water, "non-water seam at x={:.3}: {a:?}", p[0]);
        assert!(a.distance.abs() < 0.001, "water surface groove/step at x={:.3}: d={:.5}", p[0], a.distance);
    }

    // Also verify 3cm above and 3cm below the water surface across the blend boundary!
    for dy in [-0.03f32, 0.03f32] {
        let shifted: Vec<[f32; 3]> = pts.iter().map(|p| [p[0], -0.3 + dy, p[2]]).collect();
        composed.sample(&shifted, SIZES[1].1, &mut ans).unwrap();
        for (p, a) in shifted.iter().zip(&ans) {
            assert_eq!(a.material, Material::Water, "seam at x={:.3} dy={dy}: {a:?}", p[0]);
            assert!((a.distance - dy).abs() < 0.002, "SDF distorted at x={:.3} dy={dy}: d={:.5}", p[0], a.distance);
        }
    }
}

#[test]
fn tiny_player_carving_at_shallow_shoreline_expands_water_without_receding_pond() {
    // Regression test: a tiny player (1.7 cm, default brush r = 1.275 cm) carving right at the
    // shallow shoreline must detect the pond water and expand it into the tiny carved channel
    // rather than carving a bite out of the pond and making it recede.
    let mut base = garden(7);
    let mut edits = EditLog::new();
    let mut water = WaterLog::new();

    // Find the exact shallow shoreline x along z = 36 where water meets mud at y = -0.30.
    let mut shore_x = 28.0f32;
    let mut probe = Vec::new();
    for i in 0..400 {
        let x = 27.5 + i as f32 * 0.01;
        base.sample(&[[x, -0.302, 36.0]], SIZES[0].1, &mut probe).unwrap();
        if probe[0].material == Material::Water && probe[0].distance <= 0.0 {
            shore_x = x;
        } else {
            break;
        }
    }

    // Tiny player carves a 3-stamp micro-channel outward from the exact shallow water edge!
    let r = 0.75 * SIZES[0].1; // 0.01275 m
    let blend = r * 0.35;
    for step in 0..3 {
        let cx = shore_x + step as f32 * (r * 0.7);
        let carve = Edit::carve(Shape::Sphere { radius: r }, [cx, -0.30, 36.0], blend, None, SIZES[0].1);
        let mut world_before = Composed::new(
            garden(7),
            Arc::new(ComposedLog::new_with_water(Arc::new(edits.clone()), Vec::new(), Arc::new(water.clone()), step as u64 + 1)),
        );
        let wb = water.expand_from_carve(&mut world_before, &edits, &carve, None).unwrap();
        assert!(wb.is_some(), "tiny carve step {step} at shallow shore x={cx:.4} must trigger water expansion");
        edits.push(carve).unwrap();
    }

    let level = water.bodies()[0].level;
    assert!((level - (-0.3)).abs() < 0.001, "tiny detected level drifted: {level}");

    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits), Vec::new(), Arc::new(water), 10)),
    );
    // Verify BOTH the old pond edge (shore_x - 0.02) AND the newly carved tiny channel
    // (shore_x + 1.4*r) have water right at y = -0.30 with zero recession!
    let mut ans = Vec::new();
    composed
        .sample(
            &[
                [shore_x - 0.02, -0.304, 36.0],
                [shore_x, -0.304, 36.0],
                [shore_x + r * 1.4, -0.304, 36.0],
            ],
            SIZES[0].1,
            &mut ans,
        )
        .unwrap();
    for (idx, a) in ans.iter().enumerate() {
        assert_eq!(a.material, Material::Water, "tiny shoreline sample {idx} receded/dried: {a:?}");
        assert!(a.distance < 0.0, "tiny shoreline sample {idx} should be submerged water: {a:?}");
    }
}

#[test]
fn tiny_cave_breaching_gentle_pond_sketched_shoreline_fills_with_water() {
    // Regression test against the real `made/pond-sketched/area.wasm` (`ground * 0.65`, gentle
    // sloping mud/sand beach, LEVEL = -0.35): a tiny player digs a dry cave inland from the
    // shoreline and then breaches the thin beach lip into the pond — water must flood the entire
    // tiny cave cleanly without receding!
    let wasm_path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("made/pond-sketched/area.wasm");
    if !wasm_path.exists() {
        return;
    }
    let bytes = std::fs::read(&wasm_path).unwrap();
    let compiled = plateau_engine::Compiled::new(&bytes, plateau_engine::Limits::default()).unwrap();
    let mut base = compiled.instantiate(7).unwrap();

    // Walk north (-Z) from dry southern beach (29, -0.351, 53) inward until hitting pond water!
    let mut shore_z = 53.0f32;
    let mut probe = Vec::new();
    for i in 0..1500 {
        let z = 53.0 - i as f32 * 0.01;
        base.sample(&[[29.0, -0.351, z]], SIZES[0].1, &mut probe).unwrap();
        if probe[0].material == Material::Water && probe[0].distance <= 0.0 {
            shore_z = z;
            break;
        }
    }
    assert!(shore_z < 52.5 && shore_z > 40.0, "expected south pond shore in (40, 52.5), found {shore_z}");

    let r = 0.75 * SIZES[0].1; // 0.01275 m
    let blend = r * 0.35;
    let mut edits = EditLog::new();
    let mut water = WaterLog::new();

    // 1. Dig a tiny dry cave 3 cm inland from the water's edge first (separated by ~9 mm of solid mud: stays dry!).
    let cave = Edit::carve(Shape::Sphere { radius: r }, [29.0, -0.354, shore_z + 0.030], blend, None, SIZES[0].1);
    let mut world0 = Composed::new(compiled.instantiate(7).unwrap(), Arc::new(ComposedLog::empty()));
    assert!(water.expand_from_carve(&mut world0, &edits, &cave, None).unwrap().is_none(), "inland tiny cave should start dry");
    edits.push(cave).unwrap();

    // 2. Breach the mud lip right at `shore_z + 0.012` (grazing the razor-thin shoreline and connecting to the cave!).
    let breach = Edit::carve(Shape::Sphere { radius: r }, [29.0, -0.352, shore_z + 0.012], blend, None, SIZES[0].1);
    let mut world1 = Composed::new(
        compiled.instantiate(7).unwrap(),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits.clone()), Vec::new(), Arc::new(water.clone()), 1)),
    );
    let wb = water.expand_from_carve(&mut world1, &edits, &breach, None).unwrap();
    assert!(wb.is_some(), "breaching pond-sketched gentle beach at tiny scale must trigger water expansion");
    edits.push(breach).unwrap();

    let level = water.bodies()[0].level;
    assert!((level - (-0.35)).abs() < 0.002, "expected pond-sketched level ~ -0.35, got {level}");

    // Verify BOTH the breach AND the previously dry tiny cave are now filled with water!
    let mut composed = Composed::new(
        compiled.instantiate(7).unwrap(),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits), Vec::new(), Arc::new(water), 2)),
    );
    let mut ans = Vec::new();
    composed
        .sample(
            &[
                [29.0, -0.354, shore_z - 0.01],
                [29.0, -0.354, shore_z + 0.012],
                [29.0, -0.354, shore_z + 0.030],
            ],
            SIZES[0].1,
            &mut ans,
        )
        .unwrap();
    for (idx, a) in ans.iter().enumerate() {
        assert_eq!(a.material, Material::Water, "tiny cave sample {idx} stayed dry: {a:?}");
        assert!(a.distance < 0.0, "tiny cave sample {idx} should be submerged water: {a:?}");
    }
}

#[test]
fn distinct_water_elevations_do_not_merge_or_lift_main_pond() {
    // Regression test: carving inside an elevated pool (like the tiny grotto pool at y = -0.252
    // in `pond-sketched`, ~10 cm higher than the main pond) must NOT cause a subsequent person-scale
    // expansion of the main pond to merge into the elevated pool and lift the pond water up!
    let mut base = garden(7);
    let mut edits = EditLog::new();
    let mut water = WaterLog::new();

    // Seed an elevated tiny pool WaterBody ~9 cm higher than the garden pond (-0.30).
    water
        .push_body(WaterBody {
            id: 1,
            level: -0.21,
            seed_point: [10.0, -0.21, 10.0],
            basins: vec![WaterBasin { shape: Shape::Sphere { radius: 0.05 }, center: [10.0, -0.22, 10.0], pad: 0.01 }],
            author: None,
        })
        .unwrap();

    // Now expand the main garden pond (y = -0.30) at person scale!
    let breach = Edit::carve(Shape::Sphere { radius: 1.5 }, [29.4, -0.45, 36.0], 0.35 * 1.5, None, SIZES[1].1);
    let wb = water.expand_from_carve(&mut base, &edits, &breach, None).unwrap();
    assert!(wb.is_some());
    edits.push(breach).unwrap();

    assert_eq!(water.bodies().len(), 2, "distinct water elevations must remain separate WaterBodies");
    assert!((water.bodies()[0].level - (-0.21)).abs() < 0.001);
    assert!(
        (water.bodies()[1].level - (-0.30)).abs() < 0.002,
        "main pond expansion must stay at -0.30, not lift to -0.21 (got {})",
        water.bodies()[1].level
    );

    // At y = -0.25 (5 cm above the main pond, below the elevated pool), the expanded pond must be AIR!
    let mut composed = Composed::new(
        garden(7),
        Arc::new(ComposedLog::new_with_water(Arc::new(edits), Vec::new(), Arc::new(water), 2)),
    );
    let mut ans = Vec::new();
    composed.sample(&[[29.4, -0.25, 36.0], [29.4, -0.35, 36.0]], SIZES[1].1, &mut ans).unwrap();
    assert!(ans[0].distance > 0.04, "water must NOT lift above -0.30 at main pond: {:?}", ans[0]);
    assert!(ans[1].distance < -0.03 && ans[1].material == Material::Water, "water must fill below -0.30: {:?}", ans[1]);
}
