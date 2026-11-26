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
use std::path::Path;
use std::sync::Arc;

use wasmtime::bail;

use crate::compose::PlacedElement;
use crate::cut::ClipPlane;
use crate::graph::{Kind, Node as GraphNode, Relation};
use crate::modules::{SavedModule, parse_module_line, write_module_records};
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
}

impl Edit {
    pub fn add(shape: Shape, center: [f32; 3], blend: f32, material: Material, stamp_size: f32) -> Edit {
        Edit { op: Op::Add, shape, center, blend, material: Some(material), stamp_size }
    }

    pub fn carve(shape: Shape, center: [f32; 3], blend: f32, material: Option<Material>, stamp_size: f32) -> Edit {
        Edit { op: Op::Carve, shape, center, blend, material, stamp_size }
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

/// One node in the chronological interval BVH over `EditLog::edits`.
/// Left child is always strictly earlier in time than right child so CSG order is preserved.
#[derive(Clone, Debug)]
struct Node {
    start: usize,
    end: usize,
    lo: [f32; 3],
    hi: [f32; 3],
    max_blend: f32,
    has_add: bool,
    has_carve: bool,
    kind: NodeKind,
}

#[derive(Clone, Copy, Debug)]
enum NodeKind {
    Leaf(usize),
    Branch { left: usize, right: usize },
}

/// Ordered log of SDF edits with a chronological BVH for fast exact batch evaluation and revision tracking.
#[derive(Clone, Debug, Default)]
pub struct EditLog {
    edits: Vec<Edit>,
    nodes: Vec<Node>,
    revision: u64,
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
        let mut log = EditLog { edits, nodes: Vec::new(), revision };
        log.rebuild_index();
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
        self.edits.push(edit);
        self.revision += 1;
        self.rebuild_index();
        Ok(self.revision)
    }

    /// Removes and returns the most recent edit (undo), bumping `revision` if non-empty.
    pub fn pop(&mut self) -> Option<Edit> {
        let removed = self.edits.pop()?;
        self.revision += 1;
        self.rebuild_index();
        Some(removed)
    }

    /// Clears all edits, bumping `revision` if non-empty.
    pub fn clear(&mut self) {
        if !self.edits.is_empty() {
            self.edits.clear();
            self.nodes.clear();
            self.revision += 1;
        }
    }

    /// Whether any edit in this log can affect chunk `key`.
    pub fn affects_chunk(&self, key: chunk::Key) -> bool {
        self.edits.iter().any(|e| e.affects_chunk(key))
    }

    /// Applies all edits in chronological order to `(points, out)` using exact BVH pruning.
    pub fn apply_batch(&self, points: &[[f32; 3]], out: &mut [Answer]) {
        self.apply_batch_range(0, self.edits.len(), points, out);
    }

    /// Applies edits in the chronological slice `[range_start..range_end)` to `(points, out)`
    /// using exact BVH range & spatial pruning (bit-exact with sequential evaluation).
    pub fn apply_batch_range(&self, range_start: usize, range_end: usize, points: &[[f32; 3]], out: &mut [Answer]) {
        let range_end = range_end.min(self.edits.len());
        if range_start >= range_end || self.edits.is_empty() || points.is_empty() {
            return;
        }
        debug_assert_eq!(points.len(), out.len());
        let mut blo = [f32::INFINITY; 3];
        let mut bhi = [f32::NEG_INFINITY; 3];
        for p in points {
            for a in 0..3 {
                blo[a] = min_f32(blo[a], p[a]);
                bhi[a] = max_f32(bhi[a], p[a]);
            }
        }
        let mut batch_min_d = f32::INFINITY;
        let mut batch_max_d = f32::NEG_INFINITY;
        for ans in out.iter() {
            batch_min_d = min_f32(batch_min_d, ans.distance);
            batch_max_d = max_f32(batch_max_d, ans.distance);
        }
        if !self.nodes.is_empty() {
            self.visit(0, range_start, range_end, blo, bhi, &mut batch_min_d, &mut batch_max_d, points, out);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn visit(
        &self,
        idx: usize,
        range_start: usize,
        range_end: usize,
        blo: [f32; 3],
        bhi: [f32; 3],
        batch_min_d: &mut f32,
        batch_max_d: &mut f32,
        points: &[[f32; 3]],
        out: &mut [Answer],
    ) {
        let node = &self.nodes[idx];
        if node.end <= range_start || node.start >= range_end {
            return;
        }
        let box_d = aabb_to_aabb_distance(blo, bhi, node.lo, node.hi);
        if box_d > 0.0 {
            let need_add = node.has_add && *batch_max_d > box_d - node.max_blend;
            let need_carve = node.has_carve && -*batch_min_d > box_d - node.max_blend;
            if !need_add && !need_carve {
                return;
            }
        }
        match node.kind {
            NodeKind::Branch { left, right } => {
                self.visit(left, range_start, range_end, blo, bhi, batch_min_d, batch_max_d, points, out);
                self.visit(right, range_start, range_end, blo, bhi, batch_min_d, batch_max_d, points, out);
            }
            NodeKind::Leaf(edit_idx) => {
                let edit = &self.edits[edit_idx];
                let (elo, ehi) = (node.lo, node.hi);
                let blend = edit.blend;
                match edit.op {
                    Op::Add => {
                        for (&p, ans) in points.iter().zip(out.iter_mut()) {
                            let pd = point_to_aabb_distance(p, elo, ehi);
                            if pd > 0.0 && ans.distance <= pd - blend {
                                continue;
                            }
                            let next = edit.apply(p, *ans);
                            *ans = next;
                            *batch_min_d = min_f32(*batch_min_d, next.distance);
                            *batch_max_d = max_f32(*batch_max_d, next.distance);
                        }
                    }
                    Op::Carve => {
                        for (&p, ans) in points.iter().zip(out.iter_mut()) {
                            let pd = point_to_aabb_distance(p, elo, ehi);
                            if pd > 0.0 && -ans.distance <= pd - blend {
                                continue;
                            }
                            let next = edit.apply(p, *ans);
                            *ans = next;
                            *batch_min_d = min_f32(*batch_min_d, next.distance);
                            *batch_max_d = max_f32(*batch_max_d, next.distance);
                        }
                    }
                }
            }
        }
    }

    fn rebuild_index(&mut self) {
        self.nodes.clear();
        if self.edits.is_empty() {
            return;
        }
        self.nodes.reserve(self.edits.len() * 2);
        self.build_node(0, self.edits.len());
    }

    fn build_node(&mut self, start: usize, end: usize) -> usize {
        if end - start == 1 {
            let e = &self.edits[start];
            let (lo, hi) = e.bounds();
            let id = self.nodes.len();
            self.nodes.push(Node {
                start,
                end,
                lo,
                hi,
                max_blend: e.blend,
                has_add: e.op == Op::Add,
                has_carve: e.op == Op::Carve,
                kind: NodeKind::Leaf(start),
            });
            return id;
        }
        let id = self.nodes.len();
        self.nodes.push(Node {
            start,
            end,
            lo: [0.0; 3],
            hi: [0.0; 3],
            max_blend: 0.0,
            has_add: false,
            has_carve: false,
            kind: NodeKind::Leaf(0),
        });
        let mid = start + (end - start) / 2;
        let left = self.build_node(start, mid);
        let right = self.build_node(mid, end);
        let (nl, nr) = (&self.nodes[left], &self.nodes[right]);
        let lo = [min_f32(nl.lo[0], nr.lo[0]), min_f32(nl.lo[1], nr.lo[1]), min_f32(nl.lo[2], nr.lo[2])];
        let hi = [max_f32(nl.hi[0], nr.hi[0]), max_f32(nl.hi[1], nr.hi[1]), max_f32(nl.hi[2], nr.hi[2])];
        self.nodes[id] = Node {
            start,
            end,
            lo,
            hi,
            max_blend: max_f32(nl.max_blend, nr.max_blend),
            has_add: nl.has_add || nr.has_add,
            has_carve: nl.has_carve || nr.has_carve,
            kind: NodeKind::Branch { left, right },
        };
        id
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
}

impl WorldSave {
    pub fn new(area: impl Into<String>, seed: u64, edits: Vec<Edit>) -> WorldSave {
        WorldSave {
            version: SAVE_VERSION,
            area: area.into(),
            seed,
            tiles: Vec::new(),
            modules: Vec::new(),
            rules: RuleBook::new(),
            water: Vec::new(),
            edits,
            edit_authors: Vec::new(),
            elements: Vec::new(),
            relations: Vec::new(),
        }
    }

    pub fn with_elements(area: impl Into<String>, seed: u64, edits: Vec<Edit>, elements: Vec<PlacedElement>) -> WorldSave {
        WorldSave {
            version: SAVE_VERSION,
            area: area.into(),
            seed,
            tiles: Vec::new(),
            modules: Vec::new(),
            rules: RuleBook::new(),
            water: Vec::new(),
            edits,
            edit_authors: Vec::new(),
            elements,
            relations: Vec::new(),
        }
    }

    /// Serializes to a human-readable, deterministic, bit-exact text representation.
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("plateau-world {SAVE_VERSION}\n"));
        s.push_str(&format!("area {}\n", self.area.replace(['\n', '\r'], " ")));
        s.push_str(&format!("seed {}\n", self.seed));
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
            s.push('\n');
            for le in &el.local_edits {
                write_sdf_record(&mut s, &format!("el-sdf {}", el.id), le, None);
            }
        }
        for r in &self.relations {
            s.push_str(&r.to_record());
            s.push('\n');
        }
        s
    }

    /// Parses and validates a world save file.
    pub fn from_text(text: &str) -> Result<WorldSave> {
        let mut version = None;
        let mut area = String::new();
        let mut seed = None;
        let mut tiles = Vec::new();
        let mut modules: Vec<SavedModule> = Vec::new();
        let mut rules = RuleBook::new();
        let mut water: Vec<WaterBody> = Vec::new();
        let mut edits = Vec::new();
        let mut edit_authors: Vec<Option<String>> = Vec::new();
        let mut any_edit_author = false;
        let mut elements: Vec<PlacedElement> = Vec::new();
        let mut relations: Vec<Relation> = Vec::new();
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
        Ok(WorldSave { version, area, seed, tiles, modules, rules, water, edits, edit_authors, elements, relations })
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
    let auth = author.map_or_else(String::new, |a| format!(" author {a}"));
    match e.shape {
        Shape::Sphere { radius } => {
            s.push_str(&format!(
                "{prefix} {} sphere {} {} {} {} blend {} mat {} size {}{auth}\n",
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
                "{prefix} {} box {} {} {} {} {} {} round {} blend {} mat {} size {}{auth}\n",
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
                "{prefix} {} cyl {} {} {} {} {} round {} blend {} mat {} size {}{auth}\n",
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
    let author = if tok.len() == tail + 6 {
        None
    } else if tok.len() == tail + 8 && tok[tail + 6] == "author" {
        Some(tok[tail + 7].to_string())
    } else {
        bail!("line {line_no}: trailing tokens in sdf record");
    };
    Ok((Edit { op, shape, center, blend, material, stamp_size }, author))
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
            other => bail!("line {line_no}: unknown element field `{other}`"),
        }
    }
    Ok(PlacedElement { id, module, seed, pos, yaw, rot, scale, stamp_size, after_edits, promoted, clips, local_edits: Vec::new(), author })
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

#[inline]
fn aabb_to_aabb_distance(alo: [f32; 3], ahi: [f32; 3], blo: [f32; 3], bhi: [f32; 3]) -> f32 {
    let dx = max_f32(max_f32(blo[0] - ahi[0], 0.0), alo[0] - bhi[0]);
    let dy = max_f32(max_f32(blo[1] - ahi[1], 0.0), alo[1] - bhi[1]);
    let dz = max_f32(max_f32(blo[2] - ahi[2], 0.0), alo[2] - bhi[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}
