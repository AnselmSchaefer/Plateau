//! Scale-stamped SDF sculpting & digging overlay (POC_PLAN.md, step 8; AGENTS.md).
//!
//! Lets players raise hills, dig caves, and build rooms at their current size (tiny, person,
//! giant) as an append-only list of exact SDF edits laid over any `Region` (`Edited<R>`).
//!
//! - **Deterministic & Lipschitz-safe**: every primitive (`Sphere`, `Box`, `Cylinder`) has an exact
//!   Euclidean SDF (`|grad d| == 1`), combined with the same deterministic `min`/`max`/`smooth_union`
//!   /`smooth_cut` math as `plateau-sdk`, so `chunk::make`'s coarse pass, `Body::step`, `reach`,
//!   and `laws` work automatically without missing caves or overshooting surfaces.
//! - **Exact chronological spatial index**: a binary tree over chronological edit ranges prunes
//!   batches and points outside an edit's influence (`box_d > 0` and `d` outside blend reach)
//!   while producing bit-for-bit identical answers to sequential evaluation.
//! - **Composable world save format (`WorldSave`)**: versioned, human-readable line format
//!   persisting area identity, seed, and `sdf` edits with exact `f32` round-trip precision,
//!   designed to accommodate future composable `.wasm` elements and portable `Scene` objects.

use std::borrow::Borrow;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use wasmtime::bail;

use crate::compose::PlacedElement;
use crate::cut::ClipPlane;
use crate::graph::{Kind, Node as GraphNode, Relation};
use crate::modules::{SavedModule, parse_module_line, write_module_records};
use crate::physics::{Inventory, Tool, WorldMode};
use crate::rules::RuleBook;
use crate::water::{WaterBody, WaterLog};
use crate::{Answer, Command, Description, ElementBounds, Material, PlayerState, Region, Result, Scene, chunk};

/// Whether an SDF edit adds material or carves space away.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Op {
    Add,
    Carve,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Add => "add",
            Op::Carve => "carve",
        }
    }

    pub fn from_name(s: &str) -> Option<Op> {
        match s {
            "add" => Some(Op::Add),
            "carve" => Some(Op::Carve),
            _ => None,
        }
    }
}

/// Exact Euclidean SDF primitives (`|grad d| == 1` everywhere).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    /// A ball of radius `radius` (smooth-blended hills, domes, caves, tunnels).
    Sphere { radius: f32 },
    /// An axis-aligned cuboid reaching `half` from its center, edges rounded by `round` within `half`.
    Box { half: [f32; 3], round: f32 },
    /// An upright cylinder of `radius` and `half_height`, edges rounded by `round` within those extents.
    Cylinder { radius: f32, half_height: f32, round: f32 },
}

impl Shape {
    /// Tight half-extents of the shape about its center.
    pub fn half_extents(self) -> [f32; 3] {
        match self {
            Shape::Sphere { radius } => [radius, radius, radius],
            Shape::Box { half, .. } => half,
            Shape::Cylinder { radius, half_height, .. } => [radius, half_height, radius],
        }
    }

    /// Characteristic radius for UI/brush sizing.
    pub fn radius(self) -> f32 {
        match self {
            Shape::Sphere { radius } => radius,
            Shape::Box { half, .. } => max_f32(half[0], max_f32(half[1], half[2])),
            Shape::Cylinder { radius, half_height, .. } => max_f32(radius, half_height),
        }
    }

    /// Scales every linear dimension of this shape uniformly by `s`.
    pub fn scaled(self, s: f32) -> Shape {
        match self {
            Shape::Sphere { radius } => Shape::Sphere { radius: radius * s },
            Shape::Box { half, round } => Shape::Box { half: [half[0] * s, half[1] * s, half[2] * s], round: round * s },
            Shape::Cylinder { radius, half_height, round } => {
                Shape::Cylinder { radius: radius * s, half_height: half_height * s, round: round * s }
            }
        }
    }

    /// Analytical geometric volume of this shape in cubic metres (deterministic).
    pub fn volume_m3(self) -> f32 {
        match self {
            Shape::Sphere { radius } => (4.0 / 3.0) * std::f32::consts::PI * radius * radius * radius,
            Shape::Box { half, .. } => 8.0 * half[0] * half[1] * half[2],
            Shape::Cylinder { radius, half_height, .. } => std::f32::consts::PI * radius * radius * (2.0 * half_height),
        }
    }

    /// Exact Euclidean signed distance from `q = p - center` to the shape's surface.
    pub fn distance(self, q: [f32; 3]) -> f32 {
        match self {
            Shape::Sphere { radius } => (q[0] * q[0] + q[1] * q[1] + q[2] * q[2]).sqrt() - radius,
            Shape::Box { half, round } => {
                let r = min_f32(round, min_f32(half[0], min_f32(half[1], half[2])));
                let bx = abs_f32(q[0]) - (half[0] - r);
                let by = abs_f32(q[1]) - (half[1] - r);
                let bz = abs_f32(q[2]) - (half[2] - r);
                let ox = max_f32(bx, 0.0);
                let oy = max_f32(by, 0.0);
                let oz = max_f32(bz, 0.0);
                let out = (ox * ox + oy * oy + oz * oz).sqrt();
                let inside = min_f32(max_f32(bx, max_f32(by, bz)), 0.0);
                out + inside - r
            }
            Shape::Cylinder { radius, half_height, round } => {
                let r = min_f32(round, min_f32(radius, half_height));
                let radial = (q[0] * q[0] + q[2] * q[2]).sqrt() - (radius - r);
                let vertical = abs_f32(q[1]) - (half_height - r);
                let ox = max_f32(radial, 0.0);
                let oy = max_f32(vertical, 0.0);
                let out = (ox * ox + oy * oy).sqrt();
                let inside = min_f32(max_f32(radial, vertical), 0.0);
                out + inside - r
            }
        }
    }

    pub fn validate(self) -> Result<()> {
        match self {
            Shape::Sphere { radius } => {
                if !radius.is_finite() || radius <= 0.0 {
                    bail!("sphere radius must be finite and positive, got {radius}");
                }
            }
            Shape::Box { half, round } => {
                for a in half {
                    if !a.is_finite() || a <= 0.0 {
                        bail!("box half-extents must be finite and positive, got {half:?}");
                    }
                }
                if !round.is_finite() || round < 0.0 {
                    bail!("box round must be finite and non-negative, got {round}");
                }
            }
            Shape::Cylinder { radius, half_height, round } => {
                if !radius.is_finite() || radius <= 0.0 || !half_height.is_finite() || half_height <= 0.0 {
                    bail!("cylinder dimensions must be finite and positive, got r={radius}, h={half_height}");
                }
                if !round.is_finite() || round < 0.0 {
                    bail!("cylinder round must be finite and non-negative, got {round}");
                }
            }
        }
        Ok(())
    }
}

/// One scale-stamped SDF sculpt or carve operation.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Edit {
    pub op: Op,
    pub shape: Shape,
    pub center: [f32; 3],
    /// Smooth blend width in metres (`0.0` for sharp union/subtraction).
    pub blend: f32,
    /// Material of added solid (`Some`), or optional exposed interior material when carving (`None` keeps base material).
    pub material: Option<Material>,
    /// Player height in metres when this edit was stamped.
    pub stamp_size: f32,
    /// Tool used to make this edit (`Tool::All` for legacy saves/logs that omit it; `WORLD_PLAN.md`, phase H).
    #[serde(default)]
    pub tool: Tool,
    /// The mode the world was in when this edit was made (`Workshop` for legacy saves and engine-made
    /// edits), so a save that changed mode is verified edit by edit under the rules then in force.
    #[serde(default, skip_serializing_if = "WorldMode::is_workshop")]
    pub mode: WorldMode,
}

impl Edit {
    pub fn add(shape: Shape, center: [f32; 3], blend: f32, material: Material, stamp_size: f32) -> Edit {
        Edit { op: Op::Add, shape, center, blend, material: Some(material), stamp_size, tool: Tool::All, mode: WorldMode::Workshop }
    }

    pub fn carve(shape: Shape, center: [f32; 3], blend: f32, material: Option<Material>, stamp_size: f32) -> Edit {
        Edit { op: Op::Carve, shape, center, blend, material, stamp_size, tool: Tool::All, mode: WorldMode::Workshop }
    }

