//! Material physics table, tool-material interaction capabilities, and Archimedes buoyancy
//! for floating & navigable procedural `Scene` objects (Phase B; AGENTS.md §1.3 & §2).
//!
//! Zero prefab enums, zero WIT changes, zero renderer dependencies:
//! - Every `Material` has deterministic physical properties (`density_kg_m3`, `hardness`,
//!   `fibrous`, `earthy`, `buoyant`).
//! - `Tool` (`All`, `Saw`, `Pickaxe`, `Shovel`, `Hands`) validates cutting (`K`) and sculpting (`B`)
//!   strictly against the aimed surface's `Material` (e.g. a Saw cuts wooden trees, bridges, and
//!   boats, but refuses Stone; a Pickaxe cuts/carves Stone & earth; a Shovel digs earth/mud/sand).
//! - `compute_buoyancy` and `step_floating_move` compute deterministic Archimedes displacement on
//!   **any** discrete `Scene` element (wooden boats, felled oak logs, stone cairns) directly against
//!   `Region` (`Composed<Atlas, ComposedLog>`), so boats and logs automatically float and sail across
//!   **both** base area ponds and Phase A dynamically expanded pond channels, while stones sink and
//!   oversized riders (e.g. a Giant stepping on a person-scale boat) swamp the hull!

use std::sync::Arc;

use crate::chunk;
use crate::compose::{PlacementTransform, quat_mul, quat_rotate};
use crate::cut::{ObjectPhysics, format_mass};
use crate::edit::{Edit, Shape};
use crate::mesh_sdf::MeshSdf;
use crate::scene::SceneWorld;
use crate::{Answer, Material, Region, Result, SIZES, math};

const PERSON_HEIGHT: f32 = SIZES[1].1;
/// Water density in kg/m³.
pub const WATER_DENSITY: f32 = 1000.0;

/// Deterministic physical properties of one engine `Material`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MaterialProps {
    /// Physical bulk density in kg/m³ (`< 1000.0` floats naturally in water).
    pub density_kg_m3: f32,
    /// Relative hardness (`0` liquid, `1` soft foliage, `2` loose earth, `3` fibrous wood, `4` stone).
    pub hardness: u8,
    /// Fibrous organic material cuttable by a Saw (`Wood`, `Grass`, `Leaf`, `Moss`).
    pub fibrous: bool,
    /// Loose earth/granules diggable by a Shovel (`Soil`, `Mud`, `Sand`, `Grass`, `Moss`).
    pub earthy: bool,
    /// Naturally buoyant (`density_kg_m3 < WATER_DENSITY`).
    pub buoyant: bool,
}

/// Returns the deterministic physical properties of `mat`.
pub const fn material_props(mat: Material) -> MaterialProps {
    match mat {
        Material::Wood => MaterialProps { density_kg_m3: 600.0, hardness: 3, fibrous: true, earthy: false, buoyant: true },
        Material::Stone => MaterialProps { density_kg_m3: 2200.0, hardness: 4, fibrous: false, earthy: false, buoyant: false },
        Material::Soil => MaterialProps { density_kg_m3: 1400.0, hardness: 2, fibrous: false, earthy: true, buoyant: false },
        Material::Mud => MaterialProps { density_kg_m3: 1450.0, hardness: 2, fibrous: false, earthy: true, buoyant: false },
        Material::Sand => MaterialProps { density_kg_m3: 1500.0, hardness: 2, fibrous: false, earthy: true, buoyant: false },
        Material::Water => MaterialProps { density_kg_m3: WATER_DENSITY, hardness: 0, fibrous: false, earthy: false, buoyant: false },
        Material::Grass => MaterialProps { density_kg_m3: 180.0, hardness: 1, fibrous: true, earthy: true, buoyant: true },
        Material::Leaf => MaterialProps { density_kg_m3: 160.0, hardness: 1, fibrous: true, earthy: false, buoyant: true },
        Material::Moss => MaterialProps { density_kg_m3: 190.0, hardness: 1, fibrous: true, earthy: true, buoyant: true },
    }
}

/// Scale-proportional body mass in kg (`75 kg` for a `1.7 m` person, scaling with `size^3`).
pub fn player_mass_kg(player_size: f32) -> f32 {
    let r = max_f32(player_size / PERSON_HEIGHT, 1e-4);
    75.0 * r * r * r
}

/// Active player tool governing material-aware cutting (`K`) and terrain/object carving (`B`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Tool {
    /// Workshop sandbox tool: cuts & carves every solid material.
    #[default]
    All,
    /// Wood & foliage saw: cuts trees, wooden bridges, logs, and boats; refuses Stone.
    Saw,
    /// Stone pickaxe/chisel: splits and carves Stone and earth; refuses fibrous Wood.
    Pickaxe,
    /// Earth shovel: digs Soil, Mud, Sand, Grass, and Moss; refuses Stone and Wood.
    Shovel,
    /// Bare hands: grab/carry/interact only; cannot cut or carve solid terrain/objects.
    Hands,
}

impl Tool {
    pub const ALL: [Tool; 5] = [Tool::All, Tool::Saw, Tool::Pickaxe, Tool::Shovel, Tool::Hands];

