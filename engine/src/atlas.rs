//! Multi-area world atlas & seamless border stitching (POC_PLAN.md, step 9).
//!
//! Arranges one or more 64 m × 64 m areas on an integer tile grid `(gx, gz)`:
//! tile `(gx, gz)` covers world `x ∈ [gx * 64, (gx + 1) * 64]`, `z ∈ [gz * 64, (gz + 1) * 64]`.
//!
//! - **Single-tile fast path**: a 1-tile atlas at `(0, 0)` delegates directly to the underlying
//!   `Region` with zero overhead and bit-identical answers.
//! - **Seamless C¹ border blending**: within `SEAM_HALF` metres of any shared edge between two
//!   present tiles, `Atlas::sample` queries both tiles in local coordinates and blends their SDF
//!   distances with a symmetric deterministic smoothstep (extending bilinearly at 4-tile corners).
//!   Small border height or detail-noise differences become a gentle slope invisible to giants,
//!   persons, and tiny players alike.
//! - **Per-tile `Scene` & runtime plugins**: each tile keeps its own local `[0, 64]` coordinate
//!   space and isolated `SceneWorld` owner (`tile_owner(idx)`), so existing `.wasm` areas work at
//!   any grid position without recompiling and without colliding with player-placed elements.
//! - **Border extraction (`BorderProfile`)**: samples any `Region` along one of its four edges
//!   (ground heights, underwater bed heights, water level, and materials) to feed directly into
//!   `laws::borders_match` and Claude's adjacent-area prompt.

use std::collections::HashMap;
use std::sync::Arc;

use wasmtime::bail;

use crate::compose::PlacementTransform;
use crate::reach::{HIGHEST, LOWEST};
use crate::scene::{Applied, SceneWorld, tile_owner};
use crate::{Answer, Borders, Compiled, Description, Edge, ElementBounds, Material, Place, PlayerState, Region, Result, SIZES, Scene, WIDTH, Wasm, chunk};

/// Half-width in metres of the smooth C¹ SDF blend band on each side of a shared tile border.
/// At 1.5 m (3.0 m total band), the maximum blend slope contribution for a 0.15 m border tolerance
/// is `0.075` (4.3°), staying well below Lipschitz 1.0 and tiny-scale step limits while leaving
/// `START = (32, 60)` and interior features completely untouched.
pub const SEAM_HALF: f32 = 1.5;

/// Cardinal side of a 64 m square area (north `-z`, east `+x`, south `+z`, west `-x`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    North,
    East,
    South,
    West,
}

impl Side {
    pub const ALL: [Side; 4] = [Side::North, Side::East, Side::South, Side::West];

    pub fn name(self) -> &'static str {
        match self {
            Side::North => "north",
            Side::East => "east",
            Side::South => "south",
            Side::West => "west",
        }
    }

    pub fn from_name(s: &str) -> Option<Side> {
        match s.trim().to_ascii_lowercase().as_str() {
            "north" | "n" => Some(Side::North),
            "east" | "e" => Some(Side::East),
            "south" | "s" => Some(Side::South),
            "west" | "w" => Some(Side::West),
            _ => None,
        }
    }

    pub fn opposite(self) -> Side {
        match self {
            Side::North => Side::South,
            Side::East => Side::West,
            Side::South => Side::North,
            Side::West => Side::East,
        }
    }

    /// Grid offset `[dgx, dgz]` from a tile to the neighbour on this side.
    pub fn step(self) -> [i32; 2] {
        match self {
            Side::North => [0, -1],
            Side::East => [1, 0],
            Side::South => [0, 1],
            Side::West => [-1, 0],
        }
    }

    /// The declared `Edge` on this side of `borders`.
    pub fn edge(self, borders: &Borders) -> &Edge {
        match self {
            Side::North => &borders.north,
            Side::East => &borders.east,
            Side::South => &borders.south,
            Side::West => &borders.west,
        }
    }

    /// Local `(x, z)` point along this edge at distance `s ∈ [0, WIDTH]` from its first corner
    /// (north/south run west→east along `x`; east/west run north→south along `z`).
    pub fn edge_point(self, s: f32) -> [f32; 2] {
        match self {
            Side::North => [s, 0.0],
            Side::East => [WIDTH, s],
            Side::South => [s, WIDTH],
            Side::West => [0.0, s],
        }
    }

    /// Unit inward horizontal normal `[nx, nz]` pointing from this edge into the area's interior.
    pub fn inward_normal(self) -> [f32; 2] {
        match self {
            Side::North => [0.0, 1.0],
            Side::East => [-1.0, 0.0],
            Side::South => [0.0, -1.0],
            Side::West => [1.0, 0.0],
        }
    }

    /// Coordinate axis that varies along this edge (`"x"` for north/south, `"z"` for east/west).
    pub fn along_axis(self) -> &'static str {
        match self {
            Side::North | Side::South => "x",
            Side::East | Side::West => "z",
        }
    }

    /// Human-readable equation of this edge in local metres.
    pub fn equation(self) -> &'static str {
        match self {
            Side::North => "z = 0",
            Side::East => "x = 64",
            Side::South => "z = 64",
            Side::West => "x = 0",
        }
    }
}

