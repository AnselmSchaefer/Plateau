//! Plateau, played (POC_PLAN.md, steps 2, 7, 8, and 9): one or more areas stitched side by side,
//! walked at three sizes, the player seen from behind (`figure`) or through their own eyes, with
//! live multi-scale SDF sculpting (`B`), composable standalone `.wasm` element placement (`P`),
//! and physical chopping (`K`) & carrying (`G`).
//!
//! `cargo run --release -p plateau-play [-- area.wasm] [--north|--east|--south|--west area.wasm] [--tile gx,gz:area.wasm ...] [--seed N] [--save file.plateau [--accept-changed-modules]] [--log actions.jsonl] [--trace trace.json] [--element elem.wasm ...] [--tour folder [--behind]]`;
//! the walled garden (`areas/modules/garden.wasm`) unless another module is given, or the areas the
//! save names. The world itself (`plateau_engine::live::World`) is the engine's: this crate asks it
//! for changes (`World::apply`) and shows what changed.

mod carry;
mod chunks;
mod figure;
mod flags;
mod place;
mod scene;
mod sculpt;
mod surface;
mod trace;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::asset::embedded_asset;
use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::ecs::system::SystemParam;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll};
use bevy::log::LogPlugin;
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::pbr::{DistanceFog, FogFalloff, MaterialPlugin};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use carry::{CarriedItem, CarryAssets, CarryGhost, CarryState, HeldVisual};
use chunks::Chunks;
use place::{ElementGhost, FittedSnap, PlaceMode, PreviewAssets};
use plateau_engine::atlas::{CompiledAtlas, Side};
use plateau_engine::body::{self, Body, Intent};
use plateau_engine::cut::{check_carry, format_mass};
use plateau_engine::edit::{Op, SavedTile, WorldSave, surface_normal};
use plateau_engine::fit::fit_transform;
use plateau_engine::laws::length;
use plateau_engine::live::{self, ActionLog, Actor, LayerChange, Outcome, SculptTarget, World};
use plateau_engine::modules::{self, ModuleCheck};
use plateau_engine::rules::LOCAL_PLAYER;
use plateau_engine::scene::{Applied, BASE_OWNER, SceneWorld};
use plateau_engine::site::{SiteContext, SiteProbe};
use plateau_engine::{Limits, PlayerState, Region, SIZES, START, Scene, WIDTH, rings};
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
    let mut module_given = false;
    let mut module_check = ModuleCheck::Strict;
    let mut log_arg: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).expect("--seed takes a number"),
            "--player" => player_id = args.next().expect("--player takes a player id"),
            "--tour" => tour = Some(PathBuf::from(args.next().expect("--tour takes a folder"))),
            "--behind" => behind = true,
            "--save" => save_arg = Some(PathBuf::from(args.next().expect("--save takes a file path"))),
            "--accept-changed-modules" => module_check = ModuleCheck::AcceptChanged,
            "--log" => log_arg = Some(PathBuf::from(args.next().expect("--log takes a file path"))),
            "--trace" => {
                let _ = trace::TRACE_TO.set(PathBuf::from(args.next().expect("--trace takes a file path")));
            }
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
            _ => {
                module = PathBuf::from(arg);
                module_given = true;
            }
        }
    }
    let loaded = match &save_arg {
        Some(path) if path.exists() => Some((path.clone(), WorldSave::load(path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))),
        _ => None,
    };
    if let Some((path, loaded)) = &loaded
        && loaded.seed != seed
    {
        eprintln!("note: save {} uses seed {} (overriding --seed {seed})", path.display(), loaded.seed);
        seed = loaded.seed;
    }
    let save = loaded.as_ref().map_or_else(|| WorldSave::new("", seed, Vec::new()), |(_, l)| l.clone());
    // The areas the save was played on, unless others are named.
    let save_tiles = !module_given && extra_tiles.is_empty() && save.tiles.iter().any(|t| t.at == [0, 0]);
    let tiles = (!save_tiles).then(|| {
        let mut tiles = vec![SavedTile { at: [0, 0], module: module.to_string_lossy().into_owned(), seed }];
        tiles.extend(extra_tiles.iter().map(|(at, p)| SavedTile { at: *at, module: p.to_string_lossy().into_owned(), seed }));
        tiles
    });
    if save_tiles && let Some(t) = save.tiles.iter().find(|t| t.at == [0, 0]) {
        module = modules::locate(&t.module, &here).unwrap_or_else(|e| panic!("{e:#}"));
    }
    let what = loaded.as_ref().map_or_else(|| module.display().to_string(), |(p, _)| p.display().to_string());
    let (mut world, area_scenes) =
        World::open(save, tiles, &here, Limits::default(), module_check, SIZES[1].1).unwrap_or_else(|e| panic!("{what}: {e:#} (areas/build.sh builds the areas)"));
    if !world.changed_modules.is_empty() {
        eprintln!("warning: changed since {what} was saved, loaded anyway: {}", world.changed_modules.join(", "));
    }
    if let Some(path) = &log_arg {
        world.set_log(Some(ActionLog::create(path).unwrap_or_else(|e| panic!("{}: {e:#}", path.display()))));
        eprintln!("logging actions to {}", path.display());
    }
    let stem = module
        .parent()
        .and_then(|p| p.file_name())
        .filter(|n| *n != "modules")
        .or_else(|| module.file_stem())
        .and_then(|s| s.to_str())
        .unwrap_or("area");
    let save_path = save_arg.unwrap_or_else(|| here.join("target/saves").join(format!("{stem}-seed{seed}.plateau")));

    let place_mode = PlaceMode::new(extra_elements, &mut world);
    let tile_names: Vec<([i32; 2], String)> =
        world.region.region.describe_tiles().expect("the areas described").into_iter().map(|(at, d)| (at, d.name)).collect();
    let described = world.region.describe().expect("the area described");
    world.area_name = described.name.clone();
    let flags: Vec<_> = {
        let mut combined = world.combined();
        described.places.iter().filter(|p| flags::flagged(p.size)).map(|p| (p.size, flags::foot(&mut combined, p.at.map(|c| c as f64)))).collect()
    };
    let body = Body::standing(&mut world.combined(), [START[0], 40.0, START[1]], SIZES[1].1).expect("the area asked");
    let changed_note = (!world.changed_modules.is_empty())
        .then(|| (format!("changed since saved, loaded anyway: {}", world.changed_modules.join(", ")), Instant::now() + Duration::from_secs(10)));

    let present_mode = if tour.is_some() || std::env::var_os("PLATEAU_NO_VSYNC").is_some() { PresentMode::AutoNoVsync } else { PresentMode::AutoVsync };
    let window = Window { title: format!("Plateau: {}", described.name), present_mode, resolution: (1600, 900).into(), ..default() };
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }).set(LogPlugin { custom_layer: trace::layer, ..default() }),
    );
    embedded_asset!(app, "play/src", "surface.wesl");
    app.add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(MaterialPlugin::<SurfaceMaterial>::default())
        .insert_resource(ClearColor(SKY))
        .insert_resource(GlobalAmbientLight { brightness: 500.0, ..default() })
        .insert_resource(Area {
            atlas: world.atlas.clone(),
            tile_names,
            places: described.places.iter().map(|p| (p.name.clone(), p.size, p.at.map(|c| c as f64))).collect(),
            flags,
            initial_area_scenes: area_scenes,
        })
        .insert_resource(Sculpt::new(player_id, save_path))
        .insert_resource(place_mode)
        .insert_resource(CarryState::default())
        .insert_resource(LiveScene { elapsed: 0.0, interact_pressed: false, holding_use: false })
        .insert_resource(Player {
            body,
            pitch: -0.15,
            eye: None,
            focus: None,
            said: changed_note,
            intent: Intent::default(),
            owed: 0.0,
            third: tour.is_none() || behind,
            back: 0.0,
            lift: PIVOT,
            flags: tour.is_none() || behind,
        })
        .insert_resource(figure::Gait::default())
        .insert_non_send(Live(world))
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (look_and_ask, show_world_changes, step, frame, draw, sculpt::place_ghost, place::place_ghost, carry::sync_visuals, figure::animate, flags::place, hud).chain(),
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
}