    pub fn name(self) -> &'static str {
        match self {
            Tool::All => "workshop (all)",
            Tool::Saw => "saw (wood)",
            Tool::Pickaxe => "pickaxe (stone/earth)",
            Tool::Shovel => "shovel (earth)",
            Tool::Hands => "hands (carry only)",
        }
    }

    pub fn short_name(self) -> &'static str {
        match self {
            Tool::All => "Workshop",
            Tool::Saw => "Saw",
            Tool::Pickaxe => "Pickaxe",
            Tool::Shovel => "Shovel",
            Tool::Hands => "Hands",
        }
    }

    /// Stable lowercase token used in `plateau-world 2` save files (`tool <name>`).
    pub fn save_name(self) -> &'static str {
        match self {
            Tool::All => "all",
            Tool::Saw => "saw",
            Tool::Pickaxe => "pickaxe",
            Tool::Shovel => "shovel",
            Tool::Hands => "hands",
        }
    }

    pub fn from_name(s: &str) -> Option<Tool> {
        match s {
            "all" | "workshop" => Some(Tool::All),
            "saw" => Some(Tool::Saw),
            "pickaxe" => Some(Tool::Pickaxe),
            "shovel" => Some(Tool::Shovel),
            "hands" => Some(Tool::Hands),
            _ => None,
        }
    }

    pub fn next(self) -> Tool {
        let i = Tool::ALL.iter().position(|&t| t == self).unwrap_or(0);
        Tool::ALL[(i + 1) % Tool::ALL.len()]
    }

    /// Checks whether this tool can place/raise an SDF volume of material `mat` (`Op::Add`).
    pub fn can_add(self, mat: Material) -> std::result::Result<(), String> {
        let p = material_props(mat);
        match self {
            Tool::All => Ok(()),
            Tool::Saw => {
                if p.fibrous {
                    Ok(())
                } else {
                    let hint = if mat == Material::Stone { "Pickaxe" } else { "Shovel" };
                    Err(format!("Saw cannot place {} — switch to {hint}", mat.name()))
                }
            }
            Tool::Pickaxe => {
                if mat == Material::Stone || p.earthy {
                    Ok(())
                } else {
                    Err(format!("Pickaxe cannot place {} — switch to Saw", mat.name()))
                }
            }
            Tool::Shovel => {
                if p.earthy {
                    Ok(())
                } else {
                    let hint = if mat == Material::Stone { "Pickaxe" } else { "Saw" };
                    Err(format!("Shovel cannot place {} — switch to {hint}", mat.name()))
                }
            }
            Tool::Hands => Err("Bare hands cannot place terrain — switch tool".into()),
        }
    }

    /// Checks whether this tool can chop/split a discrete object made of `mat`.
    pub fn can_cut(self, mat: Material) -> std::result::Result<(), String> {
        let p = material_props(mat);
        match self {
            Tool::All => Ok(()),
            Tool::Saw => {
                if p.fibrous {
                    Ok(())
                } else {
                    Err(format!("Saw cannot cut {} — switch to Pickaxe", mat.name()))
                }
            }
            Tool::Pickaxe => {
                if mat == Material::Stone || p.earthy {
                    Ok(())
                } else {
                    Err(format!("Pickaxe cannot cleanly cut {} — switch to Saw", mat.name()))
                }
            }
            Tool::Shovel => {
                if p.earthy {
                    Ok(())
                } else {
                    let hint = if mat == Material::Stone { "Pickaxe" } else { "Saw" };
                    Err(format!("Shovel cannot cut {} — switch to {hint}", mat.name()))
                }
            }
            Tool::Hands => Err("Bare hands cannot cut — switch to Saw or Pickaxe".into()),
        }
    }

    /// Checks whether this tool can carve/excavate an SDF surface of material `mat`.
    pub fn can_carve(self, mat: Material) -> std::result::Result<(), String> {
        if mat == Material::Water {
            return Ok(());
        }
        let p = material_props(mat);
        match self {
            Tool::All => Ok(()),
            Tool::Saw => {
                if p.fibrous {
                    Ok(())
                } else {
                    let hint = if mat == Material::Stone { "Pickaxe" } else { "Shovel" };
                    Err(format!("Saw cannot carve {} — switch to {hint}", mat.name()))
                }
            }
            Tool::Pickaxe => {
                if mat == Material::Stone || p.earthy {
                    Ok(())
                } else {
                    Err(format!("Pickaxe cannot carve {} — switch to Saw", mat.name()))
                }
            }
            Tool::Shovel => {
                if p.earthy {
                    Ok(())
                } else {
                    let hint = if mat == Material::Stone { "Pickaxe" } else { "Saw" };
                    Err(format!("Shovel cannot dig {} — switch to {hint}", mat.name()))
                }
            }
            Tool::Hands => Err("Bare hands cannot carve — switch tool".into()),
        }
    }
}

/// Whether building is free (`Workshop`) or governed by tools and carried materials (`World`)
/// (`WORLD_PLAN.md`, phase H).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum WorldMode {
    /// Sandbox for authors: `Tool::All` allowed, free `Add`, carves may repaint walls.
    #[default]
    Workshop,
    /// Play mode: `Tool::All` refused, carves may not repaint walls, `Add` gated by tool and
    /// inventory, carving yields material (volume × density) into inventory, and carrying weight
    /// counts inventory.
    World,
}

