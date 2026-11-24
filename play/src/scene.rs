//! GPU procedural scene renderer & runtime sync (`Scene` / `SceneWorld`).
//!
//! - Static instances (`id == 0`) are merged once on load into a single batched Bevy `Mesh`
//!   (1 draw call), sharing the exact same `SurfaceMaterial` and vertex attributes as terrain.
//! - Dynamic instances (`id > 0`) share uploaded `Handle<Mesh>` assets (GPU instancing) and
//!   update their `Transform`, `Visibility`, and mesh index every frame from `SceneWorld`.
//! - Dynamic point lights (`SceneWorld::lights`) are spawned/updated/despawned in Bevy, with
//!   positions, ranges, and physically scaled intensities (`intensity * scale²`) so lighting
//!   looks consistent at tiny, person, and giant scales.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::picture::colour;
use plateau_engine::scene::SceneWorld;
use plateau_engine::{Light, Material, MeshData, Scene};

use crate::surface::{SurfaceMaterial, properties};

/// Shared marker for every entity owned by the procedural scene (static batch, dynamic instances,
/// and dynamic lights) so chunk placement queries skip them cleanly.
#[derive(Component)]
pub struct SceneNode;

#[derive(Component)]
pub struct StaticSceneMesh;

#[derive(Component)]
pub struct DynamicInstance(pub u32);

#[derive(Component)]
pub struct SceneLight(pub u32);

#[derive(Resource)]
pub struct SceneVisuals {
    mesh_handles: Vec<Option<Handle<Mesh>>>,
    light_entities: HashMap<u32, Entity>,
    pub triangles: usize,
}

fn vertex_color(mat: Material, tint: [u8; 3]) -> [f32; 4] {
    let [r, g, b] = colour(mat);
    [r * (tint[0] as f32 / 255.0), g * (tint[1] as f32 / 255.0), b * (tint[2] as f32 / 255.0), 1.0]
}

fn to_bevy_mesh(m: &MeshData) -> Option<Mesh> {
    if m.vertices.is_empty() || m.indices.is_empty() {
        return None;
    }
    let mut positions = Vec::with_capacity(m.vertices.len());
    let mut normals = Vec::with_capacity(m.vertices.len());
    let mut colors = Vec::with_capacity(m.vertices.len());
    let mut uvs = Vec::with_capacity(m.vertices.len());
    for v in &m.vertices {
        positions.push(v.pos);
        normals.push(Vec3::from_array(v.normal).normalize_or(Vec3::Y).to_array());
        colors.push(vertex_color(v.material, v.tint));
        uvs.push(properties(v.material));
    }
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    out.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    out.insert_indices(Indices::U32(m.indices.clone()));
    Some(out)
}

fn build_static_batch(scene: &Scene) -> Option<(Mesh, usize)> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    for inst in &scene.instances {
        if inst.id != 0 {
            continue;
        }
        let Some(m) = scene.meshes.get(inst.mesh as usize) else { continue };
        if m.vertices.is_empty() || m.indices.is_empty() {
            continue;
        }
        let base = positions.len() as u32;
        let q = Quat::from_array(inst.rot);
        let rot = if q.length_squared() > 1e-6 { q.normalize() } else { Quat::IDENTITY };
        let sc = Vec3::from_array(inst.scale);
        let inv_sc = Vec3::new(
            if sc.x.abs() > 1e-5 { 1.0 / sc.x } else { 1.0 },
            if sc.y.abs() > 1e-5 { 1.0 / sc.y } else { 1.0 },
            if sc.z.abs() > 1e-5 { 1.0 / sc.z } else { 1.0 },
        );
        let pos = Vec3::from_array(inst.pos);
        let flip = sc.x * sc.y * sc.z < 0.0;

        for v in &m.vertices {
            let p = pos + rot * (Vec3::from_array(v.pos) * sc);
            let n = (rot * (Vec3::from_array(v.normal) * inv_sc)).normalize_or(Vec3::Y);
            positions.push(p.to_array());
            normals.push(n.to_array());
            colors.push(vertex_color(v.material, v.tint));
            uvs.push(properties(v.material));
        }
        for tri in m.indices.as_chunks::<3>().0 {
            if flip {
                indices.extend_from_slice(&[base + tri[0], base + tri[2], base + tri[1]]);
            } else {
                indices.extend_from_slice(&[base + tri[0], base + tri[1], base + tri[2]]);
            }
        }
    }

    if indices.is_empty() {
        return None;
    }
    let tris = indices.len() / 3;
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    out.insert_attribute(Mesh::ATTRIBUTE_COLOR, colors);
    out.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    out.insert_indices(Indices::U32(indices));
    Some((out, tris))
}

fn light_components(l: &Light, eye: [f64; 3], scale: f64) -> (PointLight, Transform) {
    let s = scale as f32;
    let pl = PointLight {
        color: Color::srgb(l.color[0], l.color[1], l.color[2]),
        intensity: (l.intensity * s * s).min(1e9),
        range: (l.radius * s).max(0.05),
        shadow_maps_enabled: false,
        ..default()
    };
    let tf = Transform::from_translation(Vec3::new(
        ((l.pos[0] as f64 - eye[0]) * scale) as f32,
        ((l.pos[1] as f64 - eye[1]) * scale) as f32,
        ((l.pos[2] as f64 - eye[2]) * scale) as f32,
    ));
    (pl, tf)
}