/// Time in the world and the use key's state, for the procedural scenes' `tick` and `interact`.
#[derive(Resource)]
struct LiveScene {
    elapsed: f32,
    interact_pressed: bool,
    holding_use: bool,
}

/// The live world (`plateau_engine::live::World`): its areas, sculpting, elements and their
/// sandboxed runtimes, scenes, rules and history. Every change to it is made by the engine.
struct Live(World);

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
    mut world: NonSendMut<Live>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut surface_materials: ResMut<Assets<SurfaceMaterial>>,
) {
    let surface = Materials::new(&mut surface_materials);
    let area_scenes = std::mem::take(&mut area.initial_area_scenes);
    let world = &mut world.0;
    let mut visuals = scene::spawn_areas(&mut commands, &mut meshes, surface.opaque.clone(), &area_scenes, &world.scene);
    // The elements the save placed; the chunks are made from the SDF as it now is.
    let changes = world.take_changes();
    show_layers(&mut commands, &mut meshes, &mut visuals, &world.scene, changes.layers);
    commands.insert_resource(visuals);
    let composed = world.composed();
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
            "W A S D walk  /  Shift run  /  Space jump  /  F interact  /  K chop  /  G grab & carry (H flat, R/T turn, Wheel tilt)  /  B sculpt  /  P place (O fit cavity)  /  I measure site\n\
             1 2 3 tiny, person, giant  /  Q E shrink, grow  /  Z/Y undo/redo  /  Ctrl+S save  /  Ctrl+L load  /  N tool  /  C sit  /  V view  /  M flags  /  F12 shot",
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
struct EditCtx<'w> {
    meshes: ResMut<'w, Assets<Mesh>>,
    preview: ResMut<'w, PreviewAssets>,
    sculpt: ResMut<'w, Sculpt>,
    place: ResMut<'w, PlaceMode>,
    carry: ResMut<'w, CarryState>,
    carry_assets: Res<'w, CarryAssets>,
    live: ResMut<'w, LiveScene>,
    world: NonSendMut<'w, Live>,
}