impl WorldMode {
    pub fn name(self) -> &'static str {
        match self {
            WorldMode::Workshop => "workshop",
            WorldMode::World => "world",
        }
    }

    pub fn from_name(s: &str) -> Option<WorldMode> {
        match s {
            "workshop" => Some(WorldMode::Workshop),
            "world" => Some(WorldMode::World),
            _ => None,
        }
    }

    /// Default tool when entering this mode (`Hands` in `world`, `All` in `workshop`).
    pub fn default_tool(self) -> Tool {
        match self {
            WorldMode::Workshop => Tool::All,
            WorldMode::World => Tool::Hands,
        }
    }

    /// Tools available to cycle through in this mode.
    pub fn tools(self) -> &'static [Tool] {
        const WORLD_TOOLS: [Tool; 4] = [Tool::Hands, Tool::Shovel, Tool::Pickaxe, Tool::Saw];
        match self {
            WorldMode::Workshop => &Tool::ALL,
            WorldMode::World => &WORLD_TOOLS,
        }
    }

    pub fn toggle(self) -> WorldMode {
        match self {
            WorldMode::Workshop => WorldMode::World,
            WorldMode::World => WorldMode::Workshop,
        }
    }
}

/// Per-player carried bulk materials in kilograms, indexed by `Material as usize`
/// (`WORLD_PLAN.md`, phase H). Deterministic and bit-exact.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Inventory {
    pub kg: [f32; 9],
}

impl Inventory {
    pub fn new() -> Inventory {
        Inventory::default()
    }

    pub fn get(&self, mat: Material) -> f32 {
        self.kg[mat as usize]
    }

    pub fn total_mass_kg(&self) -> f32 {
        let mut sum = 0.0f32;
        for &m in &self.kg {
            sum += m;
        }
        sum
    }

    pub fn is_empty(&self) -> bool {
        self.kg.iter().all(|&m| m <= 0.0)
    }

    pub fn add(&mut self, mat: Material, kg: f32) {
        if kg.is_finite() && kg > 0.0 {
            self.kg[mat as usize] += kg;
        }
    }

    pub fn take(&mut self, mat: Material, kg: f32) -> std::result::Result<(), String> {
        if !kg.is_finite() || kg < 0.0 {
            return Err("invalid material amount".into());
        }
        let have = self.kg[mat as usize];
        // Tiny relative slack so placing back the exact scoop just dug never fails to float rounding.
        let slack = max_f32(kg * 1e-4, 1e-7);
        if have + slack < kg {
            return Err(format!(
                "not enough {} (have {}, need {}) — dig {} first",
                mat.name(),
                format_mass(have),
                format_mass(kg),
                mat.name()
            ));
        }
        let next = have - kg;
        self.kg[mat as usize] = if next <= 1e-9 { 0.0 } else { next };
        Ok(())
    }

    /// Applies a signed mass delta (used by undo/redo), clamping tiny residuals to zero.
    pub fn apply_delta(&mut self, mat: Material, delta_kg: f32) {
        if !delta_kg.is_finite() {
            return;
        }
        let next = self.kg[mat as usize] + delta_kg;
        self.kg[mat as usize] = if next <= 1e-9 { 0.0 } else { next };
    }

    /// Non-zero `(Material, kg)` entries in `Material::ALL` order.
    pub fn iter(&self) -> impl Iterator<Item = (Material, f32)> + '_ {
        Material::ALL.into_iter().filter_map(|m| {
            let v = self.kg[m as usize];
            (v > 0.0).then_some((m, v))
        })
    }

    /// Human-readable pack summary (e.g. `"soil 120 kg, stone 45 kg"`).
    pub fn summary(&self) -> String {
        let parts: Vec<String> = self.iter().map(|(m, kg)| format!("{} {}", m.name(), format_mass(kg))).collect();
        parts.join(", ")
    }
}

/// Outcome of the whole-brush material check before a carve (`WORLD_PLAN.md`, phase H).
#[derive(Clone, Debug, PartialEq)]
pub enum BrushCheck {
    /// Full brush fits without hitting any disallowed solid material.
    Ok { yield_kg: [f32; 9] },
    /// Brush hit disallowed solid `blocked` and was shrunk to `edit` until only allowed materials remain.
    Shrunk { edit: Edit, blocked: Material, yield_kg: [f32; 9] },
    /// Brush cannot fit at any valid size because of disallowed solid `blocked`.
    Refused { blocked: Material, reason: String },
}

const BRUSH_GRID: usize = 5;
const SHRINK_STEPS: [f32; 4] = [0.75, 0.50, 0.35, 0.25];

/// Combines `sdf` (from `region`) with exact signed distances of any unpromoted `MeshSdf` elements.
fn sample_with_unpromoted(
    p: [f32; 3],
    mut sdf: Answer,
    unpromoted: &[(PlacementTransform, Arc<MeshSdf>)],
) -> Answer {
    for (tf, msdf) in unpromoted {
        let lp = tf.world_to_local_point(p);
        if let Some(ans) = msdf.sample(lp, f32::INFINITY) {
            let d = ans.distance * tf.scale;
            if d < sdf.distance || (sdf.material == Material::Water && d <= 0.0) {
                sdf = Answer { distance: d, material: ans.material };
            }
        }
    }
    sdf
}