/// One sampled column along an area's border edge (as seen by a person, `1.7 m`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BorderSample {
    /// Coordinate along the edge (`0.0..=64.0` m).
    pub s: f32,
    /// Height of the topmost surface (water surface if water is present, else solid ground).
    pub surface_y: f32,
    /// Height of the topmost solid ground (below the water surface when `water.is_some()`).
    pub solid_y: f32,
    /// Material of the topmost surface (`Material::Water` when water is present).
    pub material: Material,
    /// Material of the solid ground (underwater bed material when `water.is_some()`, else `material`).
    pub solid_material: Material,
    /// Water surface height at this station, if water is present.
    pub water: Option<f32>,
}

/// Extracted profile of one edge of an area (`65` stations at `s = 0..=64` m), used both to brief
/// Claude when generating an adjacent area and to verify border continuity.
#[derive(Clone, Debug)]
pub struct BorderProfile {
    pub side: Side,
    pub area_name: String,
    pub samples: Vec<BorderSample>,
    pub declared: Edge,
}

impl BorderProfile {
    /// Extracts the 65-station (`0..=64` m) border profile along `side` of `area`.
    pub fn extract(area: &mut dyn Region, side: Side) -> Result<BorderProfile> {
        let d = area.describe()?;
        let samples = sample_edge(area, side, WIDTH as usize)?;
        Ok(BorderProfile { side, area_name: d.name, samples, declared: side.edge(&d.borders).clone() })
    }

    /// Ground heights at each integer metre `0..=64` (top solid ground, including underwater beds).
    pub fn solid_heights(&self) -> Vec<f32> {
        self.samples.iter().map(|s| s.solid_y).collect()
    }

    /// Common water level along this edge, if any sampled station has water.
    pub fn water_level(&self) -> Option<f32> {
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for s in &self.samples {
            if let Some(w) = s.water {
                sum += w;
                count += 1;
            }
        }
        if count > 0 { Some(sum / count as f32) } else { self.declared.water }
    }

