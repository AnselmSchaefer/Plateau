//! Plateau, played (POC_PLAN.md, steps 2, 7, 8, and 9): one or more areas stitched side by side,
//! walked at three sizes, the player seen from behind (`figure`) or through their own eyes, with
//! live multi-scale SDF sculpting (`B`), composable standalone `.wasm` element placement (`P`),
//! and physical chopping (`K`) & carrying (`G`).
//!
//! `cargo run --release -p plateau-play [-- area.wasm] [--north|--east|--south|--west area.wasm] [--tile gx,gz:area.wasm ...] [--seed N] [--save file.plateau] [--element elem.wasm ...] [--tour folder [--behind]]`;
//! the walled garden (`areas/modules/garden.wasm`) unless another module is given.

mod carry;
mod chunks;
mod figure;
mod flags;
mod place;
mod scene;
mod sculpt;
mod surface;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::asset::embedded_asset;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::pbr::{DistanceFog, FogFalloff, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use carry::{CarriedItem, CarryAssets, CarryGhost, CarryState, FallingElement, HeldVisual};
use chunks::{Chunks, Invalidate};
use place::{ElementGhost, PlaceMode, PreviewAssets, WorldAction};
use plateau_engine::atlas::{Atlas, CompiledAtlas, Side, TileSpec};
use plateau_engine::body::{self, Body, Intent};
use plateau_engine::compose::{Composed, ComposedLog, ElementRuntime, PlacedElement, PlacementTransform};
use plateau_engine::cut::{CarryDecision, check_carry, check_cut_with_tool, choose_cut_plane, clip_scene, format_mass, toppled_transform, world_edit_to_local};
use plateau_engine::edit::{Edit, EditLog, Op, SavedTile, WorldSave, surface_normal};
use plateau_engine::physics::{BuoyancyState, compute_buoyancy, player_mass_kg, step_floating_move};
use plateau_engine::rules::{Action, LOCAL_PLAYER, RuleBook};
use plateau_engine::scene::{Applied, BASE_OWNER, Combined, SceneWorld, is_area_owner};
use plateau_engine::water::WaterLog;
use plateau_engine::{Compiled, ElementBounds, Limits, Material, PlayerState, Region, SIZES, START, Scene, WIDTH, Wasm, rings};
use scene::SceneVisuals;
use sculpt::{BrushGhost, Sculpt};
use surface::{Materials, SurfaceMaterial};

/// The physics' steps a second.
const HZ: f32 = 120.0;
/// A person's height: the world is drawn scaled so the player is always this tall.
const DRAWN_AS: f32 = 1.7;
/// How far ahead, in seconds, the chunks where the player is going are made.
const AHEAD: f32 = 0.75;
/// Seen from behind: camera pivot & arm length in person-scaled renderer units.
const PIVOT: f32 = 0.35;
const BEHIND: f32 = 3.4;
const MARGIN: f32 = 0.25;
const SKY: Color = Color::srgb(0.62, 0.72, 0.84);

fn main() {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut args = std::env::args().skip(1);
    let (mut module, mut seed, mut tour, mut behind, mut save_arg) = (here.join("areas/modules/garden.wasm"), 7u64, None, false, None);
    let mut player_id = LOCAL_PLAYER.to_string();
    let mut extra_elements: Vec<PathBuf> = Vec::new();
    let mut extra_tiles: Vec<([i32; 2], PathBuf)> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).expect("--seed takes a number"),
            "--player" => player_id = args.next().expect("--player takes a player id"),
            "--tour" => tour = Some(PathBuf::from(args.next().expect("--tour takes a folder"))),
            "--behind" => behind = true,
            "--save" => save_arg = Some(PathBuf::from(args.next().expect("--save takes a file path"))),
            "--element" => extra_elements.push(PathBuf::from(args.next().expect("--element takes a .wasm path"))),
            "--north" => extra_tiles.push((Side::North.step(), PathBuf::from(args.next().expect("--north takes a .wasm path")))),
            "--east" => extra_tiles.push((Side::East.step(), PathBuf::from(args.next().expect("--east takes a .wasm path")))),
            "--south" => extra_tiles.push((Side::South.step(), PathBuf::from(args.next().expect("--south takes a .wasm path")))),
            "--west" => extra_tiles.push((Side::West.step(), PathBuf::from(args.next().expect("--west takes a .wasm path")))),
            "--tile" => {
                let spec = args.next().expect("--tile takes GX,GZ:PATH");
                let (coord, path) = spec.split_once(':').expect("--tile expects GX,GZ:PATH");
                let (gx_s, gz_s) = coord.split_once(',').expect("--tile expects GX,GZ:PATH");
                let gx: i32 = gx_s.parse().expect("bad tile gx");
                let gz: i32 = gz_s.parse().expect("bad tile gz");
                extra_tiles.push(([gx, gz], PathBuf::from(path)));
            }
            _ => module = PathBuf::from(arg),
        }
    }
    let (initial_edits, initial_edit_authors, initial_water, initial_rules, initial_elements) = match &save_arg {
        Some(path) if path.exists() => {
            let loaded = WorldSave::load(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            if loaded.seed != seed {
                eprintln!("note: save {} uses seed {} (overriding --seed {seed})", path.display(), loaded.seed);
                seed = loaded.seed;
            }
            let log = Arc::new(EditLog::from_edits(loaded.edits).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
            let water = Arc::new(WaterLog::from_bodies(loaded.water).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
            (log, loaded.edit_authors, water, loaded.rules, loaded.elements)
        }
        _ => (Arc::new(EditLog::new()), Vec::new(), Arc::new(WaterLog::new()), RuleBook::new(), Vec::new()),
    };
    let compile_tile = |path: &PathBuf, at: [i32; 2], seed: u64| -> TileSpec {
        let bytes = std::fs::read(path).unwrap_or_else(|e| panic!("{}: {e} (areas/build.sh builds the areas)", path.display()));
        let compiled = Arc::new(Compiled::new(&bytes, Limits::default()).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
        TileSpec { at, seed, compiled }
    };
    let mut tile_specs = vec![compile_tile(&module, [0, 0], seed)];
    let mut saved_tiles = vec![SavedTile { at: [0, 0], module: module.to_string_lossy().into_owned(), seed }];
    for (at, path) in &extra_tiles {
        tile_specs.push(compile_tile(path, *at, seed));
        saved_tiles.push(SavedTile { at: *at, module: path.to_string_lossy().into_owned(), seed });
    }
    let compiled_atlas = Arc::new(CompiledAtlas::new(tile_specs).expect("valid atlas tiles"));
    let stem = module
        .parent()
        .and_then(|p| p.file_name())
        .filter(|n| *n != "modules")
        .or_else(|| module.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("area");
    let save_path = save_arg.unwrap_or_else(|| here.join("target/saves").join(format!("{stem}-seed{seed}.plateau")));

    let mut place_mode = PlaceMode::new(here.clone(), extra_elements, initial_elements);
    let initial_composed = place_mode.build_composed(initial_edits.clone(), initial_water.clone());
    let mut atlas_live = compiled_atlas.instantiate().expect("the area atlas made");
    let tile_names: Vec<([i32; 2], String)> = atlas_live
        .describe_tiles()
        .expect("the areas described")
        .into_iter()
        .map(|(at, d)| (at, d.name))
        .collect();
    let mut world = SceneWorld::default();
    let initial_area_scenes = atlas_live.load_scenes_into(&mut world).expect("the areas' scenes");
    let mut physics = Composed::new(atlas_live, initial_composed.clone());
    let described = physics.describe().expect("the area described");

    // Spawn any placed elements loaded from the save file into SceneWorld & ElementRuntime.
    let mut element_runtime = ElementRuntime::new();
    let mut initial_element_scenes: Vec<(u32, Scene, bool)> = Vec::new();
    let loaded_list = place_mode.placed.clone();
    for pl in &loaded_list {
        match place_mode.catalog.get_or_load(&pl.module) {
            Ok(el) => match element_runtime.spawn(pl.clone(), &el, &mut world) {
                Ok(world_sc) => initial_element_scenes.push((pl.id, world_sc, pl.is_promoted())),
                Err(e) => eprintln!("could not instantiate saved element {} ({}): {e}", pl.id, pl.module),
            },
            Err(e) => eprintln!("could not load saved element {} ({}): {e}", pl.id, pl.module),
        }
    }

    let flags: Vec<_> = {
        let mut combined = Combined { region: &mut physics, world: &world };
        described
            .places
            .iter()
            .filter(|p| flags::flagged(p.size))
            .map(|p| (p.size, flags::foot(&mut combined, p.at.map(|c| c as f64))))
            .collect()
    };
    let body = {
        let mut combined = Combined { region: &mut physics, world: &world };
        Body::standing(&mut combined, [START[0], 40.0, START[1]], SIZES[1].1).expect("the area asked")
    };

    let present_mode = if tour.is_some() || std::env::var_os("PLATEAU_NO_VSYNC").is_some() { PresentMode::AutoNoVsync } else { PresentMode::AutoVsync };
    let window = Window { title: format!("Plateau: {}", described.name), present_mode, resolution: (1600, 900).into(), ..default() };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }));
    embedded_asset!(app, "play/src", "surface.wesl");
    app.add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(MaterialPlugin::<SurfaceMaterial>::default())
        .insert_resource(ClearColor(SKY))
        .insert_resource(GlobalAmbientLight { brightness: 500.0, ..default() })
        .insert_resource(Area {
            atlas: compiled_atlas,
            tile_names,
            places: described.places.iter().map(|p| (p.name.clone(), p.size, p.at.map(|c| c as f64))).collect(),
            flags,
            initial_area_scenes,
            initial_element_scenes,
            initial_composed: Some(initial_composed),
        })
        .insert_resource(Sculpt::new(
            initial_edits,
            initial_edit_authors,
            initial_water,
            initial_rules,
            player_id,
            saved_tiles,
            save_path,
            described.name.clone(),
            seed,
        ))
        .insert_resource(place_mode)
        .insert_resource(CarryState::default())
        .insert_resource(LiveScene { world, elapsed: 0.0, interact_pressed: false, holding_use: false })
        .insert_resource(Player {
            body,
            pitch: -0.15,
            eye: None,
            focus: None,
            said: None,
            intent: Intent::default(),
            owed: 0.0,
            third: tour.is_none() || behind,
            back: 0.0,
            lift: PIVOT,
            flags: tour.is_none() || behind,
        })
        .insert_resource(figure::Gait::default())
        .insert_non_send(Physics(physics))
        .insert_non_send(Elements(element_runtime))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (look_and_ask, step, frame, draw, sculpt::place_ghost, place::place_ghost, carry::sync_visuals, figure::animate, flags::place, hud).chain(),
        );
    if let Some(folder) = tour {
        std::fs::create_dir_all(&folder).expect("the tour's folder");
        app.insert_resource(Tour::new(folder)).add_systems(Update, go_on_tour.before(step));
    } else {
        app.add_systems(Update, (grab_the_mouse.after(look_and_ask), screenshot));
    }
    app.run();
}

/// The loaded area atlas: compiled tiles for chunk worker threads, tile names, and world-space places.
#[derive(Resource)]
struct Area {
    atlas: Arc<CompiledAtlas>,
    tile_names: Vec<([i32; 2], String)>,
    places: Vec<(String, plateau_engine::Size, [f64; 3])>,
    /// Where each flagged place's flag stands (`flags::flagged`), in the places' order.
    flags: Vec<(plateau_engine::Size, [f64; 3])>,
    /// Initial world-space procedural scenes for each base area tile `(owner, scene)`, taken once in `setup`.
    initial_area_scenes: Vec<(u32, Scene)>,
    /// Initial placed element scenes loaded from `--save`, taken once in `setup` (`(id, scene, skip_solid)`).
    initial_element_scenes: Vec<(u32, Scene, bool)>,
    /// Initial composed SDF snapshot, taken once in `setup` to seed `Chunks`.
    initial_composed: Option<Arc<ComposedLog>>,
}

/// Live procedural scene state, dynamic solid collision, lights, and interaction edge state.
#[derive(Resource)]
struct LiveScene {
    world: SceneWorld,
    elapsed: f32,
    interact_pressed: bool,
    holding_use: bool,
}

/// The instance the player's body is moved through (base multi-area atlas + live SDF edits + placed SDF elements).
struct Physics(Composed<Atlas<Wasm>, Arc<ComposedLog>>);

/// Live per-placement sandboxed Wasmtime instances for runtime `tick` / `interact`.
struct Elements(ElementRuntime);

#[derive(Resource)]
struct Player {
    body: Body,
    pitch: f32,
    eye: Option<[f64; 3]>,
    focus: Option<[f64; 3]>,
    said: Option<(String, Instant)>,
    intent: Intent,
    owed: f32,
    third: bool,
    back: f32,
    lift: f32,
    flags: bool,
}

impl Player {
    fn say(&mut self, what: impl Into<String>) {
        self.said = Some((what.into(), Instant::now() + Duration::from_secs(3)));
    }

    fn looking(&self) -> Vec3 {
        let rotation = look(self.body.yaw, self.pitch);
        rotation * Vec3::NEG_Z
    }
}

#[derive(Component)]
struct Hud;

#[derive(Component)]
struct Sun;

fn look(yaw: f32, pitch: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
}

