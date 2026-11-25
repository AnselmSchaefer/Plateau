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

use crate::cut::{ClipPlane, ObjectPhysics, clip_scene};
use crate::edit::{Edit, EditLog, Op};
use crate::mesh_sdf::MeshSdf;
use crate::scene::{Applied, SceneWorld};
use crate::wasm::MAX_IMPULSE;
use crate::water::WaterLog;
use crate::{Answer, Command, Compiled, Description, ElementBounds, Instance, LARGEST, Light, Limits, PlayerState, Region, Result, SMALLEST, Scene, Wasm, chunk};

/// Maximum number of clip planes stored on one `PlacedElement`.
pub const MAX_CLIPS: usize = 16;

/// Maximum `viewer_size / placed.scale` ratio at which a lazily promoted `Scene` `MeshSdf` is
/// rendered in Surface Nets SDF chunks. Above this ratio (e.g., a 17 m giant viewing a 1x-scale
/// carved oak trunk whose diameter is smaller than a 0.5–1.0 m giant voxel), coarse SDF chunks
/// skip the sub-voxel `MeshSdf` while the element's crisp procedural `Scene` mesh takes over as a
/// macro-scale proxy — preventing thin trunks/planks from disintegrating at giant LOD.
pub const PROMOTED_SDF_MAX_SIZE_RATIO: f32 = 6.5;

/// World placement transform of a standalone element: position, full 3D unit quaternion rotation,
/// and uniform scale. Rigid rotation + uniform scale preserves exact SDF Lipschitz safety (`|grad d| <= 1`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlacementTransform {
    pub pos: [f32; 3],
    /// Unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
    pub scale: f32,
}

impl PlacementTransform {
    pub const IDENTITY: PlacementTransform = PlacementTransform { pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0], scale: 1.0 };

    /// Upright placement rotated by `yaw` radians around +Y.
    pub fn new(pos: [f32; 3], yaw: f32, scale: f32) -> PlacementTransform {
        PlacementTransform { pos, rot: yaw_quat(yaw), scale }
    }

    /// Full 3D placement with normalized unit quaternion `rot`.
    pub fn with_rot(pos: [f32; 3], rot: [f32; 4], scale: f32) -> PlacementTransform {
        PlacementTransform { pos, rot: quat_normalize(rot), scale }
    }

    /// Placement from intrinsic YXZ Euler angles (`yaw` about +Y, `pitch` about +X, `roll` about +Z).
    pub fn from_euler(pos: [f32; 3], yaw: f32, pitch: f32, roll: f32, scale: f32) -> PlacementTransform {
        PlacementTransform { pos, rot: euler_quat(yaw, pitch, roll), scale }
    }

    /// Unit quaternion `[x, y, z, w]` of this placement.
    #[inline]
    pub fn rot_quat(self) -> [f32; 4] {
        self.rot
    }

    #[inline]
    pub fn local_to_world_dir(self, d: [f32; 3]) -> [f32; 3] {
        quat_rotate(self.rot, d)
    }

    #[inline]
    pub fn world_to_local_dir(self, d: [f32; 3]) -> [f32; 3] {
        quat_rotate(quat_conj(self.rot), d)
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
        quat_mul(self.rot, q)
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
    /// Optional full 3D unit quaternion `[x, y, z, w]` overriding `yaw` (for felled/tilted objects).
    pub rot: Option<[f32; 4]>,
    pub scale: f32,
    /// Player height in metres when placed.
    pub stamp_size: f32,
    /// Number of SDF `Edit`s that existed before this element was placed (preserves chronological
    /// CSG order so later carves cut into placed SDF hills while earlier pits stay beneath them).
    pub after_edits: usize,
    /// When a pure-`Scene` element is lazily promoted to carvable SDF on first sculpt/carve, holds
    /// the chronological edit index before which its solid `MeshSdf` is inserted (`None` until carved).
    pub promoted: Option<usize>,
    /// Optional local half-space clip planes created by chopping/cutting this element (Step 8c).
    pub clips: Vec<ClipPlane>,
    /// Optional element-local SDF edits (tunnels, notches, additions) sculpted directly onto this
    /// promoted `Scene` element so they travel and rotate with the object when chopped or carried!
    pub local_edits: Vec<Edit>,
    /// Optional creator/owner player identifier (`None` for legacy saves / unclaimed elements).
    pub author: Option<String>,
}

impl PlacedElement {
    pub fn transform(&self) -> PlacementTransform {
        match self.rot {
            Some(q) => PlacementTransform::with_rot(self.pos, q, self.scale),
            None => PlacementTransform::new(self.pos, self.yaw, self.scale),
        }
    }