impl EditCtx<'_> {
    /// Asks the world for `action` as the player; what came of it is said.
    fn apply(&mut self, player: &mut Player, action: live::Action) -> Outcome {
        let id = self.sculpt.player_id.clone();
        let outcome = self.world.0.apply(Actor { id: &id, body: &mut player.body }, action);
        if let Some(said) = outcome.said()
            && !said.is_empty()
        {
            player.say(said);
        }
        outcome
    }

    /// Puts the carried object back where it was picked up (saying so if `say`).
    fn cancel_carry(&mut self, player: &mut Player, say: bool) {
        if self.carry.held.take().is_none() {
            return;
        }
        let id = self.sculpt.player_id.clone();
        let outcome = self.world.0.apply(Actor { id: &id, body: &mut player.body }, live::Action::Return);
        if say && let Some(said) = outcome.said() {
            player.say(said);
        }
    }

    /// Sets the carried object down at `aim_world` as it is held (turned, tilted, flat or upright),
    /// as solid walkable geometry. Whether it was.
    fn drop_carried_at(&mut self, aim_world: [f64; 3], player: &mut Player) -> bool {
        let Some(item) = &self.carry.held else { return false };
        let tf = item.target_transform(aim_world);
        let done = self.apply(player, live::Action::Drop { pos: tf.pos, rot: tf.rot }).is_done();
        if done {
            self.carry.held = None;
        }
        done
    }
}

/// Shows a placed element's scene layer changes (`World::take_changes`).
fn show_layers(commands: &mut Commands, meshes: &mut Assets<Mesh>, visuals: &mut SceneVisuals, world: &SceneWorld, layers: Vec<LayerChange>) {
    for change in layers {
        match change {
            LayerChange::Spawn { owner, scene, skip_solid } => scene::spawn_layer_filtered(commands, meshes, visuals, owner, &scene, world, skip_solid),
            LayerChange::Despawn { owner } => scene::despawn_layer(commands, visuals, owner),
            LayerChange::DespawnAllPlaced => scene::despawn_placed_layers(commands, visuals),
            LayerChange::Moved { owner, from, to } => visuals.set_layer_reloc(owner, from, to),
        }
    }
}

/// Shows what the world changed this frame: the new SDF to the chunks, the elements' layers, and
/// what the player is told.
fn show_world_changes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut visuals: ResMut<SceneVisuals>,
    mut chunks: ResMut<Chunks>,
    mut world: NonSendMut<Live>,
    mut player: ResMut<Player>,
) {
    let world = &mut world.0;
    let changes = world.take_changes();
    if changes.is_empty() {
        return;
    }
    for (composed, inv) in changes.composed {
        chunks.set_composed(composed, &inv);
    }
    show_layers(&mut commands, &mut meshes, &mut visuals, &world.scene, changes.layers);
    for said in changes.said {
        player.say(said);
    }
}

/// What the player asks this frame, and from where they look: for the modes' handlers.
#[derive(Clone, Copy)]
struct Asking<'a> {
    keys: &'a ButtonInput<KeyCode>,
    buttons: &'a ButtonInput<MouseButton>,
    scroll: &'a AccumulatedMouseScroll,
    time: &'a Time,
    /// The mouse is held by the window.
    grabbed: bool,
    ctrl: bool,
    /// Where rays from the crosshair start, and their way.
    pivot: [f64; 3],
    dir: [f32; 3],
    psize: f32,
    /// Sculpting, placing or carrying.
    tool_active: bool,
}

