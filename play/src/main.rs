//! Plateau, played (POC_PLAN.md, steps 2, 7, and 8): one area, walked at three sizes, the player
//! seen from behind (`figure`) or through their own eyes, with live multi-scale SDF sculpting (`B`)
//! and composable standalone `.wasm` element placement (`P`).
//!
//! `cargo run --release -p plateau-play [-- area.wasm] [--seed N] [--save file.plateau] [--element elem.wasm ...] [--tour folder [--behind]]`;
//! the walled garden (`areas/modules/garden.wasm`) unless another module is given.

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
use chunks::{Chunks, Invalidate};
use place::{ElementGhost, PlaceMode, PreviewAssets, WorldAction};
use plateau_engine::body::{self, Body, Intent};
use plateau_engine::compose::{Composed, ComposedLog, ElementRuntime, PlacedElement, PlacementTransform};
use plateau_engine::edit::{Edit, EditLog, Op, WorldSave, surface_normal};
use plateau_engine::scene::{Applied, BASE_OWNER, Combined, SceneWorld};
use plateau_engine::{Command, Compiled, ElementBounds, Limits, PlayerState, Region, SIZES, START, Scene, Wasm, rings};
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
    let mut extra_elements: Vec<PathBuf> = Vec::new();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).expect("--seed takes a number"),
            "--tour" => tour = Some(PathBuf::from(args.next().expect("--tour takes a folder"))),
            "--behind" => behind = true,
            "--save" => save_arg = Some(PathBuf::from(args.next().expect("--save takes a file path"))),
            "--element" => extra_elements.push(PathBuf::from(args.next().expect("--element takes a .wasm path"))),
            _ => module = PathBuf::from(arg),
        }
    }
    let bytes = std::fs::read(&module).unwrap_or_else(|e| panic!("{}: {e} (areas/build.sh builds the areas)", module.display()));
    let compiled = Arc::new(Compiled::new(&bytes, Limits::default()).unwrap_or_else(|e| panic!("{}: {e}", module.display())));
    let (initial_edits, initial_elements) = match &save_arg {
        Some(path) if path.exists() => {
            let loaded = WorldSave::load(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            if loaded.seed != seed {
                eprintln!("note: save {} uses seed {} (overriding --seed {seed})", path.display(), loaded.seed);
                seed = loaded.seed;
            }
            let log = Arc::new(EditLog::from_edits(loaded.edits).unwrap_or_else(|e| panic!("{}: {e}", path.display())));
            (log, loaded.elements)
        }
        _ => (Arc::new(EditLog::new()), Vec::new()),
    };
    let stem = module
        .parent()
        .and_then(|p| p.file_name())
        .filter(|n| *n != "modules")
        .or_else(|| module.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("area");
    let save_path = save_arg.unwrap_or_else(|| here.join("target/saves").join(format!("{stem}-seed{seed}.plateau")));

    let mut place_mode = PlaceMode::new(here.clone(), extra_elements, initial_elements);
    let initial_composed = place_mode.build_composed(initial_edits.clone());
    let mut physics = Composed::new(compiled.instantiate(seed).expect("the area made"), initial_composed.clone());
    let described = physics.describe().expect("the area described");
    let initial_scene = physics.scene().expect("the area's scene");
    let mut world = SceneWorld::from_scene(&initial_scene);

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
            compiled,
            seed,
            places: described.places.iter().map(|p| (p.name.clone(), p.size, p.at.map(|c| c as f64))).collect(),
            flags,
            initial_scene: Some(initial_scene),
            initial_element_scenes,
            initial_composed: Some(initial_composed),
        })
        .insert_resource(Sculpt::new(initial_edits, save_path, described.name.clone(), seed))
        .insert_resource(place_mode)
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
        .add_systems(Update, (look_and_ask, step, frame, draw, sculpt::place_ghost, place::place_ghost, figure::animate, flags::place, hud).chain());
    if let Some(folder) = tour {
        std::fs::create_dir_all(&folder).expect("the tour's folder");
        app.insert_resource(Tour::new(folder)).add_systems(Update, go_on_tour.before(step));
    } else {
        app.add_systems(Update, (grab_the_mouse.after(look_and_ask), screenshot));
    }
    app.run();
}