/// Evaluates a `5×5×5` cell-centered grid inside `edit`'s shape against `region`, `scene`, and
/// `unpromoted` solid scene elements, returning `(first_disallowed_material, solid_cell_count,
/// yield_kg_per_material)`.
fn probe_carve_volume(
    tool: Tool,
    edit: &Edit,
    region: &mut dyn Region,
    scene: &SceneWorld,
    unpromoted: &[(PlacementTransform, Arc<MeshSdf>)],
) -> Result<(Option<Material>, usize, [f32; 9])> {
    let half = edit.shape.half_extents();
    let mut pts = Vec::with_capacity(BRUSH_GRID * BRUSH_GRID * BRUSH_GRID);
    let mut depths = Vec::with_capacity(BRUSH_GRID * BRUSH_GRID * BRUSH_GRID);
    for iz in 0..BRUSH_GRID {
        let uz = -1.0 + (2.0 * iz as f32 + 1.0) / BRUSH_GRID as f32;
        for iy in 0..BRUSH_GRID {
            let uy = -1.0 + (2.0 * iy as f32 + 1.0) / BRUSH_GRID as f32;
            for ix in 0..BRUSH_GRID {
                let ux = -1.0 + (2.0 * ix as f32 + 1.0) / BRUSH_GRID as f32;
                let p = [
                    edit.center[0] + ux * half[0],
                    edit.center[1] + uy * half[1],
                    edit.center[2] + uz * half[2],
                ];
                let bd = edit.distance(p);
                if bd <= 0.0 {
                    pts.push(p);
                    depths.push(-bd);
                }
            }
        }
    }
    if pts.is_empty() {
        pts.push(edit.center);
        depths.push(edit.shape.radius());
    }
    let mut out = Vec::new();
    chunk::sample(region, &pts, edit.stamp_size, &mut out)?;

    let dx = half[0] / BRUSH_GRID as f32;
    let dy = half[1] / BRUSH_GRID as f32;
    let dz = half[2] / BRUSH_GRID as f32;
    let cell_r = (dx * dx + dy * dy + dz * dz).sqrt();
    let per_cell_vol = edit.shape.volume_m3() / pts.len() as f32;

    let mut blocked: Option<Material> = None;
    let mut solid_count = 0usize;
    let mut yield_kg = [0.0f32; 9];

    for ((&p, &d_in), &raw_ans) in pts.iter().zip(&depths).zip(&out) {
        let ans = sample_with_unpromoted(p, raw_ans, unpromoted);
        let mat = if ans.distance <= 0.0 && ans.material != Material::Water {
            Some(ans.material)
        } else if unpromoted.is_empty() {
            // Fallback for base-area scene solids not in `unpromoted`.
            let margin = min_f32(cell_r, d_in);
            if margin > 0.0 {
                scene.distance_to_solids(p, margin).map(|h| h.material)
            } else {
                None
            }
        } else {
            None
        };
        if let Some(m) = mat {
            solid_count += 1;
            if blocked.is_none() && tool.can_carve(m).is_err() {
                blocked = Some(m);
            }
            yield_kg[m as usize] += per_cell_vol * material_props(m).density_kg_m3;
        }
    }
    Ok((blocked, solid_count, yield_kg))
}

/// Checks the whole brush volume (`5×5×5` grid at `edit.stamp_size`) before carving (`WORLD_PLAN.md`,
/// phase H). If a disallowed solid material lies inside the brush, tries shrinking the brush toward
/// `hit` until only allowed materials remain; if it still cannot fit (or `hit` itself is disallowed),
/// refuses the stroke with a message naming the blocking material.
pub fn check_carve_brush(
    tool: Tool,
    edit: Edit,
    hit: [f32; 3],
    region: &mut dyn Region,
    scene: &SceneWorld,
    unpromoted: &[(PlacementTransform, Arc<MeshSdf>)],
) -> Result<BrushCheck> {
    if tool == Tool::Hands {
        return Ok(BrushCheck::Refused {
            blocked: Material::Stone,
            reason: tool.can_carve(Material::Stone).unwrap_err(),
        });
    }
    if tool == Tool::All {
        let (_, _, yield_kg) = probe_carve_volume(tool, &edit, region, scene, unpromoted)?;
        return Ok(BrushCheck::Ok { yield_kg });
    }

    // 1. Check the aimed point itself first: shrinking around `hit` can never avoid `hit`.
    let r = edit.shape.radius();
    let hit_margin = max_f32(0.05 * r, 0.01 * edit.stamp_size);
    let mut hit_out = Vec::new();
    chunk::sample(region, &[hit], edit.stamp_size, &mut hit_out)?;
    let hit_mat = {
        let sdf = sample_with_unpromoted(hit, hit_out[0], unpromoted);
        if sdf.distance <= 0.0 && sdf.material != Material::Water {
            Some(sdf.material)
        } else if let Some(sh) = scene.distance_to_solids(hit, hit_margin) {
            if sdf.distance <= hit_margin && sdf.distance < sh.distance && sdf.material != Material::Water {
                Some(sdf.material)
            } else {
                Some(sh.material)
            }
        } else if sdf.distance <= hit_margin && sdf.material != Material::Water {
            Some(sdf.material)
        } else {
            None
        }
    };
    if let Some(m) = hit_mat
        && let Err(reason) = tool.can_carve(m)
    {
        return Ok(BrushCheck::Refused { blocked: m, reason });
    }

    // 2. Check the full brush volume.
    let (blocked, _, yield_kg) = probe_carve_volume(tool, &edit, region, scene, unpromoted)?;
    let Some(blocked_mat) = blocked else {
        return Ok(BrushCheck::Ok { yield_kg });
    };

    // 3. Try shrinking the brush toward `hit` until only allowed materials lie inside it.
    let min_radius = max_f32(0.05 * edit.stamp_size, 0.003);
    for &s in &SHRINK_STEPS {
        let shape = edit.shape.scaled(s);
        if shape.radius() < min_radius {
            continue;
        }
        let center = [
            hit[0] + (edit.center[0] - hit[0]) * s,
            hit[1] + (edit.center[1] - hit[1]) * s,
            hit[2] + (edit.center[2] - hit[2]) * s,
        ];
        let shrunk = Edit { shape, center, blend: edit.blend * s, ..edit };
        let (b, solid_n, y_kg) = probe_carve_volume(tool, &shrunk, region, scene, unpromoted)?;
        if b.is_none() && (solid_n > 0 || hit_mat.is_some()) {
            return Ok(BrushCheck::Shrunk { edit: shrunk, blocked: blocked_mat, yield_kg: y_kg });
        }
    }

    let detail = tool.can_carve(blocked_mat).unwrap_err();
    Ok(BrushCheck::Refused {
        blocked: blocked_mat,
        reason: format!("{} stopped at {} inside brush ({detail})", tool.short_name(), blocked_mat.name()),
    })
}