fn setup(
    mut commands: Commands,
    mut area: ResMut<Area>,
    live: Res<LiveScene>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut surface_materials: ResMut<Assets<SurfaceMaterial>>,
) {
    let surface = Materials::new(&mut surface_materials);
    let area_scenes = std::mem::take(&mut area.initial_area_scenes);
    let mut visuals = scene::spawn_areas(&mut commands, &mut meshes, surface.opaque.clone(), &area_scenes, &live.world);
    for (owner, elem_scene, skip_solid) in area.initial_element_scenes.drain(..) {
        scene::spawn_layer_filtered(&mut commands, &mut meshes, &mut visuals, owner, &elem_scene, &live.world, skip_solid);
    }
    commands.insert_resource(visuals);
    let composed = area.initial_composed.take().unwrap_or_else(|| Arc::new(ComposedLog::empty()));
    let surface_opaque = surface.opaque.clone();
    commands.insert_resource(Chunks::new(area.atlas.clone(), composed, surface));
    figure::spawn(&mut commands, &mut meshes, &mut materials);
    flags::spawn(&mut commands, &mut meshes, &mut materials, &area.flags);
    sculpt::spawn_ghost(&mut commands, &mut meshes, &mut materials);
    place::spawn_ghost(&mut commands, &mut meshes, &mut materials);
    carry::spawn_visuals(&mut commands, &mut meshes, &mut materials, surface_opaque);
    let sun = Vec3::new(0.48, 0.70, 0.53).normalize();
    commands.spawn((
        DirectionalLight { illuminance: 9000.0, shadow_maps_enabled: true, ..default() },
        Transform::default().looking_to(-sun, Vec3::Y),
        CascadeShadowConfigBuilder { num_cascades: 4, minimum_distance: 0.05, first_cascade_far_bound: 6.0, maximum_distance: 220.0, ..default() }.build(),
        Sun,
    ));
    commands.spawn((
        Camera3d::default(),
        Projection::Perspective(PerspectiveProjection { fov: 75f32.to_radians(), near: 0.02, ..default() }),
        DistanceFog { color: SKY, falloff: FogFalloff::from_visibility(400.0), ..default() },
        Transform::default(),
    ));
    let text = |size: f32| TextFont { font_size: bevy::text::FontSize::Px(size), ..default() };
    commands.spawn((Text::new(""), text(18.0), TextColor(Color::WHITE), Node { position_type: PositionType::Absolute, top: px(10.0), left: px(12.0), ..default() }, Hud));
    commands.spawn((
        Text::new(
            "W A S D walk  /  Shift run  /  Space jump  /  F interact  /  K chop  /  G grab & carry (H flat, R/T turn, Wheel tilt)  /  B sculpt  /  P place\n\
             1 2 3 tiny, person, giant  /  Q E shrink, grow  /  Z/Y undo/redo  /  Ctrl+S save  /  Ctrl+L load  /  C sit  /  V view  /  M flags  /  F12 shot",
        ),
        text(14.0),
        TextColor(Color::srgba(1.0, 1.0, 1.0, 0.8)),
        Node { position_type: PositionType::Absolute, bottom: px(10.0), left: px(12.0), ..default() },
    ));
    // A mark at the middle of the view.
    commands.spawn((Text::new("+"), text(22.0), TextColor(Color::srgba(1.0, 1.0, 1.0, 0.6)), Node { position_type: PositionType::Absolute, top: percent(50.0), left: percent(50.0), margin: UiRect { left: px(-6.0), top: px(-14.0), ..default() }, ..default() }));
}

fn grab_the_mouse(mut cursor: Query<&mut CursorOptions, With<PrimaryWindow>>, buttons: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>) {
    let Ok(mut cursor) = cursor.single_mut() else { return };
    if buttons.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

/// The three sizes, smallest first.
fn sizes() -> [f32; 3] {
    SIZES.map(|(_, h)| h)
}

/// Bundled resources for world editing (`B` sculpt, `P` place element, `K` chop, `G` carry) to keep
/// Bevy's system parameter count compact.
#[derive(SystemParam)]
struct EditCtx<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    visuals: ResMut<'w, SceneVisuals>,
    preview: ResMut<'w, PreviewAssets>,
    sculpt: ResMut<'w, Sculpt>,
    place: ResMut<'w, PlaceMode>,
    carry: ResMut<'w, CarryState>,
    carry_assets: Res<'w, CarryAssets>,
    chunks: ResMut<'w, Chunks>,
    live: ResMut<'w, LiveScene>,
    physics: NonSendMut<'w, Physics>,
    elements: NonSendMut<'w, Elements>,
}

impl EditCtx<'_, '_> {
    fn excluded_sdf_ids(&self) -> Vec<u32> {
        let mut ids: Vec<u32> = self.carry.falling.iter().map(|f| f.id).collect();
        if let Some(h) = &self.carry.held {
            ids.push(h.id);
        }
        ids
    }

    fn commit_composed(&mut self, inv: Invalidate<'_>) {
        let exclude = self.excluded_sdf_ids();
        let composed = self.place.build_composed_excluding(self.sculpt.edits.clone(), self.sculpt.water.clone(), &exclude);
        self.physics.0.log = composed.clone();
        self.physics.0.prune_instances();
        self.chunks.set_composed(composed, inv);
    }

