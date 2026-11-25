//! Multi-layer GPU procedural scene renderer & runtime sync (`Scene` / `SceneWorld`).
//!
//! - Base area (`owner = 0`) and each placed standalone `.wasm` element (`owner > 0`) live in their
//!   own layer so placing or removing elements never rebuilds the base area's static mesh batch.
//! - Static instances (`id == 0`) per layer are merged into 1 draw call using the shared
//!   `SurfaceMaterial`.
//! - Dynamic instances (`id > 0`) share uploaded `Handle<Mesh>` assets per owner and update their
//!   `Transform`, `Visibility`, and mesh index every frame from `SceneWorld`.
//! - Dynamic point lights (`SceneWorld::lights`) are spawned/updated/despawned per `(owner, id)`
//!   with physically scaled intensities (`intensity * scale²`) across tiny, person, and giant scales.

use std::collections::HashMap;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::compose::PlacementTransform;
use plateau_engine::picture::colour;
use plateau_engine::scene::{SceneWorld, is_area_owner};
use plateau_engine::{Light, Material, MeshData, Scene};

use crate::surface::{SurfaceMaterial, properties};

/// Shared marker for every entity owned by the procedural scene so chunk placement skips them.
#[derive(Component)]
pub struct SceneNode;

#[derive(Component)]
pub struct StaticSceneMesh {
    pub owner: u32,
}

#[derive(Component)]
pub struct DynamicInstance {
    pub owner: u32,
    pub id: u32,
}

#[derive(Component)]
pub struct SceneLight {
    pub owner: u32,
    pub id: u32,
}

#[derive(Resource)]
pub struct SceneVisuals {
    pub material: Handle<SurfaceMaterial>,
    layer_meshes: HashMap<u32, Vec<Option<Handle<Mesh>>>>,
    layer_entities: HashMap<u32, Vec<Entity>>,
    layer_triangles: HashMap<u32, usize>,
    /// Optional rigid relocation `(baked_tf, current_tf)` for placed static layers so bobbing,
    /// sailing, and toppling move the existing GPU mesh in-place with zero mesh re-uploads!
    layer_reloc: HashMap<u32, (PlacementTransform, PlacementTransform)>,
    light_entities: HashMap<(u32, u32), Entity>,
    pub triangles: usize,
}

impl SceneVisuals {
    /// Moves an already-spawned static layer from its baked `spawn_tf` to `cur_tf` on the GPU
    /// without despawning entities or re-uploading vertex buffers.
    pub fn set_layer_reloc(&mut self, owner: u32, spawn_tf: PlacementTransform, cur_tf: PlacementTransform) {
        self.layer_reloc.entry(owner).and_modify(|e| e.1 = cur_tf).or_insert((spawn_tf, cur_tf));
    }
}

fn vertex_color(mat: Material, tint: [u8; 3]) -> [f32; 4] {
    let [r, g, b] = colour(mat);
    [r * (tint[0] as f32 / 255.0), g * (tint[1] as f32 / 255.0), b * (tint[2] as f32 / 255.0), 1.0]
}