/// Estimates the mass (kg) of `mat` needed to add `edit` into open space (`WORLD_PLAN.md`, phase H):
/// fraction of brush cells currently in air/water × `shape.volume_m3()` × `density_kg_m3`.
pub fn estimate_add_mass_kg(
    edit: &Edit,
    mat: Material,
    region: &mut dyn Region,
    scene: &SceneWorld,
    unpromoted: &[(PlacementTransform, Arc<MeshSdf>)],
) -> Result<f32> {
    let half = edit.shape.half_extents();
    let mut pts = Vec::with_capacity(BRUSH_GRID * BRUSH_GRID * BRUSH_GRID);
    let mut depths = Vec::with_capacity(BRUSH_GRID * BRUSH_GRID * BRUSH_GRID);
    for iz in 0..BRUSH_GRID {
        let uz = -1.0 + (2.0 * iz as f32 + 1.0) / BRUSH_GRID as f32;
        for iy in 0..BRUSH_GRID {
            let uy = -1.0 + (2.0 * iy as f32 + 1.0) / BRUSH_GRID as f32;
            for ix in 0..BRUSH_GRID {
                let ux = -1.0 + (2.0 * ix as f32 + 1.0) / BRUSH_GRID as f32;
                let p = [
                    edit.center[0] + ux * half[0],
                    edit.center[1] + uy * half[1],
                    edit.center[2] + uz * half[2],
                ];
                let bd = edit.distance(p);
                if bd <= 0.0 {
                    pts.push(p);
                    depths.push(-bd);
                }
            }
        }
    }
    if pts.is_empty() {
        pts.push(edit.center);
        depths.push(edit.shape.radius());
    }
    let mut out = Vec::new();
    chunk::sample(region, &pts, edit.stamp_size, &mut out)?;

    let dx = half[0] / BRUSH_GRID as f32;
    let dy = half[1] / BRUSH_GRID as f32;
    let dz = half[2] / BRUSH_GRID as f32;
    let cell_r = (dx * dx + dy * dy + dz * dz).sqrt();

    let mut open_count = 0usize;
    for ((&p, &d_in), &raw_ans) in pts.iter().zip(&depths).zip(&out) {
        let ans = sample_with_unpromoted(p, raw_ans, unpromoted);
        let occupied = (ans.distance <= 0.0 && ans.material != Material::Water)
            || (unpromoted.is_empty() && min_f32(cell_r, d_in) > 0.0 && scene.distance_to_solids(p, min_f32(cell_r, d_in)).is_some());
        if !occupied {
            open_count += 1;
        }
    }
    let vol = edit.shape.volume_m3() * (open_count as f32 / pts.len() as f32);
    Ok(vol * material_props(mat).density_kg_m3)
}

/// Checks whether `edit`'s tool is valid against the world state *without* shrinking (for server /
/// save-replay verification of recorded edits; `WORLD_PLAN.md`, phase H).
pub fn verify_edit_tool(
    mode: WorldMode,
    edit: &Edit,
    region: &mut dyn Region,
    scene: &SceneWorld,
    unpromoted: &[(PlacementTransform, Arc<MeshSdf>)],
) -> Result<std::result::Result<(), String>> {
    if mode == WorldMode::World {
        if edit.tool == Tool::All {
            return Ok(Err("workshop tool (all) is not allowed in world mode".into()));
        }
        if edit.op == crate::edit::Op::Carve && edit.material.is_some() {
            return Ok(Err("carving cannot repaint walls in world mode".into()));
        }
    }
    match edit.op {
        crate::edit::Op::Add => {
            let mat = edit.material.unwrap_or(Material::Soil);
            Ok(edit.tool.can_add(mat))
        }
        crate::edit::Op::Carve => {
            if edit.tool == Tool::All {
                return Ok(Ok(()));
            }
            if edit.tool == Tool::Hands {
                return Ok(Err(edit.tool.can_carve(Material::Stone).unwrap_err()));
            }
            let (blocked, _, _) = probe_carve_volume(edit.tool, edit, region, scene, unpromoted)?;
            match blocked {
                None => Ok(Ok(())),
                Some(m) => Ok(Err(format!(
                    "{} cannot carve {} inside brush ({})",
                    edit.tool.short_name(),
                    m.name(),
                    edit.tool.can_carve(m).unwrap_err()
                ))),
            }
        }
    }
}

/// Computes `Shape::volume_m3` helper validation (sanity check for `Shape`).
#[allow(dead_code)]
fn _shape_uses(_: Shape) {}