    /// Syncs macro-LOD solid `Scene` proxies for promoted elements when the player grows/shrinks
    /// across `PROMOTED_SDF_MAX_SIZE_RATIO` (e.g., switching to Giant keeps carved tree trunks solid!).
    fn sync_macro_proxies(&mut self, viewer_size: f32) {
        let changed = self.elements.0.sync_macro_proxies(viewer_size, &mut self.live.world);
        for (id, world_scene, skip_solid) in changed {
            scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, id, &world_scene, &self.live.world, skip_solid);
        }
    }

    fn spawn_placed(&mut self, pl: PlacedElement, player: &mut Player) -> plateau_engine::Result<String> {
        let compiled = self.place.catalog.get_or_load(&pl.module)?;
        let name = compiled.name.clone();
        let world_scene = self.elements.0.spawn_for_size(pl.clone(), &compiled, &mut self.live.world, player.body.size)?;
        let skip_solid = self.elements.0.should_skip_solid(pl.id);
        scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, pl.id, &world_scene, &self.live.world, skip_solid);
        self.place.placed.push(pl.clone());
        self.sculpt.dirty = true;
        let sdf_bounds = if let Some(bounds) = compiled.sdf_bounds {
            Some(bounds)
        } else if pl.is_promoted() {
            let _ = self.place.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips)?;
            self.place.sdf_bounds_of(&pl)
        } else {
            None
        };
        if let Some(bounds) = sdf_bounds {
            self.commit_composed(Invalidate::SdfElement(pl.transform(), bounds));
            let (_, hi) = pl.transform().world_influence_bounds(bounds);
            let max_lift = (hi[1] as f64 - player.body.feet[1]).max(0.0) + player.body.size as f64 * 2.0;
            let mut combined = Combined { region: &mut self.physics.0, world: &self.live.world };
            let _ = player.body.surface_from_solid(&mut combined, max_lift);
        } else {
            // Check Archimedes buoyancy for newly placed un-rooted procedural objects (boats float, stones sink!).
            let _ = self.settle_buoyancy_for(pl.id, 0.0);
            let max_lift = (pl.scale * 3.0) as f64 + player.body.size as f64;
            let mut combined = Combined { region: &mut self.physics.0, world: &self.live.world };
            let _ = player.body.surface_from_solid(&mut combined, max_lift);
        }
        Ok(name)
    }

    /// Evaluates Archimedes buoyancy for a placed un-rooted `Scene` element `id`, snapping it to
    /// its waterline (if floating) or pond bed (if sinking) and updating `carry.floating`.
    fn settle_buoyancy_for(&mut self, id: u32, extra_load_kg: f32) -> Option<BuoyancyState> {
        let pos = self.place.placed.iter().position(|e| e.id == id)?;
        let pl = &self.place.placed[pos];
        if pl.is_promoted() || self.place.catalog.get(&pl.module).is_some_and(|c| c.is_sdf()) {
            self.carry.floating.remove(&id);
            return None;
        }
        let phys = self.elements.0.physics_of(id)?;
        if phys.rooted {
            self.carry.floating.remove(&id);
            return None;
        }
        let old_tf = pl.transform();
        let buoy = compute_buoyancy(&phys, old_tf, &mut self.physics.0, extra_load_kg).ok()?;
        match buoy {
            BuoyancyState::Floating { target_y, .. } => {
                self.carry.floating.insert(id);
                let mut tf = old_tf;
                tf.pos[1] = target_y;
                if (tf.pos[1] - old_tf.pos[1]).abs() > 1e-4 {
                    self.relocate_placed(id, tf, None);
                }
            }
            BuoyancyState::Sinking { target_y, .. } => {
                self.carry.floating.remove(&id);
                if self.carry.piloting == Some(id) {
                    self.carry.piloting = None;
                }
                let mut tf = old_tf;
                tf.pos[1] = target_y;
                if (tf.pos[1] - old_tf.pos[1]).abs() > 1e-4 {
                    self.relocate_placed(id, tf, None);
                }
            }
            BuoyancyState::Dry { .. } => {
                self.carry.floating.remove(&id);
                if self.carry.piloting == Some(id) {
                    self.carry.piloting = None;
                }
            }
        }
        Some(buoy)
    }

    /// Re-evaluates buoyancy for all placed un-rooted `Scene` elements (e.g. after expanding a pond
    /// channel underneath a beached boat, or after loading a saved world).
    fn refresh_all_buoyancy(&mut self) {
        let ids: Vec<u32> = self.place.placed.iter().map(|e| e.id).collect();
        for id in ids {
            let _ = self.settle_buoyancy_for(id, 0.0);
        }
    }

    /// Updates passive bobbing, rider weight draft, and active WASD boat/raft paddling each frame.
    fn tick_buoyancy(&mut self, dt: f32, forward_in: f32, turn_in: f32, player: &mut Player) {
        self.carry.float_time += dt;
        let standing_id = self.live.world.standing_on_placed(player.body.feet, player.body.size);
        let rider_kg = player_mass_kg(player.body.size);

        // 1. Active piloting (`F` while standing on a floating boat/log).
        if let Some(boat_id) = self.carry.piloting {
            let still_exists = self.place.placed.iter().position(|e| e.id == boat_id);
            let phys = self.elements.0.physics_of(boat_id);
            if let (Some(pos), Some(phys)) = (still_exists, phys) {
                let old_tf = self.place.placed[pos].transform();
                let boat_scale = old_tf.scale.max(0.01);
                let speed = forward_in * (2.8 * boat_scale);
                let turn_rate = turn_in * 1.65;
                if let Ok((new_tf, buoy)) = step_floating_move(
                    &phys,
                    old_tf,
                    speed,
                    turn_rate,
                    dt,
                    &mut self.physics.0,
                    rider_kg,
                    self.carry.float_time,
                ) {
                    if let BuoyancyState::Sinking { .. } = buoy {
                        self.carry.piloting = None;
                        self.carry.floating.remove(&boat_id);
                        self.relocate_placed(boat_id, new_tf, None);
                        let name = self
                            .place
                            .catalog
                            .get(&self.place.placed[pos].module)
                            .map_or_else(|| format!("#{boat_id}"), |c| c.name.clone());
                        player.say(format!("too heavy — {name} is sinking!"));
                        return;
                    }
                    let dyaw = turn_rate * dt;
                    let dx = (new_tf.pos[0] - old_tf.pos[0]) as f64;
                    let dy = (new_tf.pos[1] - old_tf.pos[1]) as f64;
                    let dz = (new_tf.pos[2] - old_tf.pos[2]) as f64;
                    // Rotate rider offset around the boat's yaw pivot so turning carries the rider cleanly.
                    let rx = player.body.feet[0] - old_tf.pos[0] as f64;
                    let rz = player.body.feet[2] - old_tf.pos[2] as f64;
                    let (s, c) = (dyaw as f64).sin_cos();
                    let nrx = rx * c + rz * s;
                    let nrz = -rx * s + rz * c;
                    player.body.feet[0] = old_tf.pos[0] as f64 + nrx + dx;
                    player.body.feet[1] += dy;
                    player.body.feet[2] = old_tf.pos[2] as f64 + nrz + dz;
                    player.body.yaw += dyaw;
                    self.relocate_placed(boat_id, new_tf, None);
                }
            } else {
                self.carry.piloting = None;
            }
        }

        // 2. Gentle bobbing & rider-weight draft for un-piloted floating objects.
        let mut active_ids: Vec<u32> = self.carry.floating.iter().copied().collect();
        if let Some(sid) = standing_id
            && !active_ids.contains(&sid)
        {
            active_ids.push(sid);
        }
        for id in active_ids {
            if self.carry.piloting == Some(id) || self.carry.held.as_ref().is_some_and(|h| h.id == id) {
                continue;
            }
            if self.carry.falling.iter().any(|f| f.id == id) {
                continue;
            }
            let Some(pos) = self.place.placed.iter().position(|e| e.id == id) else {
                self.carry.floating.remove(&id);
                continue;
            };
            if self.place.placed[pos].is_promoted() {
                self.carry.floating.remove(&id);
                continue;
            }
            let Some(phys) = self.elements.0.physics_of(id) else { continue };
            if phys.rooted {
                self.carry.floating.remove(&id);
                continue;
            }
            let old_tf = self.place.placed[pos].transform();
            let load = if standing_id == Some(id) { rider_kg } else { 0.0 };
            if let Ok(buoy) = compute_buoyancy(&phys, old_tf, &mut self.physics.0, load) {
                match buoy {
                    BuoyancyState::Floating { target_y, bob_amplitude, .. } => {
                        self.carry.floating.insert(id);
                        let phase = self.carry.float_time * 2.2 + id as f32 * 1.13;
                        let desired_y = target_y + phase.sin() * bob_amplitude;
                        let next_y = old_tf.pos[1] + (desired_y - old_tf.pos[1]) * (dt * 6.0).min(1.0);
                        if (next_y - old_tf.pos[1]).abs() > 1e-4 {
                            let mut tf = old_tf;
                            tf.pos[1] = next_y;
                            let dy = (next_y - old_tf.pos[1]) as f64;
                            self.relocate_placed(id, tf, None);
                            if standing_id == Some(id) {
                                player.body.feet[1] += dy;
                            }
                        }
                    }
                    BuoyancyState::Sinking { target_y, .. } => {
                        let next_y = old_tf.pos[1] + (target_y - old_tf.pos[1]) * (dt * 4.0).min(1.0);
                        if (next_y - old_tf.pos[1]).abs() > 1e-4 {
                            let mut tf = old_tf;
                            tf.pos[1] = next_y;
                            let dy = (next_y - old_tf.pos[1]) as f64;
                            self.relocate_placed(id, tf, None);
                            if standing_id == Some(id) {
                                player.body.feet[1] += dy;
                            }
                        } else if load == 0.0 {
                            self.carry.floating.remove(&id);
                        }
                    }
                    BuoyancyState::Dry { .. } => {
                        self.carry.floating.remove(&id);
                    }
                }
            }
        }
    }

    fn remove_placed(&mut self, id: u32) -> Option<PlacedElement> {
        self.carry.forget_element(id);
        let idx = self.place.placed.iter().position(|e| e.id == id)?;
        let removed = self.place.placed.remove(idx);
        let sdf_bounds = self.place.sdf_bounds_of(&removed);
        self.elements.0.remove(id, &mut self.live.world);
        scene::despawn_layer(&mut self.commands, &mut self.visuals, id);
        self.sculpt.dirty = true;
        if let Some(bounds) = sdf_bounds {
            self.commit_composed(Invalidate::SdfElement(removed.transform(), bounds));
        }
        Some(removed)
    }

    /// Updates a placed element's world transform in `PlaceMode`, `ElementRuntime`, `SceneWorld`,
    /// GPU visuals, and (for promoted elements not mid-fall) `ComposedLog`.
    fn relocate_placed(&mut self, id: u32, tf: PlacementTransform, player: Option<&mut Player>) -> bool {
        let Some(pos) = self.place.placed.iter().position(|e| e.id == id) else {
            return false;
        };
        let old_tf = self.place.placed[pos].transform();
        let old_bounds = self.place.sdf_bounds_of(&self.place.placed[pos]);
        self.place.placed[pos].set_transform(tf);
        let scale = self.place.placed[pos].scale;
        let mid_fall = self.carry.falling.iter().any(|f| f.id == id);
        let skip_solid = if mid_fall { false } else { self.elements.0.should_skip_solid(id) };
        if let Some(world_scene) = self.elements.0.relocate(id, tf, &mut self.live.world) {
            scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, id, &world_scene, &self.live.world, skip_solid);
        }
        if !mid_fall && let Some(bounds) = old_bounds {
            let inv_bounds = [(old_tf, bounds), (tf, bounds)];
            self.commit_composed(Invalidate::Combined { edit: None, bounds: &inv_bounds, water: None });
        }
        self.sculpt.dirty = true;
        if let Some(p) = player {
            let max_lift = (scale * 3.0) as f64 + p.body.size as f64;
            let mut combined = Combined { region: &mut self.physics.0, world: &self.live.world };
            let _ = p.body.surface_from_solid(&mut combined, max_lift);
        }
        true
    }

    /// Cancels an active carry and restores the object cleanly to where it was picked up.
    fn cancel_carry(&mut self) -> Option<String> {
        let item = self.carry.held.take()?;
        let sdf_bounds = self.place.placed.iter().find(|e| e.id == item.id).and_then(|pl| self.place.sdf_bounds_of(pl));
        if let Some(pos) = self.place.placed.iter().position(|e| e.id == item.id) {
            self.place.placed[pos].set_transform(item.original_tf);
        }
        let _ = self.elements.0.relocate(item.id, item.original_tf, &mut self.live.world);
        if let Some(world_scene) = self.elements.0.set_held(item.id, false, &mut self.live.world) {
            let skip_solid = self.elements.0.should_skip_solid(item.id);
            scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, item.id, &world_scene, &self.live.world, skip_solid);
        }
        if let Some(bounds) = sdf_bounds {
            self.commit_composed(Invalidate::SdfElement(item.original_tf, bounds));
        }
        Some(item.name)
    }

    /// Places the currently carried object at `aim_world`, restoring solid collision / SDF chunks
    /// and optionally pushing an undoable `RelocateElement` action.
    fn drop_carried_at(&mut self, aim_world: [f64; 3], player: &mut Player, record_undo: bool) -> Option<String> {
        let item = self.carry.held.take()?;
        let new_tf = item.target_transform(aim_world);
        let sdf_bounds = self.place.placed.iter().find(|e| e.id == item.id).and_then(|pl| self.place.sdf_bounds_of(pl));
        if let Some(pos) = self.place.placed.iter().position(|e| e.id == item.id) {
            self.place.placed[pos].set_transform(new_tf);
        }
        let _ = self.elements.0.relocate(item.id, new_tf, &mut self.live.world);
        if let Some(world_scene) = self.elements.0.set_held(item.id, false, &mut self.live.world) {
            let skip_solid = self.elements.0.should_skip_solid(item.id);
            scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, item.id, &world_scene, &self.live.world, skip_solid);
        }
        let _ = self.settle_buoyancy_for(item.id, 0.0);
        if let Some(bounds) = sdf_bounds {
            self.commit_composed(Invalidate::SdfElement(new_tf, bounds));
        }
        self.sculpt.dirty = true;
        if record_undo {
            self.place.undo_stack.push(WorldAction::RelocateElement { id: item.id, before: item.original_tf, after: new_tf });
            self.place.redo_stack.clear();
        }
        let max_lift = (item.scale * 3.0) as f64 + player.body.size as f64;
        let mut combined = Combined { region: &mut self.physics.0, world: &self.live.world };
        let _ = player.body.surface_from_solid(&mut combined, max_lift);
        Some(item.name)
    }

    /// Snaps any in-flight topple animations immediately to their resting transforms (called before
    /// save, undo/redo, or reload so world state is always settled).
    fn finish_all_falls(&mut self, player: Option<&mut Player>) {
        let pending = std::mem::take(&mut self.carry.falling);
        for fall in pending {
            let to_tf = PlacementTransform::with_rot(fall.to_pos, fall.to_rot, fall.scale);
            self.relocate_placed(fall.id, to_tf, None);
        }
        if let Some(p) = player {
            let max_lift = p.body.size as f64 * 3.0;
            let mut combined = Combined { region: &mut self.physics.0, world: &self.live.world };
            let _ = p.body.surface_from_solid(&mut combined, max_lift);
        }
    }

    /// Advances smooth gravity-eased topple animations for newly chopped objects.
    fn tick_falling(&mut self, dt: f32, player: &mut Player) {
        if self.carry.falling.is_empty() {
            return;
        }
        let mut done = Vec::new();
        let mut updates = Vec::new();
        for (idx, fall) in self.carry.falling.iter_mut().enumerate() {
            fall.elapsed += dt;
            let tf = fall.sample();
            let finished = fall.elapsed >= fall.duration;
            updates.push((fall.id, tf, finished));
            if finished {
                done.push(idx);
            }
        }
        for idx in done.into_iter().rev() {
            self.carry.falling.remove(idx);
        }
        for (id, tf, finished) in updates {
            self.relocate_placed(id, tf, if finished { Some(&mut *player) } else { None });
            if finished {
                let _ = self.settle_buoyancy_for(id, 0.0);
            }
        }
    }

    /// Lazily promotes any solid `Scene` placed elements overlapping `edit` (or `aimed_owner`)
    /// into carvable `MeshSdf`s right before `edit` is applied at chronological index `edit_idx`.
    /// Returns `(promoted_ids_with_idx, invalidated_bounds, promoted_names)`.
    fn promote_overlapping_scene_elements(
        &mut self,
        edit: &Edit,
        aimed_owner: Option<u32>,
        edit_idx: usize,
    ) -> (Vec<(u32, usize)>, Vec<(PlacementTransform, ElementBounds)>, Vec<String>) {
        let (elo, ehi) = edit.influence_bounds();
        let mut candidates = self.live.world.solid_placed_overlapping(elo, ehi);
        if let Some(owner) = aimed_owner
            && !candidates.contains(&owner)
        {
            candidates.push(owner);
        }
        let mut promoted = Vec::new();
        let mut bounds_list = Vec::new();
        let mut names = Vec::new();
        for id in candidates {
            // Never accidentally voxelize a floating boat/raft when sculpting nearby terrain!
            if self.carry.floating.contains(&id) && aimed_owner != Some(id) {
                continue;
            }
            let Some(pos) = self.place.placed.iter().position(|e| e.id == id) else { continue };
            if self.place.placed[pos].is_promoted() {
                continue;
            }
            let module = self.place.placed[pos].module.clone();
            let seed = self.place.placed[pos].seed;
            let clips = self.place.placed[pos].clips.clone();
            let Ok(compiled) = self.place.catalog.get_or_load(&module) else { continue };
            if compiled.is_sdf() {
                continue;
            }
            let Ok(Some(mesh_sdf)) = self.place.catalog.get_or_build_mesh_sdf(&module, seed, &clips) else { continue };
            self.place.placed[pos].promoted = Some(edit_idx);
            let tf = self.place.placed[pos].transform();
            if let Some(world_scene) = self.elements.0.set_promoted(id, Some(edit_idx), &mut self.live.world) {
                let skip = self.elements.0.should_skip_solid(id);
                scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, id, &world_scene, &self.live.world, skip);
            }
            promoted.push((id, edit_idx));
            bounds_list.push((tf, mesh_sdf.bounds()));
            names.push(compiled.name.clone());
        }
        (promoted, bounds_list, names)
    }

    /// Sets promotion state on a list of placed elements (for undo/redo), returning their world
    /// bounds for targeted chunk invalidation.
    fn set_elements_promoted(&mut self, ids: &[(u32, Option<usize>)]) -> Vec<(PlacementTransform, ElementBounds)> {
        let mut bounds_list = Vec::new();
        for &(id, next_state) in ids {
            let Some(pos) = self.place.placed.iter().position(|e| e.id == id) else { continue };
            let module = self.place.placed[pos].module.clone();
            let seed = self.place.placed[pos].seed;
            let clips = self.place.placed[pos].clips.clone();
            let tf = self.place.placed[pos].transform();
            if let Ok(Some(mesh_sdf)) = self.place.catalog.get_or_build_mesh_sdf(&module, seed, &clips) {
                bounds_list.push((tf, mesh_sdf.bounds()));
            }
            self.place.placed[pos].promoted = next_state;
            if let Some(world_scene) = self.elements.0.set_promoted(id, next_state, &mut self.live.world) {
                let skip = self.elements.0.should_skip_solid(id);
                scene::spawn_layer_filtered(
                    &mut self.commands,
                    &mut self.meshes,
                    &mut self.visuals,
                    id,
                    &world_scene,
                    &self.live.world,
                    skip,
                );
            }
        }
        bounds_list
    }

    fn reload_world(&mut self) -> plateau_engine::Result<(usize, usize)> {
        self.carry.held = None;
        self.carry.falling.clear();
        self.carry.floating.clear();
        self.carry.piloting = None;
        let saved = self.sculpt.load_world()?;
        let edit_count = saved.edits.len();
        let water = WaterLog::from_bodies(saved.water)?;
        self.sculpt.replace_world_state(saved.edits, saved.edit_authors, water, saved.rules)?;
        self.elements.0.clear(&mut self.live.world);
        scene::despawn_placed_layers(&mut self.commands, &mut self.visuals);
        self.place.placed.clear();
        self.place.undo_stack.clear();
        self.place.redo_stack.clear();
        let mut elem_count = 0usize;
        for pl in saved.elements {
            match self.place.catalog.get_or_load(&pl.module) {
                Ok(compiled) => match self.elements.0.spawn(pl.clone(), &compiled, &mut self.live.world) {
                    Ok(world_scene) => {
                        scene::spawn_layer_filtered(
                            &mut self.commands,
                            &mut self.meshes,
                            &mut self.visuals,
                            pl.id,
                            &world_scene,
                            &self.live.world,
                            pl.is_promoted(),
                        );
                        self.place.next_id = self.place.next_id.max(pl.id + 1);
                        self.place.placed.push(pl);
                        elem_count += 1;
                    }
                    Err(e) => eprintln!("reload element {} error: {e}", pl.id),
                },
                Err(e) => eprintln!("reload element {} load error: {e}", pl.module),
            }
        }
        self.commit_composed(Invalidate::All);
        self.refresh_all_buoyancy();
        Ok((edit_count, elem_count))
    }
}

