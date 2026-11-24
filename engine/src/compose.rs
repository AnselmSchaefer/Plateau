//! Composable standalone `.wasm` elements (POC_PLAN.md, step 8b; AGENTS.md).
//!
//! Lets players and Claude compose standalone sandboxed `.wasm` elements (trees, bridges,
//! lanterns, carvable hills/arches, contraptions) onto any existing area at any scale
//! (`tiny`, `person`, `giant`) without recompiling the base area:
//!
//! - **Same strict Wasmtime sandbox**: every element uses `plateau:region@0.1.0` (`check::check`,
//!   fuel & memory caps, zero host capabilities beyond `seed()`).
//! - **Hybrid Layer Ownership**:
//!   - Pure procedural `Scene` elements (`element_bounds() == None`) never touch SDF sampling or
//!     invalidate terrain chunks; they add isolated meshes, solid collision, lights, and local-space
//!     `tick` / `interact` behavior to `SceneWorld`.
//!   - Carvable SDF landform elements (`element_bounds() == Some(bounds)`) compose chronologically
//!     with `EditLog` inside `Composed<R>`, preserving exact Lipschitz safety (`|grad d| <= 1`)
//!     under uniform scaling and rotation so players can carve caves into placed hills at any scale.
//! - **Deterministic local↔world transforms**: uses deterministic IEEE-754 + polynomial `sin`/`cos`
//!   matching `plateau-sdk` so chunks and physics are bit-identical across machines and builds.

use std::borrow::Borrow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmtime::{bail, format_err};

use crate::edit::EditLog;
use crate::mesh_sdf::MeshSdf;
use crate::scene::{Applied, SceneWorld};
use crate::wasm::MAX_IMPULSE;
use crate::{Answer, Command, Compiled, Description, ElementBounds, Instance, LARGEST, Light, Limits, PlayerState, Region, Result, SMALLEST, Scene, Wasm, chunk};

/// World placement transform of a standalone element: position, yaw rotation about +Y, and uniform scale.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementTransform {
    pub pos: [f32; 3],
    pub yaw: f32,
    pub scale: f32,
}

impl PlacementTransform {
    pub const IDENTITY: PlacementTransform = PlacementTransform { pos: [0.0; 3], yaw: 0.0, scale: 1.0 };

    pub fn new(pos: [f32; 3], yaw: f32, scale: f32) -> PlacementTransform {
        PlacementTransform { pos, yaw, scale }
    }

    /// Unit quaternion for yaw rotation about +Y (matching `plateau_sdk::Quat::from_y` & Bevy).
    pub fn rot_quat(self) -> [f32; 4] {
        let half = self.yaw * 0.5;
        [0.0, sin_f32(half), 0.0, cos_f32(half)]
    }

    #[inline]
    pub fn local_to_world_dir(self, d: [f32; 3]) -> [f32; 3] {
        let (s, c) = (sin_f32(self.yaw), cos_f32(self.yaw));
        [c * d[0] + s * d[2], d[1], -s * d[0] + c * d[2]]
    }

    #[inline]
    pub fn world_to_local_dir(self, d: [f32; 3]) -> [f32; 3] {
        let (s, c) = (sin_f32(self.yaw), cos_f32(self.yaw));
        [c * d[0] - s * d[2], d[1], s * d[0] + c * d[2]]
    }

    #[inline]
    pub fn local_to_world_point(self, p: [f32; 3]) -> [f32; 3] {
        let r = self.local_to_world_dir([p[0] * self.scale, p[1] * self.scale, p[2] * self.scale]);
        [self.pos[0] + r[0], self.pos[1] + r[1], self.pos[2] + r[2]]
    }