    pub fn with_tool(mut self, tool: Tool) -> Edit {
        self.tool = tool;
        self
    }

    pub fn validate(&self) -> Result<()> {
        self.shape.validate()?;
        if !self.center.iter().all(|c| c.is_finite()) {
            bail!("edit center must be finite, got {:?}", self.center);
        }
        if !self.blend.is_finite() || self.blend < 0.0 {
            bail!("edit blend must be finite and non-negative, got {}", self.blend);
        }
        if !self.stamp_size.is_finite() || self.stamp_size <= 0.0 {
            bail!("edit stamp_size must be finite and positive, got {}", self.stamp_size);
        }
        if self.op == Op::Add && self.material.is_none() {
            bail!("add edit requires a material");
        }
        Ok(())
    }

    /// Tight AABB containing the primitive itself (without smooth blend padding).
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let h = self.shape.half_extents();
        let lo = [self.center[0] - h[0], self.center[1] - h[1], self.center[2] - h[2]];
        let hi = [self.center[0] + h[0], self.center[1] + h[1], self.center[2] + h[2]];
        (lo, hi)
    }

    /// AABB expanded by the smooth blend width.
    pub fn influence_bounds(&self) -> ([f32; 3], [f32; 3]) {
        let (lo, hi) = self.bounds();
        let b = self.blend;
        ([lo[0] - b, lo[1] - b, lo[2] - b], [hi[0] + b, hi[1] + b, hi[2] + b])
    }

    /// Whether this edit can affect the surface, collar, or ambient-occlusion shading of `key`.
    pub fn affects_chunk(&self, key: chunk::Key) -> bool {
        let (elo, ehi) = self.influence_bounds();
        let o = key.origin();
        let side = key.side();
        // Account for 1-cell border collar (2 voxels), surface-net margin, and AO probes (0.36 * size).
        let pad = 2.0 * key.voxel() + 0.4 * key.size() as f64;
        for a in 0..3 {
            if (ehi[a] as f64) < o[a] - pad || (elo[a] as f64) > o[a] + side + pad {
                return false;
            }
        }
        true
    }

    /// Exact Euclidean signed distance from `p` to this edit's shape.
    pub fn distance(&self, p: [f32; 3]) -> f32 {
        self.shape.distance([p[0] - self.center[0], p[1] - self.center[1], p[2] - self.center[2]])
    }

    /// Applies this edit to an existing SDF `Answer` at point `p`, deterministically.
    pub fn apply(&self, p: [f32; 3], cur: Answer) -> Answer {
        let d = self.distance(p);
        match self.op {
            Op::Add => {
                let mat = if d < cur.distance { self.material.unwrap_or(cur.material) } else { cur.material };
                let dist = if self.blend > 0.0 { smooth_union(cur.distance, d, self.blend) } else { min_f32(cur.distance, d) };
                Answer { distance: dist, material: mat }
            }
            Op::Carve => {
                let mat = match self.material {
                    Some(m) if cur.material != Material::Water && -d > cur.distance => m,
                    _ => cur.material,
                };
                let dist = if self.blend > 0.0 { smooth_cut(cur.distance, d, self.blend) } else { max_f32(cur.distance, -d) };
                Answer { distance: dist, material: mat }
            }
        }
    }
}