/// Deterministic buoyancy & resting result for a discrete `Scene` object at a given world placement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BuoyancyState {
    /// No water directly beneath the object's footprint.
    Dry { floor_y: f32 },
    /// Object is positively buoyant and floats at hydrostatic surface `water_y`.
    Floating {
        water_y: f32,
        bed_y: f32,
        /// Resting `PlacementTransform::pos[1]` so the hull sits at its Archimedes draft
        /// (clamped above `bed_y` in shallow water).
        target_y: f32,
        /// Fraction `[0.15, 0.92]` of the object's vertical height submerged beneath `water_y`.
        submerged_fraction: f32,
        /// Gentle deterministic bobbing amplitude proportional to object scale.
        bob_amplitude: f32,
    },
    /// Object (plus any rider load) exceeds its maximum water displacement and sinks to `bed_y`.
    Sinking {
        water_y: f32,
        bed_y: f32,
        /// Resting `PlacementTransform::pos[1]` seated on the underwater pond bed.
        target_y: f32,
    },
}

impl BuoyancyState {
    pub fn is_floating(&self) -> bool {
        matches!(self, BuoyancyState::Floating { .. })
    }

    pub fn water_y(&self) -> Option<f32> {
        match *self {
            BuoyancyState::Floating { water_y, .. } | BuoyancyState::Sinking { water_y, .. } => Some(water_y),
            BuoyancyState::Dry { .. } => None,
        }
    }
}

/// Computes deterministic Archimedes displacement volume (m³) for a discrete object at `(rot, scale)`.
///
/// Solid objects and branching timber (e.g. felled oak logs, stone pillars) displace their own
/// solid volume (`V_solid`). Upright, low-profile hollow shells made primarily of buoyant material
/// (e.g. a wooden rowboat whose height is less than 65% of its narrower horizontal footprint and
/// whose planks occupy `< 45%` of its bounding box while enclosing an air-filled cockpit) displace
/// up to `55%` of their bounding hull volume — zero prefab tags required!
pub fn effective_displacement_m3(phys: &ObjectPhysics, rot: [f32; 4], scale: f32) -> f32 {
    let s3 = scale * scale * scale;
    let solid_vol = max_f32(phys.local_volume_m3 * s3, 1e-9);
    // Compute world-oriented AABB extents so capsized or cylindrical logs don't get hull boost.
    let mut wlo = [f32::INFINITY; 3];
    let mut whi = [f32::NEG_INFINITY; 3];
    for sx in [phys.local_lo[0], phys.local_hi[0]] {
        for sy in [phys.local_lo[1], phys.local_hi[1]] {
            for sz in [phys.local_lo[2], phys.local_hi[2]] {
                let p = quat_rotate(rot, [sx * scale, sy * scale, sz * scale]);
                for a in 0..3 {
                    wlo[a] = min_f32(wlo[a], p[a]);
                    whi[a] = max_f32(whi[a], p[a]);
                }
            }
        }
    }
    let wx = max_f32(whi[0] - wlo[0], 0.01 * scale);
    let wy = max_f32(whi[1] - wlo[1], 0.01 * scale);
    let wz = max_f32(whi[2] - wlo[2], 0.01 * scale);
    let aabb_vol = wx * wy * wz;
    let fill_ratio = solid_vol / max_f32(aabb_vol, 1e-9);
    let props = material_props(phys.dominant_material);
    let low_wide_hull = wy < 0.65 * min_f32(wx, wz);
    if props.buoyant && low_wide_hull && fill_ratio < 0.55 {
        // Upright hollow wooden/foliage vessel displaces water with its enclosed hull cavity.
        max_f32(solid_vol, aabb_vol * 0.55)
    } else {
        solid_vol
    }
}

/// Evaluates deterministic Archimedes buoyancy for an un-rooted `Scene` element at `tf` against
/// `region` (which sees both base-area ponds and Phase A dynamically expanded pond channels!).
pub fn compute_buoyancy(
    phys: &ObjectPhysics,
    tf: PlacementTransform,
    region: &mut dyn Region,
    extra_load_kg: f32,
) -> Result<BuoyancyState> {
    let scale = max_f32(tf.scale, 1e-4);
    let (min_y, max_y) = phys.rotated_y_extents(tf.rot, scale);
    let height = max_f32(max_y - min_y, 0.02 * scale);

    // Probe 5 horizontal points (center + 4 mid-hull quadrants) around the object's world footprint.
    let cx = phys.local_center[0];
    let cy = phys.local_center[1];
    let cz = phys.local_center[2];
    let hx = (phys.local_hi[0] - phys.local_lo[0]) * 0.32;
    let hz = (phys.local_hi[2] - phys.local_lo[2]) * 0.32;
    let local_probes = [
        [cx, cy, cz],
        [cx - hx, cy, cz - hz],
        [cx + hx, cy, cz - hz],
        [cx - hx, cy, cz + hz],
        [cx + hx, cy, cz + hz],
    ];
    let probe_size = max_f32(min_f32(phys.world_span * 0.6, 17.0), 0.017);
    let scan_top = tf.pos[1] + max_y + max_f32(height * 1.2, 0.6 * scale);
    let scan_bot = min_f32(tf.pos[1] + min_y - max_f32(height * 4.0, 4.0 * scale), tf.pos[1] + min_y - 3.0);

    let mut water_sum = 0.0f32;
    let mut water_hits = 0u32;
    let mut best_bed = f32::NEG_INFINITY;
    let mut any_bed = false;

    for lp in local_probes {
        let wp = tf.local_to_world_point(lp);
        if let Some(wy) = probe_water_surface_y(region, wp[0], wp[2], scan_top, scan_bot, probe_size)? {
            water_sum += wy;
            water_hits += 1;
        }
        if let Some(by) = probe_solid_floor_y(region, wp[0], wp[2], scan_top, scan_bot, probe_size)? {
            best_bed = max_f32(best_bed, by);
            any_bed = true;
        }
    }

    let floor_y = if any_bed { best_bed } else { tf.pos[1] + min_y };
    if water_hits == 0 {
        return Ok(BuoyancyState::Dry { floor_y });
    }

    let water_y = water_sum / water_hits as f32;
    let bed_y = if any_bed { min_f32(best_bed, water_y) } else { water_y - height };
    let seat_on_bed_y = bed_y - min_y - 0.02 * scale;

    // If the pond bed sits higher than the water surface here (beached on dry bank), stay dry.
    if bed_y >= water_y - 0.01 * scale {
        return Ok(BuoyancyState::Dry { floor_y: bed_y });
    }

    let total_mass = phys.world_mass_kg + max_f32(extra_load_kg, 0.0);
    let disp_m3 = effective_displacement_m3(phys, tf.rot, scale);
    let max_buoyant_kg = disp_m3 * WATER_DENSITY;

    if total_mass >= max_buoyant_kg {
        return Ok(BuoyancyState::Sinking { water_y, bed_y, target_y: seat_on_bed_y });
    }

    // Archimedes draft: fraction of hull height submerged beneath `water_y`.
    // Upright hollow hulls keep a dry cockpit freeboard until heavily loaded (`raw_ratio > 0.75`).
    let solid_vol = max_f32(phys.local_volume_m3 * scale * scale * scale, 1e-9);
    let is_hollow_hull = disp_m3 > solid_vol * 1.25;
    let raw_ratio = total_mass / max_f32(max_buoyant_kg, 1e-9);
    let submerged_fraction = if is_hollow_hull && raw_ratio < 0.75 {
        0.10 + (raw_ratio / 0.75) * 0.18
    } else {
        clamp_f32(raw_ratio, 0.16, 0.92)
    };
    let float_y = water_y - min_y - submerged_fraction * height;
    // Never clip below the solid pond floor in very shallow water!
    let target_y = max_f32(float_y, seat_on_bed_y);
    let bob_amplitude = clamp_f32(0.014 * scale, 0.0005, 0.20);

    Ok(BuoyancyState::Floating { water_y, bed_y, target_y, submerged_fraction, bob_amplitude })
}