/// `I`: measures the site or opening at the crosshair, writing a `.site` file and the world beside it.
fn measure_site(ctx: &mut EditCtx<'_>, player: &mut Player, a: &Asking<'_>) {
    let Asking { keys, ctrl, pivot, dir, psize, .. } = *a;
    if !ctrl && keys.just_pressed(KeyCode::KeyI) {
        ctx.world.0.finish_all_falls(None);
        let reach = psize * 45.0;
        let measured = {
            let mut combined = ctx.world.0.combined();
            match body::look(&mut combined, pivot, dir, psize, reach).ok().flatten() {
                Some(hit) => {
                    let normal = surface_normal(&mut combined, hit, psize).unwrap_or([0.0, 1.0, 0.0]);
                    let probe = SiteProbe {
                        target: [hit[0] as f32, hit[1] as f32, hit[2] as f32],
                        normal,
                        from: Some([pivot[0] as f32, pivot[1] as f32, pivot[2] as f32]),
                        size: psize,
                    };
                    Some(SiteContext::measure(&mut combined, &probe))
                }
                None => None,
            }
        };
        match measured {
            None => player.say("aim at a surface or opening to measure a site (I)"),
            Some(Err(e)) => player.say(format!("site measure failed: {e}")),
            Some(Ok(mut site)) => {
                site.area = ctx.world.0.area_name.clone();
                let summary = site.summary();
                ctx.place.last_site = Some(site.clone());
                let stem = ctx
                    .sculpt
                    .save_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("world");
                let since = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let sites_dir = ctx
                    .sculpt
                    .save_path
                    .parent()
                    .and_then(|p| p.parent())
                    .unwrap_or_else(|| std::path::Path::new("target"))
                    .join("sites");
                let site_path = sites_dir.join(format!("{stem}-{since}.site"));
                let world_path = site_path.with_extension("plateau");
                let root = ctx.world.0.root.clone();
                match (site.save(&site_path), ctx.world.0.to_save().save_sealed(&world_path, &root)) {
                    (Ok(()), Ok(())) => {
                        println!("site: {summary}");
                        println!("saved: {} (+ {})", site_path.display(), world_path.display());
                        println!(
                            "make an element for it:\n  cargo run --release -p plateau-make --bin plateau -- make-element \"<words>\" --site {} --save {} --place",
                            site_path.display(),
                            ctx.sculpt.save_path.display()
                        );
                        let short = site_path.file_name().and_then(|s| s.to_str()).unwrap_or("site");
                        player.say(format!("site: {summary} (saved {short})"));
                    }
                    (Err(e), _) | (_, Err(e)) => {
                        eprintln!("site save failed: {e}");
                        player.say(format!("site: {summary} (save failed: {e})"));
                    }
                }
            }
        }
    }

}

/// `B` mode: brush shape, material and size; strokes on the terrain or on a placed element.
fn sculpt_input(ctx: &mut EditCtx<'_>, player: &mut Player, a: &Asking<'_>) {
    let Asking { keys, buttons, scroll, time, grabbed, ctrl, pivot, dir, psize, .. } = *a;
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
            let mut combined = ctx.world.0.combined();
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
                    // Sculpting straight onto a placed scene element (promoted or not) keeps the
                    // stroke in its own frame, so a carved tunnel travels with it when it is
                    // chopped or carried; anything else goes on the terrain.
                    let world = &ctx.world.0;
                    let hit_dist = ((h[0] - pivot[0]).powi(2) + (h[1] - pivot[1]).powi(2) + (h[2] - pivot[2]).powi(2)).sqrt() as f32;
                    let tol = ctx.sculpt.radius(psize) * 2.0 + psize * 0.5;
                    let aimed_unpromoted = world
                        .scene
                        .raycast_solid_placed(pivot.map(|c| c as f32), dir, reach)
                        .and_then(|(owner, t)| if (t - hit_dist).abs() <= tol { Some(owner) } else { None });
                    let h32 = [h[0] as f32, h[1] as f32, h[2] as f32];
                    let aimed_promoted_scene = world.raycast_placed_sdf(pivot.map(|c| c as f32), dir, reach).and_then(|(owner, _)| {
                        let pl = world.find(owner)?;
                        if !pl.is_promoted() || world.is_sdf_element(pl) {
                            return None;
                        }
                        let (lo, hi) = pl.transform().world_bounds(world.sdf_bounds_of(pl)?);
                        let pad = ctx.sculpt.radius(psize) * 1.2 + psize * 0.25;
                        (0..3).all(|a| h32[a] >= lo[a] - pad && h32[a] <= hi[a] + pad).then_some(owner)
                    });
                    let element = aimed_unpromoted
                        .or(aimed_promoted_scene)
                        .filter(|&owner| world.find(owner).is_some_and(|pl| !world.is_sdf_element(pl)));
                    let target = match element {
                        Some(id) => SculptTarget::Element(id),
                        None => SculptTarget::Terrain { aimed: aimed_unpromoted },
                    };
                    let tool = ctx.sculpt.tool;
                    match ctx.apply(player, live::Action::Sculpt { edit, hit: h32, tool, target }) {
                        Outcome::Done(_) => {
                            ctx.sculpt.last_stamp = Some(edit.center);
                            ctx.sculpt.hold_timer = if click_op.is_some() { 0.25 } else { 0.14 };
                        }
                        Outcome::Denied(_) => ctx.sculpt.hold_timer = 0.35,
                        Outcome::Unchanged(_) => {}
                    }
                }
            }
        }
    }

}

