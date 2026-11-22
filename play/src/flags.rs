//! Flags over the places for the tiny, to find them by: what is meant for the tiny is too small to
//! see from any distance, while a giant's place (a seat 4.5 m up) is plain to see. A flag is a pole
//! with a pennant in the tiny's colour (orange, as on the reach maps), standing on whatever is
//! highest above the place. So the cave inside the garden's wall has its flag on the wall's top,
//! straight over the crack, seen from across the garden.
//!
//! A flag is drawn the same size on the screen whatever the player's size (as the figure is), and
//! turns its pennant to face them. M hides or shows them.

use bevy::prelude::*;
use plateau_engine::picture::COLOURS;
use plateau_engine::{Region, Size, body};

use crate::{DRAWN_AS, Player};

/// How tall a flag is drawn, in the renderer's units: a little taller than the player.
const TALL: f32 = 2.4;

/// A flag: where its foot stands, in the area's metres.
#[derive(Component)]
pub struct Flag {
    foot: [f64; 3],
}

/// Whether a place for `size` has a flag: those for the tiny.
pub fn flagged(size: Size) -> bool {
    size == Size::Tiny
}

/// Where a place's flag stands: on the highest surface over it, as a person sees the area, or at
/// the place itself if nothing is found.
pub fn foot(area: &mut dyn Region, at: [f64; 3]) -> [f64; 3] {
    let from = [at[0], at[1].max(0.0) + 60.0, at[2]];
    match body::look(area, from, [0.0, -1.0, 0.0], 1.7, 200.0) {
        Ok(Some(hit)) => hit,
        _ => at,
    }
}

/// The flags made, one for each place: its size and where its foot stands.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>, places: &[(Size, [f64; 3])]) {
    let pole = meshes.add(Cylinder::new(0.025, TALL));
    let pennant = meshes.add(Cuboid::new(0.7, 0.42, 0.015));
    let white = materials.add(StandardMaterial { base_color: Color::srgb(0.92, 0.92, 0.9), ..default() });
    for &(size, foot) in places {
        let [r, g, b] = COLOURS[size as usize].map(|c| c as f32 / 255.0);
        let colour = materials.add(StandardMaterial {
            base_color: Color::srgb(r, g, b),
            // A little light of its own, so it shows in shadow and far off.
            emissive: LinearRgba::rgb(r * 0.6, g * 0.6, b * 0.6),
            double_sided: true,
            cull_mode: None,
            ..default()
        });
        commands.spawn((Flag { foot }, Transform::default(), Visibility::Hidden)).with_children(|flag| {
            flag.spawn((Mesh3d(pole.clone()), MeshMaterial3d(white.clone()), Transform::from_xyz(0.0, TALL * 0.5, 0.0)));
            flag.spawn((Mesh3d(pennant.clone()), MeshMaterial3d(colour), Transform::from_xyz(0.37, TALL - 0.23, 0.0)));
        });
    }
}

/// The flags placed about the eye the world is drawn from, facing the player; shown unless hidden.
pub fn place(player: Res<Player>, mut flags: Query<(&Flag, &mut Transform, &mut Visibility)>) {
    let size = player.body.size;
    let eye = player.eye.unwrap_or(player.body.eye());
    let scale = (DRAWN_AS / size) as f64;
    for (flag, mut t, mut visibility) in &mut flags {
        visibility.set_if_neq(if player.flags { Visibility::Visible } else { Visibility::Hidden });
        let f = flag.foot;
        t.translation = Vec3::new(((f[0] - eye[0]) * scale) as f32, ((f[1] - eye[1]) * scale) as f32, ((f[2] - eye[2]) * scale) as f32);
        t.rotation = Quat::from_rotation_y(player.body.yaw);
    }
}

/// Which way a point lies from another across the ground, in words: north is -z.
pub fn compass(from: [f64; 3], to: [f64; 3]) -> &'static str {
    let (dx, dz) = (to[0] - from[0], to[2] - from[2]);
    // Clockwise from north, in eighths of a turn.
    let turn = dx.atan2(-dz).to_degrees().rem_euclid(360.0);
    ["north", "north-east", "east", "south-east", "south", "south-west", "west", "north-west"][((turn + 22.5) / 45.0) as usize % 8]
}
