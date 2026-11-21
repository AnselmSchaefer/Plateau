//! Plateau, played (POC_PLAN.md, step 2): one area, walked at three sizes, the player seen from
//! behind (`figure`) or through their own eyes.
//!
//! `cargo run --release -p plateau-play [-- area.wasm] [--seed N] [--tour folder [--behind]]`; the walled
//! garden (`areas/modules/garden.wasm`) unless another module is given. W A S D to walk, Shift to
//! run, Space to jump, the mouse to look (click to take it, Esc to let it go); 1, 2 and 3 to become
//! tiny, a person or a giant, Q and E to shrink and grow a size; C to sit or stand; V to see from
//! behind or through the eyes; F12 for a screenshot (into `target/shots/`).
//!
//! `--tour folder` goes instead to fixed places at each size, waits for the chunks there, measures
//! the frames a second, takes a screenshot of each into the folder, writes what it measured to
//! `tour.txt` there, and exits (vsync off, to see how many frames it could). It sees through the
//! eyes, as it did when its numbers were first written, unless `--behind` is given too.
//!
//! The engine does the rest (`plateau_engine`): the body walks at its size (`body`), the surface is
//! made in chunks off the frame (`chunks`). The world is drawn about the player's eye and scaled so
//! the player is always the same height on the screen's side of things: shadows, the near plane and
//! the haze are made for a person, and hold at every size.

mod chunks;
mod figure;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::{CascadeShadowConfigBuilder, GlobalAmbientLight};
use bevy::pbr::{DistanceFog, FogFalloff};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{CursorGrabMode, CursorOptions, PresentMode, PrimaryWindow};
use chunks::Chunks;
use plateau_engine::body::{self, Body, Intent};
use plateau_engine::{Compiled, Limits, Region, SIZES, START, Wasm};

/// The physics' steps a second.
const HZ: f32 = 120.0;
/// A person's height: the world is drawn scaled so the player is always this tall.
const DRAWN_AS: f32 = 1.7;
/// How far ahead, in seconds, the chunks where the player is going are made.
const AHEAD: f32 = 0.75;
/// Seen from behind: the camera turns about a point this far above the eye (over the head, so the
/// figure is below the middle of the view), this far behind it at most, and no nearer to what is in
/// the way than `MARGIN`; in the renderer's units, a person's metres.
const PIVOT: f32 = 0.35;
const BEHIND: f32 = 3.4;
const MARGIN: f32 = 0.25;
const SKY: Color = Color::srgb(0.62, 0.72, 0.84);

fn main() {
    let here = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut args = std::env::args().skip(1);
    let (mut module, mut seed, mut tour, mut behind) = (here.join("areas/modules/garden.wasm"), 7u64, None, false);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).expect("--seed takes a number"),
            "--tour" => tour = Some(PathBuf::from(args.next().expect("--tour takes a folder"))),
            "--behind" => behind = true,
            _ => module = PathBuf::from(arg),
        }
    }
    let bytes = std::fs::read(&module).unwrap_or_else(|e| panic!("{}: {e} (areas/build.sh builds the areas)", module.display()));
    let compiled = Arc::new(Compiled::new(&bytes, Limits::default()).unwrap_or_else(|e| panic!("{}: {e}", module.display())));
    let mut physics = compiled.instantiate(seed).expect("the area made");
    let described = physics.describe().expect("the area described");
    let body = Body::standing(&mut physics, [START[0], 40.0, START[1]], SIZES[1].1).expect("the area asked");

    let present_mode = if tour.is_some() || std::env::var_os("PLATEAU_NO_VSYNC").is_some() { PresentMode::AutoNoVsync } else { PresentMode::AutoVsync };
    let window = Window { title: format!("Plateau: {}", described.name), present_mode, resolution: (1600, 900).into(), ..default() };
    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }))
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .insert_resource(ClearColor(SKY))
        .insert_resource(GlobalAmbientLight { brightness: 500.0, ..default() })
        .insert_resource(Area { compiled, seed, places: described.places.iter().map(|p| (p.name.clone(), p.size, p.at.map(|c| c as f64))).collect() })
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
        })
        .insert_resource(figure::Gait::default())
        .insert_non_send(Physics(physics))
        .add_systems(Startup, setup)
        .add_systems(Update, (look_and_ask, step, frame, draw, figure::animate, hud).chain());
    if let Some(folder) = tour {
        std::fs::create_dir_all(&folder).expect("the tour's folder");
        app.insert_resource(Tour::new(folder)).add_systems(Update, go_on_tour.before(step));
    } else {
        app.add_systems(Update, (grab_the_mouse, screenshot));
    }
    app.run();
}