thread_local! {
    static EDITS_ASKED: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// How many times, on this thread, an edit was asked of a point (`EditLog::apply_batch_range`):
/// the work asking the edits costs, counted, for budgets (`engine/tests/budgets.rs`).
pub fn edits_asked() -> u64 {
    EDITS_ASKED.with(|c| c.get())
}

/// Points a batch is asked in at once, at most (`EditLog::apply_batch_range`).
const CLUSTER: usize = 64;
/// The grid edits are kept on (`Spatial`): a level's cell is this many metres times two to the level.
const SPATIAL_BASE: f32 = 1.0;

/// Edits by where they are: each kept in the cells it covers on the grid of its own size (a cell at
/// least as wide as it, so at most two a side), so what reaches a place is found in the few cells
/// about it, however many edits there are and in whatever order they were made.
/// Level → cell → the edits in it, in order.
type Grids = BTreeMap<i32, HashMap<[i32; 3], Vec<u32>>>;

#[derive(Clone, Debug, Default)]
struct Spatial {
    /// For adds (0) and carves (1).
    levels: [Grids; 2],
    /// Each edit's bounds (its shape's, without its blend), and how far it can move a distance
    /// (its half-diagonal, and its blend).
    bounds: Vec<([f32; 3], [f32; 3])>,
    reach: Vec<f32>,
    max_blend: f32,
}

impl Spatial {
    fn level_of(lo: [f32; 3], hi: [f32; 3]) -> i32 {
        let extent = (0..3).map(|a| hi[a] - lo[a]).fold(0.0f32, f32::max);
        let mut level = -20;
        while SPATIAL_BASE * 2f32.powi(level) < extent && level < 30 {
            level += 1;
        }
        level
    }

    fn cells(level: i32, lo: [f32; 3], hi: [f32; 3]) -> impl Iterator<Item = [i32; 3]> {
        let c = SPATIAL_BASE * 2f32.powi(level);
        let a = lo.map(|v| (v / c).floor() as i32);
        let b = hi.map(|v| (v / c).floor() as i32);
        (a[0]..=b[0]).flat_map(move |x| (a[1]..=b[1]).flat_map(move |y| (a[2]..=b[2]).map(move |z| [x, y, z])))
    }

    fn push(&mut self, edit: &Edit) {
        let i = self.bounds.len() as u32;
        let (lo, hi) = edit.bounds();
        let level = Self::level_of(lo, hi);
        let grid = self.levels[Self::kind(edit)].entry(level).or_default();
        for cell in Self::cells(level, lo, hi) {
            grid.entry(cell).or_default().push(i);
        }
        let h = edit.shape.half_extents();
        self.bounds.push((lo, hi));
        self.reach.push((h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt() + edit.blend);
        self.max_blend = max_f32(self.max_blend, edit.blend);
    }

    fn pop(&mut self, edit: &Edit) {
        let Some((lo, hi)) = self.bounds.pop() else { return };
        self.reach.pop();
        let level = Self::level_of(lo, hi);
        let levels = &mut self.levels[Self::kind(edit)];
        if let Some(grid) = levels.get_mut(&level) {
            for cell in Self::cells(level, lo, hi) {
                if let Some(v) = grid.get_mut(&cell) {
                    v.pop();
                    if v.is_empty() {
                        grid.remove(&cell);
                    }
                }
            }
            if grid.is_empty() {
                levels.remove(&level);
            }
        }
    }

    fn kind(edit: &Edit) -> usize {
        if edit.op == Op::Add { 0 } else { 1 }
    }

    /// The edits of `range` of the kind `kind` (adds 0, carves 1) whose bounds come within
    /// `lo..hi`, in order, into `out`.
    fn within(&self, kind: usize, lo: [f32; 3], hi: [f32; 3], range: std::ops::Range<usize>, out: &mut Vec<u32>) {
        out.clear();
        for (&level, grid) in &self.levels[kind] {
            let c = SPATIAL_BASE * 2f32.powi(level);
            let span: i64 = (0..3).map(|a| ((hi[a] / c).floor() as i64 - (lo[a] / c).floor() as i64 + 1).max(0)).product();
            if span > grid.len() as i64 {
                for (cell, v) in grid {
                    let (clo, chi) = (cell.map(|x| x as f32 * c), cell.map(|x| (x + 1) as f32 * c));
                    if (0..3).all(|a| chi[a] >= lo[a] && clo[a] <= hi[a]) {
                        out.extend(v.iter().filter(|&&i| range.contains(&(i as usize))));
                    }
                }
            } else {
                for cell in Self::cells(level, lo, hi) {
                    if let Some(v) = grid.get(&cell) {
                        out.extend(v.iter().filter(|&&i| range.contains(&(i as usize))));
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out.retain(|&i| {
            let (elo, ehi) = self.bounds[i as usize];
            (0..3).all(|a| ehi[a] >= lo[a] && elo[a] <= hi[a])
        });
    }
}

/// Edits kept exact at the end of a log however long, at least (`EditLog::bake_target`).
pub const BAKE_KEEP: usize = 64;
/// Edits baked at once (and a log's edits before any is): a bake moves on in steps of this many.
pub const BAKE_STEP: usize = 128;
/// Cells a side of a bake brick.
const BAKE_CELLS: usize = 8;
/// A bake brick's cell is a twentieth of the height of who made its edits (as the terrain's voxel
/// is for them, `chunk::VOXELS_A_HEIGHT`), a power of two.
const BAKE_CELLS_A_HEIGHT: f32 = 20.0;

/// A brick of a bake: `BAKE_CELLS + 1` points a side, `cell` apart from `origin`, where the area
/// asked at `size` was (`base`) and what the baked edits made of it (`made`, its materials).
#[derive(Debug)]
pub struct BakeBrick {
    pub origin: [f32; 3],
    pub cell: f32,
    pub size: f32,
    base: Vec<f32>,
    made: Vec<f32>,
    materials: Vec<Material>,
    /// The greatest distance (either sign) the area has at any of its points.
    pub base_reach: f32,
}

impl BakeBrick {
    fn index(i: [usize; 3]) -> usize {
        (i[0] * (BAKE_CELLS + 1) + i[1]) * (BAKE_CELLS + 1) + i[2]
    }

    fn points(origin: [f32; 3], cell: f32) -> Vec<[f32; 3]> {
        let n = BAKE_CELLS + 1;
        let mut out = Vec::with_capacity(n * n * n);
        for x in 0..n {
            for y in 0..n {
                for z in 0..n {
                    out.push([origin[0] + x as f32 * cell, origin[1] + y as f32 * cell, origin[2] + z as f32 * cell]);
                }
            }
        }
        out
    }

    /// Its bounds.
    pub fn bounds(&self) -> ([f32; 3], [f32; 3]) {
        let side = BAKE_CELLS as f32 * self.cell;
        (self.origin, self.origin.map(|o| o + side))
    }

    /// `a` (the area at `p`, as asked) as the baked edits make it: the change they made where the
    /// area was asked at `size`, interpolated, added; and, where they changed it, their material.
    fn give(&self, p: [f32; 3], a: Answer) -> Answer {
        let u = [0, 1, 2].map(|k| ((p[k] - self.origin[k]) / self.cell).clamp(0.0, BAKE_CELLS as f32));
        let i = u.map(|v| (v.floor() as usize).min(BAKE_CELLS - 1));
        let t = [0, 1, 2].map(|k| u[k] - i[k] as f32);
        let mut change = 0.0f32;
        for c in 0..8usize {
            let o = [c & 1, (c >> 1) & 1, (c >> 2) & 1];
            let w = (0..3).map(|k| if o[k] == 1 { t[k] } else { 1.0 - t[k] }).product::<f32>();
            let j = Self::index([i[0] + o[0], i[1] + o[1], i[2] + o[2]]);
            change += w * (self.made[j] - self.base[j]);
        }
        let near = Self::index([0, 1, 2].map(|k| (u[k] + 0.5).floor() as usize).map(|v| v.min(BAKE_CELLS)));
        let changed = self.made[near].to_bits() != self.base[near].to_bits();
        Answer { distance: a.distance + change, material: if changed { self.materials[near] } else { a.material } }
    }
}

/// The edits before `upto` baked (`EditLog::rebaked`): sparse bricks over where they reach, each of
/// the size of who made the edits there. A sample in a brick (and not near an element placed among
/// them, `Composed`) takes the area as asked plus the change the bake made there, then the edits
/// after `upto`; elsewhere every edit, as ever.
#[derive(Debug)]
pub struct Bake {
    pub upto: usize,
    /// Level (its cell, two to the level) → brick → it.
    levels: BTreeMap<i32, HashMap<[i32; 3], Arc<BakeBrick>>>,
    /// The farthest any baked edit can move a distance.
    pub reach: f32,
}

impl Bake {
    fn level_of(stamp: f32) -> i32 {
        let cell = stamp / BAKE_CELLS_A_HEIGHT;
        let mut level = -30;
        while 2f32.powi(level + 1) <= cell && level < 20 {
            level += 1;
        }
        level
    }

    /// The finest brick `p` is in.
    pub fn brick_at(&self, p: [f32; 3]) -> Option<&Arc<BakeBrick>> {
        for (&level, grid) in &self.levels {
            let side = BAKE_CELLS as f32 * 2f32.powi(level);
            if let Some(b) = grid.get(&p.map(|v| (v / side).floor() as i32)) {
                return Some(b);
            }
        }
        None
    }

    /// Every brick.
    pub fn bricks(&self) -> impl Iterator<Item = &Arc<BakeBrick>> {
        self.levels.values().flat_map(|g| g.values())
    }

    /// The bricks `edit` needs, at its level: those its reach (and a cell) overlaps.
    fn keys_for(edit: &Edit) -> (i32, Vec<[i32; 3]>) {
        let level = Self::level_of(edit.stamp_size);
        let cell = 2f32.powi(level);
        let side = BAKE_CELLS as f32 * cell;
        let (lo, hi) = edit.influence_bounds();
        let a = lo.map(|v| ((v - cell) / side).floor() as i32);
        let b = hi.map(|v| ((v + cell) / side).floor() as i32);
        let mut out = Vec::new();
        for x in a[0]..=b[0] {
            for y in a[1]..=b[1] {
                for z in a[2]..=b[2] {
                    out.push([x, y, z]);
                }
            }
        }
        (level, out)
    }
}

/// Ordered log of SDF edits, kept by where they are (`Spatial`) for fast exact batch evaluation,
/// and revision tracking; past `BAKE_STEP + BAKE_KEEP` edits, all but the last few baked (`Bake`).
#[derive(Clone, Debug, Default)]
pub struct EditLog {
    edits: Vec<Edit>,
    spatial: Spatial,
    revision: u64,
    bake: Option<Arc<Bake>>,
}

impl EditLog {
    pub fn new() -> EditLog {
        EditLog::default()
    }

    pub fn from_edits(edits: Vec<Edit>) -> Result<EditLog> {
        EditLog::from_edits_with_revision(edits, 0)
    }

    pub fn from_edits_with_revision(edits: Vec<Edit>, revision: u64) -> Result<EditLog> {
        for e in &edits {
            e.validate()?;
        }
        let mut log = EditLog { edits: Vec::new(), spatial: Spatial::default(), revision, bake: None };
        for e in edits {
            log.spatial.push(&e);
            log.edits.push(e);
        }
        Ok(log)
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn edits(&self) -> &[Edit] {
        &self.edits
    }

    pub fn len(&self) -> usize {
        self.edits.len()
    }

    pub fn is_empty(&self) -> bool {
        self.edits.is_empty()
    }

    /// Validates and appends an edit, rebuilding the spatial index and bumping `revision`.
    pub fn push(&mut self, edit: Edit) -> Result<u64> {
        edit.validate()?;
        self.spatial.push(&edit);
        self.edits.push(edit);
        self.revision += 1;
        Ok(self.revision)
    }

    /// Removes and returns the most recent edit (undo), bumping `revision` if non-empty.
    pub fn pop(&mut self) -> Option<Edit> {
        let removed = self.edits.pop()?;
        self.spatial.pop(&removed);
        self.revision += 1;
        // Baked past where it now ends, the bake is of edits it no longer has.
        if self.bake.as_ref().is_some_and(|b| b.upto > self.edits.len()) {
            self.bake = None;
        }
        Some(removed)
    }

    /// Clears all edits, bumping `revision` if non-empty.
    pub fn clear(&mut self) {
        if !self.edits.is_empty() {
            self.edits.clear();
            self.spatial = Spatial::default();
            self.bake = None;
            self.revision += 1;
        }
    }

    /// Whether any edit in this log can affect chunk `key`.
    pub fn affects_chunk(&self, key: chunk::Key) -> bool {
        self.edits.iter().any(|e| e.affects_chunk(key))
    }

    /// Applies all edits in chronological order to `(points, out)` (`apply_batch_range`).
    pub fn apply_batch(&self, points: &[[f32; 3]], out: &mut [Answer]) {
        self.apply_batch_range(0, self.edits.len(), points, out);
    }

    /// Applies edits in the chronological slice `[range_start..range_end)` to `(points, out)`:
    /// each point is given, in order, every edit of the slice that can change it (bit-exact with
    /// `apply_batch_reference`, which gives each point every edit).
    ///
    /// The points are asked in clusters of up to `CLUSTER` nearby ones. A cluster's edits are those
    /// whose bounds come within its points' greatest distance (and its blend) of it, found in the
    /// few cells about it (`Spatial`): an edit further away changes none of them. Asking a whole
    /// batch at once against edits kept in the order they were made reached every edit in a
    /// much-dug place for every point (played by hand 2026-11-28: 2,000 strokes in one place made
    /// asking twenty times dearer, and walking there thirty).
    pub fn apply_batch_range(&self, range_start: usize, range_end: usize, points: &[[f32; 3]], out: &mut [Answer]) {
        let range_end = range_end.min(self.edits.len());
        if range_start >= range_end || points.is_empty() {
            return;
        }
        debug_assert_eq!(points.len(), out.len());
        let mut found = Vec::new();
        if points.len() <= CLUSTER {
            self.apply_cluster(range_start..range_end, points, out, &mut found);
            return;
        }
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in points {
            for a in 0..3 {
                lo[a] = min_f32(lo[a], p[a]);
                hi[a] = max_f32(hi[a], p[a]);
            }
        }
        let scale = [0, 1, 2].map(|a| if hi[a] > lo[a] { 1023.0 / (hi[a] - lo[a]) } else { 0.0 });
        let spread = |v: u32| -> u64 {
            let mut x = v as u64 & 0x3ff;
            x = (x | (x << 16)) & 0x0300_00ff;
            x = (x | (x << 8)) & 0x0300_f00f;
            x = (x | (x << 4)) & 0x030c_30c3;
            (x | (x << 2)) & 0x0924_9249
        };
        let mut order: Vec<(u64, usize)> = points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let q = [0, 1, 2].map(|a| ((p[a] - lo[a]) * scale[a]).clamp(0.0, 1023.0) as u32);
                (spread(q[0]) | (spread(q[1]) << 1) | (spread(q[2]) << 2), i)
            })
            .collect();
        order.sort_unstable();
        let mut pts = Vec::with_capacity(CLUSTER);
        let mut ans = Vec::with_capacity(CLUSTER);
        for group in order.chunks(CLUSTER) {
            pts.clear();
            ans.clear();
            for &(_, i) in group {
                pts.push(points[i]);
                ans.push(out[i]);
            }
            self.apply_cluster(range_start..range_end, &pts, &mut ans, &mut found);
            for (k, &(_, i)) in group.iter().enumerate() {
                out[i] = ans[k];
            }
        }
    }

    /// One cluster of `apply_batch_range`.
    fn apply_cluster(&self, range: std::ops::Range<usize>, points: &[[f32; 3]], out: &mut [Answer], found: &mut Vec<u32>) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for p in points {
            for a in 0..3 {
                lo[a] = min_f32(lo[a], p[a]);
                hi[a] = max_f32(hi[a], p[a]);
            }
        }
        // An edit changes a point only if its bounds are within the point's distance as it then is
        // (and the edit's blend) of it. Asked of the edits within `r` of the cluster, and no point's
        // distance growing past `r` less the greatest blend while they are applied, no edit further
        // changed any of them: the answers are exact. A distance grows only to the reach of an edit
        // that changes it, so `r` starts there; if a point's grows further still (blends add a
        // little), it is asked again, further.
        let start = out.iter().fold(0.0f32, |m, a| max_f32(m, abs_f32(a.distance)));
        let blend = self.spatial.max_blend;
        if !start.is_finite() {
            for i in range {
                let (elo, ehi) = self.spatial.bounds[i];
                Self::apply_one(&self.edits[i], elo, ehi, points, out, &mut 0.0);
            }
            return;
        }
        let given: Vec<Answer> = out.to_vec();
        // How far into the air (`above`) and into the solid (`below`) the points are: adds are asked
        // out to the one, carves to the other (each widened by the greatest blend). An add can take
        // a point into the solid as deep as its reach, a carve into the air, so each is widened by
        // the other's reach until neither grows; then, applied, a point's distance growing past
        // what was asked (blends add a little) has them asked again, further.
        let side = |out: &[Answer]| out.iter().fold((f32::NEG_INFINITY, f32::NEG_INFINITY), |(a, b), x| (max_f32(a, x.distance), max_f32(b, -x.distance)));
        let (above0, below0) = side(out);
        let (mut ra, mut rc) = (max_f32(above0, 0.0) + blend, max_f32(below0, 0.0) + blend);
        let (mut adds, mut carves) = (Vec::new(), Vec::new());
        loop {
            loop {
                self.spatial.within(0, lo.map(|v| v - ra), hi.map(|v| v + ra), range.clone(), &mut adds);
                self.spatial.within(1, lo.map(|v| v - rc), hi.map(|v| v + rc), range.clone(), &mut carves);
                let reach = |v: &[u32]| v.iter().fold(0.0f32, |m, &i| max_f32(m, self.spatial.reach[i as usize]));
                let next_c = max_f32(rc, max_f32(below0, reach(&adds)) + blend);
                let next_a = max_f32(ra, max_f32(above0, reach(&carves)) + blend);
                if next_a <= ra && next_c <= rc {
                    break;
                }
                (ra, rc) = (next_a, next_c);
            }
            found.clear();
            let (mut x, mut y) = (0, 0);
            while x < adds.len() || y < carves.len() {
                if y >= carves.len() || (x < adds.len() && adds[x] < carves[y]) {
                    found.push(adds[x]);
                    x += 1;
                } else {
                    found.push(carves[y]);
                    y += 1;
                }
            }
            // How far into the air (`above`) and into the solid (`below`) any point now is: an add
            // changes a point only if it is nearer than the point is into the air, a carve only if
            // nearer than it is into the solid (each with its blend), so an edit further from the
            // whole cluster than that is passed over at once; distances fall as edits are applied,
            // so most are.
            let (mut above, mut below) = (above0, below0);
            let (mut most_above, mut most_below) = (above0, below0);
            for &i in found.iter() {
                let (elo, ehi) = self.spatial.bounds[i as usize];
                let edit = &self.edits[i as usize];
                let gap = (0..3).map(|a| max_f32(max_f32(elo[a] - hi[a], lo[a] - ehi[a]), 0.0)).map(|g| g * g).sum::<f32>().sqrt();
                let into = if edit.op == Op::Add { above } else { below };
                if gap > 0.0 && gap > into + edit.blend {
                    continue;
                }
                Self::apply_one(edit, elo, ehi, points, out, &mut 0.0);
                (above, below) = side(out);
                most_above = max_f32(most_above, above);
                most_below = max_f32(most_below, below);
            }
            if most_above + blend <= ra && most_below + blend <= rc {
                return;
            }
            ra = max_f32(ra, most_above + blend);
            rc = max_f32(rc, most_below + blend);
            out.copy_from_slice(&given);
        }
    }

    /// `edit` (its bounds `elo..ehi`) on each point it can change, as a sequential evaluation does;
    /// `seen`, the greatest distance (either sign) a point it changed now has, if greater.
    fn apply_one(edit: &Edit, elo: [f32; 3], ehi: [f32; 3], points: &[[f32; 3]], out: &mut [Answer], seen: &mut f32) {
        EDITS_ASKED.with(|c| c.set(c.get() + points.len() as u64));
        let blend = edit.blend;
        for (&p, ans) in points.iter().zip(out.iter_mut()) {
            let pd = point_to_aabb_distance(p, elo, ehi);
            let unreached = match edit.op {
                Op::Add => ans.distance <= pd - blend,
                Op::Carve => -ans.distance <= pd - blend,
            };
            if pd > 0.0 && unreached {
                continue;
            }
            *ans = edit.apply(p, *ans);
            *seen = max_f32(*seen, abs_f32(ans.distance));
        }
    }

    /// How many of `len` edits a log is baked up to: none until there are `BAKE_STEP + BAKE_KEEP`;
    /// then all but the last `BAKE_KEEP` to `2 × BAKE_KEEP`, in steps of `BAKE_STEP`. Of the
    /// count alone, so a log baked so is the same however it came to its edits (played, replayed,
    /// opened, undone to).
    pub fn bake_target(len: usize) -> usize {
        if len < BAKE_STEP + BAKE_KEEP { 0 } else { (len - BAKE_KEEP) / BAKE_STEP * BAKE_STEP }
    }

    /// The bake, if any.
    pub fn bake(&self) -> Option<&Arc<Bake>> {
        self.bake.as_ref()
    }

    /// This log baked to `bake_target` of its length, if it is not, with where that changed what it
    /// gives (the bricks made, and where the edits newly baked reach); the area is asked at the
    /// bricks' points by `base` (points, size). Moving on, the bricks there are kept and given
    /// the newly baked edits; going back (an undo), it is baked again from the start. Either way
    /// every brick holds, to the bit, the area at its points with every baked edit applied in order.
    #[allow(clippy::type_complexity)]
    pub fn rebaked(&self, base: &mut dyn FnMut(&[[f32; 3]], f32, &mut Vec<Answer>) -> Result<()>) -> Result<Option<(EditLog, Vec<([f32; 3], [f32; 3])>)>> {
        let target = Self::bake_target(self.edits.len());
        let current = self.bake.as_ref().map_or(0, |b| b.upto);
        if target == current {
            return Ok(None);
        }
        let mut changed = Vec::new();
        let mut out = self.clone();
        // What it gives changed: whatever was kept by its revision is made again.
        out.revision += 1;
        let (mut levels, from) = match &self.bake {
            Some(b) if target > b.upto => (b.levels.clone(), b.upto),
            Some(b) => {
                changed.extend(b.bricks().map(|k| k.bounds()));
                (BTreeMap::new(), 0)
            }
            None => (BTreeMap::new(), 0),
        };
        if target == 0 {
            out.bake = None;
            return Ok(Some((out, changed)));
        }
        // The bricks there were: given the edits newly baked.
        for grid in levels.values_mut() {
            for brick in grid.values_mut() {
                let pts = BakeBrick::points(brick.origin, brick.cell);
                let mut ans: Vec<Answer> = brick.made.iter().zip(&brick.materials).map(|(&d, &m)| Answer { distance: d, material: m }).collect();
                self.apply_batch_range(from, target, &pts, &mut ans);
                if ans.iter().zip(&brick.made).any(|(a, &m)| a.distance.to_bits() != m.to_bits()) || ans.iter().zip(&brick.materials).any(|(a, &m)| a.material != m) {
                    *brick = Arc::new(BakeBrick {
                        origin: brick.origin,
                        cell: brick.cell,
                        size: brick.size,
                        base: brick.base.clone(),
                        made: ans.iter().map(|a| a.distance).collect(),
                        materials: ans.iter().map(|a| a.material).collect(),
                        base_reach: brick.base_reach,
                    });
                }
            }
        }
        // And those the newly baked edits need, made: the area there, every baked edit on it.
        let mut reach = self.bake.as_ref().filter(|_| from > 0).map_or(0.0, |b| b.reach);
        for e in &self.edits[from..target] {
            let (level, keys) = Bake::keys_for(e);
            let cell = 2f32.powi(level);
            let side = BAKE_CELLS as f32 * cell;
            let h = e.shape.half_extents();
            reach = max_f32(reach, (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt() + e.blend);
            changed.push(e.influence_bounds());
            let grid = levels.entry(level).or_default();
            for key in keys {
                if grid.contains_key(&key) {
                    continue;
                }
                let origin = key.map(|k| k as f32 * side);
                let pts = BakeBrick::points(origin, cell);
                let size = cell * BAKE_CELLS_A_HEIGHT;
                let mut ans = Vec::new();
                base(&pts, size, &mut ans)?;
                let base_d: Vec<f32> = ans.iter().map(|a| a.distance).collect();
                let base_reach = base_d.iter().fold(0.0f32, |m, d| max_f32(m, abs_f32(*d)));
                self.apply_batch_range(0, target, &pts, &mut ans);
                let brick = BakeBrick { origin, cell, size, base: base_d, made: ans.iter().map(|a| a.distance).collect(), materials: ans.iter().map(|a| a.material).collect(), base_reach };
                changed.push(brick.bounds());
                grid.insert(key, Arc::new(brick));
            }
        }
        out.bake = Some(Arc::new(Bake { upto: target, levels, reach }));
        Ok(Some((out, changed)))
    }

    /// `apply_batch_range`, with the bake: a point in a brick `usable` allows (no element placed
    /// among the baked edits near it) is given the bake's change (if `range_start` is 0: once, at
    /// the first range asked) and the edits of the range after the bake; any other point, every
    /// edit of the range.
    pub fn apply_batch_range_baked(&self, range_start: usize, range_end: usize, points: &[[f32; 3]], out: &mut [Answer], usable: &mut dyn FnMut(&BakeBrick) -> bool) {
        let Some(bake) = self.bake.as_ref() else { return self.apply_batch_range(range_start, range_end, points, out) };
        let (mut baked, mut exact) = (Vec::new(), Vec::new());
        for (i, &p) in points.iter().enumerate() {
            match bake.brick_at(p) {
                Some(b) if usable(b) => {
                    if range_start == 0 {
                        out[i] = b.give(p, out[i]);
                    }
                    baked.push(i);
                }
                _ => exact.push(i),
            }
        }
        if baked.is_empty() {
            return self.apply_batch_range(range_start, range_end, points, out);
        }
        for (idx, from) in [(&exact, range_start), (&baked, range_start.max(bake.upto))] {
            if idx.is_empty() || from >= range_end {
                continue;
            }
            let pts: Vec<[f32; 3]> = idx.iter().map(|&i| points[i]).collect();
            let mut ans: Vec<Answer> = idx.iter().map(|&i| out[i]).collect();
            self.apply_batch_range(from, range_end, &pts, &mut ans);
            for (k, &i) in idx.iter().enumerate() {
                out[i] = ans[k];
            }
        }
    }

    /// Every edit of `[range_start..range_end)` in order on every point it can change, unindexed:
    /// what `apply_batch_range` must give, to the bit (for tests).
    #[doc(hidden)]
    pub fn apply_batch_reference(&self, range_start: usize, range_end: usize, points: &[[f32; 3]], out: &mut [Answer]) {
        for i in range_start..range_end.min(self.edits.len()) {
            let edit = &self.edits[i];
            let (elo, ehi) = edit.bounds();
            Self::apply_one(edit, elo, ehi, points, out, &mut 0.0);
        }
    }
}

/// Wraps any `Region` with a scale-stamped `EditLog` overlay so chunk meshing, `Body::step`,
/// `body::look`, `rings::open`, `reach`, `picture`, and `laws` all see edits automatically.
pub struct Edited<R, E = Arc<EditLog>> {
    pub region: R,
    pub edits: E,
}

impl<R, E> Edited<R, E> {
    pub fn new(region: R, edits: E) -> Edited<R, E> {
        Edited { region, edits }
    }
}

impl<R: Region, E: Borrow<EditLog>> Region for Edited<R, E> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        self.region.sample(points, size, out)?;
        self.edits.borrow().apply_batch(points, out);
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        self.region.describe()
    }

    fn scene(&mut self) -> Result<Scene> {
        self.region.scene()
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Result<Vec<Command>> {
        self.region.tick(dt, player)
    }

    fn interact(&mut self, player: &PlayerState) -> Result<Vec<Command>> {
        self.region.interact(player)
    }

    fn element_bounds(&mut self) -> Result<Option<ElementBounds>> {
        self.region.element_bounds()
    }
}

/// Estimates the unit outward surface normal of `area` at `at` (for a viewer `size` tall) using
/// four tetrahedral SDF probes.
pub fn surface_normal(area: &mut dyn Region, at: [f64; 3], size: f32) -> Result<[f32; 3]> {
    let eps = (0.015 * size).max(1e-4) as f64;
    let offs = [[1.0, -1.0, -1.0], [-1.0, -1.0, 1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, 1.0]];
    let pts: Vec<[f32; 3]> = offs.iter().map(|o| [0, 1, 2].map(|a| (at[a] + o[a] * eps) as f32)).collect();
    let mut out = Vec::new();
    area.sample(&pts, size, &mut out)?;
    let mut g = [0.0f32; 3];
    for (o, ans) in offs.iter().zip(&out) {
        for a in 0..3 {
            g[a] += o[a] as f32 * ans.distance;
        }
    }
    let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt();
    if len > 1e-12 { Ok([g[0] / len, g[1] / len, g[2] / len]) } else { Ok([0.0, 1.0, 0.0]) }
}

/// Current save file format version: 2 names each module with its bytes' hash (`modules`).
pub const SAVE_VERSION: u32 = 2;
/// Save versions still read: 1 (Steps 8a–9, phases A–C) has no `module` records.
pub const READ_VERSIONS: [u32; 2] = [1, 2];

/// One base area tile in a multi-area world manifest (`tile <gx> <gz> <module> seed <u64>`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SavedTile {
    pub at: [i32; 2],
    pub module: String,
    pub seed: u64,
}

/// Composable world save & manifest: base area(s) + seed + ownership `RuleBook` + engine-managed
/// hydrostatic `WaterBody` list + chronological SDF edit log + placed standalone `.wasm` elements.
/// Reads `plateau-world 1` files (Steps 8a–9, phases A–C) and writes `plateau-world 2`.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldSave {
    /// The version the save was read as (written saves are always `SAVE_VERSION`).
    pub version: u32,
    pub area: String,
    pub seed: u64,
    /// Play vs workshop mode (`mode world` in `plateau-world 2`; defaults to `Workshop` when omitted).
    pub mode: WorldMode,
    /// Optional explicit multi-area tile manifest (`tile` records). Empty for legacy single-area saves.
    pub tiles: Vec<SavedTile>,
    /// The modules its tiles and elements were made of, with their bytes' hashes (`module`
    /// records, sorted by path; filled by `seal`, empty in version 1 saves).
    pub modules: Vec<SavedModule>,
    /// Multi-scale ownership & permission rule book (`policy` and `plot` records).
    pub rules: RuleBook,
    /// Engine-managed hydrostatic water bodies (`water` and `water-basin` records).
    pub water: Vec<WaterBody>,
    pub edits: Vec<Edit>,
    /// Optional per-edit author list aligned with `edits` (`None` or shorter vector = unattributed).
    pub edit_authors: Vec<Option<String>>,
    pub elements: Vec<PlacedElement>,
    /// The world graph (`rel` records, `graph`), in its order; empty in saves made before it.
    pub relations: Vec<Relation>,
    /// Per-player carried materials (`inv <player> <mat> <kg>` records in `plateau-world 2`).
    pub inventories: BTreeMap<String, Inventory>,
}

impl WorldSave {
    pub fn new(area: impl Into<String>, seed: u64, edits: Vec<Edit>) -> WorldSave {
        WorldSave {
            version: SAVE_VERSION,
            area: area.into(),
            seed,
            mode: WorldMode::Workshop,
            tiles: Vec::new(),
            modules: Vec::new(),
            rules: RuleBook::new(),
            water: Vec::new(),
            edits,
            edit_authors: Vec::new(),
            elements: Vec::new(),
            relations: Vec::new(),
            inventories: BTreeMap::new(),
        }
    }

    pub fn with_elements(area: impl Into<String>, seed: u64, edits: Vec<Edit>, elements: Vec<PlacedElement>) -> WorldSave {
        WorldSave {
            version: SAVE_VERSION,
            area: area.into(),
            seed,
            mode: WorldMode::Workshop,
            tiles: Vec::new(),
            modules: Vec::new(),
            rules: RuleBook::new(),
            water: Vec::new(),
            edits,
            edit_authors: Vec::new(),
            elements,
            relations: Vec::new(),
            inventories: BTreeMap::new(),
        }
    }

    /// Serializes to a human-readable, deterministic, bit-exact text representation.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("plateau-world {SAVE_VERSION}\n"));
        s.push_str(&format!("area {}\n", self.area.replace(['\n', '\r'], " ")));
        s.push_str(&format!("seed {}\n", self.seed));
        if self.mode == WorldMode::World {
            s.push_str("mode world\n");
        }
        for t in &self.tiles {
            s.push_str(&format!("tile {} {} {} seed {}\n", t.at[0], t.at[1], t.module, t.seed));
        }
        write_module_records(&self.modules, &mut s);
        self.rules.write_records(&mut s);
        if !self.water.is_empty()
            && let Ok(wl) = WaterLog::from_bodies(self.water.clone())
        {
            wl.write_records(&mut s);
        }
        for (idx, e) in self.edits.iter().enumerate() {
            let author = self.edit_authors.get(idx).and_then(|a| a.as_deref());
            write_sdf_record(&mut s, "sdf", e, author);
        }
        for el in &self.elements {
            let [px, py, pz] = el.pos;
            s.push_str(&format!(
                "element {} {} seed {} pos {} {} {} yaw {} scale {} size {} after {}",
                el.id, el.module, el.seed, px, py, pz, el.yaw, el.scale, el.stamp_size, el.after_edits
            ));
            if let Some([qx, qy, qz, qw]) = el.rot {
                s.push_str(&format!(" rot {qx} {qy} {qz} {qw}"));
            }
            if let Some(p) = el.promoted {
                s.push_str(&format!(" promoted {p}"));
            }
            for c in &el.clips {
                s.push_str(&format!(" clip {} {} {} {}", c.normal[0], c.normal[1], c.normal[2], c.offset));
            }
            if let Some(author) = &el.author {
                s.push_str(&format!(" author {author}"));
            }
            if !el.params.is_empty() {
                s.push_str(&format!(" params {}", el.params.len()));
                for p in &el.params {
                    s.push_str(&format!(" {p}"));
                }
            }
            s.push('\n');
            for le in &el.local_edits {
                write_sdf_record(&mut s, &format!("el-sdf {}", el.id), le, None);
            }
        }
        for r in &self.relations {
            s.push_str(&r.to_record());
            s.push('\n');
        }
        for (player, inv) in &self.inventories {
            for (mat, kg) in inv.iter() {
                s.push_str(&format!("inv {player} {} {kg}\n", mat.name()));
            }
        }
        s
    }

    /// Parses and validates a world save file.
    pub fn from_text(text: &str) -> Result<WorldSave> {
        let mut version = None;
        let mut area = String::new();
        let mut seed = None;
        let mut mode = WorldMode::Workshop;
        let mut tiles = Vec::new();
        let mut modules: Vec<SavedModule> = Vec::new();
        let mut rules = RuleBook::new();
        let mut water: Vec<WaterBody> = Vec::new();
        let mut edits = Vec::new();
        let mut edit_authors: Vec<Option<String>> = Vec::new();
        let mut any_edit_author = false;
        let mut elements: Vec<PlacedElement> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();
        let mut inventories: BTreeMap<String, Inventory> = BTreeMap::new();
        for (line_no, raw) in text.lines().enumerate() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some(rest) = line.strip_prefix("plateau-world ") {
                let v: u32 = rest.trim().parse().map_err(|e| wasmtime::format_err!("line {}: invalid version: {e}", line_no + 1))?;
                if !READ_VERSIONS.contains(&v) {
                    bail!("line {}: unsupported save version {v} (this build reads {READ_VERSIONS:?})", line_no + 1);
                }
                version = Some(v);
            } else if let Some(rest) = line.strip_prefix("area ") {
                area = rest.trim().to_string();
            } else if let Some(rest) = line.strip_prefix("seed ") {
                seed = Some(rest.trim().parse::<u64>().map_err(|e| wasmtime::format_err!("line {}: invalid seed: {e}", line_no + 1))?);
            } else if let Some(rest) = line.strip_prefix("mode ") {
                if version == Some(1) {
                    bail!("line {}: `mode` records need `plateau-world 2`", line_no + 1);
                }
                mode = WorldMode::from_name(rest.trim())
                    .ok_or_else(|| wasmtime::format_err!("line {}: unknown mode `{}`", line_no + 1, rest.trim()))?;
            } else if line.starts_with("tile ") {
                tiles.push(parse_tile_line(line, line_no + 1)?);
            } else if line.starts_with("module ") {
                if version == Some(1) {
                    bail!("line {}: `module` records need `plateau-world 2`", line_no + 1);
                }
                let m = parse_module_line(line, line_no + 1)?;
                if modules.iter().any(|o| o.path == m.path) {
                    bail!("line {}: module {} recorded twice", line_no + 1, m.path);
                }
                modules.push(m);
            } else if let Some(rest) = line.strip_prefix("economy ") {
                rules.economy = match rest.trim() {
                    "on" => true,
                    "off" => false,
                    other => bail!("line {}: `economy {other}`: on or off", line_no + 1),
                };
            } else if line.starts_with("policy ") {
                rules.set_area(RuleBook::parse_policy_line(line, line_no + 1)?);
            } else if line.starts_with("plot ") {
                rules.set_plot(RuleBook::parse_plot_line(line, line_no + 1)?);
            } else if line.starts_with("water ") {
                water.push(WaterLog::parse_water_line(line, line_no + 1)?);
            } else if line.starts_with("water-basin ") {
                let (id, basin) = WaterLog::parse_basin_line(line, line_no + 1)?;
                let body = water
                    .iter_mut()
                    .find(|b| b.id == id)
                    .ok_or_else(|| wasmtime::format_err!("line {}: water-basin references unknown water body {id}", line_no + 1))?;
                body.basins.push(basin);
            } else if line.starts_with("sdf ") {
                let (edit, author) = parse_sdf_line(line, line_no + 1)?;
                edit.validate()?;
                if author.is_some() {
                    any_edit_author = true;
                }
                edits.push(edit);
                edit_authors.push(author);
            } else if line.starts_with("element ") {
                let el = parse_element_line(line, line_no + 1)?;
                el.validate()?;
                elements.push(el);
            } else if let Some(rest) = line.strip_prefix("el-sdf ") {
                let mut parts = rest.splitn(2, char::is_whitespace);
                let id_str = parts.next().ok_or_else(|| wasmtime::format_err!("line {}: missing element id", line_no + 1))?;
                let id: u32 = id_str.parse().map_err(|e| wasmtime::format_err!("line {}: bad element id: {e}", line_no + 1))?;
                let sdf_tail = parts.next().ok_or_else(|| wasmtime::format_err!("line {}: missing el-sdf payload", line_no + 1))?.trim();
                let (edit, _) = parse_sdf_line(&format!("sdf {sdf_tail}"), line_no + 1)?;
                edit.validate()?;
                let target = elements
                    .iter_mut()
                    .find(|e| e.id == id)
                    .ok_or_else(|| wasmtime::format_err!("line {}: el-sdf references unknown element {id}", line_no + 1))?;
                target.local_edits.push(edit);
            } else if line.starts_with("rel ") {
                if version == Some(1) {
                    bail!("line {}: `rel` records need `plateau-world 2`", line_no + 1);
                }
                let rel = Relation::parse_record(line, line_no + 1)?;
                // Its ends are recorded already: elements (but for the original a piece was cut from,
                // or an element an edit once carved, which may be gone), edits, water bodies.
                let history = matches!(rel.kind, Kind::DerivedFrom | Kind::CarvedInto);
                for (end, is_to) in [(&rel.from, false), (&rel.to, true)] {
                    let there = match *end {
                        GraphNode::Element(id) => (is_to && history) || elements.iter().any(|e| e.id == id),
                        GraphNode::Edit(i) => (i as usize) < edits.len(),
                        GraphNode::Water(id) => water.iter().any(|b| b.id == id),
                        GraphNode::Terrain | GraphNode::Player(_) => true,
                    };
                    if !there {
                        bail!("line {}: relation names {end}, not in the save", line_no + 1);
                    }
                }
                relations.push(rel);
            } else if line.starts_with("inv ") {
                if version == Some(1) {
                    bail!("line {}: `inv` records need `plateau-world 2`", line_no + 1);
                }
                let (player, mat, kg) = parse_inv_line(line, line_no + 1)?;
                if kg > 0.0 {
                    inventories.entry(player).or_default().add(mat, kg);
                }
            } else {
                bail!("line {}: unknown record `{line}`", line_no + 1);
            }
        }
        let version = version.ok_or_else(|| wasmtime::format_err!("missing `plateau-world` header"))?;
        let seed = seed.ok_or_else(|| wasmtime::format_err!("missing `seed` field"))?;
        if !any_edit_author {
            edit_authors.clear();
        }
        modules.sort();
        crate::graph::Graph::from_relations(relations.iter().cloned())?;
        Ok(WorldSave { version, area, seed, mode, tiles, modules, rules, water, edits, edit_authors, elements, relations, inventories })
    }

    /// Writes the save file atomically (write temp file in same directory, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("plateau.tmp");
        std::fs::write(&tmp, self.to_text())?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<WorldSave> {
        let text = std::fs::read_to_string(path)?;
        WorldSave::from_text(&text)
    }
}