    #[inline]
    pub fn world_to_local_point(self, p: [f32; 3]) -> [f32; 3] {
        let d = [p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]];
        let r = self.world_to_local_dir(d);
        let inv = 1.0 / self.scale;
        [r[0] * inv, r[1] * inv, r[2] * inv]
    }

    #[inline]
    pub fn local_to_world_rot(self, q: [f32; 4]) -> [f32; 4] {
        quat_mul(self.rot_quat(), q)
    }

    /// Transforms an element-local `Instance` into world coordinates.
    pub fn transform_instance(self, inst: Instance) -> Instance {
        Instance {
            id: inst.id,
            mesh: inst.mesh,
            pos: self.local_to_world_point(inst.pos),
            rot: self.local_to_world_rot(inst.rot),
            scale: [inst.scale[0] * self.scale, inst.scale[1] * self.scale, inst.scale[2] * self.scale],
            solid: inst.solid,
        }
    }

    /// Transforms an element-local `Light` into world coordinates with physically scaled intensity
    /// (`intensity * scale^2`) and range (`radius * scale`).
    pub fn transform_light(self, l: Light) -> Light {
        Light {
            id: l.id,
            pos: self.local_to_world_point(l.pos),
            color: l.color,
            intensity: (l.intensity * self.scale * self.scale).clamp(0.0, 100_000.0),
            radius: (l.radius * self.scale).clamp(0.01, 64.0),
        }
    }

    /// Transforms a world `PlayerState` into this element's local coordinate space, scoping
    /// `target` so the element only sees hits on its own instances.
    pub fn world_to_local_player(self, p: &PlayerState, owner: u32, hit_owner: u32, hit_id: u32) -> PlayerState {
        let inv = 1.0 / self.scale;
        let vel = self.world_to_local_dir(p.velocity);
        PlayerState {
            feet: self.world_to_local_point(p.feet),
            eye: self.world_to_local_point(p.eye),
            look: self.world_to_local_dir(p.look),
            velocity: [vel[0] * inv, vel[1] * inv, vel[2] * inv],
            size: (p.size * inv).clamp(SMALLEST, LARGEST),
            grounded: p.grounded,
            holding_use: p.holding_use,
            target: if hit_owner == owner { hit_id } else { 0 },
            aim: self.world_to_local_point(p.aim),
            time: p.time,
        }
    }

    /// Transforms an element-local `Command` into world space, clamping physical impulses.
    pub fn transform_command(self, cmd: Command) -> Command {
        match cmd {
            Command::SetTransform(inst) => Command::SetTransform(self.transform_instance(inst)),
            Command::SetLight(l) => Command::SetLight(self.transform_light(l)),
            Command::ApplyImpulse(dv) => {
                let mut w = self.local_to_world_dir([dv[0] * self.scale, dv[1] * self.scale, dv[2] * self.scale]);
                let mag = (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt();
                if mag > MAX_IMPULSE {
                    let s = MAX_IMPULSE / mag;
                    w = [w[0] * s, w[1] * s, w[2] * s];
                }
                Command::ApplyImpulse(w)
            }
            other => other,
        }
    }

    /// Tight world-space AABB enclosing rotated & scaled local `bounds` (without smooth blend).
    pub fn world_bounds(self, bounds: ElementBounds) -> ([f32; 3], [f32; 3]) {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for sx in [bounds.lo[0], bounds.hi[0]] {
            for sy in [bounds.lo[1], bounds.hi[1]] {
                for sz in [bounds.lo[2], bounds.hi[2]] {
                    let wp = self.local_to_world_point([sx, sy, sz]);
                    for a in 0..3 {
                        lo[a] = min_f32(lo[a], wp[a]);
                        hi[a] = max_f32(hi[a], wp[a]);
                    }
                }
            }
        }
        (lo, hi)
    }

    /// World-space AABB expanded by the scaled smooth blend width.
    pub fn world_influence_bounds(self, bounds: ElementBounds) -> ([f32; 3], [f32; 3]) {
        let (lo, hi) = self.world_bounds(bounds);
        let b = bounds.blend * self.scale;
        ([lo[0] - b, lo[1] - b, lo[2] - b], [hi[0] + b, hi[1] + b, hi[2] + b])
    }

    /// Whether an SDF element with `bounds` at this transform can affect chunk `key`.
    pub fn affects_chunk(self, bounds: ElementBounds, key: chunk::Key) -> bool {
        let (elo, ehi) = self.world_influence_bounds(bounds);
        let o = key.origin();
        let side = key.side();
        let pad = 2.0 * key.voxel() + 0.4 * key.size() as f64;
        for a in 0..3 {
            if (ehi[a] as f64) < o[a] - pad || (elo[a] as f64) > o[a] + side + pad {
                return false;
            }
        }
        true
    }
}