/// Advances a piloted floating object (e.g. a wooden rowboat or felled log raft) smoothly across
/// water for one tick `dt`, turning by `turn_rate` (rad/s) and paddling forward at `forward_speed`
/// (m/s along the object's local `-Z` / bow axis).
///
/// Stops cleanly against dry banks and solid terrain walls while sailing freely into Phase A
/// dynamically expanded pond channels!
#[allow(clippy::too_many_arguments)]
pub fn step_floating_move(
    phys: &ObjectPhysics,
    cur_tf: PlacementTransform,
    forward_speed: f32,
    turn_rate: f32,
    dt: f32,
    region: &mut dyn Region,
    extra_load_kg: f32,
    elapsed_time: f32,
) -> Result<(PlacementTransform, BuoyancyState)> {
    let dt = clamp_f32(dt, 0.0, 0.1);
    let dyaw = turn_rate * dt;
    let next_rot = if abs_f32(dyaw) > 1e-6 {
        let q_yaw = [0.0, math::sin(dyaw * 0.5), 0.0, math::cos(dyaw * 0.5)];
        quat_mul(q_yaw, cur_tf.rot)
    } else {
        cur_tf.rot
    };

    // Forward horizontal heading: local -Z rotated into world XZ (matching player forward convention).
    let fwd3 = quat_rotate(next_rot, [0.0, 0.0, -1.0]);
    let flen = (fwd3[0] * fwd3[0] + fwd3[2] * fwd3[2]).sqrt();
    let (fx, fz) = if flen > 1e-4 { (fwd3[0] / flen, fwd3[2] / flen) } else { (0.0, -1.0) };

    let mut candidate_pos = [
        cur_tf.pos[0] + fx * forward_speed * dt,
        cur_tf.pos[1],
        cur_tf.pos[2] + fz * forward_speed * dt,
    ];
    let candidate_tf = PlacementTransform::with_rot(candidate_pos, next_rot, cur_tf.scale);
    let scale = max_f32(cur_tf.scale, 1e-4);

    // Check buoyancy and ensure the bow/hull stays over navigable water (not driving onto dry land).
    let cand_buoy = compute_buoyancy(phys, candidate_tf, region, extra_load_kg)?;
    let (min_y, max_y) = phys.rotated_y_extents(next_rot, scale);
    let height = max_f32(max_y - min_y, 0.02 * scale);

    let can_advance = match cand_buoy {
        BuoyancyState::Floating { water_y, bed_y, .. } => {
            // Require at least a shallow water cushion AND open space (no solid wall) at bow!
            let enough_depth = (water_y - bed_y) >= 0.04 * scale;
            let bow_clear = if abs_f32(forward_speed) > 1e-4 {
                let sign = if forward_speed >= 0.0 { 1.0 } else { -1.0 };
                let bow_dist = max_f32((phys.local_hi[2] - phys.local_lo[2]) * 0.52 * scale, 0.25 * scale);
                let bow = [
                    candidate_pos[0] + fx * sign * bow_dist,
                    water_y + height * 0.35,
                    candidate_pos[2] + fz * sign * bow_dist,
                ];
                let probe_size = max_f32(min_f32(phys.world_span * 0.6, 17.0), 0.017);
                let mut out = Vec::new();
                chunk::sample(region, &[bow], probe_size, &mut out)?;
                out[0].material == Material::Water || out[0].distance > 0.08 * scale
            } else {
                true
            };
            enough_depth && bow_clear
        }
        BuoyancyState::Sinking { .. } | BuoyancyState::Dry { .. } => false,
    };

    let (final_xz, buoy) = if can_advance {
        ([candidate_pos[0], candidate_pos[2]], cand_buoy)
    } else {
        // Blocked by shore: keep current XZ, still allow turning in place!
        let stay_tf = PlacementTransform::with_rot(cur_tf.pos, next_rot, cur_tf.scale);
        ([cur_tf.pos[0], cur_tf.pos[2]], compute_buoyancy(phys, stay_tf, region, extra_load_kg)?)
    };

    candidate_pos[0] = final_xz[0];
    candidate_pos[2] = final_xz[1];
    candidate_pos[1] = match buoy {
        BuoyancyState::Floating { target_y, bob_amplitude, .. } => {
            let bob = math::sin(elapsed_time * 2.2) * bob_amplitude;
            target_y + bob
        }
        BuoyancyState::Sinking { target_y, .. } => target_y,
        BuoyancyState::Dry { floor_y } => floor_y - min_y - 0.02 * scale,
    };

    Ok((PlacementTransform::with_rot(candidate_pos, next_rot, cur_tf.scale), buoy))
}