/// The area: its module, to make an instance for each thread, and the places it describes.
#[derive(Resource)]
struct Area {
    compiled: Arc<Compiled>,
    seed: u64,
    places: Vec<(String, plateau_engine::Size, [f64; 3])>,
}

/// The instance the player's body is moved through.
struct Physics(Wasm);

#[derive(Resource)]
struct Player {
    body: Body,
    pitch: f32,
    /// Where the eye is drawn from: the body's, its height eased, so a step up is not a jump.
    eye: Option<[f64; 3]>,
    /// The point kept in view while growing or shrinking.
    focus: Option<[f64; 3]>,
    /// Something to tell the player, and until when.
    said: Option<(String, Instant)>,
    intent: Intent,
    /// Time not yet stepped.
    owed: f32,
    /// Seen from behind, not through the eyes.
    third: bool,
    /// How far behind the camera is, in the renderer's units: `BEHIND`, or nearer where something
    /// is in the way.
    back: f32,
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

fn setup(mut commands: Commands, area: Res<Area>, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let material = materials.add(StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.92, reflectance: 0.3, ..default() });
    commands.insert_resource(Chunks::new(area.compiled.clone(), area.seed, material));
    figure::spawn(&mut commands, &mut meshes, &mut materials);
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
        Text::new("W A S D walk  /  Shift run  /  Space jump  /  mouse look (click; Esc frees it)\n1 2 3 tiny, person, giant  /  Q E shrink, grow  /  C sit or stand  /  V from behind or through the eyes  /  F12 screenshot"),
        text(15.0),
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

/// The mouse turns the view; the keys say where to go, and change size or sit.
fn look_and_ask(
    keys: Res<ButtonInput<KeyCode>>,
    motion: Res<AccumulatedMouseMotion>,
    cursor: Query<&CursorOptions, With<PrimaryWindow>>,
    tour: Option<Res<Tour>>,
    mut player: ResMut<Player>,
    mut physics: NonSendMut<Physics>,
) {
    if tour.is_some() {
        return;
    }
    let grabbed = cursor.single().is_ok_and(|c| c.grab_mode != CursorGrabMode::None);
    if grabbed && player.body.change.is_none() {
        player.body.yaw -= motion.delta.x * 0.0025;
        player.pitch = (player.pitch - motion.delta.y * 0.0025).clamp(-1.55, 1.55);
    }
    let held = |k: KeyCode| keys.pressed(k) as i32 as f32;
    player.intent = Intent {
        forward: held(KeyCode::KeyW) - held(KeyCode::KeyS),
        right: held(KeyCode::KeyD) - held(KeyCode::KeyA),
        run: keys.pressed(KeyCode::ShiftLeft) || keys.pressed(KeyCode::ShiftRight),
        jump: keys.pressed(KeyCode::Space),
    };
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
        change_size(&mut player, &mut physics.0, to);
    }
    if keys.just_pressed(KeyCode::KeyV) {
        player.third = !player.third;
    }
    if keys.just_pressed(KeyCode::KeyC) {
        if player.body.seat.is_some() {
            let stood = player.body.seat.unwrap().stood;
            player.body.feet = stood;
            player.body.seat = None;
        } else if !player.body.sit(&mut physics.0).expect("the area asked") {
            player.say("nothing to sit on here");
        }
    }
}

/// Starts growing or shrinking to `to`, keeping in view the point looked at.
fn change_size(player: &mut Player, area: &mut Wasm, to: f32) {
    if to == player.body.size {
        return;
    }
    let (eye, size) = (player.body.eye(), player.body.size);
    let way = player.looking();
    player.focus = body::look(area, eye, way.to_array(), size, size * 300.0).expect("the area asked");
    player.body.grow_to(to);
}

