//! Interactive carrying, rotating/tilting, drop preview ghost, and felled-object topple animation
//! in `plateau-play` (Step 8c; AGENTS.md §1.3).
//!
//! Controls:
//! - **`K`**: chop/cut aimed un-promoted `Scene` element (splits standing trees into stump + felled log,
//!   or trims a felled log's crown cleanly perpendicular to its trunk)
//! - **`G`**: grab/carry aimed un-promoted `Scene` element (aware of `player.size` weight & rooting);
//!   press **`G`** or **`LMB`** while carrying to place it down as solid walkable geometry
//! - **`H`**: one-tap flip **upright ↔ horizontal** (lay a felled tree flat as a bridge)
//! - **`R` / `T`**: rotate placement yaw (-15° / +15°)
//! - **Mouse wheel** or **`[` / `]`**: tilt pitch (-10° / +10°) to bridge uneven banks or make ramps
//! - **`RMB` or `X`**: cancel carry and return object to where it was picked up

use bevy::prelude::*;
use plateau_engine::compose::{PlacementTransform, euler_quat, quat_rotate};
use plateau_engine::cut::{ObjectPhysics, placement_pos_for_aim};

use crate::surface::SurfaceMaterial;

/// Marker component for the opaque in-hand carried object mesh.
#[derive(Component)]
pub struct HeldVisual;

/// Marker component for the translucent 1:1 drop-location preview ghost while carrying.
#[derive(Component)]
pub struct CarryGhost;

/// Active object currently carried by the player.
pub struct CarriedItem {
    pub name: String,
    pub scale: f32,
    pub physics: ObjectPhysics,
    pub speed_factor: f32,
    pub yaw: f32,
    pub pitch: f32,
    pub flat: bool,
    pub aim: Option<[f64; 3]>,
    pub mesh: Handle<Mesh>,
}

impl CarriedItem {
    /// Current target 3D unit quaternion for placing the carried object.
    pub fn target_rot(&self) -> [f32; 4] {
        let base_pitch = if self.flat { std::f32::consts::FRAC_PI_2 } else { 0.0 };
        euler_quat(self.yaw, base_pitch + self.pitch, 0.0)
    }

    /// Current target world `PlacementTransform` when aimed at `aim_world`.
    pub fn target_transform(&self, aim_world: [f64; 3]) -> PlacementTransform {
        let rot = self.target_rot();
        let aim32 = [aim_world[0] as f32, aim_world[1] as f32, aim_world[2] as f32];
        let pos = placement_pos_for_aim(&self.physics, aim32, rot, self.scale);
        PlacementTransform::with_rot(pos, rot, self.scale)
    }

    /// Detects whether `rot` is already roughly horizontal (local +Y tipped sideways).
    pub fn is_rot_horizontal(rot: [f32; 4]) -> bool {
        let up = quat_rotate(rot, [0.0, 1.0, 0.0]);
        up[1].abs() < 0.5
    }
}

/// The carrying the player is doing: the item in hand, as it is shown and aimed. Where things fall,
/// float, and are piloted is the world's (`plateau_engine::live::World`).
#[derive(Resource, Default)]
pub struct CarryState {
    pub held: Option<CarriedItem>,
}

impl CarryState {
    pub fn is_carrying(&self) -> bool {
        self.held.is_some()
    }

    pub fn speed_factor(&self) -> f32 {
        self.held.as_ref().map_or(1.0, |h| h.speed_factor)
    }
}

#[derive(Resource)]
pub struct CarryAssets {
    pub fallback: Handle<Mesh>,
}

pub fn spawn_visuals(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    surface_mat: Handle<SurfaceMaterial>,
) {
    let fallback = meshes.add(Cuboid::new(0.4, 0.4, 0.4));
    let ghost_mat = materials.add(StandardMaterial {
        base_color: Color::srgba(0.95, 0.80, 0.38, 0.38),
        emissive: LinearRgba::rgb(0.32, 0.24, 0.08),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(fallback.clone()),
        MeshMaterial3d(surface_mat),
        Transform::default(),
        Visibility::Hidden,
        HeldVisual,
    ));
    commands.spawn((
        Mesh3d(fallback.clone()),
        MeshMaterial3d(ghost_mat),
        Transform::default(),
        Visibility::Hidden,
        CarryGhost,
    ));
    commands.insert_resource(CarryAssets { fallback });
}

/// Positions the in-hand carried mesh and the 1:1 translucent landing ghost each frame.
#[allow(clippy::type_complexity)]
pub fn sync_visuals(
    carry: Res<CarryState>,
    player: Res<crate::Player>,
    mut held_q: Query<(&mut Transform, &mut Visibility, &mut Mesh3d), (With<HeldVisual>, Without<CarryGhost>)>,
    mut ghost_q: Query<(&mut Transform, &mut Visibility, &mut Mesh3d), (With<CarryGhost>, Without<HeldVisual>)>,
) {
    let Ok((mut held_tf, mut held_vis, mut held_mesh)) = held_q.single_mut() else { return };
    let Ok((mut ghost_tf, mut ghost_vis, mut ghost_mesh)) = ghost_q.single_mut() else { return };

    let Some(item) = &carry.held else {
        held_vis.set_if_neq(Visibility::Hidden);
        ghost_vis.set_if_neq(Visibility::Hidden);
        return;
    };

    let player_size = player.body.size;
    let eye = player.eye.unwrap_or(player.body.eye());
    let draw_scale = (crate::DRAWN_AS / player_size) as f64;
    let target_rot = item.target_rot();
    let q_rot = Quat::from_array(target_rot).normalize();

    // 1. In-hand preview mesh positioned cleanly at lower-right of view without blocking crosshair.
    if held_mesh.0 != item.mesh {
        held_mesh.0 = item.mesh.clone();
    }
    let world_r = item.physics.world_radius(item.scale).max(1e-4);
    let compact = (player_size * 0.34 / world_r).min(1.0);
    let hand_scale = item.scale * compact * draw_scale as f32;
    let cam_rot = crate::look(player.body.yaw, player.pitch);
    let fwd = cam_rot * Vec3::NEG_Z;
    let right = cam_rot * Vec3::X;
    let up = cam_rot * Vec3::Y;
    let anchor = if player.third {
        Vec3::Y * (player.lift - 0.55) + fwd * 1.05 + right * 0.62
    } else {
        fwd * 1.15 + right * 0.58 - up * 0.42
    };
    let lc = Vec3::from_array(item.physics.local_center) * hand_scale;
    held_tf.translation = anchor - q_rot * lc;
    held_tf.rotation = q_rot;
    held_tf.scale = Vec3::splat(hand_scale);
    held_vis.set_if_neq(Visibility::Visible);

    // 2. Full 1:1 translucent landing ghost at aimed world point.
    if let Some(aim) = item.aim {
        if ghost_mesh.0 != item.mesh {
            ghost_mesh.0 = item.mesh.clone();
        }
        let tf = item.target_transform(aim);
        ghost_tf.translation = Vec3::new(
            ((tf.pos[0] as f64 - eye[0]) * draw_scale) as f32,
            ((tf.pos[1] as f64 - eye[1]) * draw_scale) as f32,
            ((tf.pos[2] as f64 - eye[2]) * draw_scale) as f32,
        );
        ghost_tf.rotation = q_rot;
        ghost_tf.scale = Vec3::splat(tf.scale * draw_scale as f32);
        ghost_vis.set_if_neq(Visibility::Visible);
    } else {
        ghost_vis.set_if_neq(Visibility::Hidden);
    }
}