/// The mouse turns the view; the keys say where to go, and change size, sit, sculpt, place, or interact.
#[allow(clippy::too_many_arguments)]
fn look_and_ask(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    tour: Option<Res<Tour>>,
    mut player: ResMut<Player>,
    mut ctx: EditCtx,
) {
    if tour.is_some() {
        return;
    }
    ctx.tick_falling(time.delta_secs().min(0.1), &mut player);
    ctx.sync_macro_proxies(player.body.size);

    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if grabbed && player.body.change.is_none() {
        player.body.yaw -= motion.delta.x * 0.0025;
        player.pitch = (player.pitch - motion.delta.y * 0.0025).clamp(-1.55, 1.55);
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let held = |k: KeyCode| keys.pressed(k) as i32 as f32;
    let dt = time.delta_secs().min(0.1);

    // Cycle active material-aware tool (`C`: Workshop -> Saw -> Pickaxe -> Shovel -> Hands).
    if !ctrl && keys.just_pressed(KeyCode::KeyC) {
        ctx.sculpt.tool = ctx.sculpt.tool.next();
        player.say(format!("tool: {}", ctx.sculpt.tool.name()));
    }

    // Handle active boat/raft piloting & passive floating buoyancy.
    let boat_fwd = if ctx.carry.is_piloting() && !ctrl { held(KeyCode::KeyW) - held(KeyCode::KeyS) } else { 0.0 };
    let boat_turn = if ctx.carry.is_piloting() { held(KeyCode::KeyA) - held(KeyCode::KeyD) } else { 0.0 };
    if ctx.carry.is_piloting() && keys.just_pressed(KeyCode::Space) {
        ctx.carry.piloting = None;
        player.say("dismounted");
    }
    ctx.tick_buoyancy(dt, boat_fwd, boat_turn, &mut player);

    let carry_speed = ctx.carry.speed_factor();
    player.intent = if ctx.carry.is_piloting() {
        Intent::default()
    } else {
        Intent {
            forward: if ctrl { 0.0 } else { (held(KeyCode::KeyW) - held(KeyCode::KeyS)) * carry_speed },
            right: (held(KeyCode::KeyD) - held(KeyCode::KeyA)) * carry_speed,
            run: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
            jump: keys.pressed(KeyCode::Space),
        }
    };

    // If shrinking made the carried object exceed current size's carry capacity, set it down safely.
    if let Some(item) = &ctx.carry.held
        && !check_carry(&item.physics, false, player.body.size).is_allowed()
    {
        let fwd = Vec3::new(-player.body.yaw.sin(), 0.0, -player.body.yaw.cos());
        let dist = (player.body.size * 1.5).max(item.physics.world_radius(item.scale) + player.body.size * 0.5) as f64;
        let drop_pos = [
            player.body.feet[0] + fwd.x as f64 * dist,
            player.body.feet[1],
            player.body.feet[2] + fwd.z as f64 * dist,
        ];
        if let Some(name) = ctx.drop_carried_at(drop_pos, &mut player, true) {
            player.say(format!("{name} too heavy at this size — set down"));
        }
    }

    if keys.just_pressed(KeyCode::KeyB) {
        if ctx.carry.is_carrying() {
            let _ = ctx.cancel_carry();
        }
        ctx.sculpt.active = !ctx.sculpt.active;
        ctx.sculpt.hold_timer = 0.0;
        ctx.sculpt.last_stamp = None;
        if ctx.sculpt.active {
            ctx.place.active = false;
            ctx.place.aim = None;
            player.say(format!("sculpt mode: {} / {}", ctx.sculpt.preset.name(), ctx.sculpt.material().name()));
        } else {
            ctx.sculpt.aim = None;
            player.say("sculpt mode off");
        }
    }

    if keys.just_pressed(KeyCode::KeyP) {
        if ctx.carry.is_carrying() {
            let _ = ctx.cancel_carry();
        }
        ctx.place.active = !ctx.place.active;
        if ctx.place.active {
            ctx.sculpt.active = false;
            ctx.sculpt.aim = None;
            if let Some(el) = ctx.place.current() {
                let kind = if el.is_sdf() { "SDF" } else { "Scene" };
                player.say(format!("place mode: {} [{kind}]", el.name));
            } else {
                player.say("place mode: no .wasm elements found (run ./elements/build.sh)");
            }
        } else {
            ctx.place.aim = None;
            player.say("place mode off");
        }
    }

    if ctrl && keys.just_pressed(KeyCode::KeyR) {
        let n = ctx.place.rescan();
        place::clear_preview_cache(&mut ctx.preview);
        player.say(format!("rescanned elements: {n} available"));
    }

    let tool_active = ctx.sculpt.active || ctx.place.active || ctx.carry.is_carrying();
    if keys.just_pressed(KeyCode::KeyF) || (!tool_active && buttons.just_pressed(MouseButton::Right)) {
        if ctx.carry.is_piloting() {
            ctx.carry.piloting = None;
            player.say("released helm");
        } else if let Some(boat_id) = ctx.live.world.standing_on_placed(player.body.feet, player.body.size)
            && ctx.carry.floating.contains(&boat_id)
        {
            ctx.sculpt.active = false;
            ctx.sculpt.aim = None;
            ctx.place.active = false;
            ctx.place.aim = None;
            ctx.carry.piloting = Some(boat_id);
            let name = ctx
                .place
                .placed
                .iter()
                .find(|e| e.id == boat_id)
                .and_then(|pl| ctx.place.catalog.get(&pl.module))
                .map_or_else(|| format!("#{boat_id}"), |c| c.name.clone());
            player.say(format!("paddling {name} — W/S row, A/D steer, F/Space dismount"));
        } else {
            let act = Action::Interact { at: player.body.feet.map(|c| c as f32) };
            match ctx.sculpt.rules.check(&ctx.sculpt.player_id, &act) {
                Ok(()) => ctx.live.interact_pressed = true,
                Err(denial) => player.say(denial.reason),
            }
        }
    }
    ctx.live.holding_use = keys.pressed(KeyCode::KeyF) || (!tool_active && buttons.pressed(MouseButton::Right));

    // Helper closure for camera-pivot raycast collinear with the `+` crosshair.
    let eye = player.eye.unwrap_or(player.body.eye());
    let dir = player.looking().to_array();
    let psize = player.body.size;
    let pivot = if player.third {
        let per = (psize / DRAWN_AS) as f64;
        [eye[0], eye[1] + player.lift as f64 * per, eye[2]]
    } else {
        eye
    };

    // Sculpting controls (Step 8a: scale-stamped SDF edits).
    if ctx.sculpt.active {
        if keys.just_pressed(KeyCode::Tab) {
            ctx.sculpt.preset = ctx.sculpt.preset.next();
            player.say(format!("brush: {}", ctx.sculpt.preset.name()));
        }
        if !ctrl && keys.just_pressed(KeyCode::KeyR) {
            ctx.sculpt.next_material();
            player.say(format!("material: {}", ctx.sculpt.material().name()));
        }
        if scroll.delta.y > 0.0 || keys.just_pressed(KeyCode::BracketRight) {
            ctx.sculpt.adjust_radius(1.18);
        } else if scroll.delta.y < 0.0 || keys.just_pressed(KeyCode::BracketLeft) {
            ctx.sculpt.adjust_radius(1.0 / 1.18);
        }
        let reach = psize * 40.0;
        let (hit, normal) = {
            let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
            let hit = body::look(&mut combined, pivot, dir, psize, reach).ok().flatten();
            let normal = hit.map(|h| surface_normal(&mut combined, h, psize).unwrap_or([0.0, 1.0, 0.0]));
            (hit, normal)
        };
        ctx.sculpt.aim = hit.zip(normal);
        let add_just = buttons.just_pressed(MouseButton::Left);
        let carve_just = buttons.just_pressed(MouseButton::Right) || keys.just_pressed(KeyCode::KeyX);
        let add_held = buttons.pressed(MouseButton::Left);
        let carve_held = buttons.pressed(MouseButton::Right) || keys.pressed(KeyCode::KeyX);
        if !add_held && !carve_held {
            ctx.sculpt.hold_timer = 0.0;
            ctx.sculpt.last_stamp = None;
        } else {
            ctx.sculpt.hold_timer = (ctx.sculpt.hold_timer - time.delta_secs()).max(0.0);
        }
        if grabbed && let Some((h, n)) = ctx.sculpt.aim {
            let click_op = if add_just {
                Some(Op::Add)
            } else if carve_just {
                Some(Op::Carve)
            } else {
                None
            };
            let hold_op = if click_op.is_none() && ctx.sculpt.hold_timer <= 0.0 {
                if add_held {
                    Some(Op::Add)
                } else if carve_held {
                    Some(Op::Carve)
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(op) = click_op.or(hold_op) {
                let edit = ctx.sculpt.make_edit(op, psize, h, n);
                let spacing_ok = click_op.is_some()
                    || ctx.sculpt.last_stamp.is_none_or(|prev| {
                        let d2 = (0..3).map(|a| (edit.center[a] - prev[a]).powi(2)).sum::<f32>();
                        let min_step = ctx.sculpt.radius(psize) * 0.45;
                        d2 >= min_step * min_step
                    });
                if spacing_ok {
                    // Enforce material-aware tool rules when carving (`Op::Carve`): e.g. a Saw
                    // carves Wood/foliage but refuses Stone; a Shovel digs earth/mud/sand only.
                    let tool_carve_err = if op == Op::Carve {
                        let mut sample_ans = Vec::new();
                        let h32 = [h[0] as f32, h[1] as f32, h[2] as f32];
                        let mat = {
                            let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
                            combined.sample(&[h32], psize, &mut sample_ans).ok().and_then(|_| sample_ans.first().map(|a| a.material))
                        };
                        mat.and_then(|m| ctx.sculpt.tool.can_carve(m).err())
                    } else {
                        None
                    };
                    if let Some(reason) = tool_carve_err {
                        player.say(reason);
                        ctx.sculpt.hold_timer = 0.35;
                    } else {
                    // Check if the player is sculpting directly onto a discrete procedural `Scene`
                    // element (either unpromoted or already promoted). If so, store the edit in
                    // that element's `local_edits` so carved tunnels & notches travel and rotate
                    // with the object if it is later chopped (`K`) or carried (`G`)!
                    let edit_idx = ctx.sculpt.edits.len();
                    let hit_dist = ((h[0] - pivot[0]).powi(2) + (h[1] - pivot[1]).powi(2) + (h[2] - pivot[2]).powi(2)).sqrt() as f32;
                    let tol = ctx.sculpt.radius(psize) * 2.0 + psize * 0.5;
                    let aimed_unpromoted = ctx
                        .live
                        .world
                        .raycast_solid_placed(pivot.map(|c| c as f32), dir, reach)
                        .and_then(|(owner, t)| if (t - hit_dist).abs() <= tol { Some(owner) } else { None });
                    let aimed_promoted_scene = ctx
                        .place
                        .raycast_placed_sdf(pivot.map(|c| c as f32), dir, reach)
                        .and_then(|(owner, _)| {
                            let pl = ctx.place.placed.iter().find(|e| e.id == owner)?;
                            if !pl.is_promoted() || ctx.place.catalog.get(&pl.module).is_some_and(|c| c.is_sdf()) {
                                return None;
                            }
                            let b = ctx.place.sdf_bounds_of(pl)?;
                            let (lo, hi) = pl.transform().world_bounds(b);
                            let h32 = [h[0] as f32, h[1] as f32, h[2] as f32];
                            let pad = ctx.sculpt.radius(psize) * 1.2 + psize * 0.25;
                            let inside = (0..3).all(|a| h32[a] >= lo[a] - pad && h32[a] <= hi[a] + pad);
                            inside.then_some(owner)
                        });
                    let direct_scene_target = aimed_unpromoted.or(aimed_promoted_scene).filter(|&owner| {
                        ctx.place
                            .placed
                            .iter()
                            .find(|e| e.id == owner)
                            .is_some_and(|pl| !ctx.place.catalog.get(&pl.module).is_some_and(|c| c.is_sdf()))
                    });

                    if let Some(target_id) = direct_scene_target
                        && let Some(pos) = ctx.place.placed.iter().position(|e| e.id == target_id)
                    {
                        let check_act = Action::SculptElement {
                            at: edit.center,
                            player_size: psize,
                            author: ctx.place.placed[pos].author.as_deref(),
                        };
                        if let Err(denial) = ctx.sculpt.rules.check(&ctx.sculpt.player_id, &check_act) {
                            player.say(denial.reason);
                            ctx.sculpt.hold_timer = 0.35;
                        } else {
                            let module = ctx.place.placed[pos].module.clone();
                            let seed = ctx.place.placed[pos].seed;
                            let clips = ctx.place.placed[pos].clips.clone();
                            let tf = ctx.place.placed[pos].transform();
                            if let Ok(Some(_)) = ctx.place.catalog.get_or_build_mesh_sdf(&module, seed, &clips) {
                                let local_edit = world_edit_to_local(&edit, tf);
                                let newly_promoted = if ctx.place.placed[pos].is_promoted() {
                                    None
                                } else {
                                    ctx.place.placed[pos].promoted = Some(edit_idx);
                                    Some(edit_idx)
                                };
                                ctx.place.placed[pos].local_edits.push(local_edit);
                                ctx.sculpt.dirty = true;
                                if newly_promoted.is_some()
                                    && let Some(world_scene) = ctx.elements.0.set_promoted(target_id, Some(edit_idx), &mut ctx.live.world)
                                {
                                    let skip = ctx.elements.0.should_skip_solid(target_id);
                                    scene::spawn_layer_filtered(&mut ctx.commands, &mut ctx.meshes, &mut ctx.visuals, target_id, &world_scene, &ctx.live.world, skip);
                                }
                                if newly_promoted.is_some() {
                                    let bounds = ctx.place.sdf_bounds_of(&ctx.place.placed[pos]);
                                    let promo_bounds: Vec<_> = bounds.into_iter().map(|b| (tf, b)).collect();
                                    ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds, water: None });
                                } else {
                                    ctx.commit_composed(Invalidate::Edit(&edit));
                                }
                                ctx.place.undo_stack.push(WorldAction::ElementSdfEdit { id: target_id, edit: local_edit, newly_promoted });
                                ctx.place.redo_stack.clear();
                                if newly_promoted.is_some() {
                                    let name = ctx.place.catalog.get(&module).map_or_else(|| format!("#{target_id}"), |c| c.name.clone());
                                    player.say(format!("{name} sculpted (still choppable & carriable)"));
                                }
                                if edit.op == Op::Add {
                                    let (_, hi) = edit.influence_bounds();
                                    let max_lift = (hi[1] as f64 - player.body.feet[1]).max(0.0) + player.body.size as f64 * 2.0;
                                    let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
                                    let _ = player.body.surface_from_solid(&mut combined, max_lift);
                                }
                                ctx.sculpt.last_stamp = Some(edit.center);
                                ctx.sculpt.hold_timer = if click_op.is_some() { 0.25 } else { 0.14 };
                            }
                        }
                    } else {
                        // Enforce RuleBook before altering terrain or expanding water.
                        let (elo, ehi) = edit.influence_bounds();
                        let sculpt_act = Action::Sculpt {
                            lo: elo,
                            hi: ehi,
                            player_size: psize,
                            is_water: edit.material == Some(Material::Water),
                        };
                        if let Err(denial) = ctx.sculpt.rules.check(&ctx.sculpt.player_id, &sculpt_act) {
                            player.say(denial.reason);
                            ctx.sculpt.hold_timer = 0.35;
                        } else {
                            // If carving touches existing water (and water editing is permitted), expand the pond!
                            let water_bounds = if edit.op == Op::Carve
                                && ctx.sculpt.rules.check(&ctx.sculpt.player_id, &Action::ModifyWater { lo: elo, hi: ehi, player_size: psize }).is_ok()
                            {
                                let mut next_water = (*ctx.sculpt.water).clone();
                                match next_water.expand_from_carve(&mut ctx.physics.0, &ctx.sculpt.edits, &edit, Some(ctx.sculpt.player_id.clone())) {
                                    Ok(Some(wb)) => {
                                        ctx.sculpt.water = Arc::new(next_water);
                                        Some(wb)
                                    }
                                    _ => None,
                                }
                            } else {
                                None
                            };
                            // World terrain sculpt: only lazily promote overlapping solid `Scene`
                            // elements when CARVING (`Op::Carve`) so adding land/islands next to a
                            // boat, bridge, or tree never turns crisp procedural objects into voxels!
                            let (promoted_ids, promo_bounds, promo_names) = if edit.op == Op::Carve {
                                ctx.promote_overlapping_scene_elements(&edit, aimed_unpromoted, edit_idx)
                            } else {
                                (Vec::new(), Vec::new(), Vec::new())
                            };
                            match ctx.sculpt.push_edit(edit) {
                                Ok(()) => {
                                    ctx.place.undo_stack.push(WorldAction::SdfEdit { edit, promoted_ids });
                                    ctx.place.redo_stack.clear();
                                    if promo_bounds.is_empty() && water_bounds.is_none() {
                                        ctx.commit_composed(Invalidate::Edit(&edit));
                                    } else {
                                        ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds, water: water_bounds });
                                        if !promo_names.is_empty() {
                                            player.say(format!("{} converted to carvable solid", promo_names.join(", ")));
                                        }
                                    }
                                    if water_bounds.is_some() || !ctx.carry.floating.is_empty() {
                                        ctx.refresh_all_buoyancy();
                                    }
                                    if edit.op == Op::Add {
                                        let (_, hi) = edit.influence_bounds();
                                        let max_lift = (hi[1] as f64 - player.body.feet[1]).max(0.0) + player.body.size as f64 * 2.0;
                                        let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
                                        let _ = player.body.surface_from_solid(&mut combined, max_lift);
                                    }
                                    ctx.sculpt.last_stamp = Some(edit.center);
                                    ctx.sculpt.hold_timer = if click_op.is_some() { 0.25 } else { 0.14 };
                                }
                                Err(e) => player.say(format!("sculpt error: {e}")),
                            }
                        }
                    }
                    }
                }
            }
        }
    }

    // Place Element controls (Step 8b: standalone composable .wasm elements).
    if ctx.place.active {
        if keys.just_pressed(KeyCode::Tab) {
            ctx.place.next_element();
            if let Some(el) = ctx.place.current() {
                let kind = if el.is_sdf() { "SDF" } else { "Scene" };
                player.say(format!("element: {} [{kind}]", el.name));
            }
        }
        if !ctrl && keys.just_pressed(KeyCode::KeyR) {
            ctx.place.yaw -= std::f32::consts::FRAC_PI_6 * 0.5; // -15 deg
        }
        if keys.just_pressed(KeyCode::KeyT) {
            ctx.place.yaw += std::f32::consts::FRAC_PI_6 * 0.5; // +15 deg
        }
        if scroll.delta.y > 0.0 || keys.just_pressed(KeyCode::BracketRight) {
            ctx.place.adjust_scale(1.15);
        } else if scroll.delta.y < 0.0 || keys.just_pressed(KeyCode::BracketLeft) {
            ctx.place.adjust_scale(1.0 / 1.15);
        }
        let reach = psize * 45.0;
        let hit = {
            let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
            body::look(&mut combined, pivot, dir, psize, reach).ok().flatten()
        };
        ctx.place.aim = hit.map(|h| {
            let n = surface_normal(&mut ctx.physics.0, h, psize).unwrap_or([0.0, 1.0, 0.0]);
            (h, n)
        });

        if grabbed && buttons.just_pressed(MouseButton::Left)
            && let (Some(elem), Some((h, _))) = (ctx.place.current().cloned(), ctx.place.aim)
        {
            let at = [h[0] as f32, h[1] as f32, h[2] as f32];
            if let Err(denial) = ctx.sculpt.rules.check(&ctx.sculpt.player_id, &Action::PlaceElement { at, player_size: psize }) {
                player.say(denial.reason);
            } else {
                let id = ctx.place.alloc_id();
                let scale = ctx.place.placement_scale(psize);
                let pl = PlacedElement {
                    id,
                    module: elem.module.clone(),
                    seed: ctx.sculpt.seed.wrapping_mul(31).wrapping_add(id as u64),
                    pos: at,
                    yaw: ctx.place.yaw,
                    rot: None,
                    scale,
                    stamp_size: psize,
                    after_edits: ctx.sculpt.edits.len(),
                    promoted: None,
                    clips: Vec::new(),
                    local_edits: Vec::new(),
                    author: Some(ctx.sculpt.player_id.clone()),
                };
                match ctx.spawn_placed(pl.clone(), &mut player) {
                    Ok(name) => {
                        let floating = ctx.carry.floating.contains(&id);
                        ctx.place.undo_stack.push(WorldAction::PlaceElement(pl));
                        ctx.place.redo_stack.clear();
                        if floating {
                            player.say(format!("placed {name} ({scale:.2}x, floating — step aboard & press F to paddle)"));
                        } else {
                            player.say(format!("placed {name} ({scale:.2}x)"));
                        }
                    }
                    Err(e) => player.say(format!("place error: {e}")),
                }
            }
        }

        if grabbed && (buttons.just_pressed(MouseButton::Right) || keys.just_pressed(KeyCode::KeyX)) {
            let piv32 = pivot.map(|c| c as f32);
            let scene_hit = ctx.live.world.raycast_placed_owner(piv32, dir, reach);
            let sdf_hit = ctx.place.raycast_placed_sdf(piv32, dir, reach);
            let target_id = match (scene_hit, sdf_hit) {
                (Some((a, ta)), Some((b, tb))) => Some(if ta <= tb { a } else { b }),
                (Some((a, _)), None) => Some(a),
                (None, Some((b, _))) => Some(b),
                (None, None) => None,
            };
            if let Some(id) = target_id {
                let denial = ctx.place.placed.iter().find(|e| e.id == id).and_then(|pl| {
                    ctx.sculpt
                        .rules
                        .check(&ctx.sculpt.player_id, &Action::RemoveElement { at: pl.pos, author: pl.author.as_deref() })
                        .err()
                });
                if let Some(denial) = denial {
                    player.say(denial.reason);
                } else if let Some(removed) = ctx.remove_placed(id) {
                    ctx.place.undo_stack.push(WorldAction::RemoveElement(removed));
                    ctx.place.redo_stack.clear();
                    player.say(format!("removed element #{id}"));
                }
            } else {
                player.say("aim at a placed element to remove it");
            }
        }
    }

    // Chop / Cut discrete procedural Scene objects (Step 8c: K key — works on both unpromoted and
    // tunnel-carved promoted Scene elements!).
    if keys.just_pressed(KeyCode::KeyK) && !ctx.carry.is_carrying() {
        ctx.sculpt.active = false;
        ctx.sculpt.aim = None;
        ctx.place.active = false;
        ctx.place.aim = None;
        let reach = psize * 10.0;
        let piv32 = pivot.map(|c| c as f32);
        let scene_hit = ctx.live.world.raycast_placed_surface_material(piv32, dir, reach);
        let sdf_hit = ctx.place.raycast_placed_sdf(piv32, dir, reach).map(|(id, t)| {
            let exact = {
                let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
                body::look(&mut combined, pivot, dir, psize, reach).ok().flatten()
            };
            let hit_p = exact.map_or([piv32[0] + dir[0] * t, piv32[1] + dir[1] * t, piv32[2] + dir[2] * t], |h| [h[0] as f32, h[1] as f32, h[2] as f32]);
            (id, t, hit_p, None)
        });
        let target_hit = match (scene_hit, sdf_hit) {
            (Some((a, ta, pa, ma)), Some((b, tb, pb, mb))) => Some(if ta <= tb { (a, pa, Some(ma)) } else { (b, pb, mb) }),
            (Some((a, _, pa, ma)), None) => Some((a, pa, Some(ma))),
            (None, Some((b, _, pb, mb))) => Some((b, pb, mb)),
            (None, None) => None,
        };
        if let Some((id, hit_world, hit_mat)) = target_hit {
            ctx.finish_all_falls(Some(&mut player));
            if let Some(original) = ctx.place.placed.iter().find(|e| e.id == id).cloned() {
                if let Err(denial) = ctx.sculpt.rules.check(
                    &ctx.sculpt.player_id,
                    &Action::CutOrCarry { at: original.pos, author: original.author.as_deref() },
                ) {
                    player.say(denial.reason);
                } else {
                let is_sdf = ctx.place.catalog.get(&original.module).is_some_and(|e| e.is_sdf());
                let elem_name = ctx.place.catalog.get(&original.module).map_or_else(|| format!("#{id}"), |e| e.name.clone());
                if let Some(phys) = ctx.elements.0.physics_of(id) {
                    match check_cut_with_tool(&phys, is_sdf, psize, ctx.sculpt.tool, hit_mat) {
                        Err(reason) => player.say(reason),
                        Ok(()) => {
                            let orig_tf = original.transform();
                            let hit_local = orig_tf.world_to_local_point(hit_world);
                            if let Some(plane) = choose_cut_plane(&phys, hit_local) {
                                let raw_local = ctx.elements.0.local_scene_of(id).cloned();
                                if let Some(cur_scene) = raw_local {
                                    let lower_sc = clip_scene(&cur_scene, &[plane.flipped()]);
                                    let upper_sc = clip_scene(&cur_scene, &[plane]);
                                    let lower_ok = !lower_sc.instances.is_empty();
                                    let upper_ok = !upper_sc.instances.is_empty();
                                    if lower_ok && upper_ok {
                                        let mut lower = original.clone();
                                        lower.clips.push(plane.flipped());

                                        let upper_id = ctx.place.alloc_id();
                                        let mut upper = original.clone();
                                        upper.id = upper_id;
                                        upper.clips.push(plane);

                                        let upper_phys = plateau_engine::cut::ObjectPhysics::compute(&upper_sc, upper.scale);
                                        let was_upright = !CarriedItem::is_rot_horizontal(orig_tf.rot);

                                        // Compute target resting transform for the detached upper half.
                                        // Probe strictly against terrain SDF (`ctx.physics.0`, NOT
                                        // `Combined` while `original` is still standing!) so the
                                        // detached half never lands on the uncut original's own roof,
                                        // then settle via Archimedes buoyancy (wood floats, stone sinks!).
                                        let mut target_upper_tf = if was_upright {
                                            let flen = (dir[0] * dir[0] + dir[2] * dir[2]).sqrt();
                                            let (fx, fz) = if flen > 1e-3 {
                                                (dir[0] / flen, dir[2] / flen)
                                            } else {
                                                (-player.body.yaw.sin(), -player.body.yaw.cos())
                                            };
                                            let probe_x = orig_tf.pos[0] as f64 + (fx * upper_phys.world_span * 0.45) as f64;
                                            let probe_z = orig_tf.pos[2] as f64 + (fz * upper_phys.world_span * 0.45) as f64;
                                            let start_y = orig_tf.pos[1] as f64 + upper_phys.world_span as f64 + 1.0;
                                            let ground_y = body::look(&mut ctx.physics.0, [probe_x, start_y, probe_z], [0.0, -1.0, 0.0], psize, upper_phys.world_span * 2.0 + 10.0)
                                                .ok()
                                                .flatten()
                                                .map_or(orig_tf.pos[1], |h| h[1] as f32);
                                            toppled_transform(orig_tf, &upper_phys, [fx, fz], ground_y)
                                        } else {
                                            // Cross-cutting an already lying log/stone: nudge apart cleanly along cut normal.
                                            let wn = orig_tf.local_to_world_dir(plane.normal);
                                            let gap = (0.14 * upper.scale).max(0.01);
                                            let shifted = [
                                                orig_tf.pos[0] + wn[0] * gap,
                                                orig_tf.pos[1] + wn[1] * gap,
                                                orig_tf.pos[2] + wn[2] * gap,
                                            ];
                                            PlacementTransform::with_rot(shifted, orig_tf.rot, orig_tf.scale)
                                        };
                                        if let Ok(buoy) = compute_buoyancy(&upper_phys, target_upper_tf, &mut ctx.physics.0, 0.0) {
                                            match buoy {
                                                BuoyancyState::Floating { target_y, .. } | BuoyancyState::Sinking { target_y, .. } => {
                                                    target_upper_tf.pos[1] = target_y;
                                                }
                                                BuoyancyState::Dry { floor_y } => {
                                                    let (min_y, _) = upper_phys.rotated_y_extents(target_upper_tf.rot, target_upper_tf.scale);
                                                    target_upper_tf.pos[1] = floor_y - min_y - 0.02 * target_upper_tf.scale;
                                                }
                                            }
                                        }

                                        let mut upper_settled = upper.clone();
                                        upper_settled.set_transform(target_upper_tf);

                                        if was_upright {
                                            ctx.carry.falling.push(FallingElement {
                                                id: upper_id,
                                                from_pos: orig_tf.pos,
                                                from_rot: orig_tf.rot,
                                                to_pos: target_upper_tf.pos,
                                                to_rot: target_upper_tf.rot,
                                                scale: orig_tf.scale,
                                                elapsed: 0.0,
                                                duration: 0.48,
                                            });
                                        }
                                        ctx.remove_placed(original.id);
                                        let spawn_lower = ctx.spawn_placed(lower.clone(), &mut player);
                                        let spawn_upper = ctx.spawn_placed(if was_upright { upper.clone() } else { upper_settled.clone() }, &mut player);
                                        if spawn_lower.is_ok() && spawn_upper.is_ok() {
                                            if was_upright && upper.is_promoted() {
                                                // Show solid procedural mesh during the 0.48s fall so
                                                // promoted carved trunks animate smoothly, then restore
                                                // exact SDF + tunnel on settle!
                                                if let Some(world_scene) = ctx.elements.0.relocate(upper_id, orig_tf, &mut ctx.live.world) {
                                                    scene::spawn_layer_filtered(&mut ctx.commands, &mut ctx.meshes, &mut ctx.visuals, upper_id, &world_scene, &ctx.live.world, false);
                                                }
                                            }
                                            ctx.place.undo_stack.push(WorldAction::CutElement {
                                                original,
                                                lower,
                                                upper: upper_settled,
                                            });
                                            ctx.place.redo_stack.clear();
                                            player.say(format!("chopped {elem_name} — press G to carry"));
                                        } else {
                                            ctx.carry.cancel_fall(upper_id);
                                        }
                                    } else {
                                        player.say("cut plane missed solid geometry — aim nearer the middle");
                                    }
                                }
                            } else {
                                player.say("too small to cut cleanly");
                            }
                        }
                    }
                }
                }
            }
        } else {
            player.say("aim at a placed object within reach to chop (K)");
        }
    }

    // Grab / Carry / Rotate / Tilt / Place discrete procedural Scene objects (Step 8c: G key).
    if ctx.carry.is_carrying() {
        let reach = (psize * 12.0).max(ctx.carry.held.as_ref().map_or(0.0, |h| h.physics.world_span * 2.5));
        let aim = {
            let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
            body::look(&mut combined, pivot, dir, psize, reach).ok().flatten()
        };
        if let Some(item) = &mut ctx.carry.held {
            item.aim = aim.or_else(|| {
                let d = (psize * 3.5).max(item.physics.world_radius(item.scale) + psize) as f64;
                Some([pivot[0] + dir[0] as f64 * d, player.body.feet[1], pivot[2] + dir[2] as f64 * d])
            });
            if keys.just_pressed(KeyCode::KeyH) {
                item.flat = !item.flat;
                item.pitch = 0.0;
                player.say(if item.flat { "orientation: horizontal (bridge)" } else { "orientation: upright" });
            }
            if !ctrl && keys.just_pressed(KeyCode::KeyR) {
                item.yaw -= std::f32::consts::FRAC_PI_6 * 0.5; // -15 deg
            }
            if keys.just_pressed(KeyCode::KeyT) {
                item.yaw += std::f32::consts::FRAC_PI_6 * 0.5; // +15 deg
            }
            if scroll.delta.y > 0.0 || keys.just_pressed(KeyCode::BracketRight) {
                item.pitch = (item.pitch + 10.0f32.to_radians()).clamp(-50.0f32.to_radians(), 50.0f32.to_radians());
            } else if scroll.delta.y < 0.0 || keys.just_pressed(KeyCode::BracketLeft) {
                item.pitch = (item.pitch - 10.0f32.to_radians()).clamp(-50.0f32.to_radians(), 50.0f32.to_radians());
            }
        }
        if keys.just_pressed(KeyCode::KeyG) || (grabbed && buttons.just_pressed(MouseButton::Left)) {
            if let Some(target_aim) = ctx.carry.held.as_ref().and_then(|h| h.aim) {
                let at = [target_aim[0] as f32, target_aim[1] as f32, target_aim[2] as f32];
                if let Err(denial) = ctx.sculpt.rules.check(&ctx.sculpt.player_id, &Action::PlaceElement { at, player_size: psize }) {
                    player.say(denial.reason);
                } else if let Some(name) = ctx.drop_carried_at(target_aim, &mut player, true) {
                    player.say(format!("placed {name} down"));
                }
            }
        } else if (grabbed && buttons.just_pressed(MouseButton::Right)) || keys.just_pressed(KeyCode::KeyX) {
            if let Some(name) = ctx.cancel_carry() {
                player.say(format!("returned {name}"));
            }
        }
    } else if keys.just_pressed(KeyCode::KeyG) {
        ctx.sculpt.active = false;
        ctx.sculpt.aim = None;
        ctx.place.active = false;
        ctx.place.aim = None;
        let reach = psize * 10.0;
        let piv32 = pivot.map(|c| c as f32);
        let scene_hit = ctx.live.world.raycast_placed_surface(piv32, dir, reach).map(|(o, t, _)| (o, t))
            .or_else(|| ctx.live.world.raycast_placed_owner(piv32, dir, reach));
        let sdf_hit = ctx.place.raycast_placed_sdf(piv32, dir, reach);
        let target_id = match (scene_hit, sdf_hit) {
            (Some((a, ta)), Some((b, tb))) => Some(if ta <= tb { a } else { b }),
            (Some((a, _)), None) => Some(a),
            (None, Some((b, _))) => Some(b),
            (None, None) => None,
        };
        if let Some(id) = target_id {
            ctx.finish_all_falls(Some(&mut player));
            if let Some(pl) = ctx.place.placed.iter().find(|e| e.id == id).cloned() {
                if let Err(denial) = ctx.sculpt.rules.check(
                    &ctx.sculpt.player_id,
                    &Action::CutOrCarry { at: pl.pos, author: pl.author.as_deref() },
                ) {
                    player.say(denial.reason);
                } else {
                let is_sdf = ctx.place.catalog.get(&pl.module).is_some_and(|e| e.is_sdf());
                let name = ctx.place.catalog.get(&pl.module).map_or_else(|| format!("#{id}"), |e| e.name.clone());
                let phys = ctx.elements.0.physics_of(id);
                if let Some(phys) = phys {
                    match check_carry(&phys, is_sdf, psize) {
                        CarryDecision::Blocked(reason) => player.say(reason),
                        CarryDecision::Allowed { speed_factor } => {
                            let orig_tf = pl.transform();
                            let sdf_bounds = ctx.place.sdf_bounds_of(&pl);
                            let flat = CarriedItem::is_rot_horizontal(orig_tf.rot);
                            let mesh_handle = ctx
                                .elements
                                .0
                                .local_scene_of(id)
                                .and_then(|sc| scene::build_merged_mesh(sc, false, false))
                                .map_or_else(|| ctx.carry_assets.fallback.clone(), |(m, _)| ctx.meshes.add(m));
                            ctx.elements.0.set_held(id, true, &mut ctx.live.world);
                            scene::despawn_layer(&mut ctx.commands, &mut ctx.visuals, id);
                            ctx.carry.held = Some(CarriedItem {
                                id,
                                name: name.clone(),
                                scale: pl.scale,
                                physics: phys,
                                original_tf: orig_tf,
                                speed_factor,
                                yaw: player.body.yaw,
                                pitch: 0.0,
                                flat,
                                aim: None,
                                mesh: mesh_handle,
                            });
                            if let Some(bounds) = sdf_bounds {
                                ctx.commit_composed(Invalidate::SdfElement(orig_tf, bounds));
                            }
                            player.say(format!(
                                "carrying {name} ({}) — H flat, R/T turn, Wheel tilt, LMB/G place",
                                format_mass(phys.world_mass_kg)
                            ));
                        }
                    }
                } else {
                    player.say("rooted in terrain — sculpt with B");
                }
                }
            }
        } else {
            player.say("aim at a placed object within reach to grab (G)");
        }
    }

    // Unified Undo / Redo across sculpt edits, lazy promotions, placements, cuts, and relocations.
    if (tool_active && keys.just_pressed(KeyCode::KeyZ)) || (ctrl && keys.just_pressed(KeyCode::KeyZ)) {
        if ctx.carry.is_carrying() {
            let _ = ctx.cancel_carry();
        }
        ctx.finish_all_falls(Some(&mut player));
        if let Some(action) = ctx.place.undo_stack.pop() {
            match action.clone() {
                WorldAction::SdfEdit { edit, promoted_ids } => {
                    if ctx.sculpt.pop_edit().is_some() {
                        let water_diff = if edit.op == Op::Carve && !ctx.sculpt.water.is_empty() {
                            if let Ok(rebuilt) = WaterLog::recompute_from_edits(&mut ctx.physics.0.region, &ctx.sculpt.edits) {
                                let diff = ctx.sculpt.water.diff_bounds(&rebuilt);
                                if rebuilt.bodies() != ctx.sculpt.water.bodies() {
                                    ctx.sculpt.water = Arc::new(rebuilt);
                                }
                                diff
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        let demote: Vec<_> = promoted_ids.iter().map(|&(id, _)| (id, None)).collect();
                        let promo_bounds = ctx.set_elements_promoted(&demote);
                        if promo_bounds.is_empty() && water_diff.is_none() {
                            ctx.commit_composed(Invalidate::Edit(&edit));
                        } else {
                            ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds, water: water_diff });
                        }
                        ctx.place.redo_stack.push(action);
                        player.say(format!("undo sculpt ({} edits)", ctx.sculpt.edits.len()));
                    }
                }
                WorldAction::ElementSdfEdit { id, newly_promoted, .. } => {
                    if let Some(pos) = ctx.place.placed.iter().position(|e| e.id == id) {
                        let old_bounds = ctx.place.sdf_bounds_of(&ctx.place.placed[pos]);
                        let tf = ctx.place.placed[pos].transform();
                        ctx.place.placed[pos].local_edits.pop();
                        if newly_promoted.is_some() {
                            ctx.place.placed[pos].promoted = None;
                            if let Some(world_scene) = ctx.elements.0.set_promoted(id, None, &mut ctx.live.world) {
                                scene::spawn_layer_filtered(&mut ctx.commands, &mut ctx.meshes, &mut ctx.visuals, id, &world_scene, &ctx.live.world, false);
                            }
                        }
                        if let Some(b) = old_bounds {
                            ctx.commit_composed(Invalidate::SdfElement(tf, b));
                        } else {
                            ctx.commit_composed(Invalidate::All);
                        }
                        ctx.place.redo_stack.push(action);
                        player.say("undo object sculpt");
                    }
                }
                WorldAction::PlaceElement(pl) => {
                    if ctx.remove_placed(pl.id).is_some() {
                        ctx.place.redo_stack.push(action);
                        player.say(format!("undo place ({} elements)", ctx.place.placed.len()));
                    }
                }
                WorldAction::RemoveElement(pl) => {
                    if ctx.spawn_placed(pl, &mut player).is_ok() {
                        ctx.place.redo_stack.push(action);
                        player.say(format!("restored element ({} elements)", ctx.place.placed.len()));
                    }
                }
                WorldAction::CutElement { original, lower, upper } => {
                    ctx.remove_placed(upper.id);
                    ctx.remove_placed(lower.id);
                    if ctx.spawn_placed(original, &mut player).is_ok() {
                        ctx.place.redo_stack.push(action);
                        player.say("undo chop");
                    }
                }
                WorldAction::RelocateElement { id, before, .. } => {
                    if ctx.relocate_placed(id, before, Some(&mut player)) {
                        ctx.place.redo_stack.push(action);
                        player.say("undo move");
                    }
                }
            }
        } else {
            player.say("nothing to undo");
        }
    }

    if (tool_active && keys.just_pressed(KeyCode::KeyY)) || (ctrl && keys.just_pressed(KeyCode::KeyY)) {
        if ctx.carry.is_carrying() {
            let _ = ctx.cancel_carry();
        }
        ctx.finish_all_falls(Some(&mut player));
        if let Some(action) = ctx.place.redo_stack.pop() {
            match action.clone() {
                WorldAction::SdfEdit { edit, promoted_ids } => {
                    let repromote: Vec<_> = promoted_ids.iter().map(|&(id, idx)| (id, Some(idx))).collect();
                    let promo_bounds = ctx.set_elements_promoted(&repromote);
                    if ctx.sculpt.push_edit(edit).is_ok() {
                        let water_diff = if edit.op == Op::Carve {
                            if let Ok(rebuilt) = WaterLog::recompute_from_edits(&mut ctx.physics.0.region, &ctx.sculpt.edits) {
                                let diff = ctx.sculpt.water.diff_bounds(&rebuilt);
                                if rebuilt.bodies() != ctx.sculpt.water.bodies() {
                                    ctx.sculpt.water = Arc::new(rebuilt);
                                }
                                diff
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if promo_bounds.is_empty() && water_diff.is_none() {
                            ctx.commit_composed(Invalidate::Edit(&edit));
                        } else {
                            ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds, water: water_diff });
                        }
                        if edit.op == Op::Add {
                            let (_, hi) = edit.influence_bounds();
                            let max_lift = (hi[1] as f64 - player.body.feet[1]).max(0.0) + player.body.size as f64 * 2.0;
                            let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
                            let _ = player.body.surface_from_solid(&mut combined, max_lift);
                        }
                        ctx.place.undo_stack.push(action);
                        player.say(format!("redo sculpt ({} edits)", ctx.sculpt.edits.len()));
                    }
                }
                WorldAction::ElementSdfEdit { id, edit, newly_promoted } => {
                    if let Some(pos) = ctx.place.placed.iter().position(|e| e.id == id) {
                        let module = ctx.place.placed[pos].module.clone();
                        let seed = ctx.place.placed[pos].seed;
                        let clips = ctx.place.placed[pos].clips.clone();
                        let _ = ctx.place.catalog.get_or_build_mesh_sdf(&module, seed, &clips);
                        ctx.place.placed[pos].local_edits.push(edit);
                        if let Some(idx) = newly_promoted {
                            ctx.place.placed[pos].promoted = Some(idx);
                            if let Some(world_scene) = ctx.elements.0.set_promoted(id, Some(idx), &mut ctx.live.world) {
                                let skip = ctx.elements.0.should_skip_solid(id);
                                scene::spawn_layer_filtered(&mut ctx.commands, &mut ctx.meshes, &mut ctx.visuals, id, &world_scene, &ctx.live.world, skip);
                            }
                        }
                        let tf = ctx.place.placed[pos].transform();
                        if let Some(b) = ctx.place.sdf_bounds_of(&ctx.place.placed[pos]) {
                            ctx.commit_composed(Invalidate::SdfElement(tf, b));
                        } else {
                            ctx.commit_composed(Invalidate::All);
                        }
                        ctx.place.undo_stack.push(action);
                        player.say("redo object sculpt");
                    }
                }
                WorldAction::PlaceElement(pl) => {
                    if ctx.spawn_placed(pl, &mut player).is_ok() {
                        ctx.place.undo_stack.push(action);
                        player.say(format!("redo place ({} elements)", ctx.place.placed.len()));
                    }
                }
                WorldAction::RemoveElement(pl) => {
                    if ctx.remove_placed(pl.id).is_some() {
                        ctx.place.undo_stack.push(action);
                        player.say(format!("redo remove ({} elements)", ctx.place.placed.len()));
                    }
                }
                WorldAction::CutElement { original, lower, upper } => {
                    ctx.remove_placed(original.id);
                    if ctx.spawn_placed(lower, &mut player).is_ok() && ctx.spawn_placed(upper, &mut player).is_ok() {
                        ctx.place.undo_stack.push(action);
                        player.say("redo chop");
                    }
                }
                WorldAction::RelocateElement { id, after, .. } => {
                    if ctx.relocate_placed(id, after, Some(&mut player)) {
                        ctx.place.undo_stack.push(action);
                        player.say("redo move");
                    }
                }
            }
        } else {
            player.say("nothing to redo");
        }
    }

    // Save / Reload complete world (base area + seed + SDF edits + placed elements).
    if keys.just_pressed(KeyCode::F5) || (ctrl && keys.just_pressed(KeyCode::KeyS)) {
        if ctx.carry.is_carrying() {
            let _ = ctx.cancel_carry();
        }
        ctx.finish_all_falls(Some(&mut player));
        match ctx.sculpt.save_world(&ctx.place.placed) {
            Ok(()) => {
                let msg = format!(
                    "saved {} edits & {} elements to {}",
                    ctx.sculpt.edits.len(),
                    ctx.place.placed.len(),
                    ctx.sculpt.save_path.display()
                );
                println!("{msg}");
                player.say(msg);
            }
            Err(e) => {
                let msg = format!("save failed: {e}");
                eprintln!("{msg}");
                player.say(msg);
            }
        }
    }
    if keys.just_pressed(KeyCode::F9) || (ctrl && keys.just_pressed(KeyCode::KeyL)) {
        match ctx.reload_world() {
            Ok((edits_n, elems_n)) => {
                let msg = format!("reloaded {edits_n} edits & {elems_n} elements from {}", ctx.sculpt.save_path.display());
                println!("{msg}");
                player.say(msg);
            }
            Err(e) => {
                let msg = format!("reload failed: {e}");
                eprintln!("{msg}");
                player.say(msg);
            }
        }
    }

    let size = player.body.change.map_or(player.body.size, |c| c.to);
    let [tiny, person, giant] = sizes();
    let to = if keys.just_pressed(KeyCode::Digit1) {
        Some(tiny)
    } else if keys.just_pressed(KeyCode::Digit2) {
        Some(person)
    } else if keys.just_pressed(KeyCode::Digit3) {
        Some(giant)
    } else if keys.just_pressed(KeyCode::KeyQ) {
        sizes().into_iter().rev().find(|&s| s < size * 0.99)
    } else if keys.just_pressed(KeyCode::KeyE) {
        sizes().into_iter().find(|&s| s > size * 1.01)
    } else {
        None
    };
    if let Some(to) = to {
        let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
        change_size(&mut player, &mut combined, to);
    }
    if keys.just_pressed(KeyCode::KeyV) {
        player.third = !player.third;
    }
    if keys.just_pressed(KeyCode::KeyM) {
        player.flags = !player.flags;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        if player.body.seat.is_some() {
            let stood = player.body.seat.unwrap().stood;
            player.body.feet = stood;
            player.body.seat = None;
        } else {
            let mut combined = Combined { region: &mut ctx.physics.0, world: &ctx.live.world };
            if !player.body.sit(&mut combined).expect("the area asked") {
                player.say("nothing to sit on here");
            }
        }
    }
}