/// One placed standalone `.wasm` element record (persisted in `WorldSave`).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacedElement {
    /// Monotonic placement id (`>= 1`; `0` is reserved for the base area).
    pub id: u32,
    /// Relative or canonical module path (no whitespace), e.g. `elements/modules/tree.wasm`.
    pub module: String,
    pub seed: u64,
    pub pos: [f32; 3],
    pub yaw: f32,
    pub scale: f32,
    /// Player height in metres when placed.
    pub stamp_size: f32,
    /// Number of SDF `Edit`s that existed before this element was placed (preserves chronological
    /// CSG order so later carves cut into placed SDF hills while earlier pits stay beneath them).
    pub after_edits: usize,
    /// When a pure-`Scene` element is lazily promoted to carvable SDF on first sculpt/carve, holds
    /// the chronological edit index before which its solid `MeshSdf` is inserted (`None` until carved).
    pub promoted: Option<usize>,
}

impl PlacedElement {
    pub fn transform(&self) -> PlacementTransform {
        PlacementTransform { pos: self.pos, yaw: self.yaw, scale: self.scale }
    }

    /// Chronological `EditLog` split index where this element enters the SDF timeline.
    pub fn sdf_after_edits(&self) -> usize {
        self.promoted.unwrap_or(self.after_edits)
    }

    pub fn is_promoted(&self) -> bool {
        self.promoted.is_some()
    }

    pub fn validate(&self) -> Result<()> {
        if self.id == 0 {
            bail!("placed element id must be >= 1 (0 is reserved for the base area)");
        }
        if self.module.is_empty() || self.module.chars().any(char::is_whitespace) {
            bail!("placed element module path must be non-empty without whitespace, got {:?}", self.module);
        }
        if !self.pos.iter().all(|c| c.is_finite()) {
            bail!("placed element pos must be finite, got {:?}", self.pos);
        }
        if !self.yaw.is_finite() {
            bail!("placed element yaw must be finite, got {}", self.yaw);
        }
        if !self.scale.is_finite() || self.scale <= 0.0 || self.scale > 100.0 {
            bail!("placed element scale must be finite and in (0, 100], got {}", self.scale);
        }
        if !self.stamp_size.is_finite() || self.stamp_size <= 0.0 {
            bail!("placed element stamp_size must be finite and positive, got {}", self.stamp_size);
        }
        Ok(())
    }
}

/// A compiled, sandbox-verified standalone element module with cached metadata & preview scene.
pub struct CompiledElement {
    pub module: String,
    pub name: String,
    pub compiled: Arc<Compiled>,
    pub sdf_bounds: Option<ElementBounds>,
    pub preview_scene: Scene,
}

impl CompiledElement {
    /// Verifies, compiles, and probes metadata (`describe`, `element_bounds`, `scene`) for `bytes`.
    pub fn load(module: impl Into<String>, bytes: &[u8], limits: Limits) -> Result<Arc<CompiledElement>> {
        let module = module.into();
        let compiled = Arc::new(Compiled::new(bytes, limits)?);
        let mut probe = compiled.instantiate(1)?;
        let desc = probe.describe()?;
        let sdf_bounds = probe.element_bounds()?;
        let preview_scene = probe.scene()?;
        Ok(Arc::new(CompiledElement { module, name: desc.name, compiled, sdf_bounds, preview_scene }))
    }