    /// Formats this neighbour's edge as instructions + a copy-pasteable Rust height table for the
    /// new area's matching edge `my_side` (`my_side == self.side.opposite()`).
    pub fn describe_for_prompt(&self, my_side: Side) -> String {
        let axis = my_side.along_axis();
        let const_name = format!("{}_GROUND", my_side.name().to_ascii_uppercase());
        let mut out = format!(
            "### {} edge (`{}`) — meets **{}** (its {} edge)\n\n",
            capitalize(my_side.name()),
            my_side.equation(),
            self.area_name,
            self.side.name()
        );

        // Copy-pasteable 65-element ground height array.
        out.push_str(&format!(
            "Solid ground heights every metre along `{axis} = 0..=64` (use `edge_height(&{const_name}, p.{axis})` to interpolate cleanly):\n\n```rust\nconst {const_name}: [f32; 65] = [\n"
        ));
        for row in self.samples.chunks(11) {
            out.push_str("    ");
            for (i, s) in row.iter().enumerate() {
                if i > 0 {
                    out.push(' ');
                }
                out.push_str(&format!("{:.3},", s.solid_y));
            }
            out.push('\n');
        }
        out.push_str("];\n```\n\n");

        // Water summary.
        let water_runs = collect_runs(&self.samples, |s| s.water.is_some());
        if water_runs.is_empty() {
            out.push_str("- **Water along this edge**: none (keep dry land along this edge).\n");
        } else {
            let level = self.water_level().unwrap_or(0.0);
            let spans: Vec<String> = water_runs
                .iter()
                .map(|&(a, b)| format!("{axis} = {:.0}..{:.0} m", self.samples[a].s, self.samples[b].s))
                .collect();
            out.push_str(&format!(
                "- **Water along this edge**: water level `y = {level:.3}` m across {} (match this water level within 5 cm and continue the shoreline smoothly).\n",
                spans.join(", ")
            ));
        }

        // Material runs along the edge.
        let mut mat_runs: Vec<(usize, usize, Material, Material)> = Vec::new();
        for (i, s) in self.samples.iter().enumerate() {
            if let Some(last) = mat_runs.last_mut()
                && last.2 == s.material
                && last.3 == s.solid_material
            {
                last.1 = i;
            } else {
                mat_runs.push((i, i, s.material, s.solid_material));
            }
        }
        let mat_desc: Vec<String> = mat_runs
            .iter()
            .map(|&(a, b, m, sm)| {
                let range = if a == b {
                    format!("{axis} = {:.0} m", self.samples[a].s)
                } else {
                    format!("{axis} = {:.0}..{:.0} m", self.samples[a].s, self.samples[b].s)
                };
                if m == Material::Water {
                    format!("{range}: water (bed: {})", sm.name())
                } else {
                    format!("{range}: {}", m.name())
                }
            })
            .collect();
        out.push_str(&format!("- **Surface materials along this edge**: {}.\n", mat_desc.join("; ")));
        out.push_str(&format!(
            "- **Blend smoothly**: match these heights (within 15 cm), water level (within 5 cm), and materials right along `{}`, \
             fade into your interior terrain over 4–8 m, and keep walls, cliffs, or steep obstacles at least 3 m back from the seam.\n\n",
            my_side.equation()
        ));
        out
    }
}

