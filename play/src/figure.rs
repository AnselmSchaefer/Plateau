//! The player, seen: a figure of a person drawn where the body is, walking, running, jumping,
//! falling, sitting and standing as the body does, and turning to face the way it goes.
//!
//! It is made once, a person's 1.7 m tall, and never scaled: the world is drawn scaled so the
//! player is always that tall (`DRAWN_AS`), so the same figure is the tiny, a person and a giant.
//! Its parts hang from joints (hips, chest, neck, shoulders, elbows, hips, knees), each turned every
//! frame from the body's motion alone: how fast it goes across the ground in its own heights a
//! second (walking at `WALK`, running at `RUN`), whether it is on the ground, whether it sits.

use std::f32::consts::{PI, TAU};

use bevy::prelude::*;
use plateau_engine::body::{RUN, WALK};

use crate::{DRAWN_AS, Player};

/// The figure's root: its feet, on the ground, facing -z.
#[derive(Component)]
pub struct Figure;

#[derive(Clone, Copy, PartialEq)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    /// Which way is outward along x for this side, the figure facing -z (its right is +x).
    fn x(self) -> f32 {
        if self == Side::Left { -1.0 } else { 1.0 }
    }
}

/// A joint, turned every frame.
#[derive(Component, Clone, Copy)]
pub enum Joint {
    Chest,
    Neck,
    Shoulder(Side),
    Elbow(Side),
    Hip(Side),
    Knee(Side),
}

/// How the figure is moving, eased from frame to frame so nothing snaps.
#[derive(Resource, Default)]
pub struct Gait {
    /// Through the stride: a whole turn is two steps.
    phase: f32,
    /// Which way it faces, as the body's yaw is reckoned.
    facing: f32,
    /// How much it walks (0 to 1), how much of that is running, how much it is in the air, how much
    /// it sits.
    moving: f32,
    running: f32,
    air: f32,
    sitting: f32,
    /// Since the figure was made, for breathing.
    time: f32,
    /// Where the body's feet were last frame: a body moved without moving (placed anew) faces the
    /// way the player looks.
    placed: Option<[f64; 3]>,
}

/// Each joint's turn: about x (forward, for a limb hanging down, is positive) and about z.
#[derive(Clone, Copy, Default)]
struct Pose {
    chest: f32,
    neck: f32,
    shoulder: [(f32, f32); 2],
    elbow: [f32; 2],
    hip: [f32; 2],
    knee: [f32; 2],
    /// The whole figure up or down, as a fraction of its height.
    lift: f32,
}

impl Pose {
    fn mix(self, o: Pose, t: f32) -> Pose {
        let m = |a: f32, b: f32| a + (b - a) * t;
        let m2 = |a: [f32; 2], b: [f32; 2]| [m(a[0], b[0]), m(a[1], b[1])];
        Pose {
            chest: m(self.chest, o.chest),
            neck: m(self.neck, o.neck),
            shoulder: [0, 1].map(|i| (m(self.shoulder[i].0, o.shoulder[i].0), m(self.shoulder[i].1, o.shoulder[i].1))),
            elbow: m2(self.elbow, o.elbow),
            hip: m2(self.hip, o.hip),
            knee: m2(self.knee, o.knee),
            lift: m(self.lift, o.lift),
        }
    }
}