    pub fn is_sdf(&self) -> bool {
        self.sdf_bounds.is_some()
    }

    /// Builds a deterministic element-local `MeshSdf` from all `solid: true` instances emitted by
    /// this element for `seed`. Returns `Ok(None)` if the element has no solid scene triangles.
    pub fn build_mesh_sdf(&self, seed: u64) -> Result<Option<Arc<MeshSdf>>> {
        let mut inst = self.compiled.instantiate(seed)?;
        let sc = inst.scene()?;
        Ok(MeshSdf::from_solid_scene(&sc).map(Arc::new))
    }
}

/// Caches compiled standalone element `.wasm` modules and lazily promoted `MeshSdf`s.
#[derive(Default)]
pub struct ElementCatalog {
    root: PathBuf,
    limits: Limits,
    entries: HashMap<String, Arc<CompiledElement>>,
    promoted_sdfs: HashMap<(String, u64), Option<Arc<MeshSdf>>>,
}

impl ElementCatalog {
    pub fn new(root: impl Into<PathBuf>, limits: Limits) -> ElementCatalog {
        ElementCatalog { root: root.into(), limits, entries: HashMap::new(), promoted_sdfs: HashMap::new() }
    }

    /// Builds (or retrieves from cache) the exact local `MeshSdf` for a lazily promoted `Scene`
    /// element `(module, seed)`.
    pub fn get_or_build_mesh_sdf(&mut self, module: &str, seed: u64) -> Result<Option<Arc<MeshSdf>>> {
        let key = (module.to_string(), seed);
        if let Some(cached) = self.promoted_sdfs.get(&key) {
            return Ok(cached.clone());
        }
        let el = self.get_or_load(module)?;
        let sdf = el.build_mesh_sdf(seed)?;
        self.promoted_sdfs.insert(key, sdf.clone());
        Ok(sdf)
    }

    pub fn cached_mesh_sdf(&self, module: &str, seed: u64) -> Option<&Arc<MeshSdf>> {
        self.promoted_sdfs.get(&(module.to_string(), seed)).and_then(Option::as_ref)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Inserts an already-compiled element directly (useful in tests or in-memory modules).
    pub fn insert(&mut self, element: Arc<CompiledElement>) {
        self.entries.insert(element.module.clone(), element);
    }

    /// Resolves `module` (either absolute or relative to `self.root`), compiles once, and caches it.
    pub fn get_or_load(&mut self, module: &str) -> Result<Arc<CompiledElement>> {
        if let Some(existing) = self.entries.get(module) {
            return Ok(existing.clone());
        }
        let path = Path::new(module);
        let full = if path.is_absolute() { path.to_path_buf() } else { self.root.join(path) };
        let bytes = std::fs::read(&full).map_err(|e| format_err!("{}: {e}", full.display()))?;
        let compiled = CompiledElement::load(module.to_string(), &bytes, self.limits)?;
        self.entries.insert(module.to_string(), compiled.clone());
        Ok(compiled)
    }

    pub fn get(&self, module: &str) -> Option<&Arc<CompiledElement>> {
        self.entries.get(module)
    }

    /// Discovers `.wasm` files under `elements/modules/*.wasm` and `made/elements/*/element.wasm`.
    pub fn discover(&mut self) -> Vec<Arc<CompiledElement>> {
        let mut found = Vec::new();
        let mut try_add = |rel: String, full: &Path| {
            if let Ok(bytes) = std::fs::read(full)
                && let Ok(el) = CompiledElement::load(rel.clone(), &bytes, self.limits)
            {
                self.entries.insert(rel, el.clone());
                found.push(el);
            }
        };

        let modules_dir = self.root.join("elements/modules");
        if let Ok(read) = std::fs::read_dir(&modules_dir) {
            let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|ext| ext == "wasm")).collect();
            paths.sort();
            for p in paths {
                if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                    try_add(format!("elements/modules/{name}"), &p);
                }
            }
        }

        let made_dir = self.root.join("made/elements");
        if let Ok(read) = std::fs::read_dir(&made_dir) {
            let mut dirs: Vec<PathBuf> = read.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
            dirs.sort();
            for d in dirs {
                let wasm = d.join("element.wasm");
                if wasm.exists()
                    && let Some(name) = d.file_name().and_then(|n| n.to_str())
                {
                    try_add(format!("made/elements/{name}/element.wasm"), &wasm);
                }
            }
        }
        found
    }
}