/// Samples `steps + 1` evenly spaced columns from `s = 0.0` to `s = WIDTH` along `side` of `area`
/// (inset `0.05 m` into the area along `inward_normal`) at `Size::Person` (`1.7 m`), finding both
/// top surface and underwater solid ground height + materials.
pub fn sample_edge(area: &mut dyn Region, side: Side, steps: usize) -> Result<Vec<BorderSample>> {
    let n = steps + 1;
    let size = SIZES[1].1;
    let close = 0.015f32;
    let least = 0.04f32;
    let inset = 0.05f32;
    let normal = side.inward_normal();

    let coords: Vec<(f32, [f32; 2])> = (0..n)
        .map(|i| {
            let s = WIDTH * (i as f32 / steps as f32);
            let p = side.edge_point(s);
            (s, [p[0] + normal[0] * inset, p[1] + normal[1] * inset])
        })
        .collect();

    // Pass 1: march straight down from HIGHEST to find the first surface (solid or water).
    let mut y = vec![HIGHEST; n];
    let mut prev_y = vec![HIGHEST; n];
    let mut hit: Vec<Option<(f32, Material)>> = vec![None; n];
    let mut active: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    for _ in 0..400 {
        if active.is_empty() {
            break;
        }
        let pts: Vec<[f32; 3]> = active.iter().map(|&k| [coords[k].1[0], y[k], coords[k].1[1]]).collect();
        chunk::sample(area, &pts, size, &mut out)?;
        let mut still = Vec::with_capacity(active.len());
        for (j, &k) in active.iter().enumerate() {
            let a = out[j];
            if a.distance <= close {
                hit[k] = Some((y[k], a.material));
            } else {
                prev_y[k] = y[k];
                y[k] -= a.distance.max(least);
                if y[k] >= LOWEST {
                    still.push(k);
                }
            }
        }
        active = still;
    }

    // Refine first-hit heights with 10 bisection steps between `prev_y` (air) and `y - close` (solid).
    let mut refine: Vec<(usize, f32, f32, Material)> = hit
        .iter()
        .enumerate()
        .filter_map(|(k, h)| h.map(|(hy, m)| (k, prev_y[k], hy - close, m)))
        .collect();
    for _ in 0..10 {
        if refine.is_empty() {
            break;
        }
        let pts: Vec<[f32; 3]> = refine.iter().map(|&(k, hi, lo, _)| [coords[k].1[0], (hi + lo) * 0.5, coords[k].1[1]]).collect();
        chunk::sample(area, &pts, size, &mut out)?;
        for (j, r) in refine.iter_mut().enumerate() {
            let mid = (r.1 + r.2) * 0.5;
            if out[j].distance <= 0.0 {
                r.2 = mid;
                r.3 = out[j].material;
            } else {
                r.1 = mid;
            }
        }
    }
    for (k, hi, lo, m) in refine {
        hit[k] = Some(((hi + lo) * 0.5, m));
    }

    // Pass 2: for columns that hit Water, march down through the water to locate the solid floor.
    let mut solid_y: Vec<f32> = hit.iter().map(|h| h.map_or(0.0, |(hy, _)| hy)).collect();
    let mut solid_mat: Vec<Material> = hit.iter().map(|h| h.map_or(Material::Soil, |(_, m)| m)).collect();
    let mut water_cols: Vec<(usize, f32)> = hit
        .iter()
        .enumerate()
        .filter_map(|(k, h)| match h {
            Some((wy, Material::Water)) => Some((k, *wy - 0.04)),
            _ => None,
        })
        .collect();
    for _ in 0..300 {
        if water_cols.is_empty() {
            break;
        }
        let pts: Vec<[f32; 3]> = water_cols.iter().map(|&(k, cy)| [coords[k].1[0], cy, coords[k].1[1]]).collect();
        chunk::sample(area, &pts, size, &mut out)?;
        let mut still = Vec::with_capacity(water_cols.len());
        for (j, &(k, cy)) in water_cols.iter().enumerate() {
            let a = out[j];
            if a.material != Material::Water && a.distance <= close {
                solid_y[k] = cy;
                solid_mat[k] = a.material;
            } else {
                let next_y = cy - 0.05;
                if next_y >= LOWEST {
                    still.push((k, next_y));
                }
            }
        }
        water_cols = still;
    }

    Ok((0..n)
        .map(|k| {
            let (surface_y, material) = hit[k].unwrap_or((0.0, Material::Soil));
            let water = (material == Material::Water).then_some(surface_y);
            BorderSample {
                s: coords[k].0,
                surface_y,
                solid_y: solid_y[k],
                material,
                solid_material: solid_mat[k],
                water,
            }
        })
        .collect())
}

fn collect_runs<T>(items: &[T], pred: impl Fn(&T) -> bool) -> Vec<(usize, usize)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (i, item) in items.iter().enumerate() {
        if pred(item) {
            if start.is_none() {
                start = Some(i);
            }
        } else if let Some(a) = start.take() {
            runs.push((a, i - 1));
        }
    }
    if let Some(a) = start {
        runs.push((a, items.len() - 1));
    }
    runs
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) => c.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

/// One compiled area placed on the atlas tile grid.
#[derive(Clone)]
pub struct TileSpec {
    pub at: [i32; 2],
    pub seed: u64,
    pub compiled: Arc<Compiled>,
}

/// A compiled multi-area atlas ready to be instantiated per worker thread and physics loop.
#[derive(Clone)]
pub struct CompiledAtlas {
    tiles: Vec<TileSpec>,
}

impl CompiledAtlas {
    /// A single area at grid tile `(0, 0)`.
    pub fn single(compiled: Arc<Compiled>, seed: u64) -> CompiledAtlas {
        CompiledAtlas { tiles: vec![TileSpec { at: [0, 0], seed, compiled }] }
    }

    /// A grid of compiled areas; rejects empty lists or duplicate grid coordinates.
    pub fn new(tiles: Vec<TileSpec>) -> Result<CompiledAtlas> {
        if tiles.is_empty() {
            bail!("atlas must contain at least one area tile");
        }
        let mut seen = HashMap::new();
        for t in &tiles {
            if seen.insert(t.at, ()).is_some() {
                bail!("duplicate area tile at grid coordinate ({}, {})", t.at[0], t.at[1]);
            }
        }
        Ok(CompiledAtlas { tiles })
    }

    pub fn len(&self) -> usize {
        self.tiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }

    pub fn specs(&self) -> &[TileSpec] {
        &self.tiles
    }

    pub fn tile_coords(&self) -> Vec<[i32; 2]> {
        self.tiles.iter().map(|t| t.at).collect()
    }