/// `P` mode: the palette, turning and scaling, fitting to an opening (`O`), placing and removing.
fn place_input(ctx: &mut EditCtx<'_>, player: &mut Player, a: &Asking<'_>) {
    let Asking { keys, buttons, scroll, grabbed, ctrl, pivot, dir, psize, .. } = *a;
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
            ctx.place.fitted = None;
        }
        if keys.just_pressed(KeyCode::KeyT) {
            ctx.place.yaw += std::f32::consts::FRAC_PI_6 * 0.5; // +15 deg
            ctx.place.fitted = None;
        }
        if scroll.delta.y > 0.0 || keys.just_pressed(KeyCode::BracketRight) {
            ctx.place.adjust_scale(1.15);
        } else if scroll.delta.y < 0.0 || keys.just_pressed(KeyCode::BracketLeft) {
            ctx.place.adjust_scale(1.0 / 1.15);
        }
        let reach = psize * 45.0;
        let aim_hit = {
            let mut combined = ctx.world.0.combined();
            body::look(&mut combined, pivot, dir, psize, reach).ok().flatten().map(|h| {
                let n = surface_normal(&mut combined, h, psize).unwrap_or([0.0, 1.0, 0.0]);
                (h, n)
            })
        };
        ctx.place.aim = aim_hit;

        // `O`: auto-fit / snap selected element to aimed carved opening (or its authored `element.site`).
        if !ctrl && keys.just_pressed(KeyCode::KeyO)
            && let Some(elem) = ctx.place.current().cloned()
        {
            let cavity_attempt = aim_hit.map(|(h, n)| {
                let probe = SiteProbe {
                    target: [h[0] as f32, h[1] as f32, h[2] as f32],
                    normal: n,
                    from: Some([pivot[0] as f32, pivot[1] as f32, pivot[2] as f32]),
                    size: psize,
                };
                let world = &mut ctx.world.0;
                let site = SiteContext::measure(&mut world.combined(), &probe);
                site.and_then(|s| {
                    let fitted = fit_transform(&mut world.region, &world.scene, &s, &elem.preview_scene)?;
                    Ok((s, fitted))
                })
            });
            match cavity_attempt {
                Some(Ok((site, Ok((tf, rim))))) => {
                    let summary = site.summary();
                    ctx.place.last_site = Some(site);
                    ctx.place.fitted = Some(FittedSnap {
                        module: elem.module.clone(),
                        transform: tf,
                        summary: summary.clone(),
                    });
                    player.say(format!(
                        "fitted {} ({} to {} into rim) — LMB to place",
                        elem.name,
                        length(rim.least),
                        length(rim.most)
                    ));
                }
                other => {
                    // Fallback: if this element was authored for a specific site (`element.site` beside `.wasm`), snap to it.
                    let authored = ctx.place.element_site_path(&elem.module, &ctx.world.0.root).and_then(|p| SiteContext::load(&p).ok());
                    if let Some(saved_site) = authored {
                        let summary = saved_site.summary();
                        ctx.place.fitted = Some(FittedSnap {
                            module: elem.module.clone(),
                            transform: saved_site.frame,
                            summary,
                        });
                        player.say(format!("snapped {} to its authored site — LMB to place", elem.name));
                    } else {
                        ctx.place.fitted = None;
                        match other {
                            Some(Ok((_, Err(why)))) => player.say(format!("cannot fit {}: {why}", elem.name)),
                            Some(Err(e)) => player.say(format!("fit measure failed: {e}")),
                            None => player.say("aim at a carved opening to auto-fit (O)"),
                            _ => {}
                        }
                    }
                }
            }
        }

        if grabbed && buttons.just_pressed(MouseButton::Left)
            && let Some(elem) = ctx.place.current().cloned()
        {
            let fitted_snap = ctx.place.fitted.clone().filter(|s| s.module == elem.module);
            let placement = if let Some(snap) = fitted_snap {
                Some((snap.transform.pos, 0.0, Some(snap.transform.rot), snap.transform.scale, true))
            } else {
                ctx.place.aim.map(|(h, _)| {
                    ([h[0] as f32, h[1] as f32, h[2] as f32], ctx.place.yaw, None, ctx.place.placement_scale(psize), false)
                })
            };
            if let Some((at, yaw, rot, scale, was_fitted)) = placement {
                let place = live::Action::Place { module: elem.module.clone(), pos: at, yaw, rot, scale, fitted: was_fitted };
                if ctx.apply(player, place).is_done() && was_fitted {
                    ctx.place.fitted = None;
                }
            }
        }

        if grabbed && (buttons.just_pressed(MouseButton::Right) || keys.just_pressed(KeyCode::KeyX)) {
            let piv32 = pivot.map(|c| c as f32);
            let scene_hit = ctx.world.0.scene.raycast_placed_owner(piv32, dir, reach);
            let sdf_hit = ctx.world.0.raycast_placed_sdf(piv32, dir, reach);
            let target_id = match (scene_hit, sdf_hit) {
                (Some((a, ta)), Some((b, tb))) => Some(if ta <= tb { a } else { b }),
                (Some((a, _)), None) => Some(a),
                (None, Some((b, _))) => Some(b),
                (None, None) => None,
            };
            match target_id {
                Some(id) => {
                    ctx.apply(player, live::Action::Remove { id });
                }
                None => player.say("aim at a placed element to remove it"),
            }
        }
    }

}