    /// Updates `pos`, `scale`, and orientation (`yaw` / `rot`) from a world `PlacementTransform`.
    pub fn set_transform(&mut self, tf: PlacementTransform) {
        self.pos = tf.pos;
        self.scale = tf.scale;
        let q = quat_normalize(tf.rot);
        if quat_is_yaw_only(q) {
            self.yaw = 2.0 * q[1].atan2(q[3]);
            self.rot = None;
        } else {
            self.rot = Some(q);
        }
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
        if let Some(q) = self.rot {
            if !q.iter().all(|c| c.is_finite()) {
                bail!("placed element rot must be finite, got {:?}", q);
            }
            let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
            if !(0.95..=1.05).contains(&len) {
                bail!("placed element rot must be a unit quaternion, got length {len}");
            }
        }
        if !self.scale.is_finite() || self.scale <= 0.0 || self.scale > 100.0 {
            bail!("placed element scale must be finite and in (0, 100], got {}", self.scale);
        }
        if !self.stamp_size.is_finite() || self.stamp_size <= 0.0 {
            bail!("placed element stamp_size must be finite and positive, got {}", self.stamp_size);
        }
        if self.clips.len() > MAX_CLIPS {
            bail!("placed element has {} clip planes (max {MAX_CLIPS})", self.clips.len());
        }
        for c in &self.clips {
            c.validate()?;
        }
        for e in &self.local_edits {
            e.validate()?;
        }
        if let Some(a) = &self.author
            && (a.is_empty() || a.chars().any(char::is_whitespace))
        {
            bail!("placed element author must be non-empty without whitespace, got {:?}", a);
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
    /// this element for `seed`, clipped by `clips`. Returns `Ok(None)` if no solid triangles remain.
    pub fn build_mesh_sdf(&self, seed: u64, clips: &[ClipPlane]) -> Result<Option<Arc<MeshSdf>>> {
        let mut inst = self.compiled.instantiate(seed)?;
        let raw = inst.scene()?;
        let sc = clip_scene(&raw, clips);
        Ok(MeshSdf::from_solid_scene(&sc).map(Arc::new))
    }
}

type PromotedKey = (String, u64, Vec<[u32; 4]>);

fn promoted_key(module: &str, seed: u64, clips: &[ClipPlane]) -> PromotedKey {
    (module.to_string(), seed, clips.iter().map(|c| c.key_bits()).collect())
}

/// Caches compiled standalone element `.wasm` modules and lazily promoted `MeshSdf`s.
#[derive(Default)]
pub struct ElementCatalog {
    root: PathBuf,
    limits: Limits,
    entries: HashMap<String, Arc<CompiledElement>>,
    promoted_sdfs: HashMap<PromotedKey, Option<Arc<MeshSdf>>>,
}

impl ElementCatalog {
    pub fn new(root: impl Into<PathBuf>, limits: Limits) -> ElementCatalog {
        ElementCatalog { root: root.into(), limits, entries: HashMap::new(), promoted_sdfs: HashMap::new() }
    }

    /// Builds (or retrieves from cache) the exact local `MeshSdf` for a lazily promoted `Scene`
    /// element `(module, seed, clips)`.
    pub fn get_or_build_mesh_sdf(&mut self, module: &str, seed: u64, clips: &[ClipPlane]) -> Result<Option<Arc<MeshSdf>>> {
        let key = promoted_key(module, seed, clips);
        if let Some(cached) = self.promoted_sdfs.get(&key) {
            return Ok(cached.clone());
        }
        let el = self.get_or_load(module)?;
        let sdf = el.build_mesh_sdf(seed, clips)?;
        self.promoted_sdfs.insert(key, sdf.clone());
        Ok(sdf)
    }

    pub fn cached_mesh_sdf(&self, module: &str, seed: u64, clips: &[ClipPlane]) -> Option<&Arc<MeshSdf>> {
        self.promoted_sdfs.get(&promoted_key(module, seed, clips)).and_then(Option::as_ref)
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
    /// Optional element-local SDF edits (tunnels, notches, additions) applied in local space.
    pub local_edits: Arc<EditLog>,
    /// True if `local_edits` contains at least one `Op::Add` (which can expand surface past `MeshSdf`).
    pub has_local_add: bool,
}

impl SdfElementEntry {
    pub fn new(placed: PlacedElement, compiled: Arc<Compiled>, bounds: ElementBounds) -> SdfElementEntry {
        let tf = placed.transform();
        let (world_lo, world_hi) = tf.world_bounds(bounds);
        let world_blend = bounds.blend * placed.scale;
        SdfElementEntry {
            placed,
            source: SdfSource::Wasm(compiled),
            bounds,
            world_lo,
            world_hi,
            world_blend,
            local_edits: Arc::new(EditLog::new()),
            has_local_add: false,
        }
    }

    pub fn from_promoted_mesh(placed: PlacedElement, mesh_sdf: Arc<MeshSdf>) -> SdfElementEntry {
        let mut bounds = mesh_sdf.bounds();
        let mut has_local_add = false;
        for e in &placed.local_edits {
            if e.op == Op::Add {
                has_local_add = true;
                let (elo, ehi) = e.influence_bounds();
                for a in 0..3 {
                    bounds.lo[a] = min_f32(bounds.lo[a], elo[a]);
                    bounds.hi[a] = max_f32(bounds.hi[a], ehi[a]);
                }
            }
        }
        let local_edits = Arc::new(EditLog::from_edits(placed.local_edits.clone()).unwrap_or_default());
        let tf = placed.transform();
        let (world_lo, world_hi) = tf.world_bounds(bounds);
        SdfElementEntry {
            placed,
            source: SdfSource::PromotedMesh(mesh_sdf),
            bounds,
            world_lo,
            world_hi,
            world_blend: 0.0,
            local_edits,
            has_local_add,
        }
    }

    pub fn affects_chunk(&self, key: chunk::Key) -> bool {
        self.placed.transform().affects_chunk(self.bounds, key)
    }
}

/// Immutable, thread-safe snapshot of all terrain edits (`EditLog`), placed SDF elements
/// (both native SDF landforms and lazily promoted `Scene` meshes), and engine-managed hydrostatic
/// water bodies (`WaterLog`), shared via `Arc<ComposedLog>`.
#[derive(Clone)]
pub struct ComposedLog {
    pub edits: Arc<EditLog>,
    /// Chronologically ordered by `(placed.sdf_after_edits(), placed.id)`.
    pub sdf_elements: Vec<SdfElementEntry>,
    /// Hydrostatic water bodies composited after all solid edits and SDF elements.
    pub water: Arc<WaterLog>,
    revision: u64,
}

impl Default for ComposedLog {
    fn default() -> Self {
        ComposedLog::empty()
    }
}

impl ComposedLog {
    pub fn empty() -> ComposedLog {
        ComposedLog { edits: Arc::new(EditLog::new()), sdf_elements: Vec::new(), water: Arc::new(WaterLog::new()), revision: 0 }
    }

    pub fn new(edits: Arc<EditLog>, mut sdf_elements: Vec<SdfElementEntry>, revision: u64) -> ComposedLog {
        sdf_elements.sort_by_key(|e| (e.placed.sdf_after_edits(), e.placed.id));
        ComposedLog { edits, sdf_elements, water: Arc::new(WaterLog::new()), revision }
    }

    pub fn new_with_water(edits: Arc<EditLog>, mut sdf_elements: Vec<SdfElementEntry>, water: Arc<WaterLog>, revision: u64) -> ComposedLog {
        sdf_elements.sort_by_key(|e| (e.placed.sdf_after_edits(), e.placed.id));
        ComposedLog { edits, sdf_elements, water, revision }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn with_edits(edits: Arc<EditLog>) -> ComposedLog {
        let rev = edits.revision();
        ComposedLog { edits, sdf_elements: Vec::new(), water: Arc::new(WaterLog::new()), revision: rev }
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
            log.water.apply_batch(points, out);
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
                    // Macro-LOD guard: when the chunk/viewer scale is far coarser than the promoted
                    // element (e.g., giant 0.5–1.0 m voxels over a 1x-scale tree trunk), skip the
                    // sub-voxel `PromotedMesh` so thin trunks never disintegrate into floating shards;
                    // the crisp procedural `Scene` proxy renders & collides at macro scale instead.
                    if size > scale * PROMOTED_SDF_MAX_SIZE_RATIO {
                        continue;
                    }
                    let inv_scale = 1.0 / scale;
                    let has_local_edits = !entry.local_edits.is_empty();
                    let has_local_add = entry.has_local_add;
                    for (&p, cur) in points.iter().zip(out.iter_mut()) {
                        let pd = point_to_aabb_distance(p, entry.world_lo, entry.world_hi);
                        if pd > 0.0 && cur.distance <= pd {
                            continue;
                        }
                        let local_p = tf.world_to_local_point(p);
                        // Tight upper bound: only care about triangles closer than `cur.distance`
                        // when outside, or any distance when inside the element AABB (`pd == 0`).
                        let max_local = if pd > 0.0 && !has_local_add { cur.distance * inv_scale } else { f32::INFINITY };
                        let mut ans_local = match mesh_sdf.sample(local_p, max_local) {
                            Some(a) => a,
                            None if has_local_add => Answer { distance: 1e4, material: cur.material },
                            None => continue,
                        };
                        if has_local_edits {
                            let mut one = [ans_local];
                            entry.local_edits.apply_batch(&[local_p], &mut one);
                            ans_local = one[0];
                        }
                        let d_world = ans_local.distance * scale;
                        if d_world < cur.distance {
                            *cur = Answer { distance: d_world, material: ans_local.material };
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
        log.water.apply_batch(points, out);
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
    physics: ObjectPhysics,
    wasm: Wasm,
    held: bool,
    /// When true for a promoted element (viewer size > `scale * PROMOTED_SDF_MAX_SIZE_RATIO`),
    /// keeps the crisp procedural solid mesh visible & collidable as a macro-scale LOD proxy.
    solid_proxy: bool,
}

impl LiveElement {
    fn skip_solid(&self) -> bool {
        self.placed.is_promoted() && !self.solid_proxy
    }
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

    /// Returns the deterministic physical summary (local bounds, world mass, span, rooted state)
    /// of a live placed element.
    pub fn physics_of(&self, id: u32) -> Option<ObjectPhysics> {
        self.live.iter().find(|l| l.placed.id == id).map(|l| l.physics)
    }

    /// Returns the element-local (already clipped) `Scene` of a live placed element, useful for
    /// rendering in-hand carry previews and drop ghosts.
    pub fn local_scene_of(&self, id: u32) -> Option<&Scene> {
        self.live.iter().find(|l| l.placed.id == id).map(|l| &l.local_scene)
    }

    /// Whether `id` should currently omit its solid meshes from `SceneWorld` / Bevy rendering.
    pub fn should_skip_solid(&self, id: u32) -> bool {
        self.live.iter().find(|l| l.placed.id == id).is_some_and(LiveElement::skip_solid)
    }

    /// Instantiates a placed element, applies any `placed.clips` deterministically, registers its
    /// `Scene` layer in `world` (omitting `solid: true` instances when promoted at fine scale),
    /// and returns the world-transformed `Scene` ready for GPU upload.
    pub fn spawn(&mut self, placed: PlacedElement, compiled: &CompiledElement, world: &mut SceneWorld) -> Result<Scene> {
        self.spawn_for_size(placed, compiled, world, SIZES_PERSON)
    }

    /// Like `spawn`, initializing the macro-LOD solid proxy state for `viewer_size`.
    pub fn spawn_for_size(&mut self, placed: PlacedElement, compiled: &CompiledElement, world: &mut SceneWorld, viewer_size: f32) -> Result<Scene> {
        placed.validate()?;
        let mut wasm = compiled.compiled.instantiate(placed.seed)?;
        let raw_scene = wasm.scene()?;
        let local_scene = clip_scene(&raw_scene, &placed.clips);
        let physics = ObjectPhysics::compute(&local_scene, placed.scale);
        let solid_proxy = placed.is_promoted() && viewer_size > placed.scale * PROMOTED_SDF_MAX_SIZE_RATIO;
        let skip_solid = placed.is_promoted() && !solid_proxy;
        let tf = placed.transform();
        world.add_layer_filtered(placed.id, &local_scene, Some(tf), skip_solid);
        let world_scene = transform_scene(&local_scene, tf);
        self.live.push(LiveElement { placed, local_scene, physics, wasm, held: false, solid_proxy });
        Ok(world_scene)
    }

    /// Syncs macro-LOD solid proxies for all promoted elements against `viewer_size`.
    /// Returns `(id, world_scene, skip_solid)` for any element whose proxy visibility flipped.
    pub fn sync_macro_proxies(&mut self, viewer_size: f32, world: &mut SceneWorld) -> Vec<(u32, Scene, bool)> {
        let mut changed = Vec::new();
        for l in &mut self.live {
            if !l.placed.is_promoted() || l.held {
                continue;
            }
            let want_proxy = viewer_size > l.placed.scale * PROMOTED_SDF_MAX_SIZE_RATIO;
            if want_proxy != l.solid_proxy {
                l.solid_proxy = want_proxy;
                let tf = l.placed.transform();
                let skip = l.skip_solid();
                world.add_layer_filtered(l.placed.id, &l.local_scene, Some(tf), skip);
                changed.push((l.placed.id, transform_scene(&l.local_scene, tf), skip));
            }
        }
        changed
    }

    /// Moves and/or rotates a live placed element to `tf` without resetting its WASM instance,
    /// refreshing its `SceneWorld` layer (unless currently held) and returning the updated
    /// world-transformed `Scene` for GPU refresh.
    pub fn relocate(&mut self, id: u32, tf: PlacementTransform, world: &mut SceneWorld) -> Option<Scene> {
        let l = self.live.iter_mut().find(|l| l.placed.id == id)?;
        l.placed.set_transform(tf);
        let tf = l.placed.transform();
        l.physics = ObjectPhysics::compute(&l.local_scene, l.placed.scale);
        if !l.held {
            world.add_layer_filtered(id, &l.local_scene, Some(tf), l.skip_solid());
        }
        Some(transform_scene(&l.local_scene, tf))
    }

    /// Marks a live element as carried in-hand (`held = true`, removing its solid collision from
    /// `SceneWorld` and pausing its `tick`/`interact`) or sets it back down into the world
    /// (`held = false`, restoring solid collision and returning its world `Scene`).
    pub fn set_held(&mut self, id: u32, held: bool, world: &mut SceneWorld) -> Option<Scene> {
        let l = self.live.iter_mut().find(|l| l.placed.id == id)?;
        l.held = held;
        let tf = l.placed.transform();
        if held {
            world.remove_layer(id);
        } else {
            world.add_layer_filtered(id, &l.local_scene, Some(tf), l.skip_solid());
        }
        Some(transform_scene(&l.local_scene, tf))
    }

    /// Updates a live element's promotion state (`promoted: Option<usize>`), refreshing its
    /// `SceneWorld` layer so promoted elements hide their solid meshes (now owned by the SDF layer)
    /// while keeping non-solid decorative parts and lights alive. Returns the world-transformed
    /// `Scene` so the renderer can refresh that layer.
    pub fn set_promoted(&mut self, id: u32, promoted: Option<usize>, world: &mut SceneWorld) -> Option<Scene> {
        let l = self.live.iter_mut().find(|l| l.placed.id == id)?;
        l.placed.promoted = promoted;
        if promoted.is_none() {
            l.solid_proxy = false;
        }
        let tf = l.placed.transform();
        if !l.held {
            world.add_layer_filtered(id, &l.local_scene, Some(tf), l.skip_solid());
        }
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
            if l.held {
                continue;
            }
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
            if l.held {
                continue;
            }
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

// Deterministic math & quaternion helpers (matching `plateau-sdk` and `edit.rs`).

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;
const SIZES_PERSON: f32 = 1.7;

/// Deterministic unit quaternion for rotation by `yaw` radians around +Y.
#[inline]
pub fn yaw_quat(yaw: f32) -> [f32; 4] {
    let half = yaw * 0.5;
    [0.0, sin_f32(half), 0.0, cos_f32(half)]
}

/// Deterministic unit quaternion from YXZ Euler angles (`yaw` about +Y, `pitch` about +X, `roll` about +Z).
#[inline]
pub fn euler_quat(yaw: f32, pitch: f32, roll: f32) -> [f32; 4] {
    let qy = yaw_quat(yaw);
    let qx = [sin_f32(pitch * 0.5), 0.0, 0.0, cos_f32(pitch * 0.5)];
    let qz = [0.0, 0.0, sin_f32(roll * 0.5), cos_f32(roll * 0.5)];
    quat_mul(qy, quat_mul(qx, qz))
}

#[inline]
pub fn quat_normalize(q: [f32; 4]) -> [f32; 4] {
    let len = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if len > 1e-8 { [q[0] / len, q[1] / len, q[2] / len, q[3] / len] } else { [0.0, 0.0, 0.0, 1.0] }
}

#[inline]
pub fn quat_is_yaw_only(q: [f32; 4]) -> bool {
    abs_f32(q[0]) < 1e-5 && abs_f32(q[2]) < 1e-5
}

#[inline]
pub fn quat_conj(q: [f32; 4]) -> [f32; 4] {
    [-q[0], -q[1], -q[2], q[3]]
}

#[inline]
pub fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
}

#[inline]
pub fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let tx = 2.0 * (qy * v[2] - qz * v[1]);
    let ty = 2.0 * (qz * v[0] - qx * v[2]);
    let tz = 2.0 * (qx * v[1] - qy * v[0]);
    [
        v[0] + qw * tx + (qy * tz - qz * ty),
        v[1] + qw * ty + (qz * tx - qx * tz),
        v[2] + qw * tz + (qx * ty - qy * tx),
    ]
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