    /// Instantiates every tile's Wasmtime module into an `Atlas<Wasm>`.
    pub fn instantiate(&self) -> Result<Atlas<Wasm>> {
        let mut live = Vec::with_capacity(self.tiles.len());
        for t in &self.tiles {
            live.push((t.at, t.compiled.instantiate(t.seed)?));
        }
        Atlas::new(live)
    }
}

/// One live area tile inside an `Atlas`.
pub struct AtlasTile<R> {
    pub at: [i32; 2],
    pub owner: u32,
    pub region: R,
}

impl<R> AtlasTile<R> {
    /// World-space translation transform of this tile (`[gx * 64, 0, gz * 64]`).
    pub fn transform(&self) -> PlacementTransform {
        tile_transform(self.at)
    }

    /// World-space origin `(x0, z0)` of this tile's northwest corner.
    pub fn origin(&self) -> [f32; 2] {
        [self.at[0] as f32 * WIDTH, self.at[1] as f32 * WIDTH]
    }
}

/// World-space `PlacementTransform` of grid tile `[gx, gz]`.
pub fn tile_transform(at: [i32; 2]) -> PlacementTransform {
    PlacementTransform::new([at[0] as f32 * WIDTH, 0.0, at[1] as f32 * WIDTH], 0.0, 1.0)
}

/// One or more 64 m × 64 m `Region`s stitched side by side on an integer tile grid with smooth C¹
/// seam blending across every shared border.
pub struct Atlas<R> {
    tiles: Vec<AtlasTile<R>>,
    grid: HashMap<[i32; 2], usize>,
    seam_half: f32,
}

impl<R> Atlas<R> {
    /// Wraps a single area at grid `(0, 0)`.
    pub fn single(region: R) -> Atlas<R> {
        let mut grid = HashMap::new();
        grid.insert([0, 0], 0);
        Atlas { tiles: vec![AtlasTile { at: [0, 0], owner: tile_owner(0), region }], grid, seam_half: SEAM_HALF }
    }

    /// Creates an atlas from `(grid_coord, region)` pairs.
    pub fn new(tiles: Vec<([i32; 2], R)>) -> Result<Atlas<R>> {
        if tiles.is_empty() {
            bail!("atlas must contain at least one area tile");
        }
        let mut grid = HashMap::with_capacity(tiles.len());
        let mut built = Vec::with_capacity(tiles.len());
        for (idx, (at, region)) in tiles.into_iter().enumerate() {
            if grid.insert(at, idx).is_some() {
                bail!("duplicate area tile at grid coordinate ({}, {})", at[0], at[1]);
            }
            built.push(AtlasTile { at, owner: tile_owner(idx), region });
        }
        Ok(Atlas { tiles: built, grid, seam_half: SEAM_HALF })
    }

    /// Overrides the border seam half-width (in metres); `0.0` disables blending.
    pub fn with_seam_half(mut self, seam_half: f32) -> Self {
        self.seam_half = if seam_half > 0.0 { seam_half } else { 0.0 };
        self
    }

    pub fn tiles(&self) -> &[AtlasTile<R>] {
        &self.tiles
    }

    pub fn tiles_mut(&mut self) -> &mut [AtlasTile<R>] {
        &mut self.tiles
    }

    pub fn tile_coords(&self) -> Vec<[i32; 2]> {
        self.tiles.iter().map(|t| t.at).collect()
    }

    pub fn seam_half(&self) -> f32 {
        self.seam_half
    }

    /// Index of the tile containing world `(x, z)`, or `None` if outside all tiles.
    pub fn tile_index_at(&self, x: f64, z: f64) -> Option<usize> {
        let gx = (x / WIDTH as f64).floor() as i32;
        let gz = (z / WIDTH as f64).floor() as i32;
        self.grid.get(&[gx, gz]).copied()
    }