/// Where an SDF element entry gets its local-space signed distance field.
#[derive(Clone)]
pub enum SdfSource {
    /// Native carvable SDF `.wasm` element (`element_bounds() == Some(...)`).
    Wasm(Arc<Compiled>),
    /// Solid triangle meshes of a `Scene` element lazily promoted to carvable SDF on first sculpt.
    PromotedMesh(Arc<MeshSdf>),
}

/// One placed SDF element entry inside `ComposedLog`, with precomputed world AABB for fast pruning.
#[derive(Clone)]
pub struct SdfElementEntry {
    pub placed: PlacedElement,
    pub source: SdfSource,
    pub bounds: ElementBounds,
    pub world_lo: [f32; 3],
    pub world_hi: [f32; 3],
    pub world_blend: f32,
}

impl SdfElementEntry {
    pub fn new(placed: PlacedElement, compiled: Arc<Compiled>, bounds: ElementBounds) -> SdfElementEntry {
        let tf = placed.transform();
        let (world_lo, world_hi) = tf.world_bounds(bounds);
        let world_blend = bounds.blend * placed.scale;
        SdfElementEntry { placed, source: SdfSource::Wasm(compiled), bounds, world_lo, world_hi, world_blend }
    }

    pub fn from_promoted_mesh(placed: PlacedElement, mesh_sdf: Arc<MeshSdf>) -> SdfElementEntry {
        let bounds = mesh_sdf.bounds();
        let tf = placed.transform();
        let (world_lo, world_hi) = tf.world_bounds(bounds);
        SdfElementEntry { placed, source: SdfSource::PromotedMesh(mesh_sdf), bounds, world_lo, world_hi, world_blend: 0.0 }
    }

    pub fn affects_chunk(&self, key: chunk::Key) -> bool {
        self.placed.transform().affects_chunk(self.bounds, key)
    }
}

/// Immutable, thread-safe snapshot of all terrain edits (`EditLog`) and placed SDF elements
/// (both native SDF landforms and lazily promoted `Scene` meshes), shared via `Arc<ComposedLog>`.
#[derive(Clone)]
pub struct ComposedLog {
    pub edits: Arc<EditLog>,
    /// Chronologically ordered by `(placed.sdf_after_edits(), placed.id)`.
    pub sdf_elements: Vec<SdfElementEntry>,
    revision: u64,
}

impl Default for ComposedLog {
    fn default() -> Self {
        ComposedLog::empty()
    }
}

impl ComposedLog {
    pub fn empty() -> ComposedLog {
        ComposedLog { edits: Arc::new(EditLog::new()), sdf_elements: Vec::new(), revision: 0 }
    }

    pub fn new(edits: Arc<EditLog>, mut sdf_elements: Vec<SdfElementEntry>, revision: u64) -> ComposedLog {
        sdf_elements.sort_by_key(|e| (e.placed.sdf_after_edits(), e.placed.id));
        ComposedLog { edits, sdf_elements, revision }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn with_edits(edits: Arc<EditLog>) -> ComposedLog {
        let rev = edits.revision();
        ComposedLog { edits, sdf_elements: Vec::new(), revision: rev }
    }
}

/// Wraps any base `Region` with `ComposedLog` (`EditLog` + placed SDF `.wasm` elements) so chunk
/// meshing, `Body::step`, `body::look`, `rings::open`, `reach`, `picture`, and `laws` all see both
/// sculpt edits and placed SDF elements automatically in chronological order.
pub struct Composed<R, L = Arc<ComposedLog>> {
    pub region: R,
    pub log: L,
    /// Per-thread cache of instantiated SDF element Wasmtime modules keyed by `PlacedElement::id`.
    instances: HashMap<u32, (u64, Wasm)>,
}

impl<R, L> Composed<R, L> {
    pub fn new(region: R, log: L) -> Composed<R, L> {
        Composed { region, log, instances: HashMap::new() }
    }