pub fn to_bevy_mesh(m: &MeshData) -> Option<Mesh> {
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

pub fn build_merged_mesh(scene: &Scene, static_only: bool, skip_solid: bool) -> Option<(Mesh, usize)> {
    let mut positions = Vec::new();
    let mut normals = Vec::new();
    let mut colors = Vec::new();
    let mut uvs = Vec::new();
    let mut indices = Vec::new();

    for inst in &scene.instances {
        if (static_only && inst.id != 0) || (skip_solid && inst.solid) {
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

/// Initializes `SceneVisuals` and spawns all base area tile layers (`(owner, world_scene)`).
pub fn spawn_areas(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    material: Handle<SurfaceMaterial>,
    areas: &[(u32, Scene)],
    world: &SceneWorld,
) -> SceneVisuals {
    let mut visuals = SceneVisuals {
        material,
        layer_meshes: HashMap::new(),
        layer_entities: HashMap::new(),
        layer_triangles: HashMap::new(),
        layer_reloc: HashMap::new(),
        light_entities: HashMap::new(),
        triangles: 0,
    };
    for (owner, scene) in areas {
        spawn_layer(commands, meshes, &mut visuals, *owner, scene, world);
    }
    visuals
}

/// Uploads and spawns one scene layer (`owner = 0` for base area, `owner > 0` for a placed element).
pub fn spawn_layer(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    visuals: &mut SceneVisuals,
    owner: u32,
    scene: &Scene,
    world: &SceneWorld,
) {
    spawn_layer_filtered(commands, meshes, visuals, owner, scene, world, false);
}

/// Uploads and spawns one scene layer, optionally omitting `solid: true` static instances when the
/// layer's solid geometry has been lazily promoted into the carvable SDF layer.
pub fn spawn_layer_filtered(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    visuals: &mut SceneVisuals,
    owner: u32,
    scene: &Scene,
    world: &SceneWorld,
    skip_solid: bool,
) {
    despawn_layer(commands, visuals, owner);

    let mut entities = Vec::new();
    let mut tris = 0usize;

    if let Some((batch, batch_tris)) = build_merged_mesh(scene, true, skip_solid) {
        tris += batch_tris;
        let entity = commands
            .spawn((
                Mesh3d(meshes.add(batch)),
                MeshMaterial3d(visuals.material.clone()),
                Transform::default(),
                Visibility::Visible,
                SceneNode,
                StaticSceneMesh { owner },
            ))
            .id();
        entities.push(entity);
    }

    let mesh_handles: Vec<Option<Handle<Mesh>>> = scene.meshes.iter().map(|m| to_bevy_mesh(m).map(|bm| meshes.add(bm))).collect();

    for (&(inst_owner, id), live) in &world.dynamic {
        if inst_owner != owner {
            continue;
        }
        let Some(Some(handle)) = mesh_handles.get(live.instance.mesh as usize) else { continue };
        if let Some(m) = scene.meshes.get(live.instance.mesh as usize) {
            tris += m.indices.len() / 3;
        }
        let entity = commands
            .spawn((
                Mesh3d(handle.clone()),
                MeshMaterial3d(visuals.material.clone()),
                Transform::default(),
                if live.visible { Visibility::Visible } else { Visibility::Hidden },
                SceneNode,
                DynamicInstance { owner, id },
            ))
            .id();
        entities.push(entity);
    }

    for (&(light_owner, id), l) in &world.lights {
        if light_owner != owner {
            continue;
        }
        let (pl, tf) = light_components(l, [0.0; 3], 1.0);
        let entity = commands.spawn((pl, tf, SceneNode, SceneLight { owner, id })).id();
        visuals.light_entities.insert((owner, id), entity);
    }

    visuals.layer_meshes.insert(owner, mesh_handles);
    visuals.layer_entities.insert(owner, entities);
    visuals.layer_triangles.insert(owner, tris);
    visuals.triangles += tris;
}

/// Despawns every GPU entity and light belonging to `owner`.
pub fn despawn_layer(commands: &mut Commands, visuals: &mut SceneVisuals, owner: u32) {
    if let Some(entities) = visuals.layer_entities.remove(&owner) {
        for entity in entities {
            commands.entity(entity).despawn();
        }
    }
    visuals.layer_meshes.remove(&owner);
    visuals.layer_reloc.remove(&owner);
    if let Some(tris) = visuals.layer_triangles.remove(&owner) {
        visuals.triangles = visuals.triangles.saturating_sub(tris);
    }
    let light_keys: Vec<(u32, u32)> = visuals.light_entities.keys().copied().filter(|&(o, _)| o == owner).collect();
    for key in light_keys {
        if let Some(entity) = visuals.light_entities.remove(&key) {
            commands.entity(entity).despawn();
        }
    }
}

/// Despawns all placed element layers (`!is_area_owner(o)`), leaving all base area tiles intact.
pub fn despawn_placed_layers(commands: &mut Commands, visuals: &mut SceneVisuals) {
    let owners: Vec<u32> = visuals.layer_entities.keys().copied().filter(|&o| !is_area_owner(o)).collect();
    for owner in owners {
        despawn_layer(commands, visuals, owner);
    }
}

/// Keeps static batches, dynamic instances, and dynamic lights synced with the player's eye & scale.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub fn sync(
    commands: &mut Commands,
    visuals: &mut SceneVisuals,
    world: &SceneWorld,
    eye: [f64; 3],
    scale: f64,
    static_q: &mut Query<
        (&StaticSceneMesh, &mut Transform),
        (With<SceneNode>, Without<DynamicInstance>, Without<SceneLight>, Without<Camera3d>, Without<crate::Sun>),
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
    for (sm, mut tf) in static_q.iter_mut() {
        if let Some(&(baked, cur)) = visuals.layer_reloc.get(&sm.owner) {
            let norm_q = |a: [f32; 4]| {
                let q = Quat::from_array(a);
                if q.length_squared() > 1e-6 { q.normalize() } else { Quat::IDENTITY }
            };
            let q0 = norm_q(baked.rot);
            let q1 = norm_q(cur.rot);
            let dq = q1 * q0.inverse();
            let dq = if dq.length_squared() > 1e-6 { dq.normalize() } else { Quat::IDENTITY };
            let ds = cur.scale / baked.scale.max(1e-5);
            let p0 = Vec3::from_array(baked.pos);
            let rot_p0 = dq * (p0 * ds);
            tf.rotation = dq;
            tf.scale = Vec3::splat(ds * s);
            tf.translation = Vec3::new(
                ((cur.pos[0] as f64 - eye[0] - rot_p0.x as f64) * scale) as f32,
                ((cur.pos[1] as f64 - eye[1] - rot_p0.y as f64) * scale) as f32,
                ((cur.pos[2] as f64 - eye[2] - rot_p0.z as f64) * scale) as f32,
            );
        } else {
            tf.rotation = Quat::IDENTITY;
            tf.translation = Vec3::new((-eye[0] * scale) as f32, (-eye[1] * scale) as f32, (-eye[2] * scale) as f32);
            tf.scale = Vec3::splat(s);
        }
    }

    for (di, mut tf, mut vis, mut mesh3d) in dynamic_q.iter_mut() {
        let key = (di.owner, di.id);
        let Some(live) = world.dynamic.get(&key) else {
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
        if let Some(Some(handle)) = visuals.layer_meshes.get(&di.owner).and_then(|v| v.get(live.instance.mesh as usize))
            && mesh3d.0 != *handle
        {
            mesh3d.0 = handle.clone();
        }
    }

    // Despawn lights removed by SetLight(intensity == 0) or despawned layers.
    let removed: Vec<(u32, u32)> = visuals.light_entities.keys().copied().filter(|k| !world.lights.contains_key(k)).collect();
    for key in removed {
        if let Some(entity) = visuals.light_entities.remove(&key) {
            commands.entity(entity).despawn();
        }
    }

    // Update existing lights.
    for (sl, mut tf, mut pl) in light_q.iter_mut() {
        if let Some(l) = world.lights.get(&(sl.owner, sl.id)) {
            let (next_pl, next_tf) = light_components(l, eye, scale);
            *pl = next_pl;
            *tf = next_tf;
        }
    }

    // Spawn newly added lights.
    for (&(owner, id), l) in &world.lights {
        if !visuals.light_entities.contains_key(&(owner, id)) {
            let (pl, tf) = light_components(l, eye, scale);
            let entity = commands.spawn((pl, tf, SceneNode, SceneLight { owner, id })).id();
            visuals.light_entities.insert((owner, id), entity);
        }
    }
}