/// The figure made, standing at the origin, hidden until placed.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let mut paint = |r: f32, g: f32, b: f32, rough: f32| {
        materials.add(StandardMaterial { base_color: Color::srgb(r, g, b), perceptual_roughness: rough, ..default() })
    };
    let shirt = paint(0.78, 0.34, 0.20, 0.8);
    let trousers = paint(0.20, 0.26, 0.40, 0.9);
    let skin = paint(0.88, 0.69, 0.55, 0.6);
    let hair = paint(0.24, 0.15, 0.09, 0.9);
    let shoes = paint(0.22, 0.16, 0.11, 0.7);
    let dark = paint(0.05, 0.05, 0.06, 0.3);
    let mut shape = |m: Mesh| meshes.add(m);
    let torso = shape(Capsule3d::new(0.16, 0.30).into());
    let pelvis = shape(Cuboid::new(0.34, 0.18, 0.21).into());
    let head = shape(Sphere::new(0.115).into());
    let cap = shape(Sphere::new(0.123).into());
    let eye = shape(Sphere::new(0.017).into());
    let nose = shape(Sphere::new(0.022).into());
    let upper_arm = shape(Capsule3d::new(0.056, 0.20).into());
    let forearm = shape(Capsule3d::new(0.048, 0.18).into());
    let hand = shape(Sphere::new(0.054).into());
    let thigh = shape(Capsule3d::new(0.078, 0.28).into());
    let shin = shape(Capsule3d::new(0.064, 0.30).into());
    let foot = shape(Cuboid::new(0.11, 0.07, 0.25).into());
    let part = |mesh: &Handle<Mesh>, material: &Handle<StandardMaterial>, at: Vec3| (Mesh3d(mesh.clone()), MeshMaterial3d(material.clone()), Transform::from_translation(at));

    commands.spawn((Figure, Transform::default(), Visibility::Hidden)).with_children(|root| {
        // The hips, 0.95 up: everything hangs from them.
        root.spawn((Transform::from_xyz(0.0, 0.95, 0.0), Visibility::Inherited)).with_children(|hips| {
            hips.spawn(part(&pelvis, &trousers, Vec3::new(0.0, -0.01, 0.0)));
            hips.spawn((Joint::Chest, Transform::from_xyz(0.0, 0.02, 0.0), Visibility::Inherited)).with_children(|chest| {
                chest.spawn(part(&torso, &shirt, Vec3::new(0.0, 0.27, 0.0))).insert(Transform::from_xyz(0.0, 0.27, 0.0).with_scale(Vec3::new(1.15, 1.0, 0.75)));
                chest.spawn((Joint::Neck, Transform::from_xyz(0.0, 0.53, 0.0), Visibility::Inherited)).with_children(|neck| {
                    neck.spawn(part(&head, &skin, Vec3::new(0.0, 0.10, 0.0)));
                    // Hair over the top and back; the face is to -z.
                    neck.spawn(part(&cap, &hair, Vec3::new(0.0, 0.13, 0.02))).insert(Transform::from_xyz(0.0, 0.13, 0.02).with_scale(Vec3::new(1.0, 0.86, 1.0)));
                    for side in [Side::Left, Side::Right] {
                        neck.spawn(part(&eye, &dark, Vec3::new(0.042 * side.x(), 0.115, -0.103)));
                    }
                    neck.spawn(part(&nose, &skin, Vec3::new(0.0, 0.085, -0.115)));
                });
                for side in [Side::Left, Side::Right] {
                    chest.spawn((Joint::Shoulder(side), Transform::from_xyz(0.235 * side.x(), 0.43, 0.0), Visibility::Inherited)).with_children(|shoulder| {
                        shoulder.spawn(part(&upper_arm, &shirt, Vec3::new(0.0, -0.14, 0.0)));
                        shoulder.spawn((Joint::Elbow(side), Transform::from_xyz(0.0, -0.29, 0.0), Visibility::Inherited)).with_children(|elbow| {
                            elbow.spawn(part(&forearm, &skin, Vec3::new(0.0, -0.13, 0.0)));
                            elbow.spawn(part(&hand, &skin, Vec3::new(0.0, -0.27, 0.0)));
                        });
                    });
                }
            });
            for side in [Side::Left, Side::Right] {
                hips.spawn((Joint::Hip(side), Transform::from_xyz(0.1 * side.x(), -0.04, 0.0), Visibility::Inherited)).with_children(|hip| {
                    hip.spawn(part(&thigh, &trousers, Vec3::new(0.0, -0.21, 0.0)));
                    hip.spawn((Joint::Knee(side), Transform::from_xyz(0.0, -0.43, 0.0), Visibility::Inherited)).with_children(|knee| {
                        knee.spawn(part(&shin, &trousers, Vec3::new(0.0, -0.2, 0.0)));
                        knee.spawn(part(&foot, &shoes, Vec3::new(0.0, -0.445, -0.04)));
                    });
                });
            }
        });
    });
}

/// The way from `a` to `b`, the shorter way round.
fn turn(a: f32, b: f32) -> f32 {
    (b - a + PI).rem_euclid(TAU) - PI
}

/// The figure's root, to place and show or hide.
type Root = (&'static mut Transform, &'static mut Visibility);