    /// Evicts cached Wasmtime instances for elements that were removed from the current log.
    pub fn prune_instances(&mut self)
    where
        L: Borrow<ComposedLog>,
    {
        let log = self.log.borrow();
        self.instances.retain(|id, _| log.sdf_elements.iter().any(|e| e.placed.id == *id));
    }
}

impl<R: Region, L: Borrow<ComposedLog>> Region for Composed<R, L> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        self.region.sample(points, size, out)?;
        let log = self.log.borrow();
        if log.sdf_elements.is_empty() {
            log.edits.apply_batch(points, out);
            return Ok(());
        }
        if points.is_empty() {
            return Ok(());
        }

        let mut blo = [f32::INFINITY; 3];
        let mut bhi = [f32::NEG_INFINITY; 3];
        for p in points {
            for a in 0..3 {
                blo[a] = min_f32(blo[a], p[a]);
                bhi[a] = max_f32(bhi[a], p[a]);
            }
        }

        let mut cursor = 0usize;
        let mut local_pts: Vec<[f32; 3]> = Vec::new();
        let mut active_idx: Vec<usize> = Vec::new();
        let mut elem_out: Vec<Answer> = Vec::new();

        for entry in &log.sdf_elements {
            let end = entry.placed.sdf_after_edits().min(log.edits.len());
            if cursor < end {
                log.edits.apply_batch_range(cursor, end, points, out);
                cursor = end;
            }

            let box_d = aabb_to_aabb_distance(blo, bhi, entry.world_lo, entry.world_hi);
            if box_d > 0.0 {
                let mut batch_max_d = f32::NEG_INFINITY;
                for ans in out.iter() {
                    batch_max_d = max_f32(batch_max_d, ans.distance);
                }
                if batch_max_d <= box_d - entry.world_blend {
                    continue;
                }
            }

            let tf = entry.placed.transform();
            let scale = entry.placed.scale;
            let blend = entry.world_blend;

            match &entry.source {
                SdfSource::PromotedMesh(mesh_sdf) => {
                    let inv_scale = 1.0 / scale;
                    for (&p, cur) in points.iter().zip(out.iter_mut()) {
                        let pd = point_to_aabb_distance(p, entry.world_lo, entry.world_hi);
                        if pd > 0.0 && cur.distance <= pd {
                            continue;
                        }
                        let local_p = tf.world_to_local_point(p);
                        // Tight upper bound: only care about triangles closer than `cur.distance`
                        // when outside, or any distance when inside the mesh AABB (`pd == 0`).
                        let max_local = if pd > 0.0 { cur.distance * inv_scale } else { f32::INFINITY };
                        if let Some(ans_e) = mesh_sdf.sample(local_p, max_local) {
                            let d_world = ans_e.distance * scale;
                            if d_world < cur.distance {
                                *cur = Answer { distance: d_world, material: ans_e.material };
                            }
                        }
                    }
                }
                SdfSource::Wasm(compiled) => {
                    local_pts.clear();
                    active_idx.clear();
                    for (idx, (&p, ans)) in points.iter().zip(out.iter()).enumerate() {
                        let pd = point_to_aabb_distance(p, entry.world_lo, entry.world_hi);
                        if pd > 0.0 && ans.distance <= pd - blend {
                            continue;
                        }
                        active_idx.push(idx);
                        local_pts.push(tf.world_to_local_point(p));
                    }
                    if local_pts.is_empty() {
                        continue;
                    }

                    let id = entry.placed.id;
                    let seed = entry.placed.seed;
                    let need_new = self.instances.get(&id).is_none_or(|(s, _)| *s != seed);
                    if need_new {
                        let inst = compiled.instantiate(seed)?;
                        self.instances.insert(id, (seed, inst));
                    }
                    let local_size = (size / scale).clamp(SMALLEST, LARGEST);
                    let res = {
                        let (_, wasm) = self.instances.get_mut(&id).unwrap();
                        wasm.sample(&local_pts, local_size, &mut elem_out)
                    };
                    if let Err(e) = res {
                        self.instances.remove(&id);
                        return Err(e);
                    }

                    for (&idx, &ans_e) in active_idx.iter().zip(elem_out.iter()) {
                        let d_world = ans_e.distance * scale;
                        let cur = out[idx];
                        let mat = if d_world < cur.distance { ans_e.material } else { cur.material };
                        let dist = if blend > 0.0 { smooth_union(cur.distance, d_world, blend) } else { min_f32(cur.distance, d_world) };
                        out[idx] = Answer { distance: dist, material: mat };
                    }
                }
            }
        }

