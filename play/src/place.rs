//! Interactive standalone `.wasm` element placement, cavity fitting & preview ghost in `plateau-play` (Step 8b / Phase C).
//!
//! Toggle Place Element mode with `P`:
//! - **Left-click**: place selected element at aimed surface (or snapped cavity fit), scaled to current player size
//! - **Right-click** or **`X`**: remove aimed placed element
//! - **`O`**: auto-fit / snap selected element to the aimed carved opening (or its authored `element.site`)
//! - **`Tab`**: cycle element palette (`elements/modules/*.wasm`, `made/elements/*/element.wasm`, `--element`)
//! - **`R` / `T`**: rotate placement yaw (-15° / +15°)
//! - **Mouse wheel** or **`[` / `]`**: fine-adjust placement scale relative to player size
//! - **`Ctrl+R`**: rescan disk for newly built Claude elements live without restarting
//! - **`Z` / `Y`** (or **`Ctrl+Z` / `Ctrl+Y`**): unified undo/redo across sculpt edits & placed elements

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::compose::{CompiledElement, ElementCatalog, PlacementTransform};
use plateau_engine::live::World;
use plateau_engine::site::SiteContext;
use plateau_engine::{ElementBounds, Limits, Region, SIZES};

use crate::scene::build_merged_mesh;

/// Person height in metres (default scale = `player_size / PERSON_HEIGHT`).
const PERSON_HEIGHT: f32 = SIZES[1].1;

/// Active cavity/site auto-fit snap locked in Place Mode via `O`.
#[derive(Clone, Debug)]
pub struct FittedSnap {
    pub module: String,
    pub transform: PlacementTransform,
    pub summary: String,
}

#[derive(Resource)]
pub struct PlaceMode {
    pub active: bool,
    pub palette: Vec<Arc<CompiledElement>>,
    pub selected: usize,
    /// `--element` modules offered beside those found on disk.
    pub extra_paths: Vec<PathBuf>,
    pub yaw: f32,
    pub scale_mult: f32,
    pub aim: Option<([f64; 3], [f32; 3])>,
    pub fitted: Option<FittedSnap>,
    pub last_site: Option<SiteContext>,
}

impl PlaceMode {
    /// The palette: the elements found under the world's root, and `extra_paths`.
    pub fn new(extra_paths: Vec<PathBuf>, world: &mut World) -> PlaceMode {
        let palette = palette_of(&mut world.catalog, &extra_paths);
        PlaceMode { active: false, palette, selected: 0, extra_paths, yaw: 0.0, scale_mult: 1.0, aim: None, fitted: None, last_site: None }
    }

    /// Looks on disk again for elements (rebuilt or newly made), keeping the world's placed ones
    /// loaded; how many there are.
    pub fn rescan(&mut self, world: &mut World) -> usize {
        let prev_mod = self.current().map(|e| e.module.clone());
        let mut catalog = ElementCatalog::new(world.root.clone(), Limits::default());
        catalog.expect_hashes(&world.modules, world.module_check);
        let palette = palette_of(&mut catalog, &self.extra_paths);
        for pl in &world.placed {
            let _ = catalog.get_or_load(&pl.module);
        }
        world.catalog = catalog;
        self.palette = palette;
        self.selected = prev_mod.and_then(|m| self.palette.iter().position(|e| e.module == m)).unwrap_or(0);
        self.fitted = None;
        self.palette.len()
    }

    pub fn current(&self) -> Option<&Arc<CompiledElement>> {
        self.palette.get(self.selected)
    }

    pub fn next_element(&mut self) {
        if !self.palette.is_empty() {
            self.selected = (self.selected + 1) % self.palette.len();
            self.fitted = None;
        }
    }

    pub fn adjust_scale(&mut self, factor: f32) {
        self.scale_mult = (self.scale_mult * factor).clamp(0.25, 4.0);
        self.fitted = None;
    }

    /// Returns the `element.site` file beside a site-authored element module, if one exists.
    pub fn element_site_path(&self, module: &str, root: &Path) -> Option<PathBuf> {
        let p = Path::new(module);
        let full = if p.is_absolute() { p.to_path_buf() } else { root.join(p) };
        let site = full.with_extension("site");
        site.exists().then_some(site)
    }

    /// World placement scale for the player's current height (`1.0` at person height with `scale_mult = 1.0`).
    pub fn placement_scale(&self, player_size: f32) -> f32 {
        ((player_size / PERSON_HEIGHT) * self.scale_mult).clamp(0.002, 50.0)
    }
}