/// The figure placed where the body is, turned and posed as it moves; hidden in first person, or
/// when the camera is too close behind to see past it.
pub fn animate(
    time: Res<Time>,
    player: Res<Player>,
    mut gait: ResMut<Gait>,
    mut figure: Query<Root, (With<Figure>, Without<Joint>)>,
    mut joints: Query<(&Joint, &mut Transform), Without<Figure>>,
) {
    let Ok((mut root, mut visibility)) = figure.single_mut() else { return };
    let shown = player.third && player.back > 0.6;
    visibility.set_if_neq(if shown { Visibility::Visible } else { Visibility::Hidden });
    let dt = time.delta_secs().min(0.1);
    let b = &player.body;
    let h = b.size;
    // Across the ground, in its own heights a second.
    let speed = (b.velocity[0].hypot(b.velocity[2])) / h;
    let sitting = b.seat.is_some();
    let ease = |rate: f32| 1.0 - (-rate * dt).exp();
    let g = &mut *gait;
    g.moving += (if sitting { 0.0 } else { (speed / WALK).min(1.0) } - g.moving) * ease(10.0);
    g.running += (((speed - WALK) / (RUN - WALK)).clamp(0.0, 1.0) - g.running) * ease(6.0);
    g.air += ((b.on.is_none() && !sitting) as i32 as f32 - g.air) * ease(12.0);
    g.sitting += (sitting as i32 as f32 - g.sitting) * ease(8.0);
    g.time += dt;
    // Facing the way it goes; sat down, the way the player looks (the seat says nothing of which
    // way it faces); placed anew, the way the player looks.
    let jumped = g.placed.is_none_or(|p| (0..3).map(|a| (p[a] - b.feet[a]).powi(2)).sum::<f64>().sqrt() > 0.5 * h as f64);
    if jumped && speed == 0.0 {
        g.facing = b.yaw;
    }
    g.placed = Some(b.feet);
    if sitting {
        g.facing += turn(g.facing, b.yaw) * ease(8.0);
    } else if speed > 0.05 {
        let heading = (-b.velocity[0]).atan2(-b.velocity[2]);
        g.facing += turn(g.facing, heading) * ease(12.0);
    }
    // Two steps a stride: a stride is about a height long walking, nearly two running.
    if b.on.is_some() {
        g.phase = (g.phase + TAU * dt * speed / (0.9 + 0.8 * g.running)).rem_euclid(TAU);
    }

    let (m, r) = (g.moving, g.running);
    let leg = |p: f32| {
        let swing = p.sin() * (0.42 + 0.4 * r) * m;
        // The knee bends most as the leg swings through, more running.
        let bend = m * (0.1 + (0.35 + 0.95 * r) * p.cos().max(0.0));
        (swing, -bend)
    };
    let (left, right) = (leg(g.phase), leg(g.phase + PI));
    let arm = (0.32 + 0.45 * r) * m;
    let breath = (g.time * 1.7).sin() * 0.012;
    let walking = Pose {
        chest: -(0.04 * m + 0.2 * r * m) + breath,
        neck: 0.03 * m + 0.15 * r * m,
        shoulder: [(-g.phase.sin() * arm, -0.06), (g.phase.sin() * arm, 0.06)],
        elbow: [0.15 + 0.15 * m + 1.1 * r * m; 2],
        hip: [left.0, right.0],
        knee: [left.1, right.1],
        // Down a little at each step, more running.
        lift: -(0.008 + 0.025 * r) * m * (1.0 - (2.0 * g.phase).cos()) * 0.5,
    };
    // Jumping or falling: knees drawn up, arms out.
    let up = b.velocity[1] > 0.0;
    let air = Pose {
        chest: -0.12,
        neck: 0.1,
        shoulder: [(0.35, -0.55), (if up { 0.6 } else { 0.2 }, 0.55)],
        elbow: [0.5; 2],
        hip: [if up { 0.9 } else { 0.4 }, if up { 0.2 } else { 0.5 }],
        knee: [if up { -1.3 } else { -0.6 }, if up { -0.4 } else { -0.7 }],
        lift: 0.0,
    };
    // Sitting: thighs forward along the seat, shins down, hands on the knees, the hips on the seat.
    let seated = Pose {
        chest: 0.05 + breath,
        neck: -0.05,
        shoulder: [(0.45, -0.05), (0.45, 0.05)],
        elbow: [0.7; 2],
        hip: [1.5, 1.5],
        knee: [-1.45, -1.45],
        lift: -0.5,
    };
    let pose = walking.mix(air, g.air).mix(seated, g.sitting);

    // Where: the body's feet (or the seat) from the eye the world is drawn about, in the renderer's
    // units, the figure always a person's height.
    let eye = player.eye.unwrap_or(b.eye());
    let scale = (DRAWN_AS / h) as f64;
    let feet = b.seat.map_or(b.feet, |s| s.at);
    let at = Vec3::new(((feet[0] - eye[0]) * scale) as f32, ((feet[1] - eye[1]) * scale) as f32, ((feet[2] - eye[2]) * scale) as f32);
    root.translation = at + Vec3::Y * pose.lift * DRAWN_AS;
    root.rotation = Quat::from_rotation_y(g.facing);

    let side = |s: Side| if s == Side::Left { 0 } else { 1 };
    for (joint, mut t) in &mut joints {
        t.rotation = match *joint {
            Joint::Chest => Quat::from_rotation_x(pose.chest),
            Joint::Neck => Quat::from_rotation_x(pose.neck),
            Joint::Shoulder(s) => {
                let (x, z) = pose.shoulder[side(s)];
                Quat::from_rotation_z(z) * Quat::from_rotation_x(x)
            }
            Joint::Elbow(s) => Quat::from_rotation_x(pose.elbow[side(s)]),
            Joint::Hip(s) => Quat::from_rotation_x(pose.hip[side(s)]),
            Joint::Knee(s) => Quat::from_rotation_x(pose.knee[side(s)]),
        };
    }
}