/// The area: its module, to make an instance for each thread, and the places it describes.
#[derive(Resource)]
struct Area {
    compiled: Arc<Compiled>,
    seed: u64,
    places: Vec<(String, plateau_engine::Size, [f64; 3])>,
    /// Where each flagged place's flag stands (`flags::flagged`), in the places' order.
    flags: Vec<(plateau_engine::Size, [f64; 3])>,
    /// Initial procedural scene payload, taken once in `setup` to upload GPU meshes.
    initial_scene: Option<Scene>,
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

/// The instance the player's body is moved through (base WASM area + live SDF edits + placed SDF elements).
struct Physics(Composed<Wasm, Arc<ComposedLog>>);

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
    let initial = area.initial_scene.take().unwrap_or_default();
    let mut visuals = scene::spawn(&mut commands, &mut meshes, surface.opaque.clone(), &initial, &live.world);
    for (owner, elem_scene, skip_solid) in area.initial_element_scenes.drain(..) {
        scene::spawn_layer_filtered(&mut commands, &mut meshes, &mut visuals, owner, &elem_scene, &live.world, skip_solid);
    }
    commands.insert_resource(visuals);
    let composed = area.initial_composed.take().unwrap_or_else(|| Arc::new(ComposedLog::empty()));
    commands.insert_resource(Chunks::new(area.compiled.clone(), area.seed, composed, surface));
    figure::spawn(&mut commands, &mut meshes, &mut materials);
    flags::spawn(&mut commands, &mut meshes, &mut materials, &area.flags);
    sculpt::spawn_ghost(&mut commands, &mut meshes, &mut materials);
    place::spawn_ghost(&mut commands, &mut meshes, &mut materials);
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
            "W A S D walk  /  Shift run  /  Space jump  /  F interact  /  B sculpt  /  P place element (LMB place, RMB/X remove, Tab next, R/T rotate, Wheel scale, Ctrl+R rescan)\n\
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

/// Bundled resources for world editing (`B` sculpt & `P` place element) to keep Bevy's system
/// parameter count compact.
#[derive(SystemParam)]
struct EditCtx<'w, 's> {
    commands: Commands<'w, 's>,
    meshes: ResMut<'w, Assets<Mesh>>,
    visuals: ResMut<'w, SceneVisuals>,
    preview: ResMut<'w, PreviewAssets>,
    sculpt: ResMut<'w, Sculpt>,
    place: ResMut<'w, PlaceMode>,
    chunks: ResMut<'w, Chunks>,
    live: ResMut<'w, LiveScene>,
    physics: NonSendMut<'w, Physics>,
    elements: NonSendMut<'w, Elements>,
}

impl EditCtx<'_, '_> {
    fn commit_composed(&mut self, inv: Invalidate<'_>) {
        let composed = self.place.build_composed(self.sculpt.edits.clone());
        self.physics.0.log = composed.clone();
        self.physics.0.prune_instances();
        self.chunks.set_composed(composed, inv);
    }