    /// Index of the tile nearest to `(x, z)` in world space.
    fn nearest_tile(&self, x: f32, z: f32) -> usize {
        let mut best_idx = 0;
        let mut best_d2 = f32::INFINITY;
        for (idx, tile) in self.tiles.iter().enumerate() {
            let ox = tile.at[0] as f32 * WIDTH;
            let oz = tile.at[1] as f32 * WIDTH;
            let dx = if x < ox {
                ox - x
            } else if x > ox + WIDTH {
                x - (ox + WIDTH)
            } else {
                0.0
            };
            let dz = if z < oz {
                oz - z
            } else if z > oz + WIDTH {
                z - (oz + WIDTH)
            } else {
                0.0
            };
            let d2 = dx * dx + dz * dz;
            if d2 < best_d2 {
                best_d2 = d2;
                best_idx = idx;
            }
        }
        best_idx
    }

    fn is_single_origin(&self) -> bool {
        self.tiles.len() == 1 && self.tiles[0].at == [0, 0]
    }
}

impl<R: Region> Atlas<R> {
    /// Extracts each tile's `(owner, transform, local_scene)` for loading into `SceneWorld` and the renderer.
    pub fn scenes(&mut self) -> Result<Vec<(u32, PlacementTransform, Scene)>> {
        let mut out = Vec::with_capacity(self.tiles.len());
        for tile in &mut self.tiles {
            let tf = tile.transform();
            let sc = tile.region.scene()?;
            out.push((tile.owner, tf, sc));
        }
        Ok(out)
    }

    /// Extracts each tile's `(owner, world_scene)` with all instances and lights shifted into world space,
    /// while also registering each tile's local scene into `world` under its isolated owner namespace.
    pub fn load_scenes_into(&mut self, world: &mut SceneWorld) -> Result<Vec<(u32, Scene)>> {
        let mut out = Vec::with_capacity(self.tiles.len());
        for tile in &mut self.tiles {
            let tf = tile.transform();
            let local = tile.region.scene()?;
            world.add_layer(tile.owner, &local, Some(tf));
            let world_scene = Scene {
                meshes: local.meshes,
                instances: local.instances.into_iter().map(|i| tf.transform_instance(i)).collect(),
                lights: local.lights.into_iter().map(|l| tf.transform_light(l)).collect(),
            };
            out.push((tile.owner, world_scene));
        }
        Ok(out)
    }

    /// Describes every tile individually as `([gx, gz], Description)`.
    pub fn describe_tiles(&mut self) -> Result<Vec<([i32; 2], Description)>> {
        let mut out = Vec::with_capacity(self.tiles.len());
        for tile in &mut self.tiles {
            out.push((tile.at, tile.region.describe()?));
        }
        Ok(out)
    }

    /// Ticks every area tile's sandboxed `tick()` hook with `PlayerState` shifted into each tile's
    /// local `[0, 64]` coordinates, applying returned commands to `world` under each tile's owner.
    pub fn tick_all(&mut self, dt: f32, world_ps: &PlayerState, hit_owner: u32, hit_id: u32, world: &mut SceneWorld) -> Vec<Applied> {
        let mut effects = Vec::new();
        for tile in &mut self.tiles {
            let tf = tile.transform();
            let local_ps = to_tile_player_state(world_ps, tile.at, if hit_owner == tile.owner { hit_id } else { 0 });
            match tile.region.tick(dt, &local_ps) {
                Ok(cmds) if !cmds.is_empty() => {
                    effects.push(world.apply_for(tile.owner, &cmds, Some(tf)));
                }
                Ok(_) => {}
                Err(e) => eprintln!("area tile {:?} tick error: {e}", tile.at),
            }
        }
        effects
    }

    /// Invokes `interact()` on the area tile targeted by `hit_owner` (or the tile the player is
    /// currently standing in when `hit_id == 0`), applying commands under that tile's owner.
    pub fn interact_tile(&mut self, world_ps: &PlayerState, hit_owner: u32, hit_id: u32, world: &mut SceneWorld) -> Option<Applied> {
        let idx = if hit_id > 0 {
            self.tiles.iter().position(|t| t.owner == hit_owner)?
        } else {
            self.tile_index_at(world_ps.feet[0] as f64, world_ps.feet[2] as f64)
                .unwrap_or_else(|| self.nearest_tile(world_ps.feet[0], world_ps.feet[2]))
        };
        let tile = &mut self.tiles[idx];
        let tf = tile.transform();
        let local_ps = to_tile_player_state(world_ps, tile.at, if hit_owner == tile.owner { hit_id } else { 0 });
        match tile.region.interact(&local_ps) {
            Ok(cmds) if !cmds.is_empty() => Some(world.apply_for(tile.owner, &cmds, Some(tf))),
            Ok(_) => None,
            Err(e) => {
                eprintln!("area tile {:?} interact error: {e}", tile.at);
                None
            }
        }
    }
}