        if cursor < log.edits.len() {
            log.edits.apply_batch_range(cursor, log.edits.len(), points, out);
        }
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

/// One live placed element running on the main thread for `tick` / `interact`.
struct LiveElement {
    placed: PlacedElement,
    local_scene: Scene,
    wasm: Wasm,
}

/// Owns live per-placement sandboxed Wasmtime instances for runtime `tick` / `interact` and
/// synchronizes their procedural `Scene` layers into `SceneWorld`.
#[derive(Default)]
pub struct ElementRuntime {
    live: Vec<LiveElement>,
}

impl ElementRuntime {
    pub fn new() -> ElementRuntime {
        ElementRuntime::default()
    }

    pub fn placed(&self) -> impl Iterator<Item = &PlacedElement> {
        self.live.iter().map(|l| &l.placed)
    }

    pub fn len(&self) -> usize {
        self.live.len()
    }

    pub fn is_empty(&self) -> bool {
        self.live.is_empty()
    }

    pub fn get(&self, id: u32) -> Option<&PlacedElement> {
        self.live.iter().find(|l| l.placed.id == id).map(|l| &l.placed)
    }

    /// Instantiates a placed element, registers its `Scene` layer in `world` (omitting `solid: true`
    /// instances when `placed.is_promoted()`), and returns the world-transformed `Scene` ready for
    /// GPU upload.
    pub fn spawn(&mut self, placed: PlacedElement, compiled: &CompiledElement, world: &mut SceneWorld) -> Result<Scene> {
        placed.validate()?;
        let mut wasm = compiled.compiled.instantiate(placed.seed)?;
        let local_scene = wasm.scene()?;
        let tf = placed.transform();
        world.add_layer_filtered(placed.id, &local_scene, Some(tf), placed.is_promoted());
        let world_scene = transform_scene(&local_scene, tf);
        self.live.push(LiveElement { placed, local_scene, wasm });
        Ok(world_scene)
    }

    /// Updates a live element's promotion state (`promoted: Option<usize>`), refreshing its
    /// `SceneWorld` layer so promoted elements hide their solid meshes (now owned by the SDF layer)
    /// while keeping non-solid decorative parts and lights alive. Returns the world-transformed
    /// `Scene` so the renderer can refresh that layer.
    pub fn set_promoted(&mut self, id: u32, promoted: Option<usize>, world: &mut SceneWorld) -> Option<Scene> {
        let l = self.live.iter_mut().find(|l| l.placed.id == id)?;
        l.placed.promoted = promoted;
        let tf = l.placed.transform();
        world.add_layer_filtered(id, &l.local_scene, Some(tf), promoted.is_some());
        Some(transform_scene(&l.local_scene, tf))
    }

    /// Removes a placed element by `id`, dropping its Wasmtime store and removing its `SceneWorld` layer.
    pub fn remove(&mut self, id: u32, world: &mut SceneWorld) -> Option<PlacedElement> {
        let pos = self.live.iter().position(|l| l.placed.id == id)?;
        let removed = self.live.remove(pos);
        world.remove_layer(id);
        Some(removed.placed)
    }