/// `K`: chops the placed object aimed at.
fn chop_input(ctx: &mut EditCtx<'_>, player: &mut Player, a: &Asking<'_>) {
    let Asking { keys, pivot, dir, psize, .. } = *a;
    if keys.just_pressed(KeyCode::KeyK) && !ctx.carry.is_carrying() {
        ctx.sculpt.active = false;
        ctx.sculpt.aim = None;
        ctx.place.active = false;
        ctx.place.aim = None;
        ctx.place.fitted = None;
        let reach = psize * 10.0;
        let piv32 = pivot.map(|c| c as f32);
        let scene_hit = ctx.world.0.scene.raycast_placed_surface_material(piv32, dir, reach);
        let sdf_hit = ctx.world.0.raycast_placed_sdf(piv32, dir, reach).map(|(id, t)| {
            let exact = {
                let mut combined = ctx.world.0.combined();
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
        match target_hit {
            Some((id, hit, material)) => {
                let tool = ctx.sculpt.tool;
                ctx.apply(player, live::Action::Cut { id, hit, dir, tool, material });
            }
            None => player.say("aim at a placed object within reach to chop (K)"),
        }
    }

}

/// `G`: picks up what is aimed at; while carrying, turns, tilts, lays flat, sets down or puts back.
fn carry_input(ctx: &mut EditCtx<'_>, player: &mut Player, a: &Asking<'_>) {
    let Asking { keys, buttons, scroll, grabbed, ctrl, pivot, dir, psize, .. } = *a;
    if ctx.carry.is_carrying() {
        let reach = (psize * 12.0).max(ctx.carry.held.as_ref().map_or(0.0, |h| h.physics.world_span * 2.5));
        let aim = {
            let mut combined = ctx.world.0.combined();
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
                ctx.drop_carried_at(target_aim, player);
            }
        } else if (grabbed && buttons.just_pressed(MouseButton::Right)) || keys.just_pressed(KeyCode::KeyX) {
            ctx.cancel_carry(player, true);
        }
    } else if keys.just_pressed(KeyCode::KeyG) {
        ctx.sculpt.active = false;
        ctx.sculpt.aim = None;
        ctx.place.active = false;
        ctx.place.aim = None;
        ctx.place.fitted = None;
        let reach = psize * 10.0;
        let piv32 = pivot.map(|c| c as f32);
        let scene_hit = ctx.world.0.scene.raycast_placed_surface(piv32, dir, reach).map(|(o, t, _)| (o, t))
            .or_else(|| ctx.world.0.scene.raycast_placed_owner(piv32, dir, reach));
        let sdf_hit = ctx.world.0.raycast_placed_sdf(piv32, dir, reach);
        let target_id = match (scene_hit, sdf_hit) {
            (Some((a, ta)), Some((b, tb))) => Some(if ta <= tb { a } else { b }),
            (Some((a, _)), None) => Some(a),
            (None, Some((b, _))) => Some(b),
            (None, None) => None,
        };
        match target_id {
            Some(id) => {
                if ctx.apply(player, live::Action::Grab { id }).is_done() {
                    // How it is shown in hand, and aimed: the world holds it.
                    let world = &ctx.world.0;
                    if let (Some(pl), Some(phys)) = (world.find(id), world.physics_of(id)) {
                        let orig_tf = pl.transform();
                        let merged = world.runtime.local_scene_of(id).and_then(|sc| scene::build_merged_mesh(sc, false, false));
                        let mesh = merged.map_or_else(|| ctx.carry_assets.fallback.clone(), |(m, _)| ctx.meshes.add(m));
                        ctx.carry.held = Some(CarriedItem {
                            name: world.element_name(id),
                            scale: pl.scale,
                            physics: phys,
                            speed_factor: check_carry(&phys, false, psize).speed_factor(),
                            yaw: player.body.yaw,
                            pitch: 0.0,
                            flat: CarriedItem::is_rot_horizontal(orig_tf.rot),
                            aim: None,
                            mesh,
                        });
                    }
                }
            }
            None => player.say("aim at a placed object within reach to grab (G)"),
        }
    }

}

/// Undo and redo; saving and loading the world.
fn history_input(ctx: &mut EditCtx<'_>, player: &mut Player, a: &Asking<'_>) {
    let Asking { keys, ctrl, tool_active, .. } = *a;
    if (tool_active || ctrl) && keys.just_pressed(KeyCode::KeyZ) {
        ctx.cancel_carry(player, false);
        ctx.apply(player, live::Action::Undo);
    }
    if (tool_active || ctrl) && keys.just_pressed(KeyCode::KeyY) {
        ctx.cancel_carry(player, false);
        ctx.apply(player, live::Action::Redo);
    }

    // Save / reload the whole world (areas, sculpting, water, rules, placed elements).
    if keys.just_pressed(KeyCode::F5) || (ctrl && keys.just_pressed(KeyCode::KeyS)) {
        ctx.cancel_carry(player, false);
        let path = ctx.sculpt.save_path.clone();
        let world = &mut ctx.world.0;
        world.finish_all_falls(Some(&mut player.body));
        let msg = match world.save(&path) {
            Ok(()) => format!("saved {} edits & {} elements to {}", world.edits.len(), world.placed.len(), path.display()),
            Err(e) => format!("save failed: {e:#}"),
        };
        println!("{msg}");
        player.say(msg);
    }
    if keys.just_pressed(KeyCode::F9) || (ctrl && keys.just_pressed(KeyCode::KeyL)) {
        ctx.carry.held = None;
        let path = ctx.sculpt.save_path.clone();
        let msg = match ctx.world.0.reload(&path) {
            Ok((edits_n, elems_n)) => format!("reloaded {edits_n} edits & {elems_n} elements from {}", path.display()),
            Err(e) => format!("reload failed: {e:#}"),
        };
        println!("{msg}");
        player.say(msg);
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
    ctx.world.0.tick_falling(time.delta_secs().min(0.1), &mut player.body);
    ctx.world.0.sync_viewer_size(player.body.size);

    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if grabbed && player.body.change.is_none() {
        player.body.yaw -= motion.delta.x * 0.0025;
        player.pitch = (player.pitch - motion.delta.y * 0.0025).clamp(-1.55, 1.55);
    }
    let ctrl = keys.pressed(KeyCode::ControlLeft) || keys.pressed(KeyCode::ControlRight);
    let held = |k: KeyCode| keys.pressed(k) as i32 as f32;
    let dt = time.delta_secs().min(0.1);

    // Cycle active material-aware tool (`N`: Workshop -> Saw -> Pickaxe -> Shovel -> Hands). `C` is sitting.
    if !ctrl && keys.just_pressed(KeyCode::KeyN) {
        ctx.sculpt.tool = ctx.sculpt.tool.next();
        player.say(format!("tool: {}", ctx.sculpt.tool.name()));
    }

    // Handle active boat/raft piloting & passive floating buoyancy.
    let piloting = ctx.world.0.piloting.is_some();
    let boat_fwd = if piloting && !ctrl { held(KeyCode::KeyW) - held(KeyCode::KeyS) } else { 0.0 };
    let boat_turn = if piloting { held(KeyCode::KeyA) - held(KeyCode::KeyD) } else { 0.0 };
    if piloting && keys.just_pressed(KeyCode::Space) {
        ctx.apply(&mut player, live::Action::Pilot { id: None });
    } else if piloting && (boat_fwd != 0.0 || boat_turn != 0.0) {
        ctx.apply(&mut player, live::Action::Steer { forward: boat_fwd, turn: boat_turn, dt });
    }
    ctx.world.0.animate(dt, &mut player.body);

    let carry_speed = ctx.carry.speed_factor();
    player.intent = if ctx.world.0.piloting.is_some() {
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
        let name = item.name.clone();
        if ctx.drop_carried_at(drop_pos, &mut player) {
            player.say(format!("{name} too heavy at this size — set down"));
        }
    }

    if keys.just_pressed(KeyCode::KeyB) {
        if ctx.carry.is_carrying() {
            ctx.cancel_carry(&mut player, false);
        }
        ctx.sculpt.active = !ctx.sculpt.active;
        ctx.sculpt.hold_timer = 0.0;
        ctx.sculpt.last_stamp = None;
        if ctx.sculpt.active {
            ctx.place.active = false;
            ctx.place.aim = None;
            ctx.place.fitted = None;
            player.say(format!("sculpt mode: {} / {}", ctx.sculpt.preset.name(), ctx.sculpt.material().name()));
        } else {
            ctx.sculpt.aim = None;
            player.say("sculpt mode off");
        }
    }

    if keys.just_pressed(KeyCode::KeyP) {
        if ctx.carry.is_carrying() {
            ctx.cancel_carry(&mut player, false);
        }
        ctx.place.active = !ctx.place.active;
        ctx.place.fitted = None;
        if ctx.place.active {
            ctx.sculpt.active = false;
            ctx.sculpt.aim = None;
            if let Some(el) = ctx.place.current() {
                let kind = if el.is_sdf() { "SDF" } else { "Scene" };
                player.say(format!("place mode: {} [{kind}] (O fits an opening)", el.name));
            } else {
                player.say("place mode: no .wasm elements found (run ./elements/build.sh)");
            }
        } else {
            ctx.place.aim = None;
            player.say("place mode off");
        }
    }

    if ctrl && keys.just_pressed(KeyCode::KeyR) {
        let n = ctx.place.rescan(&mut ctx.world.0);
        place::clear_preview_cache(&mut ctx.preview);
        player.say(format!("rescanned elements: {n} available"));
    }

    let tool_active = ctx.sculpt.active || ctx.place.active || ctx.carry.is_carrying();
    if keys.just_pressed(KeyCode::KeyF) || (!tool_active && buttons.just_pressed(MouseButton::Right)) {
        if ctx.world.0.piloting.is_some() {
            ctx.apply(&mut player, live::Action::Pilot { id: None });
        } else if let Some(boat_id) = ctx.world.0.scene.standing_on_placed(player.body.feet, player.body.size)
            && ctx.world.0.floating.contains_key(&boat_id)
        {
            if ctx.apply(&mut player, live::Action::Pilot { id: Some(boat_id) }).is_done() {
                ctx.sculpt.active = false;
                ctx.sculpt.aim = None;
                ctx.place.active = false;
                ctx.place.aim = None;
            }
        } else {
            let at = player.body.feet.map(|c| c as f32);
            if ctx.apply(&mut player, live::Action::Interact { at }).is_done() {
                ctx.live.interact_pressed = true;
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
    let asking = Asking { keys: &keys, buttons: &buttons, scroll: &scroll, time: &time, grabbed, ctrl, pivot, dir, psize, tool_active };

    measure_site(&mut ctx, &mut player, &asking);
    sculpt_input(&mut ctx, &mut player, &asking);
    place_input(&mut ctx, &mut player, &asking);
    chop_input(&mut ctx, &mut player, &asking);
    carry_input(&mut ctx, &mut player, &asking);
    history_input(&mut ctx, &mut player, &asking);
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
        let mut combined = ctx.world.0.combined();
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
            let mut combined = ctx.world.0.combined();
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
fn step(time: Res<Time>, mut player: ResMut<Player>, mut live: ResMut<LiveScene>, mut world: NonSendMut<Live>) {
    let world = &mut world.0;
    let dt = time.delta_secs().min(0.1);
    live.elapsed += dt;

    let eye64 = player.body.eye();
    let eye32 = eye64.map(|c| c as f32);
    let look_dir = player.looking().to_array();
    let size = player.body.size;
    let (hit_owner, hit_id) = world.scene.raycast_dynamic_owned(eye32, look_dir, size * 12.0).map_or((BASE_OWNER, 0), |(o, id, _)| (o, id));
    let aim = {
        let mut combined = world.combined();
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
        for eff in world.tick(dt, &ps_world, hit_owner, hit_id) {
            apply_side_effects(&mut player, &eff);
        }
    }
    if std::mem::take(&mut live.interact_pressed) {
        for eff in world.interact(&ps_world, hit_owner, hit_id) {
            apply_side_effects(&mut player, &eff);
        }
    }

    player.owed = (player.owed + time.delta_secs()).min(0.25);
    let intent = player.intent;
    while player.owed >= 1.0 / HZ {
        player.owed -= 1.0 / HZ;
        player.body.step(&mut world.combined(), intent, 1.0 / HZ).expect("the area asked");
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

fn frame(time: Res<Time>, mut player: ResMut<Player>, mut world: NonSendMut<Live>) {
    let want = if player.third { BEHIND } else { 0.0 };
    let size = player.body.size;
    let per = (size / DRAWN_AS) as f64;
    let eye = player.eye.unwrap_or(player.body.eye());
    let mut combined = world.0.combined();
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
    player: Res<Player>,
    mut world: NonSendMut<Live>,
    mut transforms: Placed,
    mut static_q: Query<
        (&scene::StaticSceneMesh, &mut Transform),
        (
            With<scene::SceneNode>,
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
    let world = &mut world.0;
    let mut open = |e: [f64; 3], s: f32| rings::open(&mut world.region, e, s).expect("the area asked");
    let here = open(eye, size);
    let ahead: Vec<_> = ahead.into_iter().map(|(e, s)| (e, s, open(e, s))).collect();
    let scale = (DRAWN_AS / size) as f64;
    chunks.materials.sync_eye(&mut surface_materials, eye, scale);
    chunks.update(eye, look_dir, size, scale, here, &ahead, time.delta_secs(), &mut commands, &mut meshes);
    chunks.place(eye, scale, &mut transforms);
    scene::sync(&mut commands, &mut visuals, &world.scene, eye, scale, &mut static_q, &mut dynamic_q, &mut light_q);
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
    world: NonSend<Live>,
) {
    let world = &world.0;
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
    let mods_note = if !world.edits.is_empty() || !world.placed.is_empty() || world.dirty {
        format!("  /  {} edits, {} elements{}", world.edits.len(), world.placed.len(), if world.dirty { "*" } else { "" })
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
        "{}  /  {doing}  /  tool: {} (N){tile_note}{mods_note}\n{fps:.0} frames a second  /  {} chunks shown, {} being made{holes}, {} ahead  /  {}k triangles ({}k scene)\nat {:.3}, {:.3}, {:.3}",
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
    if let Some(boat_id) = world.piloting {
        let name = world.element_name(boat_id);
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
            if let Some(snap) = place.fitted.as_ref().filter(|s| s.module == el.module) {
                s += &format!(
                    "\nPLACE: {} [FITTED {:.3}x]  /  {}  (LMB place, O re-fit, R/T or Wheel free)",
                    el.name, snap.transform.scale, snap.summary
                );
            } else {
                let sc = place.placement_scale(b.size);
                let deg = (place.yaw.to_degrees().rem_euclid(360.0)).round() as i32;
                s += &format!(
                    "\nPLACE: {} [{kind}]  /  scale {sc:.2}x  /  yaw {deg}°{hint}  (LMB place, O fit opening, RMB/X remove, Tab next, R/T rotate, Wheel scale, Ctrl+R rescan)",
                    el.name
                );
            }
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
    mut world: NonSendMut<Live>,
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
            let mut combined = world.0.combined();
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