fn to_tile_player_state(ps: &PlayerState, at: [i32; 2], target: u32) -> PlayerState {
    let ox = at[0] as f32 * WIDTH;
    let oz = at[1] as f32 * WIDTH;
    PlayerState {
        feet: [ps.feet[0] - ox, ps.feet[1], ps.feet[2] - oz],
        eye: [ps.eye[0] - ox, ps.eye[1], ps.eye[2] - oz],
        look: ps.look,
        velocity: ps.velocity,
        size: ps.size,
        grounded: ps.grounded,
        holding_use: ps.holding_use,
        target,
        aim: [ps.aim[0] - ox, ps.aim[1], ps.aim[2] - oz],
        time: ps.time,
    }
}

/// Deterministic C¹ smoothstep matching `plateau_sdk::ease`.
#[inline]
fn ease_f32(t: f32) -> f32 {
    let c = if t < 0.0 {
        0.0
    } else if t > 1.0 {
        1.0
    } else {
        t
    };
    c * c * (3.0 - 2.0 * c)
}

/// Up to 4 tile contributions `(tile_idx, sample_idx_in_tile_batch, weight)` for one world point.
#[derive(Clone, Copy)]
struct BlendPlan {
    len: u8,
    entries: [(u16, u32, f32); 4],
}

impl<R: Region> Region for Atlas<R> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        if self.is_single_origin() {
            return self.tiles[0].region.sample(points, size, out);
        }
        let n_tiles = self.tiles.len();
        let mut tile_pts: Vec<Vec<[f32; 3]>> = (0..n_tiles).map(|_| Vec::new()).collect();
        let mut plans: Vec<BlendPlan> = Vec::with_capacity(points.len());
        let w = self.seam_half;
        let inv_2w = if w > 0.0 { 1.0 / (2.0 * w) } else { 0.0 };

        for &[px, py, pz] in points {
            let gx = (px / WIDTH).floor() as i32;
            let gz = (pz / WIDTH).floor() as i32;
            let mut plan = BlendPlan { len: 0, entries: [(0, 0, 0.0); 4] };

            let push_contrib = |plan: &mut BlendPlan, tile_pts: &mut [Vec<[f32; 3]>], tile_idx: usize, tile_at: [i32; 2], weight: f32| {
                let sample_idx = tile_pts[tile_idx].len() as u32;
                tile_pts[tile_idx].push([px - tile_at[0] as f32 * WIDTH, py, pz - tile_at[1] as f32 * WIDTH]);
                let k = plan.len as usize;
                plan.entries[k] = (tile_idx as u16, sample_idx, weight);
                plan.len += 1;
            };

            if let Some(&primary) = self.grid.get(&[gx, gz]) {
                let lx = px - gx as f32 * WIDTH;
                let lz = pz - gz as f32 * WIDTH;

                let (dx, wx_nb) = if w > 0.0 && lx < w && self.grid.contains_key(&[gx - 1, gz]) {
                    (-1, 1.0 - ease_f32((lx + w) * inv_2w))
                } else if w > 0.0 && lx > WIDTH - w && self.grid.contains_key(&[gx + 1, gz]) {
                    (1, ease_f32((lx - (WIDTH - w)) * inv_2w))
                } else {
                    (0, 0.0)
                };

                let (dz, wz_nb) = if w > 0.0 && lz < w && self.grid.contains_key(&[gx, gz - 1]) {
                    (-1, 1.0 - ease_f32((lz + w) * inv_2w))
                } else if w > 0.0 && lz > WIDTH - w && self.grid.contains_key(&[gx, gz + 1]) {
                    (1, ease_f32((lz - (WIDTH - w)) * inv_2w))
                } else {
                    (0, 0.0)
                };

                let w00 = (1.0 - wx_nb) * (1.0 - wz_nb);
                let w10 = wx_nb * (1.0 - wz_nb);
                let w01 = (1.0 - wx_nb) * wz_nb;
                let w11 = wx_nb * wz_nb;

                let diag = if dx != 0 && dz != 0 { self.grid.get(&[gx + dx, gz + dz]).copied() } else { None };
                let norm = if dx != 0 && dz != 0 && diag.is_none() { 1.0 / (w00 + w10 + w01) } else { 1.0 };

                push_contrib(&mut plan, &mut tile_pts, primary, [gx, gz], w00 * norm);
                if dx != 0 {
                    let nb = self.grid[&[gx + dx, gz]];
                    push_contrib(&mut plan, &mut tile_pts, nb, [gx + dx, gz], w10 * norm);
                }
                if dz != 0 {
                    let nb = self.grid[&[gx, gz + dz]];
                    push_contrib(&mut plan, &mut tile_pts, nb, [gx, gz + dz], w01 * norm);
                }
                if let Some(d_idx) = diag {
                    push_contrib(&mut plan, &mut tile_pts, d_idx, [gx + dx, gz + dz], w11);
                }
            } else {
                let nearest = self.nearest_tile(px, pz);
                let at = self.tiles[nearest].at;
                push_contrib(&mut plan, &mut tile_pts, nearest, at, 1.0);
            }
            plans.push(plan);
        }

        // Dispatch one batch per tile that received points.
        let mut tile_answers: Vec<Vec<Answer>> = (0..n_tiles).map(|_| Vec::new()).collect();
        for (idx, pts) in tile_pts.iter().enumerate() {
            if !pts.is_empty() {
                chunk::sample(&mut self.tiles[idx].region, pts, size, &mut tile_answers[idx])?;
            }
        }

        // Reconstruct combined answers deterministically.
        out.clear();
        out.reserve(points.len());
        for plan in plans {
            if plan.len == 1 {
                let (ti, si, _) = plan.entries[0];
                out.push(tile_answers[ti as usize][si as usize]);
                continue;
            }
            let mut dist = 0.0f32;
            let primary_ans = {
                let (ti, si, _) = plan.entries[0];
                tile_answers[ti as usize][si as usize]
            };
            let mut best_solid_w = -1.0f32;
            let mut solid_mat = primary_ans.material;
            for k in 0..plan.len as usize {
                let (ti, si, weight) = plan.entries[k];
                let ans = tile_answers[ti as usize][si as usize];
                dist += weight * ans.distance;
                if ans.distance < 0.0 && weight > best_solid_w {
                    best_solid_w = weight;
                    solid_mat = ans.material;
                }
            }
            let material = if dist < 0.0 && best_solid_w >= 0.0 { solid_mat } else { primary_ans.material };
            out.push(Answer { distance: dist, material });
        }
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        if self.is_single_origin() {
            return self.tiles[0].region.describe();
        }
        let mut names = Vec::with_capacity(self.tiles.len());
        let mut places = Vec::new();
        let mut first_borders = None;
        for tile in &mut self.tiles {
            let d = tile.region.describe()?;
            let ox = tile.at[0] as f32 * WIDTH;
            let oz = tile.at[1] as f32 * WIDTH;
            names.push(d.name);
            for p in d.places {
                places.push(Place { name: p.name, size: p.size, at: [p.at[0] + ox, p.at[1], p.at[2] + oz] });
            }
            if first_borders.is_none() {
                first_borders = Some(d.borders);
            }
        }
        Ok(Description { name: names.join(" + "), places, borders: first_borders.unwrap() })
    }

    fn scene(&mut self) -> Result<Scene> {
        if self.is_single_origin() {
            return self.tiles[0].region.scene();
        }
        // Multi-tile scenes are loaded per-owner via `Atlas::scenes()` so each tile keeps its own
        // isolated dynamic IDs, lights, and `tick` / `interact` namespace.
        Ok(Scene::default())
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Result<Vec<crate::Command>> {
        if self.is_single_origin() {
            return self.tiles[0].region.tick(dt, player);
        }
        Ok(Vec::new())
    }

    fn interact(&mut self, player: &PlayerState) -> Result<Vec<crate::Command>> {
        if self.is_single_origin() {
            return self.tiles[0].region.interact(player);
        }
        Ok(Vec::new())
    }

    fn element_bounds(&mut self) -> Result<Option<ElementBounds>> {
        if self.is_single_origin() {
            return self.tiles[0].region.element_bounds();
        }
        Ok(None)
    }
}
