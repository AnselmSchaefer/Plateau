//! Step 9 tests: two areas side by side — border extraction, `borders_match` law,
//! seamless C¹ `Atlas` SDF stitching, multi-tile chunks & `rings`, walking across the border
//! at every size (`tiny`, `person`, `giant`), `reach::World::new_in`, multi-area `SceneWorld`
//! owner isolation, and cross-border sculpting.

mod played;

use std::sync::Arc;

use plateau_engine::atlas::{Atlas, BorderProfile, SEAM_HALF, Side};
use plateau_engine::body::{Body, Intent};
use plateau_engine::chunk::{self, Key, Making};
use plateau_engine::compose::{Composed, ComposedLog};
use plateau_engine::edit::{Edit, EditLog, Op, Shape};
use plateau_engine::reach::World;
use plateau_engine::scene::{AREA_OWNER_BASE, BASE_OWNER, SceneWorld, is_area_owner, tile_owner};
use plateau_engine::{Description, PlayerState, Region, SIZES, Size, WIDTH, laws, rings};
use plateau_sdk::{
    Answer, Area, Borders, Command, Edge, Material, MeshBuilder, Place, Point, Quat, Scene, SceneBuilder, View, between, detail, ease, edge_height,
    instance, point,
};
use played::Native;

const SEED: u64 = 7;

/// Shared height profile along `x = 64` (west tile's east edge / east tile's west edge):
/// gentle rolling meadow (`0.2..0.7 m`) dipping into a stream channel (`z ∈ [24, 40]`, bed `-0.85 m`,
/// water level `-0.35 m`).
fn shared_edge_height(z: f32) -> f32 {
    let hill = 0.35 + 0.25 * plateau_sdk::sin(z * 0.12);
    let stream_d = plateau_sdk::abs(z - 32.0);
    let channel = 1.2 * ease(1.0 - stream_d / 9.0);
    hill - channel
}

const STREAM_WATER: f32 = -0.35;

struct TileArea {
    seed: u32,
    name: &'static str,
    /// `true` for the western tile `(0, 0)` (matches east edge `x = 64`), `false` for eastern tile `(1, 0)` (matches west edge `x = 0`).
    is_west: bool,
    height_offset: f32,
    water_offset: Option<f32>,
    dry_material: Material,
    lie_borders: bool,
    with_scene: bool,
}

impl TileArea {
    fn west() -> Self {
        TileArea {
            seed: 101,
            name: "West Meadow",
            is_west: true,
            height_offset: 0.0,
            water_offset: Some(STREAM_WATER),
            dry_material: Material::Grass,
            lie_borders: false,
            with_scene: true,
        }
    }

    fn east() -> Self {
        TileArea {
            seed: 202,
            name: "East Meadow",
            is_west: false,
            height_offset: 0.0,
            water_offset: Some(STREAM_WATER),
            dry_material: Material::Grass,
            lie_borders: false,
            with_scene: true,
        }
    }

    fn ground_height(&self, x: f32, z: f32) -> f32 {
        let edge_h = shared_edge_height(z) + self.height_offset;
        let dist_from_seam = if self.is_west { WIDTH - x } else { x };
        let interior = between(dist_from_seam, 0.0, 8.0);
        let extra = 0.4 * plateau_sdk::noise(point(x / 14.0, 0.0, z / 14.0), self.seed);
        edge_h + interior * extra
    }
}

impl Area for TileArea {
    fn new(_: u64) -> Self {
        Self::west()
    }

    fn describe(&self) -> plateau_sdk::Description {
        let borders = if self.lie_borders {
            Borders::of_ground(|_, _| 4.0)
        } else {
            Borders {
                north: Edge::along(|x| self.ground_height(x, 0.0)),
                east: Edge::along_with_water(|z| self.ground_height(WIDTH, z), self.water_offset),
                south: Edge::along(|x| self.ground_height(x, WIDTH)),
                west: Edge::along_with_water(|z| self.ground_height(0.0, z), self.water_offset),
            }
        };
        let place_x = if self.is_west { 20.0 } else { 32.0 };
        let place_z = 52.0;
        let place_y = self.ground_height(place_x, place_z);
        plateau_sdk::Description {
            name: self.name.into(),
            places: vec![Place::new(
                if self.is_west { "west clearing" } else { "east clearing" },
                plateau_sdk::Size::Person,
                point(place_x, place_y, place_z),
            )],
            borders,
        }
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let dist_from_seam = if self.is_west { WIDTH - p.x } else { p.x };
        let rough_fade = between(dist_from_seam, 0.0, 2.0);
        let h = self.ground_height(p.x, p.z) + rough_fade * detail(point(p.x, 0.0, p.z), view, 1.2, 0.04, self.seed ^ 0x55);
        let mat = if self.water_offset.is_some_and(|w| h < w + 0.15) { Material::Mud } else { self.dry_material };
        let ground = Answer::new((p.y - h) * 0.7, mat);
        if let Some(wl) = self.water_offset {
            let water = Answer::new(p.y - wl, Material::Water).cut(ground.distance);
            ground.or(water)
        } else {
            ground
        }
    }