/// Starts growing or shrinking to `to`, keeping in view the point looked at.
fn change_size(player: &mut Player, area: &mut dyn Region, to: f32) {
    if to == player.body.size {
        return;
    }
    let (eye, size) = (player.body.eye(), player.body.size);
    let way = player.looking();
    player.focus = body::look(area, eye, way.to_array(), size, size * 300.0).expect("the area asked");
    player.body.grow_to(to);
}

fn apply_side_effects(player: &mut Player, applied: &Applied) {
    if applied.impulse != [0.0, 0.0, 0.0] {
        for a in 0..3 {
            player.body.velocity[a] += applied.impulse[a];
        }
        if applied.impulse[1] > 0.0 {
            player.body.on = None;
        }
    }
    if let Some(msg) = &applied.said {
        player.say(msg.clone());
    }
}

/// The runtime plugins (all base area tiles + placed elements) ticked and the body moved on in steps.
fn step(
    time: Res<Time>,
    mut player: ResMut<Player>,
    mut live: ResMut<LiveScene>,
    mut physics: NonSendMut<Physics>,
    mut elements: NonSendMut<Elements>,
) {
    let dt = time.delta_secs().min(0.1);
    live.elapsed += dt;

    let eye64 = player.body.eye();
    let eye32 = eye64.map(|c| c as f32);
    let look_dir = player.looking().to_array();
    let size = player.body.size;
    let (hit_owner, hit_id) = live.world.raycast_dynamic_owned(eye32, look_dir, size * 12.0).map_or((BASE_OWNER, 0), |(o, id, _)| (o, id));
    let aim = {
        let mut combined = Combined { region: &mut physics.0, world: &live.world };
        match body::look(&mut combined, eye64, look_dir, size, size * 30.0) {
            Ok(Some(hit)) => hit.map(|c| c as f32),
            _ => [eye32[0] + look_dir[0] * size * 3.0, eye32[1] + look_dir[1] * size * 3.0, eye32[2] + look_dir[2] * size * 3.0],
        }
    };
    let ps_world = PlayerState {
        feet: player.body.feet.map(|c| c as f32),
        eye: eye32,
        look: look_dir,
        velocity: player.body.velocity,
        size,
        grounded: player.body.on.is_some(),
        holding_use: live.holding_use,
        target: hit_id,
        aim,
        time: live.elapsed,
    };

    if dt > 0.0 {
        for eff in physics.0.region.tick_all(dt, &ps_world, hit_owner, hit_id, &mut live.world) {
            apply_side_effects(&mut player, &eff);
        }
        for eff in elements.0.tick_all(dt, &ps_world, hit_owner, hit_id, &mut live.world) {
            apply_side_effects(&mut player, &eff);
        }
    }
    if std::mem::take(&mut live.interact_pressed) {
        if is_area_owner(hit_owner)
            && let Some(eff) = physics.0.region.interact_tile(&ps_world, hit_owner, hit_id, &mut live.world)
        {
            apply_side_effects(&mut player, &eff);
        }
        for eff in elements.0.interact(&ps_world, hit_owner, hit_id, &mut live.world) {
            apply_side_effects(&mut player, &eff);
        }
    }

    player.owed = (player.owed + time.delta_secs()).min(0.25);
    let intent = player.intent;
    while player.owed >= 1.0 / HZ {
        player.owed -= 1.0 / HZ;
        let mut combined = Combined { region: &mut physics.0, world: &live.world };
        player.body.step(&mut combined, intent, 1.0 / HZ).expect("the area asked");
        if player.body.change.is_some() {
            if let Some(f) = player.focus {
                let e = player.body.eye();
                let d = [f[0] - e[0], f[1] - e[1], f[2] - e[2]];
                let across = (d[0] * d[0] + d[2] * d[2]).sqrt();
                if across > 1e-9 || d[1].abs() > 1e-9 {
                    player.body.yaw = (-d[0]).atan2(-d[2]) as f32;
                    player.pitch = (d[1].atan2(across) as f32).clamp(-1.55, 1.55);
                }
            }
        } else {
            player.focus = None;
        }
    }
    if player.body.blocked {
        player.body.blocked = false;
        player.say("no room to grow here");
    }
    let target = player.body.eye();
    let k = 1.0 - (-25.0 * time.delta_secs() as f64).exp();
    player.eye = Some(match player.eye {
        Some(e) if (e[1] - target[1]).abs() < player.body.size as f64 * 0.5 => [target[0], e[1] + (target[1] - e[1]) * k, target[2]],
        _ => target,
    });
}