fn parse_tile_line(line: &str, line_no: usize) -> Result<SavedTile> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    if tok.len() != 6 || tok[4] != "seed" {
        bail!("line {line_no}: expected `tile <gx> <gz> <module> seed <u64>`, got `{line}`");
    }
    let gx: i32 = tok[1].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad tile gx: {e}"))?;
    let gz: i32 = tok[2].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad tile gz: {e}"))?;
    let module = tok[3].to_string();
    let seed: u64 = tok[5].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad tile seed: {e}"))?;
    Ok(SavedTile { at: [gx, gz], module, seed })
}

fn write_sdf_record(s: &mut String, prefix: &str, e: &Edit, author: Option<&str>) {
    let mat = e.material.map_or("none", Material::name);
    let [cx, cy, cz] = e.center;
    let tool = format!(" tool {}", e.tool.save_name());
    let tool = if e.mode == WorldMode::World { format!("{tool} mode world") } else { tool };
    let auth = author.map_or_else(String::new, |a| format!(" author {a}"));
    match e.shape {
        Shape::Sphere { radius } => {
            s.push_str(&format!(
                "{prefix} {} sphere {} {} {} {} blend {} mat {} size {}{tool}{auth}\n",
                e.op.name(),
                cx,
                cy,
                cz,
                radius,
                e.blend,
                mat,
                e.stamp_size
            ));
        }
        Shape::Box { half: [hx, hy, hz], round } => {
            s.push_str(&format!(
                "{prefix} {} box {} {} {} {} {} {} round {} blend {} mat {} size {}{tool}{auth}\n",
                e.op.name(),
                cx,
                cy,
                cz,
                hx,
                hy,
                hz,
                round,
                e.blend,
                mat,
                e.stamp_size
            ));
        }
        Shape::Cylinder { radius, half_height, round } => {
            s.push_str(&format!(
                "{prefix} {} cyl {} {} {} {} {} round {} blend {} mat {} size {}{tool}{auth}\n",
                e.op.name(),
                cx,
                cy,
                cz,
                radius,
                half_height,
                round,
                e.blend,
                mat,
                e.stamp_size
            ));
        }
    }
}