/// The elements found under `catalog`'s root, then `extra_paths` not among them.
fn palette_of(catalog: &mut ElementCatalog, extra_paths: &[PathBuf]) -> Vec<Arc<CompiledElement>> {
    let mut palette = catalog.discover();
    for p in extra_paths {
        let key = p.to_string_lossy().into_owned();
        if let Ok(el) = catalog.get_or_load(&key)
            && !palette.iter().any(|e| e.module == el.module)
        {
            palette.push(el);
        }
    }
    palette
}

/// Marker component for the translucent 3D element placement preview ghost.
#[derive(Component)]
pub struct ElementGhost;

#[derive(Resource)]
pub struct PreviewAssets {
    cache: HashMap<String, Handle<Mesh>>,
    fallback_box: Handle<Mesh>,
    material: Handle<StandardMaterial>,
}

pub fn spawn_ghost(commands: &mut Commands, meshes: &mut Assets<Mesh>, materials: &mut Assets<StandardMaterial>) {
    let fallback_box = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    let material = materials.add(StandardMaterial {
        base_color: Color::srgba(0.60, 0.92, 0.72, 0.36),
        emissive: LinearRgba::rgb(0.14, 0.36, 0.22),
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        ..default()
    });
    commands.spawn((
        Mesh3d(fallback_box.clone()),
        MeshMaterial3d(material.clone()),
        Transform::default(),
        Visibility::Hidden,
        ElementGhost,
    ));
    commands.insert_resource(PreviewAssets { cache: HashMap::new(), fallback_box, material });
}