    /// Removes all placed elements and clears their layers from `world`.
    pub fn clear(&mut self, world: &mut SceneWorld) {
        for l in self.live.drain(..) {
            world.remove_layer(l.placed.id);
        }
    }

    /// Runs `tick` on every placed element in its own local coordinate frame, applying transformed
    /// commands strictly within each element's own `SceneWorld` layer.
    pub fn tick_all(&mut self, dt: f32, player: &PlayerState, hit_owner: u32, hit_id: u32, world: &mut SceneWorld) -> Vec<Applied> {
        let mut results = Vec::new();
        for l in &mut self.live {
            let tf = l.placed.transform();
            let local_ps = tf.world_to_local_player(player, l.placed.id, hit_owner, hit_id);
            match l.wasm.tick(dt, &local_ps) {
                Ok(cmds) if !cmds.is_empty() => {
                    results.push(world.apply_for(l.placed.id, &cmds, Some(tf)));
                }
                Ok(_) => {}
                Err(e) => eprintln!("element {} ({}) tick error: {e}", l.placed.id, l.placed.module),
            }
        }
        results
    }

    /// Routes a player interaction (`F` / right-click):
    /// - If the crosshair targets a placed element (`hit_owner > 0`), calls `interact` on that element.
    /// - Otherwise, calls `interact` on placed elements within reasonable proximity of the player.
    pub fn interact(&mut self, player: &PlayerState, hit_owner: u32, hit_id: u32, world: &mut SceneWorld) -> Vec<Applied> {
        let mut results = Vec::new();
        for l in &mut self.live {
            let targeted = hit_owner == l.placed.id;
            let nearby = if hit_owner == 0 {
                let d2: f32 = (0..3).map(|a| (player.feet[a] - l.placed.pos[a]).powi(2)).sum();
                let reach = max_f32(player.size * 6.0, l.placed.scale * 6.0);
                d2 <= reach * reach
            } else {
                false
            };
            if !targeted && !nearby {
                continue;
            }
            let tf = l.placed.transform();
            let local_ps = tf.world_to_local_player(player, l.placed.id, hit_owner, hit_id);
            match l.wasm.interact(&local_ps) {
                Ok(cmds) if !cmds.is_empty() => {
                    results.push(world.apply_for(l.placed.id, &cmds, Some(tf)));
                }
                Ok(_) => {}
                Err(e) => eprintln!("element {} ({}) interact error: {e}", l.placed.id, l.placed.module),
            }
        }
        results
    }
}

/// Transforms a local `Scene`'s instances and lights into world coordinates (meshes stay local).
pub fn transform_scene(scene: &Scene, tf: PlacementTransform) -> Scene {
    Scene {
        meshes: scene.meshes.clone(),
        instances: scene.instances.iter().map(|&inst| tf.transform_instance(inst)).collect(),
        lights: scene.lights.iter().map(|&l| tf.transform_light(l)).collect(),
    }
}

// Deterministic math helpers (matching `plateau-sdk` and `edit.rs`).

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

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
fn sin_f32(x: f32) -> f32 {
    let turns = x * (1.0 / TAU);
    let mut r = (turns - (turns + 0.5).floor()) * TAU;
    if r > PI / 2.0 {
        r = PI - r;
    } else if r < -PI / 2.0 {
        r = -PI - r;
    }
    let r2 = r * r;
    r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0 + r2 * (-1.0 / 39_916_800.0))))))
}

#[inline]
fn cos_f32(x: f32) -> f32 {
    sin_f32(x + PI / 2.0)
}

#[inline]
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

#[inline]
fn smooth_union(a: f32, b: f32, k: f32) -> f32 {
    let h = max_f32(k - abs_f32(a - b), 0.0) / k;
    min_f32(a, b) - h * h * k * 0.25
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