fn frame(time: Res<Time>, mut player: ResMut<Player>, live: Res<LiveScene>, mut physics: NonSendMut<Physics>) {
    let want = if player.third { BEHIND } else { 0.0 };
    let size = player.body.size;
    let per = (size / DRAWN_AS) as f64;
    let eye = player.eye.unwrap_or(player.body.eye());
    let mut combined = Combined { region: &mut physics.0, world: &live.world };
    let over = match body::look(&mut combined, eye, [0.0, 1.0, 0.0], size, (PIVOT + 0.1) * size / DRAWN_AS).expect("the area asked") {
        Some(hit) => (((hit[1] - eye[1]) / per) as f32 - 0.1).clamp(0.0, PIVOT),
        None => PIVOT,
    };
    player.lift = if over < player.lift { over } else { player.lift + (over - player.lift) * (1.0 - (-4.0 * time.delta_secs()).exp()) };
    let pivot = [eye[0], eye[1] + player.lift as f64 * per, eye[2]];
    let away = -player.looking();
    let room = if want > 0.0 {
        match body::look(&mut combined, pivot, away.to_array(), size, want * size / DRAWN_AS).expect("the area asked") {
            Some(hit) => {
                let far = ((hit[0] - pivot[0]).powi(2) + (hit[1] - pivot[1]).powi(2) + (hit[2] - pivot[2]).powi(2)).sqrt();
                (far / per) as f32 - MARGIN
            }
            None => want,
        }
    } else {
        0.0
    };
    let target = room.clamp(0.0, want);
    player.back = if target < player.back { target } else { player.back + (target - player.back) * (1.0 - (-4.0 * time.delta_secs()).exp()) };
}