/// Finds the exact hydrostatic water surface Y along vertical column `(x, z)` between `top_y`
/// and `bot_y`, returning `None` if no `Material::Water` exists in that column.
fn probe_water_surface_y(
    region: &mut dyn Region,
    x: f32,
    z: f32,
    top_y: f32,
    bot_y: f32,
    size: f32,
) -> Result<Option<f32>> {
    const STEPS: usize = 18;
    let mut pts = Vec::with_capacity(STEPS);
    for i in 0..STEPS {
        let t = i as f32 / (STEPS - 1) as f32;
        pts.push([x, top_y + (bot_y - top_y) * t, z]);
    }
    let mut out = Vec::new();
    chunk::sample(region, &pts, size, &mut out)?;

    let mut y_in: Option<f32> = None;
    let mut y_out = top_y;
    for (p, ans) in pts.iter().zip(&out) {
        if ans.material == Material::Water && ans.distance <= 0.0 {
            y_in = Some(p[1]);
            break;
        }
        y_out = p[1];
    }
    // Shallow-water fallback: if an air sample sees `Material::Water` straight below, step into it.
    if y_in.is_none() {
        let dy = (top_y - bot_y) / (STEPS - 1) as f32;
        let mut sub_out = Vec::new();
        for (p, ans) in pts.iter().zip(&out) {
            if ans.material == Material::Water && ans.distance > 0.0 && ans.distance <= dy {
                for dip in [min_f32(0.02 * size, 0.015), 0.0005] {
                    let sy = p[1] - ans.distance - dip;
                    chunk::sample(region, &[[x, sy, z]], size, &mut sub_out)?;
                    if sub_out[0].material == Material::Water && sub_out[0].distance <= 0.0 {
                        y_in = Some(sy);
                        y_out = p[1];
                        break;
                    }
                }
                if y_in.is_some() {
                    break;
                }
            }
        }
    }
    let Some(mut lo) = y_in else { return Ok(None) };
    let mut hi = max_f32(y_out, lo + 0.05);
    // Ensure `hi` is strictly above water.
    for _ in 0..8 {
        chunk::sample(region, &[[x, hi, z]], size, &mut out)?;
        if out[0].material == Material::Water && out[0].distance <= 0.0 {
            lo = hi;
            hi += max_f32(0.25, hi - lo);
        } else {
            break;
        }
    }
    for _ in 0..12 {
        let mid = 0.5 * (lo + hi);
        chunk::sample(region, &[[x, mid, z]], size, &mut out)?;
        if out[0].material == Material::Water && out[0].distance <= 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(Some(0.5 * (lo + hi)))
}

/// Finds the highest non-water solid surface Y along vertical column `(x, z)` from `top_y` down to `bot_y`.
fn probe_solid_floor_y(
    region: &mut dyn Region,
    x: f32,
    z: f32,
    top_y: f32,
    bot_y: f32,
    size: f32,
) -> Result<Option<f32>> {
    const STEPS: usize = 14;
    let mut pts = Vec::with_capacity(STEPS);
    for i in 0..STEPS {
        let t = i as f32 / (STEPS - 1) as f32;
        pts.push([x, top_y + (bot_y - top_y) * t, z]);
    }
    let mut out = Vec::new();
    chunk::sample(region, &pts, size, &mut out)?;

    let mut hit_idx: Option<usize> = None;
    for (i, ans) in out.iter().enumerate() {
        if ans.material != Material::Water && ans.distance <= 0.0 {
            hit_idx = Some(i);
            break;
        }
    }
    let Some(idx) = hit_idx else { return Ok(None) };
    if idx == 0 {
        return Ok(Some(pts[0][1]));
    }
    let mut hi = pts[idx - 1][1];
    let mut lo = pts[idx][1];
    for _ in 0..10 {
        let mid = 0.5 * (lo + hi);
        chunk::sample(region, &[[x, mid, z]], size, &mut out)?;
        if out[0].material != Material::Water && out[0].distance <= 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(Some(0.5 * (lo + hi)))
}

#[inline]
fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn clamp_f32(x: f32, lo: f32, hi: f32) -> f32 {
    min_f32(max_f32(x, lo), hi)
}

#[inline]
fn abs_f32(x: f32) -> f32 {
    if x < 0.0 { -x } else { x }
}
