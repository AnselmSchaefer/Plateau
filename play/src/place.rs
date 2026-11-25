//! Interactive standalone `.wasm` element placement & preview ghost in `plateau-play` (Step 8b).
//!
//! Toggle Place Element mode with `P`:
//! - **Left-click**: place selected element at aimed surface, scaled to current player size
//! - **Right-click** or **`X`**: remove aimed placed element
//! - **`Tab`**: cycle element palette (`elements/modules/*.wasm`, `made/elements/*/element.wasm`, `--element`)
//! - **`R` / `T`**: rotate placement yaw (-15° / +15°)
//! - **Mouse wheel** or **`[` / `]`**: fine-adjust placement scale relative to player size
//! - **`Ctrl+R`**: rescan disk for newly built Claude elements live without restarting
//! - **`Z` / `Y`** (or **`Ctrl+Z` / `Ctrl+Y`**): unified undo/redo across sculpt edits & placed elements

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::compose::{CompiledElement, ComposedLog, ElementCatalog, PlacedElement, PlacementTransform, SdfElementEntry};
use plateau_engine::edit::Edit;
use plateau_engine::{ElementBounds, Limits, Region, SIZES};

use crate::scene::build_merged_mesh;

/// Person height in metres (default scale = `player_size / PERSON_HEIGHT`).
const PERSON_HEIGHT: f32 = SIZES[1].1;

/// Unified undoable world action across SDF sculpting (8a), element placement (8b),
/// and cutting/relocating discrete procedural objects (8c).
#[derive(Clone, Debug)]
pub enum WorldAction {
    SdfEdit {
        edit: Edit,
        /// Placed `Scene` elements lazily promoted to carvable SDF by this edit (demoted on undo).
        promoted_ids: Vec<(u32, usize)>,
    },
    /// Local SDF edit sculpted directly onto a discrete `Scene` element (travels with it when cut/carried).
    ElementSdfEdit {
        id: u32,
        edit: Edit,
        newly_promoted: Option<usize>,
    },
    PlaceElement(PlacedElement),
    RemoveElement(PlacedElement),
    CutElement {
        original: PlacedElement,
        lower: PlacedElement,
        upper: PlacedElement,
    },
    RelocateElement {
        id: u32,
        before: PlacementTransform,
        after: PlacementTransform,
    },
}

#[derive(Resource)]
pub struct PlaceMode {
    pub active: bool,
    pub catalog: ElementCatalog,
    pub palette: Vec<Arc<CompiledElement>>,
    pub selected: usize,
    pub extra_paths: Vec<PathBuf>,
    pub yaw: f32,
    pub scale_mult: f32,
    pub next_id: u32,
    pub placed: Vec<PlacedElement>,
    pub undo_stack: Vec<WorldAction>,
    pub redo_stack: Vec<WorldAction>,
    pub aim: Option<([f64; 3], [f32; 3])>,
    pub revision: u64,
}

impl PlaceMode {
    pub fn new(root: PathBuf, extra_paths: Vec<PathBuf>, initial_placed: Vec<PlacedElement>) -> PlaceMode {
        let mut catalog = ElementCatalog::new(root, Limits::default());
        let mut palette = catalog.discover();
        for p in &extra_paths {
            let key = p.to_string_lossy().into_owned();
            if let Ok(el) = catalog.get_or_load(&key)
                && !palette.iter().any(|e| e.module == el.module)
            {
                palette.push(el);
            }
        }
        let next_id = initial_placed.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        PlaceMode {
            active: false,
            catalog,
            palette,
            selected: 0,
            extra_paths,
            yaw: 0.0,
            scale_mult: 1.0,
            next_id,
            placed: initial_placed,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            aim: None,
            revision: 1,
        }
    }

    pub fn rescan(&mut self) -> usize {
        let prev_mod = self.current().map(|e| e.module.clone());
        let mut catalog = ElementCatalog::new(self.catalog.root().to_path_buf(), Limits::default());
        let mut palette = catalog.discover();
        for p in &self.extra_paths {
            let key = p.to_string_lossy().into_owned();
            if let Ok(el) = catalog.get_or_load(&key)
                && !palette.iter().any(|e| e.module == el.module)
            {
                palette.push(el);
            }
        }
        // Keep any already-placed modules loaded in the catalog so saving/rebuilding never drops them.
        for pl in &self.placed {
            let _ = catalog.get_or_load(&pl.module);
        }
        self.catalog = catalog;
        self.palette = palette;
        self.selected = prev_mod
            .and_then(|m| self.palette.iter().position(|e| e.module == m))
            .unwrap_or(0);
        self.palette.len()
    }

    pub fn current(&self) -> Option<&Arc<CompiledElement>> {
        self.palette.get(self.selected)
    }

    pub fn next_element(&mut self) {
        if !self.palette.is_empty() {
            self.selected = (self.selected + 1) % self.palette.len();
        }
    }

    pub fn adjust_scale(&mut self, factor: f32) {
        self.scale_mult = (self.scale_mult * factor).clamp(0.25, 4.0);
    }