/// Where the chunks are drawn, and whether: everything but the camera, sun, ghosts, carried item, and procedural scene.
type Placed<'w, 's> = Query<
    'w,
    's,
    (&'static mut Transform, &'static mut Visibility, Option<&'static mut MeshMaterial3d<SurfaceMaterial>>),
    (Without<Camera3d>, Without<Sun>, Without<BrushGhost>, Without<ElementGhost>, Without<HeldVisual>, Without<CarryGhost>, Without<scene::SceneNode>),
>;

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn draw(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut surface_materials: ResMut<Assets<SurfaceMaterial>>,
    mut chunks: ResMut<Chunks>,
    mut visuals: ResMut<SceneVisuals>,
    live: Res<LiveScene>,
    player: Res<Player>,
    mut physics: NonSendMut<Physics>,
    mut transforms: Placed,
    mut static_q: Query<
        &mut Transform,
        (
            With<scene::SceneNode>,
            With<scene::StaticSceneMesh>,
            Without<scene::DynamicInstance>,
            Without<scene::SceneLight>,
            Without<Camera3d>,
            Without<Sun>,
        ),
    >,
    mut dynamic_q: Query<
        (&scene::DynamicInstance, &mut Transform, &mut Visibility, &mut Mesh3d),
        (With<scene::SceneNode>, Without<scene::StaticSceneMesh>, Without<scene::SceneLight>, Without<Camera3d>, Without<Sun>),
    >,
    mut light_q: Query<
        (&scene::SceneLight, &mut Transform, &mut PointLight),
        (With<scene::SceneNode>, Without<scene::StaticSceneMesh>, Without<scene::DynamicInstance>, Without<Camera3d>, Without<Sun>),
    >,
    mut camera: Query<(&mut Transform, &mut DistanceFog), With<Camera3d>>,
) {
    let size = player.body.size;
    let eye = player.eye.unwrap_or(player.body.eye());
    let look_vec = player.looking();
    let look_dir = [look_vec.x as f64, look_vec.y as f64, look_vec.z as f64];
    let v = player.body.velocity;
    let there = [0, 1, 2].map(|a| eye[a] + (v[a] * AHEAD) as f64);
    let mut ahead = vec![(there, size)];
    if let Some(change) = player.body.change {
        ahead.extend([(eye, change.to), (there, change.to)]);
    }
    let started = Instant::now();
    let mut open = |e: [f64; 3], s: f32| rings::open(&mut physics.0, e, s).expect("the area asked");
    let here = open(eye, size);
    let ahead: Vec<_> = ahead.into_iter().map(|(e, s)| (e, s, open(e, s))).collect();
    let scale = (DRAWN_AS / size) as f64;
    chunks.materials.sync_eye(&mut surface_materials, eye, scale);
    chunks.update(eye, look_dir, size, scale, here, &ahead, time.delta_secs(), &mut commands, &mut meshes);
    chunks.place(eye, scale, &mut transforms);
    scene::sync(&mut commands, &mut visuals, &live.world, eye, scale, &mut static_q, &mut dynamic_q, &mut light_q);
    chunks.counts.longest = chunks.counts.longest.max(started.elapsed());
    if let Ok((mut camera, mut fog)) = camera.single_mut() {
        camera.rotation = look(player.body.yaw, player.pitch);
        camera.translation = if player.third { Vec3::Y * player.lift - player.looking() * player.back } else { Vec3::ZERO };
        fog.falloff = FogFalloff::from_visibility((200.0 + 60.0 * size) * DRAWN_AS / size);
    }
}

fn size_name(size: f32) -> String {
    let named = SIZES.iter().find(|(_, h)| (h / size - 1.0).abs() < 0.01).map(|(s, _)| s.name());
    let tall = if size < 1.0 { format!("{:.1} cm", size * 100.0) } else { format!("{size:.1} m") };
    match named {
        Some(n) => format!("{n}, {tall}"),
        None => tall,
    }
}

#[allow(clippy::too_many_arguments)]
fn hud(
    mut text: Query<&mut Text, With<Hud>>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    player: Res<Player>,
    chunks: Res<Chunks>,
    visuals: Res<SceneVisuals>,
    sculpt: Res<Sculpt>,
    place: Res<PlaceMode>,
    carry: Res<CarryState>,
    diagnostics: Res<DiagnosticsStore>,
    area: Res<Area>,
) {
    let Ok(mut text) = text.single_mut() else { return };
    let b = &player.body;
    let fps = diagnostics.get(&FrameTimeDiagnosticsPlugin::FPS).and_then(|d| d.smoothed()).unwrap_or(0.0);
    let doing = if b.seat.is_some() {
        "sitting".to_string()
    } else if let Some(c) = b.change {
        format!("{} to {}", if c.to > c.from { "growing" } else { "shrinking" }, size_name(c.to))
    } else {
        match b.on {
            Some(m) => format!("on {}", m.name()),
            None => "in the air".into(),
        }
    };
    let c = chunks.counts;
    let feet = b.seat.map_or(b.feet, |s| s.at);
    let holes = if c.holes > 0 { format!(", {} not yet stood in for", c.holes) } else { String::new() };
    let mods_note = if !sculpt.edits.is_empty() || !place.placed.is_empty() || sculpt.dirty {
        format!(
            "  /  {} edits, {} elements{}",
            sculpt.edits.len(),
            place.placed.len(),
            if sculpt.dirty { "*" } else { "" }
        )
    } else {
        String::new()
    };
    let tile_note = if area.tile_names.len() > 1 {
        let cur = [(feet[0] / WIDTH as f64).floor() as i32, (feet[2] / WIDTH as f64).floor() as i32];
        area.tile_names
            .iter()
            .find(|(at, _)| *at == cur)
            .map(|(_, name)| format!("  /  in {name} ({}, {})", cur[0], cur[1]))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let mut s = format!(
        "{}  /  {doing}  /  tool: {} (C){tile_note}{mods_note}\n{fps:.0} frames a second  /  {} chunks shown, {} being made{holes}, {} ahead  /  {}k triangles ({}k scene)\nat {:.3}, {:.3}, {:.3}",
        size_name(b.size),
        sculpt.tool.short_name(),
        c.shown,
        c.waiting,
        c.ahead,
        (c.triangles + visuals.triangles) / 1000,
        visuals.triangles / 1000,
        feet[0],
        feet[1],
        feet[2]
    );
    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if let Some(boat_id) = carry.piloting {
        let name = place
            .placed
            .iter()
            .find(|e| e.id == boat_id)
            .and_then(|pl| place.catalog.get(&pl.module))
            .map_or_else(|| format!("#{boat_id}"), |c| c.name.clone());
        s += &format!("\nSAIL: {name}  (W/S row, A/D steer, F or Space dismount)");
    } else if let Some(item) = &carry.held {
        let orient = if item.flat { "horizontal" } else { "upright" };
        let yaw_deg = item.yaw.to_degrees().rem_euclid(360.0).round() as i32;
        let tilt_deg = item.pitch.to_degrees().round() as i32;
        let speed_pct = (item.speed_factor * 100.0).round() as i32;
        s += &format!(
            "\nCARRY: {} ({}, pace {}%)  /  {orient}  /  yaw {yaw_deg}° tilt {tilt_deg:+}°  (LMB/G place, H flip flat, R/T turn, Wheel tilt, RMB/X cancel)",
            item.name,
            format_mass(item.physics.world_mass_kg),
            speed_pct
        );
    } else if sculpt.active {
        let r = sculpt.radius(b.size);
        let r_str = if r < 0.1 {
            format!("{:.1} mm", r * 1000.0)
        } else if r < 1.0 {
            format!("{:.1} cm", r * 1000.0)
        } else {
            format!("{r:.2} m")
        };
        let hint = if !grabbed {
            "  [click window to lock mouse]"
        } else if sculpt.aim.is_none() {
            "  [aim at a surface within reach]"
        } else {
            ""
        };
        s += &format!(
            "\nSCULPT: {}  /  {}  /  radius {r_str}{hint}  (LMB add, RMB/X carve, Tab shape, R mat, Wheel size, Ctrl+S save)",
            sculpt.preset.name(),
            sculpt.material().name()
        );
    } else if place.active {
        let hint = if !grabbed {
            "  [click window to lock mouse]"
        } else if place.aim.is_none() {
            "  [aim at a surface within reach]"
        } else {
            ""
        };
        if let Some(el) = place.current() {
            let kind = if el.is_sdf() { "SDF" } else { "Scene" };
            let sc = place.placement_scale(b.size);
            let deg = (place.yaw.to_degrees().rem_euclid(360.0)).round() as i32;
            s += &format!(
                "\nPLACE: {} [{kind}]  /  scale {sc:.2}x  /  yaw {deg}°{hint}  (LMB place, RMB/X remove, Tab next, R/T rotate, Wheel scale, Ctrl+R rescan)",
                el.name
            );
        } else {
            s += "\nPLACE: (no .wasm elements found — run ./elements/build.sh or plateau make-element)";
        }
    }
    for (name, size, at) in &area.places {
        let far = ((at[0] - feet[0]).powi(2) + (at[1] - feet[1]).powi(2) + (at[2] - feet[2]).powi(2)).sqrt();
        if far < 2.0 * b.size.min(size.height()) as f64 {
            s += &format!("\nhere: {name}, a place for the {}", size.name());
        } else if player.flags && flags::flagged(*size) {
            let far = if far < 1.0 { format!("{:.0} cm", far * 100.0) } else { format!("{far:.1} m") };
            s += &format!("\nflag: {name}, for the {}: {far} {}", size.name(), flags::compass(feet, *at));
        }
    }
    if let Some((said, until)) = &player.said
        && Instant::now() < *until
    {
        s += &format!("\n{said}");
    }
    text.0 = s;
}

fn screenshot(mut commands: Commands, keys: Res<ButtonInput<KeyCode>>, mut player: ResMut<Player>) {
    if keys.just_pressed(KeyCode::F12) {
        let folder = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/shots");
        std::fs::create_dir_all(&folder).ok();
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_secs();
        let path = folder.join(format!("{since}.png"));
        player.say(format!("screenshot: {}", path.display()));
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}

struct Stop {
    name: &'static str,
    size: f32,
    at: [f64; 2],
    yaw: f32,
    pitch: f32,
    sit: bool,
    then: Option<f32>,
    walk: Option<(f32, bool)>,
}

#[derive(Resource)]
struct Tour {
    folder: PathBuf,
    stops: Vec<Stop>,
    next: usize,
    phase: Phase,
    written: String,
}

enum Phase {
    Arrive,
    Wait(Instant),
    Measure { arrived: Instant, settled: Instant, made: usize, took: (Duration, Duration), frames: Vec<f32>, holey: usize, most: usize },
    Shot(usize),
}

impl Tour {
    fn new(folder: PathBuf) -> Tour {
        use std::f32::consts::{FRAC_PI_2, PI};
        let [tiny, person, giant] = sizes();
        let stops = vec![
            Stop { name: "person-at-the-start", size: person, at: START, yaw: 0.0, pitch: -0.05, sit: false, then: None, walk: None },
            Stop { name: "person-at-the-crack", size: person, at: [52.4, 30.0], yaw: -FRAC_PI_2, pitch: -0.55, sit: false, then: None, walk: None },
            Stop { name: "person-under-the-bench", size: person, at: [40.0, 21.5], yaw: 0.0, pitch: 0.55, sit: false, then: None, walk: None },
            Stop { name: "person-shrunk-at-the-crack", size: person, at: [53.4, 30.0], yaw: -FRAC_PI_2, pitch: -1.38, sit: false, then: Some(tiny), walk: None },
            Stop { name: "tiny-at-the-crack", size: tiny, at: [53.62, 30.0], yaw: -FRAC_PI_2, pitch: 0.05, sit: false, then: None, walk: None },
            Stop { name: "tiny-in-the-cave", size: tiny, at: [54.0, 30.25], yaw: 0.35, pitch: 0.0, sit: false, then: None, walk: None },
            Stop { name: "tiny-on-the-pond", size: tiny, at: [24.37, 36.82], yaw: 0.6, pitch: -0.08, sit: false, then: None, walk: None },
            Stop { name: "giant-over-the-garden", size: giant, at: [30.0, 62.0], yaw: 0.0, pitch: -0.45, sit: false, then: None, walk: None },
            Stop { name: "giant-on-the-bench", size: giant, at: [40.0, 23.5], yaw: PI, pitch: -0.4, sit: true, then: None, walk: None },
            Stop { name: "person-running-in-at-the-gate", size: person, at: START, yaw: 0.0, pitch: -0.1, sit: false, then: None, walk: Some((4.0, true)) },
            Stop { name: "tiny-running-along-the-wall", size: tiny, at: [53.62, 30.12], yaw: 0.0, pitch: -0.05, sit: false, then: None, walk: Some((4.0, true)) },
            Stop { name: "giant-walking-across-the-garden", size: giant, at: [20.0, 62.0], yaw: 0.3, pitch: -0.4, sit: false, then: None, walk: Some((3.0, false)) },
        ];
        Tour { folder, stops, next: 0, phase: Phase::Arrive, written: String::new() }
    }
}

fn go_on_tour(
    mut commands: Commands,
    mut tour: ResMut<Tour>,
    mut player: ResMut<Player>,
    live: Res<LiveScene>,
    mut physics: NonSendMut<Physics>,
    mut chunks: ResMut<Chunks>,
    time: Res<Time>,
    mut exit: MessageWriter<AppExit>,
) {
    let tour = &mut *tour;
    match &mut tour.phase {
        Phase::Arrive => {
            let Some(stop) = tour.stops.get(tour.next) else {
                std::fs::write(tour.folder.join("tour.txt"), &tour.written).expect("tour.txt written");
                print!("{}", tour.written);
                exit.write(AppExit::Success);
                return;
            };
            let mut combined = Combined { region: &mut physics.0, world: &live.world };
            let mut body = Body::standing(&mut combined, [stop.at[0], 0.1 + stop.size as f64 * 0.5, stop.at[1]], stop.size).expect("the area asked");
            body.yaw = stop.yaw;
            if stop.sit && !body.sit(&mut combined).expect("the area asked") {
                tour.written += &format!("{}: found nothing to sit on\n", stop.name);
            }
            (player.body, player.pitch, player.eye, player.focus, player.intent) = (body, stop.pitch, None, None, Intent::default());
            if let Some(to) = stop.then {
                change_size(&mut player, &mut combined, to);
            }
            chunks.clear_made();
            tour.phase = Phase::Wait(Instant::now());
        }
        Phase::Wait(arrived) => {
            let waited = arrived.elapsed();
            let still = player.body.change.is_none();
            if still && chunks.counts.waiting == 0 && waited > Duration::from_millis(200) || waited > Duration::from_secs(60) {
                let c = chunks.counts;
                tour.phase =
                    Phase::Measure { arrived: *arrived, settled: Instant::now(), made: c.made, took: (c.total, c.slowest), frames: Vec::new(), holey: 0, most: 0 };
            }
        }
        Phase::Measure { arrived, settled, made, took, frames, holey, most } => {
            let stop = &tour.stops[tour.next];
            if settled.elapsed() > Duration::from_secs(1) {
                frames.push(time.delta_secs());
                *holey += (chunks.counts.holes > 0) as usize;
                *most = (*most).max(chunks.counts.holes);
                if let Some((_, run)) = stop.walk {
                    player.intent = Intent { forward: 1.0, run, ..Intent::default() };
                }
            }
            if settled.elapsed() <= Duration::from_secs(1) {
                chunks.counts.longest = Duration::ZERO;
            }
            let done = match stop.walk {
                Some((seconds, _)) => frames.iter().sum::<f32>() >= seconds,
                None => frames.len() >= 240,
            };
            if !done {
                return;
            }
            player.intent = Intent::default();
            let walked = if stop.walk.is_some() {
                format!(
                    "; going, {} chunks made, {holey} of {} frames with holes, at most {most}, choosing and placing chunks {:.1} ms at most",
                    chunks.counts.made - *made,
                    frames.len(),
                    chunks.counts.longest.as_secs_f32() * 1000.0
                )
            } else {
                String::new()
            };
            frames.sort_by(f32::total_cmp);
            let mean = frames.iter().sum::<f32>() / frames.len() as f32;
            let c = chunks.counts;
            tour.written += &format!(
                "{}: {:.0} frames a second (worst frame {:.1} ms); {} chunks shown, {}k triangles; {made} chunks made in {:.2} s, \
                 {:.1} ms each, the slowest {:.0} ms{walked}\n",
                stop.name,
                1.0 / mean,
                frames[frames.len() - 1] * 1000.0,
                c.shown,
                c.triangles / 1000,
                (*settled - *arrived).as_secs_f32(),
                took.0.as_secs_f32() * 1000.0 / (*made).max(1) as f32,
                took.1.as_secs_f32() * 1000.0,
            );
            commands.spawn(Screenshot::primary_window()).observe(save_to_disk(tour.folder.join(format!("{}.png", stop.name))));
            tour.phase = Phase::Shot(15);
        }
        Phase::Shot(left) => {
            if *left == 0 {
                tour.next += 1;
                tour.phase = Phase::Arrive;
            } else {
                *left -= 1;
            }
        }
    }
}