    fn scene(&self) -> Scene {
        if !self.with_scene {
            return Scene { meshes: Vec::new(), instances: Vec::new(), lights: Vec::new() };
        }
        let mut mb = MeshBuilder::new();
        mb.cuboid(point(0.0, 0.5, 0.0), point(0.3, 0.5, 0.3), Material::Wood);
        let mut sb = SceneBuilder::new();
        let m = sb.add_mesh(mb.build());
        let y = self.ground_height(16.0, 16.0);
        sb.spawn(1, m, point(16.0, y, 16.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sb.light(1, point(16.0, y + 1.5, 16.0), [1.0, 0.8, 0.4], 800.0, 8.0);
        sb.build()
    }

    fn interact(&mut self, _player: &plateau_sdk::PlayerState) -> Vec<Command> {
        vec![Command::Say(format!("hello from {}", self.name))]
    }

    fn tick(&mut self, _dt: f32, _player: &plateau_sdk::PlayerState) -> Vec<Command> {
        let y = self.ground_height(16.0, 16.0);
        vec![Command::SetTransform(instance(1, 0, point(16.5, y, 16.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true))]
    }
}

#[test]
fn single_tile_atlas_is_bit_identical_to_raw_region() {
    let mut raw = played::area("garden", SEED);
    let d = raw.describe().unwrap();
    let keys = laws::kept_chunks(&d);
    let raw_hashes = laws::chunk_hashes(&mut raw, &keys).unwrap();

    let mut atlas = Atlas::single(played::area("garden", SEED));
    let atlas_hashes = laws::chunk_hashes(&mut atlas, &keys).unwrap();
    assert_eq!(raw_hashes, atlas_hashes);
}

#[test]
fn borders_match_passes_on_continuous_pair_and_catches_every_mismatch() {
    let mut west = Native(Box::new(TileArea::west()));
    let mut east = Native(Box::new(TileArea::east()));

    // Border extraction & prompt formatting.
    let profile = BorderProfile::extract(&mut west, Side::East).unwrap();
    assert_eq!(profile.samples.len(), 65);
    assert!((profile.water_level().unwrap() - STREAM_WATER).abs() < 0.02);
    let prompt = profile.describe_for_prompt(Side::West);
    assert!(prompt.contains("const WEST_GROUND: [f32; 65]"), "{prompt}");
    assert!(prompt.contains("water level `y = -0.35"), "{prompt}");
    assert!(prompt.contains("grass"), "{prompt}");

    // Matching pair passes both directions!
    assert_eq!(laws::borders_match(&mut east, Side::West, &mut west).unwrap(), Ok(()));
    assert_eq!(laws::borders_match(&mut west, Side::East, &mut east).unwrap(), Ok(()));

    // Also test SDK `edge_height` interpolation directly against `profile.solid_heights()`.
    let heights = profile.solid_heights();
    assert!((edge_height(&heights, 10.5) - 0.5 * (heights[10] + heights[11])).abs() < 1e-5);

    // 1. Height mismatch (> 15 cm) fails with clear message.
    let mut tall_east = Native(Box::new(TileArea { height_offset: 0.45, ..TileArea::east() }));
    let err = laws::borders_match(&mut tall_east, Side::West, &mut west).unwrap().unwrap_err();
    assert!(err.contains("borders do not match") && err.contains("ground") && err.contains("apart"), "{err}");

    // 2. Missing water where neighbour has water fails.
    let mut dry_east = Native(Box::new(TileArea { water_offset: None, ..TileArea::east() }));
    let err = laws::borders_match(&mut dry_east, Side::West, &mut west).unwrap().unwrap_err();
    assert!(err.contains("water") && err.contains("West Meadow"), "{err}");

    // 3. Mismatched water level (> 5 cm) fails.
    let mut high_water_east = Native(Box::new(TileArea { water_offset: Some(STREAM_WATER + 0.25), ..TileArea::east() }));
    let err = laws::borders_match(&mut high_water_east, Side::West, &mut west).unwrap().unwrap_err();
    assert!(err.contains("water") && err.contains("apart"), "{err}");

    // 4. Material mismatch on dry land fails.
    let mut stone_east = Native(Box::new(TileArea { dry_material: Material::Stone, ..TileArea::east() }));
    let err = laws::borders_match(&mut stone_east, Side::West, &mut west).unwrap().unwrap_err();
    assert!(err.contains("material") && err.contains("grass") && err.contains("stone"), "{err}");

    // 5. Fabricated `describe().borders` fails.
    let mut lying_east = Native(Box::new(TileArea { lie_borders: true, ..TileArea::east() }));
    let err = laws::borders_match(&mut lying_east, Side::West, &mut west).unwrap().unwrap_err();
    assert!(err.contains("declared west border disagrees"), "{err}");
}

#[test]
fn stitched_atlas_keeps_sdf_laws_and_seamless_chunks_across_border() {
    // Give east tile a slight 0.08 m vertical drift (within 0.15 m tolerance) and distinct noise seed
    // to verify smooth C¹ seam blending keeps all SDF laws and chunk continuity intact.
    let west = Native(Box::new(TileArea::west()));
    let east = Native(Box::new(TileArea { height_offset: 0.08, ..TileArea::east() }));
    let mut atlas = Atlas::new(vec![([0, 0], west), ([1, 0], east)]).unwrap();
    assert_eq!(atlas.seam_half(), SEAM_HALF);

    let seam_desc = Description {
        name: "Seam check".into(),
        places: vec![
            plateau_engine::Place { name: "seam-dry".into(), size: Size::Tiny, at: [WIDTH, shared_edge_height(12.0), 12.0] },
            plateau_engine::Place { name: "seam-water".into(), size: Size::Person, at: [WIDTH, STREAM_WATER, 32.0] },
            plateau_engine::Place { name: "seam-south".into(), size: Size::Giant, at: [WIDTH, shared_edge_height(52.0), 52.0] },
        ],
        borders: atlas.describe().unwrap().borders,
    };

    assert_eq!(laws::sizes_agree(&mut atlas, &seam_desc, 1, 200).unwrap(), Ok(()));
    assert_eq!(laws::never_too_long(&mut atlas, &seam_desc, 1, 200).unwrap(), Ok(()));
    assert_eq!(laws::surfaces_have_an_inside(&mut atlas, &seam_desc, 1, 200).unwrap(), Ok(()));

    // Coarse vs full chunk meshing agrees across the x = 64 border at person and tiny scales.
    for (level, at) in [
        (-4, [31, 0, 6]),   // person chunk ending at x = 64
        (-4, [32, 0, 6]),   // person chunk starting at x = 64
        (-11, [4095, 20, 768]), // tiny chunk right west of x = 64
        (-11, [4096, 20, 768]), // tiny chunk right east of x = 64
    ] {
        let key = Key::new(level, at);
        let coarse = chunk::make_with(&mut atlas, key, Making { coarse: true, skirts: false, shading: false }).unwrap();
        let full = chunk::make_with(&mut atlas, key, Making { coarse: false, skirts: false, shading: false }).unwrap();
        assert_eq!(coarse.mesh, full.mesh, "coarse vs full mismatch at {key:?}");
    }

    // Rings & shown_over cover both tiles with 0 holes when chunks are made.
    let tiles = atlas.tile_coords();
    let mut rings = rings::Rings::with_tiles(tiles.clone());
    let wanted = rings.wanted([WIDTH as f64, 2.0, 12.0], SIZES[1].1);
    assert!(wanted.iter().any(|k| k.origin()[0] < WIDTH as f64), "should include west tile chunks");
    assert!(wanted.iter().any(|k| k.origin()[0] >= WIDTH as f64), "should include east tile chunks");
    let (shown, holes) = rings::shown_over(&wanted, &tiles, &|_| true);
    assert_eq!(holes, 0);
    assert!(!shown.is_empty());
}

#[test]
fn walking_across_shared_border_at_every_size_and_reach() {
    let west = Native(Box::new(TileArea::west()));
    let east = Native(Box::new(TileArea { height_offset: 0.08, ..TileArea::east() }));
    let mut atlas = Atlas::new(vec![([0, 0], west), ([1, 0], east)]).unwrap();

    // Walk east across x = 64 at all three sizes: tiny (dry + over water!), person, and giant.
    let cases: [(&str, f32, [f64; 2], f64, usize); 4] = [
        ("tiny on dry land", SIZES[0].1, [63.85, 12.0], 64.15, 600),
        ("tiny on shared stream water", SIZES[0].1, [63.85, 32.0], 64.15, 600),
        ("person on dry land", SIZES[1].1, [60.0, 50.0], 68.0, 400),
        ("giant across border", SIZES[2].1, [50.0, 50.0], 78.0, 200),
    ];

    for (label, size, start_xz, target_x, max_steps) in cases {
        let mut body = Body::standing(&mut atlas, [start_xz[0], 30.0, start_xz[1]], size).unwrap();
        assert!(body.on.is_some(), "{label}: should start grounded");
        body.yaw = -std::f32::consts::FRAC_PI_2; // face east (+x)
        let mut prev_y = body.feet[1];
        let mut max_dy = 0.0f64;
        for _ in 0..max_steps {
            body.step(&mut atlas, Intent { forward: 1.0, run: true, ..Intent::default() }, 1.0 / 60.0).unwrap();
            let dy = (body.feet[1] - prev_y).abs();
            max_dy = max_dy.max(dy);
            prev_y = body.feet[1];
            if body.feet[0] >= target_x {
                break;
            }
        }
        assert!(body.feet[0] >= target_x, "{label}: failed to cross border (stopped at x = {:.3})", body.feet[0]);
        assert!(body.on.is_some(), "{label}: lost ground after crossing border");
        // Smooth transition: no sudden vertical jump larger than half a step height per 1/60s frame.
        assert!(max_dy < (size as f64) * 0.25, "{label}: vertical seam jump {max_dy:.4} m too large for size {size}");
    }

    // `reach::World::new_in` walks from tile (0, 0)'s START into tile (1, 0)'s east clearing!
    let d = atlas.describe().unwrap();
    let tiles = atlas.tile_coords();
    let mut world = World::new_in(&mut atlas, &d.places, &tiles);
    let all = world.reach(&[Size::Person]).unwrap();
    let east_place = d.places.iter().find(|p| p.name == "east clearing").unwrap();
    assert!(east_place.at[0] > WIDTH, "east clearing should be shifted into tile (1, 0)");
    let stand = world.at_place(east_place, Size::Person).unwrap().expect("person stands at east clearing");
    assert!(all.has(stand), "person should reach tile (1, 0)'s clearing across the border from START");
}

#[test]
fn multi_area_scene_world_isolation_and_cross_border_sculpting() {
    let west = Native(Box::new(TileArea::west()));
    let east = Native(Box::new(TileArea::east()));
    let mut atlas = Atlas::new(vec![([0, 0], west), ([1, 0], east)]).unwrap();

    // Load both areas' scenes into one SceneWorld with isolated area owners.
    let mut sw = SceneWorld::default();
    let loaded = atlas.load_scenes_into(&mut sw).unwrap();
    assert_eq!(loaded.len(), 2);
    assert_eq!(loaded[0].0, BASE_OWNER);
    assert_eq!(loaded[1].0, AREA_OWNER_BASE + 1);
    assert!(is_area_owner(tile_owner(0)));
    assert!(is_area_owner(tile_owner(1)));
    assert!(!is_area_owner(1));

    // Tile (1, 0)'s dynamic instance & light are shifted by +64 m along X.
    let east_dyn = &sw.dynamic[&(tile_owner(1), 1)];
    assert!((east_dyn.instance.pos[0] - 80.0).abs() < 1e-4, "expected x=80, got {:?}", east_dyn.instance.pos);
    let east_light = &sw.lights[&(tile_owner(1), 1)];
    assert!((east_light.pos[0] - 80.0).abs() < 1e-4);

    // Base area tiles are never mistaken for player-placed elements by raycast_placed_owner.
    assert_eq!(sw.raycast_placed_owner([80.0, 5.0, 16.0], [0.0, -1.0, 0.0], 20.0), None);

    // Ticking & interacting with tile (1, 0) translates coordinates cleanly and emits its own message.
    let ps = PlayerState { feet: [80.0, 1.0, 16.0], eye: [80.0, 2.5, 16.0], size: 1.7, ..PlayerState::default() };
    let effects = atlas.tick_all(0.016, &ps, tile_owner(1), 1, &mut sw);
    assert_eq!(effects.len(), 2);
    assert!((sw.dynamic[&(tile_owner(1), 1)].instance.pos[0] - 80.5).abs() < 1e-4);
    let said = atlas.interact_tile(&ps, tile_owner(1), 1, &mut sw).and_then(|a| a.said);
    assert_eq!(said.as_deref(), Some("hello from East Meadow"));

    // Sculpting a cave right on the x = 64 border works through `Composed<Atlas<_>, _>`.
    let mut log = EditLog::new();
    let seam_y = shared_edge_height(12.0);
    log.push(Edit {
        op: Op::Carve,
        shape: Shape::Sphere { radius: 2.0 },
        center: [WIDTH, seam_y, 12.0],
        blend: 0.2,
        material: None,
        stamp_size: 1.7,
        tool: plateau_engine::Tool::All,
    })
    .unwrap();
    let mut composed = Composed::new(atlas, Arc::new(ComposedLog::with_edits(Arc::new(log))));
    let mut out = Vec::new();
    composed.sample(&[[WIDTH, seam_y - 0.5, 12.0]], 1.7, &mut out).unwrap();
    assert!(out[0].distance > 0.0, "carved sphere straddling x=64 should open air on the seam: {:?}", out[0]);
}

#[test]
fn real_made_pair_pond_and_forest_walk_seamlessly_at_every_size() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let pond_path = root.join("made/pond-sketched/area.wasm");
    let forest_path = root.join("made/forest-beside-pond/area.wasm");
    if !pond_path.exists() || !forest_path.exists() {
        return;
    }
    let pond_bytes = std::fs::read(&pond_path).unwrap();
    let forest_bytes = std::fs::read(&forest_path).unwrap();

    // Verify borders_match on the real compiled .wasm pair.
    let mut pond_w = plateau_engine::Wasm::new(&pond_bytes, SEED, plateau_engine::Limits::default()).unwrap();
    let mut forest_w = plateau_engine::Wasm::new(&forest_bytes, SEED, plateau_engine::Limits::default()).unwrap();
    assert_eq!(laws::borders_match(&mut forest_w, Side::West, &mut pond_w).unwrap(), Ok(()));

    // Stitch them into an Atlas<Wasm> and load both procedural scenes into SceneWorld.
    let mut atlas = Atlas::new(vec![([0, 0], pond_w), ([1, 0], forest_w)]).unwrap();
    let mut sw = SceneWorld::default();
    let scenes = atlas.load_scenes_into(&mut sw).unwrap();
    assert_eq!(scenes.len(), 2);
    assert!(!sw.dynamic.is_empty(), "stitched real areas should populate dynamic instances");

    // Walk east across x = 64.0 from pond-sketched into forest-beside-pond at all three sizes!
    for (label, size, start_x, target_x, z, max_steps) in [
        ("tiny", SIZES[0].1, 63.85, 64.15, 28.0, 600),
        ("person", SIZES[1].1, 61.0, 67.0, 28.0, 400),
        ("giant", SIZES[2].1, 56.0, 72.0, 28.0, 200),
    ] {
        let mut body = Body::standing(&mut atlas, [start_x, 30.0, z], size).unwrap();
        assert!(body.on.is_some(), "{label}: should start grounded in pond-sketched");
        body.yaw = -std::f32::consts::FRAC_PI_2; // face east (+x)
        let mut prev_y = body.feet[1];
        let mut max_dy = 0.0f64;
        for _ in 0..max_steps {
            body.step(&mut atlas, Intent { forward: 1.0, run: true, ..Intent::default() }, 1.0 / 60.0).unwrap();
            let dy = (body.feet[1] - prev_y).abs();
            max_dy = max_dy.max(dy);
            prev_y = body.feet[1];
            if body.feet[0] >= target_x {
                break;
            }
        }
        assert!(body.feet[0] >= target_x, "{label}: failed to cross from pond into forest (stopped at x = {:.3})", body.feet[0]);
        assert!(body.on.is_some(), "{label}: not grounded after entering forest-beside-pond");
        assert!(max_dy < (size as f64) * 0.25, "{label}: seam height jump {max_dy:.4} m too large");
    }
}
