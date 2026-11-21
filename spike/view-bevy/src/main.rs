//! The garden in Bevy (POC_PLAN.md, step 0): its surface from the area's module, lit by the sun,
//! with a camera to fly. W A S D to move, Space and Shift up and down, the mouse held to look.
//! The frames a second go to the log every second; `PLATEAU_NO_VSYNC=1` to see how many it could.

use bevy::asset::RenderAssetUsages;
use bevy::diagnostic::{FrameTimeDiagnosticsPlugin, LogDiagnosticsPlugin};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::light::GlobalAmbientLight;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use bevy::window::PresentMode;

fn main() {
    let present_mode = if std::env::var_os("PLATEAU_NO_VSYNC").is_some() { PresentMode::AutoNoVsync } else { PresentMode::AutoVsync };
    let window = Window { title: "Plateau spike: Bevy".into(), present_mode, ..default() };
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin { primary_window: Some(window), ..default() }))
        .add_plugins((FrameTimeDiagnosticsPlugin::default(), LogDiagnosticsPlugin::default()))
        .insert_resource(ClearColor(Color::srgb(0.55, 0.70, 0.90)))
        .insert_resource(GlobalAmbientLight { brightness: 400.0, ..default() })
        .add_systems(Startup, setup)
        .add_systems(Update, fly)
        .run();
}

#[derive(Component)]
struct Fly {
    yaw: f32,
    pitch: f32,
}

fn setup(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    let garden = spike_host::garden_mesh().expect("the garden's module");
    let colours: Vec<[f32; 4]> = garden.materials.iter().map(|&m| {
        let [r, g, b] = spike_host::colour(m);
        [r, g, b, 1.0]
    }).collect();
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, garden.positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, garden.normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_indices(Indices::U32(garden.indices));
    let material = StandardMaterial { base_color: Color::WHITE, perceptual_roughness: 0.9, ..default() };
    commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(materials.add(material))));
    commands.spawn((
        DirectionalLight { illuminance: 8000.0, shadow_maps_enabled: true, ..default() },
        Transform::from_xyz(10.0, 30.0, 50.0).looking_at(Vec3::new(32.0, 0.0, 32.0), Vec3::Y),
    ));
    let (yaw, pitch) = (0.7f32, -0.3f32);
    commands.spawn((Camera3d::default(), Transform::from_xyz(56.0, 9.0, 60.0).with_rotation(look(yaw, pitch)), Fly { yaw, pitch }));
}

fn look(yaw: f32, pitch: f32) -> Quat {
    Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0)
}

fn fly(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut camera: Query<(&mut Transform, &mut Fly)>,
) {
    let Ok((mut transform, mut fly)) = camera.single_mut() else { return };
    if buttons.pressed(MouseButton::Left) {
        fly.yaw -= motion.delta.x * 0.003;
        fly.pitch = (fly.pitch - motion.delta.y * 0.003).clamp(-1.5, 1.5);
        transform.rotation = look(fly.yaw, fly.pitch);
    }
    let mut go = Vec3::ZERO;
    let (forward, right) = (*transform.forward(), *transform.right());
    for (key, way) in [(KeyCode::KeyW, forward), (KeyCode::KeyS, -forward), (KeyCode::KeyD, right), (KeyCode::KeyA, -right),
        (KeyCode::Space, Vec3::Y), (KeyCode::ShiftLeft, -Vec3::Y)]
    {
        if keys.pressed(key) {
            go += way;
        }
    }
    transform.translation += go.normalize_or_zero() * 8.0 * time.delta_secs();
}