    fn spawn_placed(&mut self, pl: PlacedElement, player: &mut Player) -> plateau_engine::Result<String> {
        let compiled = self.place.catalog.get_or_load(&pl.module)?;
        let name = compiled.name.clone();
        let world_scene = self.elements.0.spawn(pl.clone(), &compiled, &mut self.live.world)?;
        scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, pl.id, &world_scene, &self.live.world, pl.is_promoted());
        self.place.placed.push(pl.clone());
        self.sculpt.dirty = true;
        let sdf_bounds = if let Some(bounds) = compiled.sdf_bounds {
            Some(bounds)
        } else if pl.is_promoted() {
            self.place.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed)?.map(|m| m.bounds())
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
            let max_lift = (pl.scale * 3.0) as f64 + player.body.size as f64;
            let mut combined = Combined { region: &mut self.physics.0, world: &self.live.world };
            let _ = player.body.surface_from_solid(&mut combined, max_lift);
        }
        Ok(name)
    }

    fn remove_placed(&mut self, id: u32) -> Option<PlacedElement> {
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
            let Some(pos) = self.place.placed.iter().position(|e| e.id == id) else { continue };
            if self.place.placed[pos].is_promoted() {
                continue;
            }
            let module = self.place.placed[pos].module.clone();
            let seed = self.place.placed[pos].seed;
            let Ok(compiled) = self.place.catalog.get_or_load(&module) else { continue };
            if compiled.is_sdf() {
                continue;
            }
            let Ok(Some(mesh_sdf)) = self.place.catalog.get_or_build_mesh_sdf(&module, seed) else { continue };
            self.place.placed[pos].promoted = Some(edit_idx);
            let tf = self.place.placed[pos].transform();
            if let Some(world_scene) = self.elements.0.set_promoted(id, Some(edit_idx), &mut self.live.world) {
                scene::spawn_layer_filtered(&mut self.commands, &mut self.meshes, &mut self.visuals, id, &world_scene, &self.live.world, true);
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
            let tf = self.place.placed[pos].transform();
            if let Ok(Some(mesh_sdf)) = self.place.catalog.get_or_build_mesh_sdf(&module, seed) {
                bounds_list.push((tf, mesh_sdf.bounds()));
            }
            self.place.placed[pos].promoted = next_state;
            if let Some(world_scene) = self.elements.0.set_promoted(id, next_state, &mut self.live.world) {
                scene::spawn_layer_filtered(
                    &mut self.commands,
                    &mut self.meshes,
                    &mut self.visuals,
                    id,
                    &world_scene,
                    &self.live.world,
                    next_state.is_some(),
                );
            }
        }
        bounds_list
    }

    fn reload_world(&mut self) -> plateau_engine::Result<(usize, usize)> {
        let saved = self.sculpt.load_world()?;
        let edit_count = saved.edits.len();
        self.sculpt.replace_edits(saved.edits)?;
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
    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if grabbed && player.body.change.is_none() {
        player.body.yaw -= motion.delta.x * 0.0025;
        player.pitch = (player.pitch - motion.delta.y * 0.0025).clamp(-1.55, 1.55);
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let held = |k: KeyCode| keys.pressed(k) as i32 as f32;
    player.intent = Intent {
        forward: if ctrl { 0.0 } else { held(KeyCode::KeyW) - held(KeyCode::KeyS) },
        right: held(KeyCode::KeyD) - held(KeyCode::KeyA),
        run: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
        jump: keys.pressed(KeyCode::Space),
    };

    if keys.just_pressed(KeyCode::KeyB) {
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

    let tool_active = ctx.sculpt.active || ctx.place.active;
    if keys.just_pressed(KeyCode::KeyF) || (!tool_active && buttons.just_pressed(MouseButton::Right)) {
        ctx.live.interact_pressed = true;
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
                    // Lazy Scene → SDF conversion: if this sculpt touches any unpromoted solid
                    // placed `Scene` element (tree, bridge, etc.), promote its solid mesh to SDF
                    // right before this edit so carving/adding cuts into it seamlessly!
                    let edit_idx = ctx.sculpt.edits.len();
                    let aimed_solid = ctx
                        .live
                        .world
                        .raycast_solid_placed(pivot.map(|c| c as f32), dir, reach)
                        .and_then(|(owner, t)| {
                            let hit_dist = ((h[0] - pivot[0]).powi(2) + (h[1] - pivot[1]).powi(2) + (h[2] - pivot[2]).powi(2)).sqrt() as f32;
                            if (t - hit_dist).abs() <= ctx.sculpt.radius(psize) * 2.0 + psize * 0.5 {
                                Some(owner)
                            } else {
                                None
                            }
                        });
                    let (promoted_ids, promo_bounds, promo_names) = ctx.promote_overlapping_scene_elements(&edit, aimed_solid, edit_idx);
                    match ctx.sculpt.push_edit(edit) {
                        Ok(()) => {
                            ctx.place.undo_stack.push(WorldAction::SdfEdit { edit, promoted_ids });
                            ctx.place.redo_stack.clear();
                            if promo_bounds.is_empty() {
                                ctx.commit_composed(Invalidate::Edit(&edit));
                            } else {
                                ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds });
                                player.say(format!("{} converted to carvable solid", promo_names.join(", ")));
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
            let id = ctx.place.alloc_id();
            let scale = ctx.place.placement_scale(psize);
            let pl = PlacedElement {
                id,
                module: elem.module.clone(),
                seed: ctx.sculpt.seed.wrapping_mul(31).wrapping_add(id as u64),
                pos: [h[0] as f32, h[1] as f32, h[2] as f32],
                yaw: ctx.place.yaw,
                scale,
                stamp_size: psize,
                after_edits: ctx.sculpt.edits.len(),
                promoted: None,
            };
            match ctx.spawn_placed(pl.clone(), &mut player) {
                Ok(name) => {
                    ctx.place.undo_stack.push(WorldAction::PlaceElement(pl));
                    ctx.place.redo_stack.clear();
                    player.say(format!("placed {name} ({scale:.2}x)"));
                }
                Err(e) => player.say(format!("place error: {e}")),
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
                if let Some(removed) = ctx.remove_placed(id) {
                    ctx.place.undo_stack.push(WorldAction::RemoveElement(removed));
                    ctx.place.redo_stack.clear();
                    player.say(format!("removed element #{id}"));
                }
            } else {
                player.say("aim at a placed element to remove it");
            }
        }
    }

    // Unified Undo / Redo across sculpt edits, lazy promotions, and placed elements.
    if (tool_active && keys.just_pressed(KeyCode::KeyZ)) || (ctrl && keys.just_pressed(KeyCode::KeyZ)) {
        if let Some(action) = ctx.place.undo_stack.pop() {
            match action.clone() {
                WorldAction::SdfEdit { edit, promoted_ids } => {
                    if ctx.sculpt.pop_edit().is_some() {
                        let demote: Vec<_> = promoted_ids.iter().map(|&(id, _)| (id, None)).collect();
                        let promo_bounds = ctx.set_elements_promoted(&demote);
                        if promo_bounds.is_empty() {
                            ctx.commit_composed(Invalidate::Edit(&edit));
                        } else {
                            ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds });
                        }
                        ctx.place.redo_stack.push(action);
                        player.say(format!("undo sculpt ({} edits)", ctx.sculpt.edits.len()));
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
            }
        } else {
            player.say("nothing to undo");
        }
    }

    if (tool_active && keys.just_pressed(KeyCode::KeyY)) || (ctrl && keys.just_pressed(KeyCode::KeyY)) {
        if let Some(action) = ctx.place.redo_stack.pop() {
            match action.clone() {
                WorldAction::SdfEdit { edit, promoted_ids } => {
                    let repromote: Vec<_> = promoted_ids.iter().map(|&(id, idx)| (id, Some(idx))).collect();
                    let promo_bounds = ctx.set_elements_promoted(&repromote);
                    if ctx.sculpt.push_edit(edit).is_ok() {
                        if promo_bounds.is_empty() {
                            ctx.commit_composed(Invalidate::Edit(&edit));
                        } else {
                            ctx.commit_composed(Invalidate::Combined { edit: Some(&edit), bounds: &promo_bounds });
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
            }
        } else {
            player.say("nothing to redo");
        }
    }

    // Save / Reload complete world (base area + seed + SDF edits + placed elements).
    if keys.just_pressed(KeyCode::F5) || (ctrl && keys.just_pressed(KeyCode::KeyS)) {
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

fn apply_commands(world: &mut SceneWorld, player: &mut Player, cmds: &[Command]) {
    if cmds.is_empty() {
        return;
    }
    let applied = world.apply(cmds);
    apply_side_effects(player, &applied);
}

/// The runtime plugins (base area + placed elements) ticked and the body moved on in steps.
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
    let ps_base = PlayerState {
        feet: player.body.feet.map(|c| c as f32),
        eye: eye32,
        look: look_dir,
        velocity: player.body.velocity,
        size,
        grounded: player.body.on.is_some(),
        holding_use: live.holding_use,
        target: if hit_owner == BASE_OWNER { hit_id } else { 0 },
        aim,
        time: live.elapsed,
    };

    if dt > 0.0 {
        match physics.0.tick(dt, &ps_base) {
            Ok(cmds) => apply_commands(&mut live.world, &mut player, &cmds),
            Err(e) => eprintln!("area tick error: {e}"),
        }
        for eff in elements.0.tick_all(dt, &ps_base, hit_owner, hit_id, &mut live.world) {
            apply_side_effects(&mut player, &eff);
        }
    }
    if std::mem::take(&mut live.interact_pressed) {
        if hit_owner == BASE_OWNER {
            match physics.0.interact(&ps_base) {
                Ok(cmds) => apply_commands(&mut live.world, &mut player, &cmds),
                Err(e) => eprintln!("area interact error: {e}"),
            }
        }
        for eff in elements.0.interact(&ps_base, hit_owner, hit_id, &mut live.world) {
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

/// Where the chunks are drawn, and whether: everything but the camera, sun, ghosts, and procedural scene.
type Placed<'w, 's> = Query<
    'w,
    's,
    (&'static mut Transform, &'static mut Visibility, Option<&'static mut MeshMaterial3d<SurfaceMaterial>>),
    (Without<Camera3d>, Without<Sun>, Without<BrushGhost>, Without<ElementGhost>, Without<scene::SceneNode>),
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
    chunks.update(eye, look_dir, size, here, &ahead, time.delta_secs(), &mut commands, &mut meshes);
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
    let mut s = format!(
        "{}  /  {doing}{mods_note}\n{fps:.0} frames a second  /  {} chunks shown, {} being made{holes}, {} ahead  /  {}k triangles ({}k scene)\nat {:.3}, {:.3}, {:.3}",
        size_name(b.size),
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
    if sculpt.active {
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