fn parse_inv_line(line: &str, line_no: usize) -> Result<(String, Material, f32)> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    if tok.len() != 4 {
        bail!("line {line_no}: expected `inv <player> <material> <kg>`, got `{line}`");
    }
    let player = tok[1].to_string();
    let mat = material_from_name(tok[2]).ok_or_else(|| wasmtime::format_err!("line {line_no}: unknown material `{}`", tok[2]))?;
    let kg: f32 = tok[3].parse().map_err(|e| wasmtime::format_err!("line {line_no}: bad inventory mass: {e}"))?;
    if !kg.is_finite() || kg < 0.0 {
        bail!("line {line_no}: inventory mass must be finite and non-negative, got {kg}");
    }
    Ok((player, mat, kg))
}

fn parse_sdf_line(line: &str, line_no: usize) -> Result<(Edit, Option<String>)> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    let pf = |i: usize| -> Result<f32> {
        tok.get(i)
            .ok_or_else(|| wasmtime::format_err!("line {line_no}: unexpected end of sdf record"))?
            .parse::<f32>()
            .map_err(|e| wasmtime::format_err!("line {line_no}: bad float at token {i}: {e}"))
    };
    let expect = |i: usize, kw: &str| -> Result<()> {
        match tok.get(i) {
            Some(&s) if s == kw => Ok(()),
            other => bail!("line {line_no}: expected `{kw}` at token {i}, got {other:?}"),
        }
    };
    let op = Op::from_name(tok.get(1).copied().unwrap_or("")).ok_or_else(|| wasmtime::format_err!("line {line_no}: unknown sdf op"))?;
    let kind = tok.get(2).copied().unwrap_or("");
    let (shape, center, tail) = match kind {
        "sphere" => {
            let center = [pf(3)?, pf(4)?, pf(5)?];
            let radius = pf(6)?;
            (Shape::Sphere { radius }, center, 7)
        }
        "box" => {
            let center = [pf(3)?, pf(4)?, pf(5)?];
            let half = [pf(6)?, pf(7)?, pf(8)?];
            expect(9, "round")?;
            let round = pf(10)?;
            (Shape::Box { half, round }, center, 11)
        }
        "cyl" => {
            let center = [pf(3)?, pf(4)?, pf(5)?];
            let radius = pf(6)?;
            let half_height = pf(7)?;
            expect(8, "round")?;
            let round = pf(9)?;
            (Shape::Cylinder { radius, half_height, round }, center, 10)
        }
        _ => bail!("line {line_no}: unknown sdf shape `{kind}`"),
    };
    expect(tail, "blend")?;
    let blend = pf(tail + 1)?;
    expect(tail + 2, "mat")?;
    let mat_tok = tok.get(tail + 3).copied().ok_or_else(|| wasmtime::format_err!("line {line_no}: missing material"))?;
    let material = if mat_tok == "none" {
        None
    } else {
        Some(material_from_name(mat_tok).ok_or_else(|| wasmtime::format_err!("line {line_no}: unknown material `{mat_tok}`"))?)
    };
    expect(tail + 4, "size")?;
    let stamp_size = pf(tail + 5)?;
    let mut tool: Option<Tool> = None;
    let mut mode: Option<WorldMode> = None;
    let mut author: Option<String> = None;
    let mut idx = tail + 6;
    while idx < tok.len() {
        match tok[idx] {
            "tool" => {
                if tool.is_some() {
                    bail!("line {line_no}: duplicate `tool` in sdf record");
                }
                let name = tok.get(idx + 1).copied().ok_or_else(|| wasmtime::format_err!("line {line_no}: missing tool name"))?;
                tool = Some(Tool::from_name(name).ok_or_else(|| wasmtime::format_err!("line {line_no}: unknown tool `{name}`"))?);
                idx += 2;
            }
            "mode" => {
                if mode.is_some() {
                    bail!("line {line_no}: duplicate `mode` in sdf record");
                }
                let name = tok.get(idx + 1).copied().ok_or_else(|| wasmtime::format_err!("line {line_no}: missing mode name"))?;
                mode = Some(WorldMode::from_name(name).ok_or_else(|| wasmtime::format_err!("line {line_no}: unknown mode `{name}`"))?);
                idx += 2;
            }
            "author" => {
                if author.is_some() {
                    bail!("line {line_no}: duplicate `author` in sdf record");
                }
                let a = tok.get(idx + 1).copied().ok_or_else(|| wasmtime::format_err!("line {line_no}: missing author"))?;
                author = Some(a.to_string());
                idx += 2;
            }
            other => bail!("line {line_no}: trailing token `{other}` in sdf record"),
        }
    }
    Ok((Edit { op, shape, center, blend, material, stamp_size, tool: tool.unwrap_or(Tool::All), mode: mode.unwrap_or_default() }, author))
}