    /// World placement scale for the player's current height (`1.0` at person height with `scale_mult = 1.0`).
    pub fn placement_scale(&self, player_size: f32) -> f32 {
        ((player_size / PERSON_HEIGHT) * self.scale_mult).clamp(0.002, 50.0)
    }

    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Rebuilds an immutable `ComposedLog` snapshot from `edits` + `water` + all currently placed SDF
    /// elements (both native SDF landforms and lazily promoted `Scene` meshes), optionally
    /// excluding element ids that are currently carried in-hand or mid-fall.
    pub fn build_composed(&mut self, edits: Arc<plateau_engine::EditLog>, water: Arc<plateau_engine::WaterLog>) -> Arc<ComposedLog> {
        self.build_composed_excluding(edits, water, &[])
    }

    pub fn build_composed_excluding(
        &mut self,
        edits: Arc<plateau_engine::EditLog>,
        water: Arc<plateau_engine::WaterLog>,
        exclude_ids: &[u32],
    ) -> Arc<ComposedLog> {
        self.revision += 1;
        let mut sdf_elements = Vec::new();
        for pl in &self.placed {
            if exclude_ids.contains(&pl.id) {
                continue;
            }
            if let Ok(compiled) = self.catalog.get_or_load(&pl.module) {
                if let Some(bounds) = compiled.sdf_bounds {
                    sdf_elements.push(SdfElementEntry::new(pl.clone(), compiled.compiled.clone(), bounds));
                } else if pl.is_promoted()
                    && let Ok(Some(mesh_sdf)) = self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips)
                {
                    sdf_elements.push(SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf));
                }
            }
        }
        Arc::new(ComposedLog::new_with_water(edits, sdf_elements, water, self.revision))
    }

    /// Returns the local SDF bounds of a placed element if it is currently represented in the SDF
    /// layer (either natively or via lazy Scene→SDF promotion, expanded for any local `Op::Add`).
    pub fn sdf_bounds_of(&self, pl: &PlacedElement) -> Option<ElementBounds> {
        if let Some(el) = self.catalog.get(&pl.module)
            && let Some(bounds) = el.sdf_bounds
        {
            return Some(bounds);
        }
        if pl.is_promoted()
            && let Some(mesh_sdf) = self.catalog.cached_mesh_sdf(&pl.module, pl.seed, &pl.clips)
        {
            let mut b = mesh_sdf.bounds();
            for e in &pl.local_edits {
                if e.op == plateau_engine::edit::Op::Add {
                    let (elo, ehi) = e.influence_bounds();
                    for a in 0..3 {
                        b.lo[a] = b.lo[a].min(elo[a]);
                        b.hi[a] = b.hi[a].max(ehi[a]);
                    }
                }
            }
            return Some(b);
        }
        None
    }

    /// Finds the nearest placed SDF element (native or promoted) whose world bounding box is hit
    /// by ray `(origin, dir)`.
    pub fn raycast_placed_sdf(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<(u32, f32)> {
        let mut best_t = max_dist;
        let mut best_id = None;
        for pl in &self.placed {
            let Some(bounds) = self.sdf_bounds_of(pl) else { continue };
            let (lo, hi) = pl.transform().world_bounds(bounds);
            if let Some(t) = ray_aabb(origin, dir, lo, hi, best_t) {
                best_t = t;
                best_id = Some(pl.id);
            }
        }
        best_id.map(|id| (id, best_t))
    }
}

fn ray_aabb(ro: [f32; 3], rd: [f32; 3], lo: [f32; 3], hi: [f32; 3], max_t: f32) -> Option<f32> {
    let (mut tmin, mut tmax) = (0.0f32, max_t);
    for a in 0..3 {
        if rd[a].abs() < 1e-6 {
            if ro[a] < lo[a] || ro[a] > hi[a] {
                return None;
            }
        } else {
            let inv = 1.0 / rd[a];
            let mut t0 = (lo[a] - ro[a]) * inv;
            let mut t1 = (hi[a] - ro[a]) * inv;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
            }
            tmin = tmin.max(t0);
            tmax = tmax.min(t1);
            if tmin > tmax {
                return None;
            }
        }
    }
    Some(tmin)
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
    let (Some(elem), Some((hit, _normal))) = (place.current(), place.aim) else {
        vis.set_if_neq(Visibility::Hidden);
        return;
    };
    let handle = preview_mesh_for(elem, &mut meshes, &mut assets);
    if mesh3d.0 != handle {
        mesh3d.0 = handle;
    }
    if let Some(mut mat) = materials.get_mut(&assets.material) {
        if elem.is_sdf() {
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
    let elem_scale = place.placement_scale(player_size);
    tf.translation = Vec3::new(
        ((hit[0] - eye[0]) * draw_scale) as f32,
        ((hit[1] - eye[1]) * draw_scale) as f32,
        ((hit[2] - eye[2]) * draw_scale) as f32,
    );
    tf.rotation = Quat::from_rotation_y(place.yaw);
    tf.scale = Vec3::splat(elem_scale * draw_scale as f32);
    vis.set_if_neq(Visibility::Visible);
}