/// The body moved on in steps of the physics, as much as the frame took.
fn step(time: Res<Time>, mut player: ResMut<Player>, mut physics: NonSendMut<Physics>) {
    player.owed = (player.owed + time.delta_secs()).min(0.25);
    let intent = player.intent;
    while player.owed >= 1.0 / HZ {
        player.owed -= 1.0 / HZ;
        player.body.step(&mut physics.0, intent, 1.0 / HZ).expect("the area asked");
        // Growing or shrinking, the point looked at is kept in the middle of the view.
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
    // The eye's height eased towards the body's, quickly, unless far behind.
    let target = player.body.eye();
    let k = 1.0 - (-25.0 * time.delta_secs() as f64).exp();
    player.eye = Some(match player.eye {
        Some(e) if (e[1] - target[1]).abs() < player.body.size as f64 * 0.5 => [target[0], e[1] + (target[1] - e[1]) * k, target[2]],
        _ => target,
    });
}

/// How far behind the camera goes, seen from behind: `BEHIND`, or as near as what is in the way
/// makes it (a ray from the point it turns about, back along the view, as the player's size sees the
/// area), at once if nearer, slowly back out.
fn frame(time: Res<Time>, mut player: ResMut<Player>, mut physics: NonSendMut<Physics>) {
    let want = if player.third { BEHIND } else { 0.0 };
    let size = player.body.size;
    // Metres of the area to a unit of the renderer.
    let per = (size / DRAWN_AS) as f64;
    let eye = player.eye.unwrap_or(player.body.eye());
    let pivot = [eye[0], eye[1] + PIVOT as f64 * per, eye[2]];
    let away = -player.looking();
    let room = if want > 0.0 {
        match body::look(&mut physics.0, pivot, away.to_array(), size, want * size / DRAWN_AS).expect("the area asked") {
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

/// Where the chunks are drawn, and whether: everything but the camera and the sun.
type Placed<'w, 's> = Query<'w, 's, (&'static mut Transform, &'static mut Visibility), (Without<Camera3d>, Without<Sun>)>;

/// The chunks asked for and placed about the eye, the camera turned as the player looks.
fn draw(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut chunks: ResMut<Chunks>,
    player: Res<Player>,
    mut transforms: Placed,
    mut camera: Query<(&mut Transform, &mut DistanceFog), With<Camera3d>>,
) {
    let size = player.body.size;
    let eye = player.eye.unwrap_or(player.body.eye());
    // Ahead: where the player will be in a little under a second, and the size they are growing or
    // shrinking to, here and there.
    let v = player.body.velocity;
    let there = [0, 1, 2].map(|a| eye[a] + (v[a] * AHEAD) as f64);
    let mut ahead = vec![(there, size)];
    if let Some(change) = player.body.change {
        ahead.extend([(eye, change.to), (there, change.to)]);
    }
    let started = Instant::now();
    chunks.update(eye, size, &ahead, &mut commands, &mut meshes);
    chunks.place(eye, (DRAWN_AS / size) as f64, &mut transforms);
    chunks.counts.longest = chunks.counts.longest.max(started.elapsed());
    if let Ok((mut camera, mut fog)) = camera.single_mut() {
        camera.rotation = look(player.body.yaw, player.pitch);
        camera.translation = if player.third { Vec3::Y * PIVOT - player.looking() * player.back } else { Vec3::ZERO };
        // Haze is the air's, in metres, not the player's: a few hundred metres, a little more for a
        // giant, whose eyes are high above it.
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

fn hud(mut text: Query<&mut Text, With<Hud>>, player: Res<Player>, chunks: Res<Chunks>, diagnostics: Res<DiagnosticsStore>, area: Res<Area>) {
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
    let mut s = format!(
        "{}  /  {doing}\n{fps:.0} frames a second  /  {} chunks shown, {} being made{holes}, {} ahead  /  {}k triangles\nat {:.3}, {:.3}, {:.3}",
        size_name(b.size),
        c.shown,
        c.waiting,
        c.ahead,
        c.triangles / 1000,
        feet[0],
        feet[1],
        feet[2]
    );
    // The area's places, when at one: within twice the smaller of the player and whom it is for.
    for (name, size, at) in &area.places {
        let far = ((at[0] - feet[0]).powi(2) + (at[1] - feet[1]).powi(2) + (at[2] - feet[2]).powi(2)).sqrt();
        if far < 2.0 * b.size.min(size.height()) as f64 {
            s += &format!("\nhere: {name}, a place for the {}", size.name());
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

/// A place the tour stops at: who, where (across the ground; dropped from a little above the
/// ground there), facing which way, whether to sit, a size to grow or shrink to there, and how
/// long to go forward, measuring, and whether running.
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
    /// To go to the next stop.
    Arrive,
    /// There since then, the chunks being made.
    Wait(Instant),
    /// The chunks all made, after this long and this many: the frames measured, how many had
    /// wanted chunks with nothing shown in their space, and the most such in a frame.
    Measure { arrived: Instant, settled: Instant, made: usize, took: (Duration, Duration), frames: Vec<f32>, holey: usize, most: usize },
    /// The screenshot asked for: frames left until it is surely written.
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
            // Shrinking, the crack looked at stays in the middle of the view.
            Stop { name: "person-shrunk-at-the-crack", size: person, at: [53.4, 30.0], yaw: -FRAC_PI_2, pitch: -1.38, sit: false, then: Some(tiny), walk: None },
            Stop { name: "tiny-at-the-crack", size: tiny, at: [53.62, 30.0], yaw: -FRAC_PI_2, pitch: 0.05, sit: false, then: None, walk: None },
            Stop { name: "tiny-in-the-cave", size: tiny, at: [54.0, 30.25], yaw: 0.35, pitch: 0.0, sit: false, then: None, walk: None },
            Stop { name: "tiny-on-the-pond", size: tiny, at: [24.37, 36.82], yaw: 0.6, pitch: -0.08, sit: false, then: None, walk: None },
            Stop { name: "giant-over-the-garden", size: giant, at: [30.0, 62.0], yaw: 0.0, pitch: -0.45, sit: false, then: None, walk: None },
            Stop { name: "giant-on-the-bench", size: giant, at: [40.0, 23.5], yaw: PI, pitch: -0.4, sit: true, then: None, walk: None },
            // Going, measured as they go: in at the gate, along the wall's foot past the crack,
            // across the garden.
            Stop { name: "person-running-in-at-the-gate", size: person, at: START, yaw: 0.0, pitch: -0.1, sit: false, then: None, walk: Some((4.0, true)) },
            Stop { name: "tiny-running-along-the-wall", size: tiny, at: [53.62, 30.12], yaw: 0.0, pitch: -0.05, sit: false, then: None, walk: Some((4.0, true)) },
            Stop { name: "giant-walking-across-the-garden", size: giant, at: [20.0, 62.0], yaw: 0.3, pitch: -0.4, sit: false, then: None, walk: Some((3.0, false)) },
        ];
        Tour { folder, stops, next: 0, phase: Phase::Arrive, written: String::new() }
    }
}

/// The tour: each stop in turn, measured once its chunks are made, and a screenshot of it.
fn go_on_tour(
    mut commands: Commands,
    mut tour: ResMut<Tour>,
    mut player: ResMut<Player>,
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
            let mut body = Body::standing(&mut physics.0, [stop.at[0], 0.1 + stop.size as f64 * 0.5, stop.at[1]], stop.size).expect("the area asked");
            body.yaw = stop.yaw;
            if stop.sit && !body.sit(&mut physics.0).expect("the area asked") {
                tour.written += &format!("{}: found nothing to sit on\n", stop.name);
            }
            (player.body, player.pitch, player.eye, player.focus, player.intent) = (body, stop.pitch, None, None, Intent::default());
            if let Some(to) = stop.then {
                change_size(&mut player, &mut physics.0, to);
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
            // A second for the last chunks to be drawn, then two seconds measured, or the walk.
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