fn parse_element_line(line: &str, line_no: usize) -> Result<PlacedElement> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    if tok.len() < 17 {
        bail!("line {line_no}: expected at least 17 tokens in element record, got {}", tok.len());
    }
    let expect = |i: usize, kw: &str| -> Result<()> {
        match tok.get(i) {
            Some(&s) if s == kw => Ok(()),
            other => bail!("line {line_no}: expected `{kw}` at token {i}, got {other:?}"),
        }
    };
    let pf = |i: usize| -> Result<f32> {
        tok.get(i)
            .ok_or_else(|| wasmtime::format_err!("line {line_no}: unexpected end of element record at token {i}"))?
            .parse::<f32>()
            .map_err(|e| wasmtime::format_err!("line {line_no}: bad float at token {i}: {e}"))
    };
    let id = tok[1].parse::<u32>().map_err(|e| wasmtime::format_err!("line {line_no}: bad element id: {e}"))?;
    let module = tok[2].to_string();
    expect(3, "seed")?;
    let seed = tok[4].parse::<u64>().map_err(|e| wasmtime::format_err!("line {line_no}: bad element seed: {e}"))?;
    expect(5, "pos")?;
    let pos = [pf(6)?, pf(7)?, pf(8)?];
    expect(9, "yaw")?;
    let yaw = pf(10)?;
    expect(11, "scale")?;
    let scale = pf(12)?;
    expect(13, "size")?;
    let stamp_size = pf(14)?;
    expect(15, "after")?;
    let after_edits = tok[16].parse::<usize>().map_err(|e| wasmtime::format_err!("line {line_no}: bad after_edits: {e}"))?;

    let mut rot = None;
    let mut promoted = None;
    let mut clips = Vec::new();
    let mut author = None;
    let mut params = Vec::new();
    let mut idx = 17;
    while idx < tok.len() {
        match tok[idx] {
            "rot" => {
                rot = Some([pf(idx + 1)?, pf(idx + 2)?, pf(idx + 3)?, pf(idx + 4)?]);
                idx += 5;
            }
            "promoted" => {
                let p = tok
                    .get(idx + 1)
                    .ok_or_else(|| wasmtime::format_err!("line {line_no}: missing promoted index"))?
                    .parse::<usize>()
                    .map_err(|e| wasmtime::format_err!("line {line_no}: bad promoted index: {e}"))?;
                promoted = Some(p);
                idx += 2;
            }
            "clip" => {
                let normal = [pf(idx + 1)?, pf(idx + 2)?, pf(idx + 3)?];
                let offset = pf(idx + 4)?;
                clips.push(ClipPlane { normal, offset });
                idx += 5;
            }
            "author" => {
                let a = tok.get(idx + 1).ok_or_else(|| wasmtime::format_err!("line {line_no}: missing element author"))?;
                author = Some((*a).to_string());
                idx += 2;
            }
            "params" => {
                if !params.is_empty() {
                    bail!("line {line_no}: duplicate `params` in element record");
                }
                let n = tok
                    .get(idx + 1)
                    .ok_or_else(|| wasmtime::format_err!("line {line_no}: missing params count"))?
                    .parse::<usize>()
                    .map_err(|e| wasmtime::format_err!("line {line_no}: bad params count: {e}"))?;
                if n == 0 || n > crate::wasm::MOST_PARAMS {
                    bail!("line {line_no}: an element has 1 to {} params, not {n}", crate::wasm::MOST_PARAMS);
                }
                params = (0..n).map(|k| pf(idx + 2 + k)).collect::<Result<_>>()?;
                idx += 2 + n;
            }
            other => bail!("line {line_no}: unknown element field `{other}`"),
        }
    }
    Ok(PlacedElement { id, module, seed, pos, yaw, rot, scale, stamp_size, after_edits, promoted, clips, local_edits: Vec::new(), author, params })
}

fn material_from_name(name: &str) -> Option<Material> {
    Material::ALL.into_iter().find(|m| m.name() == name)
}

// Deterministic float helpers (matching `plateau_sdk`, avoiding signed-zero ambiguity).

#[inline]
fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn abs_f32(a: f32) -> f32 {
    if a < 0.0 { -a } else { a }
}

#[inline]
fn smooth_union(a: f32, b: f32, k: f32) -> f32 {
    let h = max_f32(k - abs_f32(a - b), 0.0) / k;
    min_f32(a, b) - h * h * k * 0.25
}

#[inline]
fn smooth_cut(a: f32, b: f32, k: f32) -> f32 {
    -smooth_union(-a, b, k)
}

#[inline]
fn point_to_aabb_distance(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let dx = max_f32(max_f32(lo[0] - p[0], 0.0), p[0] - hi[0]);
    let dy = max_f32(max_f32(lo[1] - p[1], 0.0), p[1] - hi[1]);
    let dz = max_f32(max_f32(lo[2] - p[2], 0.0), p[2] - hi[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