/// Uploads scene meshes, spawns the batched static mesh, dynamic instances, and initial lights.
pub fn spawn(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<SurfaceMaterial>,
    scene: &Scene,
    world: &SceneWorld,
) -> SceneVisuals {
    let mut triangles = 0usize;

    if let Some((batch, tris)) = build_static_batch(scene) {
        triangles += tris;
        commands.spawn((
            Mesh3d(meshes.add(batch)),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            Visibility::Visible,
            SceneNode,
            StaticSceneMesh,
        ));
    }

    let mesh_handles: Vec<Option<Handle<Mesh>>> = scene.meshes.iter().map(|m| to_bevy_mesh(m).map(|bm| meshes.add(bm))).collect();

    for (&id, live) in &world.dynamic {
        let Some(Some(handle)) = mesh_handles.get(live.instance.mesh as usize) else { continue };
        if let Some(m) = scene.meshes.get(live.instance.mesh as usize) {
            triangles += m.indices.len() / 3;
        }
        commands.spawn((
            Mesh3d(handle.clone()),
            MeshMaterial3d(material.clone()),
            Transform::default(),
            if live.visible { Visibility::Visible } else { Visibility::Hidden },
            SceneNode,
            DynamicInstance(id),
        ));
    }

    let mut light_entities = HashMap::new();
    for (&id, l) in &world.lights {
        let (pl, tf) = light_components(l, [0.0; 3], 1.0);
        let entity = commands.spawn((pl, tf, SceneNode, SceneLight(id))).id();
        light_entities.insert(id, entity);
    }

    SceneVisuals { mesh_handles, light_entities, triangles }
}

/// Keeps static batch, dynamic instances, and dynamic lights synced with the player's eye & scale.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync(
    commands: &mut Commands,
    visuals: &mut SceneVisuals,
    world: &SceneWorld,
    eye: [f64; 3],
    scale: f64,
    static_q: &mut Query<
        &mut Transform,
        (With<SceneNode>, With<StaticSceneMesh>, Without<DynamicInstance>, Without<SceneLight>, Without<Camera3d>, Without<crate::Sun>),
    >,
    dynamic_q: &mut Query<
        (&DynamicInstance, &mut Transform, &mut Visibility, &mut Mesh3d),
        (With<SceneNode>, Without<StaticSceneMesh>, Without<SceneLight>, Without<Camera3d>, Without<crate::Sun>),
    >,
    light_q: &mut Query<
        (&SceneLight, &mut Transform, &mut PointLight),
        (With<SceneNode>, Without<StaticSceneMesh>, Without<DynamicInstance>, Without<Camera3d>, Without<crate::Sun>),
    >,
) {
    let s = scale as f32;
    for mut tf in static_q.iter_mut() {
        tf.translation = Vec3::new((-eye[0] * scale) as f32, (-eye[1] * scale) as f32, (-eye[2] * scale) as f32);
        tf.scale = Vec3::splat(s);
    }

    for (DynamicInstance(id), mut tf, mut vis, mut mesh3d) in dynamic_q.iter_mut() {
        let Some(live) = world.dynamic.get(id) else {
            vis.set_if_neq(Visibility::Hidden);
            continue;
        };
        let p = live.instance.pos;
        tf.translation = Vec3::new(
            ((p[0] as f64 - eye[0]) * scale) as f32,
            ((p[1] as f64 - eye[1]) * scale) as f32,
            ((p[2] as f64 - eye[2]) * scale) as f32,
        );
        let q = Quat::from_array(live.instance.rot);
        tf.rotation = if q.length_squared() > 1e-6 { q.normalize() } else { Quat::IDENTITY };
        tf.scale = Vec3::from_array(live.instance.scale) * s;
        vis.set_if_neq(if live.visible { Visibility::Visible } else { Visibility::Hidden });
        if let Some(Some(handle)) = visuals.mesh_handles.get(live.instance.mesh as usize)
            && mesh3d.0 != *handle
        {
            mesh3d.0 = handle.clone();
        }
    }

    // Despawn lights removed by SetLight(intensity == 0).
    let removed: Vec<u32> = visuals.light_entities.keys().copied().filter(|id| !world.lights.contains_key(id)).collect();
    for id in removed {
        if let Some(entity) = visuals.light_entities.remove(&id) {
            commands.entity(entity).despawn();
        }
    }

    // Update existing lights.
    for (SceneLight(id), mut tf, mut pl) in light_q.iter_mut() {
        if let Some(l) = world.lights.get(id) {
            let (next_pl, next_tf) = light_components(l, eye, scale);
            *pl = next_pl;
            *tf = next_tf;
        }
    }

    // Spawn newly added lights.
    for (&id, l) in &world.lights {
        if !visuals.light_entities.contains_key(&id) {
            let (pl, tf) = light_components(l, eye, scale);
            let entity = commands.spawn((pl, tf, SceneNode, SceneLight(id))).id();
            visuals.light_entities.insert(id, entity);
        }
    }
}