/// Builds a quick translucent preview mesh for an SDF landform element (voxelized coarse surface
/// inside `bounds.lo..bounds.hi` so the ghost shows the real silhouette of hills, arches, etc.).
fn build_sdf_preview_mesh(elem: &CompiledElement, bounds: ElementBounds) -> Option<Mesh> {
    const N: usize = 16;
    let mut inst = elem.compiled.instantiate(1).ok()?;
    let span = [bounds.hi[0] - bounds.lo[0], bounds.hi[1] - bounds.lo[1], bounds.hi[2] - bounds.lo[2]];
    let step = [span[0] / N as f32, span[1] / N as f32, span[2] / N as f32];
    let mut pts = Vec::with_capacity((N + 1) * (N + 1) * (N + 1));
    for iz in 0..=N {
        for iy in 0..=N {
            for ix in 0..=N {
                pts.push([
                    bounds.lo[0] + ix as f32 * step[0],
                    bounds.lo[1] + iy as f32 * step[1],
                    bounds.lo[2] + iz as f32 * step[2],
                ]);
            }
        }
    }
    let mut ans = Vec::new();
    inst.sample(&pts, 1.7, &mut ans).ok()?;
    let idx = |x: usize, y: usize, z: usize| (z * (N + 1) + y) * (N + 1) + x;

    // Emit surface quads between adjacent solid/air grid cells (clean blocky-smooth preview).
    let mut positions: Vec<[f32; 3]> = Vec::new();
    let mut normals: Vec<[f32; 3]> = Vec::new();
    let mut indices: Vec<u32> = Vec::new();
    let mut push_quad = |c: [f32; 3], u: [f32; 3], v: [f32; 3], n: [f32; 3]| {
        let base = positions.len() as u32;
        let p0 = [c[0] - u[0] - v[0], c[1] - u[1] - v[1], c[2] - u[2] - v[2]];
        let p1 = [c[0] + u[0] - v[0], c[1] + u[1] - v[1], c[2] + u[2] - v[2]];
        let p2 = [c[0] + u[0] + v[0], c[1] + u[1] + v[1], c[2] + u[2] + v[2]];
        let p3 = [c[0] - u[0] + v[0], c[1] - u[1] + v[1], c[2] - u[2] + v[2]];
        positions.extend_from_slice(&[p0, p1, p2, p3]);
        normals.extend_from_slice(&[n, n, n, n]);
        indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    };

    let (hx, hy, hz) = (step[0] * 0.5, step[1] * 0.5, step[2] * 0.5);
    for iz in 0..N {
        for iy in 0..N {
            for ix in 0..N {
                let d0 = ans[idx(ix, iy, iz)].distance;
                let s0 = d0 <= 0.0;
                // +X face
                let dx = ans[idx(ix + 1, iy, iz)].distance;
                if s0 != (dx <= 0.0) {
                    let sign = if s0 { 1.0 } else { -1.0 };
                    let c = [bounds.lo[0] + (ix as f32 + 0.5) * step[0], bounds.lo[1] + iy as f32 * step[1], bounds.lo[2] + iz as f32 * step[2]];
                    push_quad(c, [0.0, hy, 0.0], [0.0, 0.0, hz * sign], [sign, 0.0, 0.0]);
                }
                // +Y face
                let dy = ans[idx(ix, iy + 1, iz)].distance;
                if s0 != (dy <= 0.0) {
                    let sign = if s0 { 1.0 } else { -1.0 };
                    let c = [bounds.lo[0] + ix as f32 * step[0], bounds.lo[1] + (iy as f32 + 0.5) * step[1], bounds.lo[2] + iz as f32 * step[2]];
                    push_quad(c, [hx, 0.0, 0.0], [0.0, 0.0, -hz * sign], [0.0, sign, 0.0]);
                }
                // +Z face
                let dz = ans[idx(ix, iy, iz + 1)].distance;
                if s0 != (dz <= 0.0) {
                    let sign = if s0 { 1.0 } else { -1.0 };
                    let c = [bounds.lo[0] + ix as f32 * step[0], bounds.lo[1] + iy as f32 * step[1], bounds.lo[2] + (iz as f32 + 0.5) * step[2]];
                    push_quad(c, [hx * sign, 0.0, 0.0], [0.0, hy, 0.0], [0.0, 0.0, sign]);
                }
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
    out.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    out.insert_indices(Indices::U32(indices));
    Some(out)
}

fn preview_mesh_for(elem: &CompiledElement, meshes: &mut Assets<Mesh>, assets: &mut PreviewAssets) -> Handle<Mesh> {
    if let Some(handle) = assets.cache.get(&elem.module) {
        return handle.clone();
    }
    let built = build_merged_mesh(&elem.preview_scene, false, false)
        .map(|(m, _)| m)
        .or_else(|| elem.sdf_bounds.and_then(|b| build_sdf_preview_mesh(elem, b)));
    let handle = match built {
        Some(m) => meshes.add(m),
        None => assets.fallback_box.clone(),
    };
    assets.cache.insert(elem.module.clone(), handle.clone());
    handle
}

/// Clears cached preview meshes (called on `Ctrl+R` palette rescan so edited elements refresh).
pub fn clear_preview_cache(assets: &mut PreviewAssets) {
    assets.cache.clear();
}

/// Positions, rotates, and scales the translucent element preview ghost at the player's aim point.
pub fn place_ghost(
    place: Res<PlaceMode>,
    player: Res<crate::Player>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut assets: ResMut<PreviewAssets>,
    mut ghost_q: Query<(&mut Transform, &mut Visibility, &mut Mesh3d), With<ElementGhost>>,
) {
    let Ok((mut tf, mut vis, mut mesh3d)) = ghost_q.single_mut() else { return };
    if !place.active {
        vis.set_if_neq(Visibility::Hidden);
        return;
    }
    let Some(elem) = place.current() else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    let snapped = place.fitted.as_ref().filter(|s| s.module == elem.module);
    if snapped.is_none() && place.aim.is_none() {
        vis.set_if_neq(Visibility::Hidden);
        return;
    }
    let handle = preview_mesh_for(elem, &mut meshes, &mut assets);
    if mesh3d.0 != handle {
        mesh3d.0 = handle;
    }
    if let Some(mut mat) = materials.get_mut(&assets.material) {
        if snapped.is_some() {
            mat.base_color = Color::srgba(0.98, 0.84, 0.42, 0.44);
            mat.emissive = LinearRgba::rgb(0.36, 0.26, 0.06);
        } else if elem.is_sdf() {
            mat.base_color = Color::srgba(0.55, 0.85, 0.95, 0.36);
            mat.emissive = LinearRgba::rgb(0.14, 0.30, 0.42);
        } else {
            mat.base_color = Color::srgba(0.60, 0.92, 0.72, 0.36);
            mat.emissive = LinearRgba::rgb(0.14, 0.36, 0.22);
        }
    }
    let player_size = player.body.size;
    let eye = player.eye.unwrap_or(player.body.eye());
    let draw_scale = (crate::DRAWN_AS / player_size) as f64;
    if let Some(snap) = snapped {
        let stf = snap.transform;
        tf.translation = Vec3::new(
            ((stf.pos[0] as f64 - eye[0]) * draw_scale) as f32,
            ((stf.pos[1] as f64 - eye[1]) * draw_scale) as f32,
            ((stf.pos[2] as f64 - eye[2]) * draw_scale) as f32,
        );
        tf.rotation = Quat::from_array(stf.rot).normalize();
        tf.scale = Vec3::splat(stf.scale * draw_scale as f32);
    } else if let Some((hit, _normal)) = place.aim {
        let elem_scale = place.placement_scale(player_size);
        tf.translation = Vec3::new(
            ((hit[0] - eye[0]) * draw_scale) as f32,
            ((hit[1] - eye[1]) * draw_scale) as f32,
            ((hit[2] - eye[2]) * draw_scale) as f32,
        );
        tf.rotation = Quat::from_rotation_y(place.yaw);
        tf.scale = Vec3::splat(elem_scale * draw_scale as f32);
    }
    vis.set_if_neq(Visibility::Visible);
}
