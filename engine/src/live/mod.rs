//! The live world (`WORLD_PLAN.md`, phase E): everything a world is while someone plays it, and
//! every change to it.
//!
//! `World` owns the areas, the sculpting, water, placed elements and their sandboxed runtimes, the
//! procedural scenes, the rule book and the undo history. A change is asked for as an `Action` and
//! made by `World::apply`, which checks the rules and the tool, makes it, and runs what follows from
//! it (promotion of a carved element, water flooding a carve, buoyancy); each is written to the
//! action log, if one is open (`log`), so a log replays onto the save it began from.
//!
//! What a window must show of a change it learns afterwards, by draining `World::take_changes`: the
//! new SDF snapshots with the chunks each makes stale, the placed elements' scene layers to make
//! again, move, or take away, and what to tell the player. The engine knows nothing of Bevy.
//!
//! **What is logical and what is only shown.** Everything a save keeps changes only by an action:
//! a fall is decided when the cut is made, and ends (`finish_all_falls`) before the next action
//! whether or not it was seen to; an element settles on water at rest, unloaded. What only the
//! player sees and stands on (the topple under way, bobbing, a rider's weight pressing a boat down)
//! is `animate`d between actions and moves no saved transform, so a log replays without it.

pub mod action;
pub mod history;
pub mod log;
mod objects;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::info_span;
use wasmtime::bail;

use crate::atlas::{Atlas, CompiledAtlas, TileSpec};
use crate::body::Body;
use crate::compose::{Composed, ComposedLog, ElementCatalog, ElementRuntime, PlacedElement, PlacementTransform, SdfElementEntry, quat_normalize};
use crate::cut::ObjectPhysics;
use crate::edit::{Edit, EditLog, Op, SavedTile, WorldSave};
use crate::modules::{ModuleCheck, SavedModule, relative_to_root};
use crate::physics::{BuoyancyState, compute_buoyancy, player_mass_kg};
use crate::rules::RuleBook;
use crate::scene::{Applied, Combined, SceneWorld, is_area_owner};
use crate::water::{WaterLog, bounds_affect_chunk};
use crate::{Compiled, ElementBounds, Limits, PlayerState, Result, Scene, Wasm, chunk, math};

pub use action::{Action, Actor, Outcome, SculptTarget, Summary};
pub use history::{History, WorldAction};
pub use log::{ActionLog, LOG_VERSION, Logged, replay};

/// Which chunks a change makes stale.
#[derive(Clone, Debug, PartialEq)]
pub enum Invalidation {
    /// Every chunk (a reload).
    All,
    /// Those an edit, a placed SDF element's bounds (where it was and where it is), or water's
    /// changed bounds reach.
    Some {
        edits: Vec<Edit>,
        bounds: Vec<(PlacementTransform, ElementBounds)>,
        water: Option<([f32; 3], [f32; 3])>,
    },
}

impl Invalidation {
    pub fn edit(edit: Edit) -> Invalidation {
        Invalidation::Some { edits: vec![edit], bounds: Vec::new(), water: None }
    }

    pub fn element(tf: PlacementTransform, bounds: ElementBounds) -> Invalidation {
        Invalidation::Some { edits: Vec::new(), bounds: vec![(tf, bounds)], water: None }
    }

    /// Whether the chunk `key` is made stale.
    pub fn affects(&self, key: chunk::Key) -> bool {
        match self {
            Invalidation::All => true,
            Invalidation::Some { edits, bounds, water } => {
                edits.iter().any(|e| e.affects_chunk(key))
                    || bounds.iter().any(|&(tf, b)| tf.affects_chunk(b, key))
                    || water.is_some_and(|(lo, hi)| bounds_affect_chunk(lo, hi, key))
            }
        }
    }
}

/// A placed element's scene layer, as a window must show it.
#[derive(Clone, Debug)]
pub enum LayerChange {
    /// Make the layer of `owner` again from `scene` (world space), leaving out its solid meshes if
    /// `skip_solid` (they are in the SDF instead).
    Spawn { owner: u32, scene: Scene, skip_solid: bool },
    /// Take the layer of `owner` away.
    Despawn { owner: u32 },
    /// Every placed element's layer away (a reload).
    DespawnAllPlaced,
    /// The layer of `owner`, made at `from`, now stands at `to` (its meshes unchanged).
    Moved { owner: u32, from: PlacementTransform, to: PlacementTransform },
}

/// What changed since the window last looked, in order.
#[derive(Default)]
pub struct Changes {
    pub composed: Vec<(Arc<ComposedLog>, Invalidation)>,
    pub layers: Vec<LayerChange>,
    pub said: Vec<String>,
}

impl Changes {
    pub fn is_empty(&self) -> bool {
        self.composed.is_empty() && self.layers.is_empty() && self.said.is_empty()
    }
}

/// A chopped piece toppling, eased as if by gravity.
#[derive(Clone, Copy, Debug)]
pub struct FallingElement {
    pub id: u32,
    pub from_pos: [f32; 3],
    pub from_rot: [f32; 4],
    pub to_pos: [f32; 3],
    pub to_rot: [f32; 4],
    pub scale: f32,
    pub elapsed: f32,
    pub duration: f32,
}

impl FallingElement {
    pub fn sample(&self) -> PlacementTransform {
        let u = (self.elapsed / self.duration.max(1e-3)).clamp(0.0, 1.0);
        // Quadratic ease-in: tipping over, faster and faster.
        let t = u * u;
        let pos = [0, 1, 2].map(|a| self.from_pos[a] + (self.to_pos[a] - self.from_pos[a]) * t);
        PlacementTransform::with_rot(pos, nlerp_quat(self.from_rot, self.to_rot, t), self.scale)
    }

    pub fn end(&self) -> PlacementTransform {
        PlacementTransform::with_rot(self.to_pos, self.to_rot, self.scale)
    }
}

/// The element carried, and where it was picked up from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Held {
    pub id: u32,
    pub from: PlacementTransform,
}

/// A floating element's buoyancy, kept while nothing about it changes, so still boats ask nothing.
#[derive(Clone, Copy, Debug)]
pub struct CachedBuoyancy {
    pub state: BuoyancyState,
    pub world_rev: u64,
    pub rider_kg: f32,
}

/// The live world.
pub struct World {
    /// The areas' modules, compiled: what chunk workers make their own instances of.
    pub atlas: Arc<CompiledAtlas>,
    /// The areas with the sculpting, water and SDF elements over them, on this thread: what bodies,
    /// rays and measures ask.
    pub region: Composed<Atlas<Wasm>, Arc<ComposedLog>>,
    /// The areas' and placed elements' procedural scenes.
    pub scene: SceneWorld,
    pub runtime: ElementRuntime,
    pub catalog: ElementCatalog,
    pub placed: Vec<PlacedElement>,
    pub edits: Arc<EditLog>,
    pub edit_authors: Vec<Option<String>>,
    pub water: Arc<WaterLog>,
    pub rules: RuleBook,
    pub tiles: Vec<SavedTile>,
    /// The loaded save's module hashes: kept for a module that cannot be read when saving again.
    pub modules: Vec<SavedModule>,
    /// The folder modules are named relative to (the repository).
    pub root: PathBuf,
    pub module_check: ModuleCheck,
    pub area_name: String,
    pub seed: u64,
    pub history: History,
    pub next_id: u32,
    /// Changed since last saved.
    pub dirty: bool,
    /// The height of whoever the world is shown to, for elements' macro proxies.
    pub viewer_size: f32,
    /// The element carried, if any: out of the SDF and of collisions while it is.
    pub held: Option<Held>,
    pub falling: Vec<FallingElement>,
    /// Elements floating, by id (ordered, so every machine walks them alike).
    pub floating: BTreeMap<u32, CachedBuoyancy>,
    pub piloting: Option<u32>,
    /// Time for bobbing (shown only).
    pub float_time: f32,
    /// Where floating elements are shown, bobbing about their rest (their height), by id.
    shown: BTreeMap<u32, f32>,
    /// Modules opened although their bytes are not those the save recorded
    /// (`ModuleCheck::AcceptChanged`).
    pub changed_modules: Vec<String>,
    composed_revision: u64,
    changes: Changes,
    log: Option<ActionLog>,
}

impl World {
    /// Opens `save` (or a bare area) at `root`, for a viewer `viewer_size` tall: its tiles compiled
    /// from the modules it names, every module checked against the hashes it recorded (`check`), its
    /// elements spawned. `tiles` are the tiles to play if they are not the save's own. Returns the
    /// world and the areas' own scenes (owner, scene), for the window to show once.
    pub fn open(save: WorldSave, tiles: Option<Vec<SavedTile>>, root: &Path, limits: Limits, check: ModuleCheck, viewer_size: f32) -> Result<(World, Vec<(u32, Scene)>)> {
        let _span = info_span!("world_open").entered();
        let tiles = tiles.unwrap_or_else(|| save.tiles.clone());
        if tiles.is_empty() {
            bail!("the save does not say which module its area is: name it");
        }
        let mut specs = Vec::with_capacity(tiles.len());
        let mut changed_modules = Vec::new();
        for t in &tiles {
            let name = relative_to_root(&t.module, root);
            let read = save.read_module(&name, root, check)?;
            if read.changed {
                changed_modules.push(name);
            }
            specs.push(TileSpec { at: t.at, seed: t.seed, compiled: Arc::new(Compiled::new(&read.bytes, limits)?) });
        }
        for name in save.elements.iter().map(|e| e.module.clone()) {
            if save.read_module(&name, root, check)?.changed && !changed_modules.contains(&name) {
                changed_modules.push(name);
            }
        }
        let atlas = Arc::new(CompiledAtlas::new(specs)?);
        let mut live = atlas.instantiate()?;
        let mut scene = SceneWorld::default();
        let area_scenes = live.load_scenes_into(&mut scene)?;
        let mut catalog = ElementCatalog::new(root, limits);
        catalog.expect_hashes(&save.modules, check);
        let edits = Arc::new(EditLog::from_edits(save.edits.clone())?);
        let water = Arc::new(WaterLog::from_bodies(save.water.clone())?);
        let next_id = save.elements.iter().map(|e| e.id).max().unwrap_or(0) + 1;
        let mut world = World {
            atlas,
            region: Composed::new(live, Arc::new(ComposedLog::empty())),
            scene,
            runtime: ElementRuntime::new(),
            catalog,
            placed: Vec::new(),
            edits,
            edit_authors: save.edit_authors.clone(),
            water,
            rules: save.rules.clone(),
            tiles,
            modules: save.modules.clone(),
            root: root.to_path_buf(),
            module_check: check,
            area_name: save.area.clone(),
            seed: save.seed,
            history: History::default(),
            next_id,
            dirty: false,
            viewer_size,
            held: None,
            falling: Vec::new(),
            floating: BTreeMap::new(),
            piloting: None,
            float_time: 0.0,
            shown: BTreeMap::new(),
            changed_modules,
            composed_revision: 0,
            changes: Changes::default(),
            log: None,
        };
        for pl in save.elements {
            world.spawn_layer_for(pl)?;
        }
        world.commit(Invalidation::All);
        world.refresh_all_buoyancy();
        world.changes.said.clear();
        world.dirty = false;
        Ok((world, area_scenes))
    }

    /// The world as it would be saved now.
    pub fn to_save(&self) -> WorldSave {
        let mut save = WorldSave::with_elements(self.area_name.clone(), self.seed, self.edits.edits().to_vec(), self.placed.clone());
        save.tiles = self.tiles.clone();
        save.modules = self.modules.clone();
        save.rules = self.rules.clone();
        save.water = self.water.bodies().to_vec();
        save.edit_authors = self.edit_authors.clone();
        save
    }

    /// Settles anything falling and writes the world to `path`, sealed against the root.
    pub fn save(&mut self, path: &Path) -> Result<()> {
        self.finish_all_falls(None);
        let mut save = self.to_save();
        save.save_sealed(path, &self.root)?;
        self.modules = save.modules;
        self.dirty = false;
        Ok(())
    }

    /// Replaces the world's sculpting, water, rules and elements with those of the save at `path`
    /// (its areas stay those already open). Every module it names is checked first, so a refused
    /// save leaves the world as it was. Returns how many edits and elements it holds.
    pub fn reload(&mut self, path: &Path) -> Result<(usize, usize)> {
        let saved = WorldSave::load(path)?;
        for name in saved.named_modules() {
            saved.read_module(&name, &self.root, self.module_check)?;
        }
        self.held = None;
        self.falling.clear();
        self.floating.clear();
        self.shown.clear();
        self.piloting = None;
        self.modules = saved.modules.clone();
        self.catalog.expect_hashes(&saved.modules, self.module_check);
        let next_rev = self.edits.revision() + 1;
        self.edits = Arc::new(EditLog::from_edits_with_revision(saved.edits, next_rev)?);
        self.edit_authors = saved.edit_authors;
        self.water = Arc::new(WaterLog::from_bodies(saved.water)?);
        self.rules = saved.rules;
        self.runtime.clear(&mut self.scene);
        self.changes.layers.push(LayerChange::DespawnAllPlaced);
        self.placed.clear();
        self.history = History::default();
        let edit_count = self.edits.len();
        for pl in saved.elements {
            let id = pl.id;
            self.spawn_layer_for(pl)?;
            self.next_id = self.next_id.max(id + 1);
        }
        self.commit(Invalidation::All);
        self.refresh_all_buoyancy();
        self.dirty = false;
        Ok((edit_count, self.placed.len()))
    }

    /// Opens an action log at `path` (written as actions are applied), or closes it with `None`.
    pub fn set_log(&mut self, log: Option<ActionLog>) {
        self.log = log;
    }

    /// What changed since this was last asked, for the window to show.
    pub fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }

    /// The SDF the world now is, as last committed.
    pub fn composed(&self) -> Arc<ComposedLog> {
        self.region.log.clone()
    }

    /// The region with the procedural scenes' solids in it: what bodies and rays meet.
    pub fn combined(&mut self) -> Combined<'_, Composed<Atlas<Wasm>, Arc<ComposedLog>>> {
        Combined { region: &mut self.region, world: &self.scene }
    }

    /// Something to tell the player.
    pub fn say(&mut self, what: impl Into<String>) {
        self.changes.said.push(what.into());
    }

    pub fn find(&self, id: u32) -> Option<&PlacedElement> {
        self.placed.iter().find(|e| e.id == id)
    }

    fn index_of(&self, id: u32) -> Option<usize> {
        self.placed.iter().position(|e| e.id == id)
    }

    /// The name of the element `id` is, or `#id`.
    pub fn element_name(&self, id: u32) -> String {
        self.find(id).and_then(|pl| self.catalog.get(&pl.module)).map_or_else(|| format!("#{id}"), |c| c.name.clone())
    }

    /// Whether `pl` is an SDF landform element (rather than a procedural scene).
    pub fn is_sdf_element(&self, pl: &PlacedElement) -> bool {
        self.catalog.get(&pl.module).is_some_and(|c| c.is_sdf())
    }

    pub fn physics_of(&self, id: u32) -> Option<ObjectPhysics> {
        self.runtime.physics_of(id)
    }

    pub fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// The bounds of `pl` in the SDF layer, if it is there (an SDF element, or a promoted one, grown
    /// by what was added to it).
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
                if e.op == Op::Add {
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

    /// The nearest placed SDF element (native or promoted) whose world bounds the ray meets.
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

    /// A new SDF snapshot from the edits, water and every placed SDF element not carried or falling,
    /// given to the region here and, with the chunks `inv` makes stale, to the window.
    pub fn commit(&mut self, inv: Invalidation) {
        let _span = info_span!("commit").entered();
        let mut exclude: Vec<u32> = self.falling.iter().map(|f| f.id).collect();
        exclude.extend(self.held.map(|h| h.id));
        self.composed_revision += 1;
        let mut sdf_elements = Vec::new();
        for pl in &self.placed {
            if exclude.contains(&pl.id) {
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
        let composed = Arc::new(ComposedLog::new_with_water(self.edits.clone(), sdf_elements, self.water.clone(), self.composed_revision));
        self.region.log = composed.clone();
        self.region.prune_instances();
        self.changes.composed.push((composed, inv));
    }

    /// Promoted elements keep a solid proxy for viewers far larger than they are: made again when the
    /// viewer grows or shrinks past it.
    pub fn sync_viewer_size(&mut self, viewer_size: f32) {
        self.viewer_size = viewer_size;
        for (owner, scene, skip_solid) in self.runtime.sync_macro_proxies(viewer_size, &mut self.scene) {
            self.changes.layers.push(LayerChange::Spawn { owner, scene, skip_solid });
        }
    }

    /// Spawns `pl`'s runtime and scene layer, adds it to the placed elements (no SDF commit).
    fn spawn_layer_for(&mut self, pl: PlacedElement) -> Result<Arc<crate::compose::CompiledElement>> {
        let compiled = self.catalog.get_or_load(&pl.module)?;
        let scene = self.runtime.spawn_for_size(pl.clone(), &compiled, &mut self.scene, self.viewer_size)?;
        let skip_solid = self.runtime.should_skip_solid(pl.id);
        self.changes.layers.push(LayerChange::Spawn { owner: pl.id, scene, skip_solid });
        if pl.is_promoted() {
            self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips)?;
        }
        self.placed.push(pl);
        Ok(compiled)
    }

    /// Places `pl` (its id already allocated): spawned, in the SDF if it is an SDF element or
    /// promoted, settled on water if it floats; `body`, if given, lifted out of it. Its name.
    pub fn spawn_placed(&mut self, pl: PlacedElement, body: Option<&mut Body>) -> Result<String> {
        let _span = info_span!("spawn_placed", id = pl.id).entered();
        let compiled = self.spawn_layer_for(pl.clone())?;
        self.dirty = true;
        let sdf_bounds = compiled.sdf_bounds.or_else(|| if pl.is_promoted() { self.sdf_bounds_of(&pl) } else { None });
        let max_lift = if let Some(bounds) = sdf_bounds {
            self.commit(Invalidation::element(pl.transform(), bounds));
            let (_, hi) = pl.transform().world_influence_bounds(bounds);
            body.as_ref().map(|b| (hi[1] as f64 - b.feet[1]).max(0.0) + b.size as f64 * 2.0)
        } else {
            // A boat floats, a stone sinks.
            let _ = self.settle_buoyancy_for(pl.id, 0.0);
            body.as_ref().map(|b| (pl.scale * 3.0) as f64 + b.size as f64)
        };
        if let (Some(body), Some(lift)) = (body, max_lift) {
            let _ = body.surface_from_solid(&mut self.combined(), lift);
        }
        Ok(compiled.name.clone())
    }

    /// Takes the element `id` out of the world.
    pub fn remove_placed(&mut self, id: u32) -> Option<PlacedElement> {
        let _span = info_span!("remove_placed", id).entered();
        self.forget_motion(id);
        let idx = self.index_of(id)?;
        let removed = self.placed.remove(idx);
        let sdf_bounds = self.sdf_bounds_of(&removed);
        self.runtime.remove(id, &mut self.scene);
        self.changes.layers.push(LayerChange::Despawn { owner: id });
        self.dirty = true;
        if let Some(bounds) = sdf_bounds {
            self.commit(Invalidation::element(removed.transform(), bounds));
        }
        Some(removed)
    }

    /// Forgets `id` falling, floating or piloted.
    fn forget_motion(&mut self, id: u32) {
        self.falling.retain(|f| f.id != id);
        self.floating.remove(&id);
        self.shown.remove(&id);
        if self.piloting == Some(id) {
            self.piloting = None;
        }
    }

    /// Moves the element `id` to `tf`, its scene made again there and (unless it is falling) its SDF
    /// committed where it was and is; `body`, if given, lifted out of it.
    pub fn relocate_placed(&mut self, id: u32, tf: PlacementTransform, body: Option<&mut Body>) -> bool {
        let Some(pos) = self.index_of(id) else { return false };
        let old_tf = self.placed[pos].transform();
        let old_bounds = self.sdf_bounds_of(&self.placed[pos]);
        self.placed[pos].set_transform(tf);
        self.shown.remove(&id);
        let scale = self.placed[pos].scale;
        let mid_fall = self.falling.iter().any(|f| f.id == id);
        let skip_solid = if mid_fall { false } else { self.runtime.should_skip_solid(id) };
        if let Some(scene) = self.runtime.relocate(id, tf, &mut self.scene) {
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene, skip_solid });
        }
        if !mid_fall && let Some(bounds) = old_bounds {
            self.commit(Invalidation::Some { edits: Vec::new(), bounds: vec![(old_tf, bounds), (tf, bounds)], water: None });
        }
        self.dirty = true;
        if let Some(body) = body {
            let max_lift = (scale * 3.0) as f64 + body.size as f64;
            let _ = body.surface_from_solid(&mut self.combined(), max_lift);
        }
        true
    }

    /// Moves the element `id` to `tf` without making its meshes again (bobbing, sailing, falling).
    pub fn relocate_placed_fast(&mut self, id: u32, tf: PlacementTransform) -> bool {
        let Some(pos) = self.index_of(id) else { return false };
        let old_tf = self.placed[pos].transform();
        self.placed[pos].set_transform(tf);
        self.shown.remove(&id);
        if self.runtime.relocate_fast(id, tf, &mut self.scene) {
            self.changes.layers.push(LayerChange::Moved { owner: id, from: old_tf, to: tf });
            true
        } else {
            self.relocate_placed(id, tf, None)
        }
    }

    /// Promotes or demotes the element `id` (`Some(edit index)` / `None`), its layer made again.
    pub fn set_promoted(&mut self, id: u32, state: Option<usize>, skip_solid: Option<bool>) {
        if let Some(pos) = self.index_of(id) {
            self.placed[pos].promoted = state;
        }
        if let Some(scene) = self.runtime.set_promoted(id, state, &mut self.scene) {
            let skip_solid = skip_solid.unwrap_or_else(|| self.runtime.should_skip_solid(id));
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene, skip_solid });
        }
    }

    /// Sets promotion on several elements (for undo and redo): their bounds, for invalidation.
    pub fn set_elements_promoted(&mut self, ids: &[(u32, Option<usize>)]) -> Vec<(PlacementTransform, ElementBounds)> {
        let mut bounds_list = Vec::new();
        for &(id, next_state) in ids {
            let Some(pos) = self.index_of(id) else { continue };
            let pl = self.placed[pos].clone();
            if let Ok(Some(mesh_sdf)) = self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips) {
                bounds_list.push((pl.transform(), mesh_sdf.bounds()));
            }
            self.set_promoted(id, next_state, None);
        }
        bounds_list
    }

    /// Promotes every solid scene element `edit` reaches (and `aimed`), before it is applied at
    /// `edit_idx`, so the carve cuts them: their ids, bounds, and names.
    pub fn promote_overlapping_scene_elements(
        &mut self,
        edit: &Edit,
        aimed: Option<u32>,
        edit_idx: usize,
    ) -> (Vec<(u32, usize)>, Vec<(PlacementTransform, ElementBounds)>, Vec<String>) {
        let _span = info_span!("promote").entered();
        let (elo, ehi) = edit.influence_bounds();
        let mut candidates = self.scene.solid_placed_overlapping(elo, ehi);
        if let Some(owner) = aimed
            && !candidates.contains(&owner)
        {
            candidates.push(owner);
        }
        let (mut promoted, mut bounds_list, mut names) = (Vec::new(), Vec::new(), Vec::new());
        for id in candidates {
            // A floating boat is not turned to voxels by sculpting beside it.
            if self.floating.contains_key(&id) && aimed != Some(id) {
                continue;
            }
            let Some(pos) = self.index_of(id) else { continue };
            if self.placed[pos].is_promoted() {
                continue;
            }
            let pl = self.placed[pos].clone();
            let Ok(compiled) = self.catalog.get_or_load(&pl.module) else { continue };
            if compiled.is_sdf() {
                continue;
            }
            let Ok(Some(mesh_sdf)) = self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips) else { continue };
            self.set_promoted(id, Some(edit_idx), None);
            promoted.push((id, edit_idx));
            bounds_list.push((pl.transform(), mesh_sdf.bounds()));
            names.push(compiled.name.clone());
        }
        (promoted, bounds_list, names)
    }

    /// Picks the element `id` up: out of collisions, its layer away, out of the SDF.
    pub fn take_held(&mut self, id: u32) {
        let Some(pl) = self.find(id).cloned() else { return };
        let sdf_bounds = self.sdf_bounds_of(&pl);
        self.runtime.set_held(id, true, &mut self.scene);
        self.changes.layers.push(LayerChange::Despawn { owner: id });
        self.held = Some(Held { id, from: pl.transform() });
        self.floating.remove(&id);
        self.shown.remove(&id);
        if self.piloting == Some(id) {
            self.piloting = None;
        }
        if let Some(bounds) = sdf_bounds {
            self.commit(Invalidation::element(pl.transform(), bounds));
        }
    }

    /// Sets the element carried down at `tf`: back in collisions and the SDF, settled on water;
    /// the move recorded for undo if `undoable`; `body`, if given, lifted out of it.
    pub fn release_held(&mut self, tf: PlacementTransform, undoable: bool, body: Option<&mut Body>) -> Option<u32> {
        let Held { id, from } = self.held.take()?;
        let sdf_bounds = self.find(id).and_then(|pl| self.sdf_bounds_of(pl));
        let scale = self.find(id).map_or(1.0, |pl| pl.scale);
        if let Some(pos) = self.index_of(id) {
            self.placed[pos].set_transform(tf);
        }
        let _ = self.runtime.relocate(id, tf, &mut self.scene);
        if let Some(scene) = self.runtime.set_held(id, false, &mut self.scene) {
            let skip_solid = self.runtime.should_skip_solid(id);
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene, skip_solid });
        }
        let _ = self.settle_buoyancy_for(id, 0.0);
        if let Some(bounds) = sdf_bounds {
            self.commit(Invalidation::Some { edits: Vec::new(), bounds: vec![(from, bounds), (tf, bounds)], water: None });
        }
        self.dirty = true;
        if undoable {
            self.history.push(WorldAction::RelocateElement { id, before: from, after: tf });
        }
        if let Some(body) = body {
            let max_lift = (scale * 3.0) as f64 + body.size as f64;
            let _ = body.surface_from_solid(&mut self.combined(), max_lift);
        }
        Some(id)
    }

    /// Shows the element `id` at `tf` with all its solid meshes (as it starts to fall, before its SDF
    /// is back where it lands).
    pub fn show_solid_at(&mut self, id: u32, tf: PlacementTransform) {
        if let Some(scene) = self.runtime.relocate(id, tf, &mut self.scene) {
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene, skip_solid: false });
        }
    }

    /// Snaps every topple under way to where it ends.
    pub fn finish_all_falls(&mut self, body: Option<&mut Body>) {
        let pending = std::mem::take(&mut self.falling);
        for fall in pending {
            self.relocate_placed(fall.id, fall.end(), None);
            let _ = self.settle_buoyancy_for(fall.id, 0.0);
        }
        if let Some(body) = body {
            let max_lift = body.size as f64 * 3.0;
            let _ = body.surface_from_solid(&mut self.combined(), max_lift);
        }
    }

    /// Carries the topples under way on by `dt`.
    pub fn tick_falling(&mut self, dt: f32, body: &mut Body) {
        if self.falling.is_empty() {
            return;
        }
        let mut updates = Vec::new();
        for fall in &mut self.falling {
            fall.elapsed += dt;
            updates.push((fall.id, fall.sample(), fall.elapsed >= fall.duration));
        }
        self.falling.retain(|f| f.elapsed < f.duration);
        for (id, tf, finished) in updates {
            if finished {
                self.relocate_placed(id, tf, Some(&mut *body));
                let _ = self.settle_buoyancy_for(id, 0.0);
            } else {
                self.relocate_placed_fast(id, tf);
            }
        }
    }

    fn world_rev(&self) -> u64 {
        (self.edits.revision() << 32) ^ self.water.revision()
    }

    /// Archimedes buoyancy for the un-rooted scene element `id`: snapped to its waterline if it
    /// floats, to the bed if it sinks.
    pub fn settle_buoyancy_for(&mut self, id: u32, extra_load_kg: f32) -> Option<BuoyancyState> {
        let pos = self.index_of(id)?;
        let pl = self.placed[pos].clone();
        if pl.is_promoted() || self.is_sdf_element(&pl) {
            self.floating.remove(&id);
            return None;
        }
        let phys = self.runtime.physics_of(id)?;
        if phys.rooted {
            self.floating.remove(&id);
            return None;
        }
        let old_tf = pl.transform();
        let world_rev = self.world_rev();
        let buoy = compute_buoyancy(&phys, old_tf, &mut self.region, extra_load_kg).ok()?;
        match buoy {
            BuoyancyState::Floating { target_y, .. } | BuoyancyState::Sinking { target_y, .. } => {
                if buoy.is_floating() {
                    self.floating.insert(id, CachedBuoyancy { state: buoy, world_rev, rider_kg: extra_load_kg });
                } else {
                    self.floating.remove(&id);
                    if self.piloting == Some(id) {
                        self.piloting = None;
                    }
                }
                let mut tf = old_tf;
                tf.pos[1] = target_y;
                if (tf.pos[1] - old_tf.pos[1]).abs() > 1e-4 {
                    self.relocate_placed_fast(id, tf);
                }
            }
            BuoyancyState::Dry { .. } => {
                self.floating.remove(&id);
                if self.piloting == Some(id) {
                    self.piloting = None;
                }
            }
        }
        Some(buoy)
    }

    /// Buoyancy again for every placed element (after water changed under them, or a load).
    pub fn refresh_all_buoyancy(&mut self) {
        let ids: Vec<u32> = self.placed.iter().map(|e| e.id).collect();
        for id in ids {
            let _ = self.settle_buoyancy_for(id, 0.0);
        }
    }

    /// What is only shown, for `dt`: floating elements bobbing about their rest, a rider's weight
    /// pressing a boat down (or under), and `body` carried up and down with what it stands on. No
    /// saved transform moves.
    pub fn animate(&mut self, dt: f32, body: &mut Body) {
        let _span = info_span!("animate").entered();
        self.float_time += dt;
        let standing = self.scene.standing_on_placed(body.feet, body.size);
        let rider_kg = player_mass_kg(body.size);
        let world_rev = self.world_rev();
        let mut ids: Vec<u32> = self.floating.keys().copied().collect();
        if let Some(sid) = standing
            && !ids.contains(&sid)
        {
            ids.push(sid);
        }
        for id in ids {
            if self.held.is_some_and(|h| h.id == id) || self.falling.iter().any(|f| f.id == id) {
                continue;
            }
            let Some(pl) = self.find(id).cloned() else {
                self.floating.remove(&id);
                self.shown.remove(&id);
                continue;
            };
            if pl.is_promoted() {
                continue;
            }
            let Some(phys) = self.runtime.physics_of(id) else { continue };
            if phys.rooted {
                continue;
            }
            let rest = pl.transform();
            let load = if standing == Some(id) { rider_kg } else { 0.0 };
            let buoy = match self.floating.get(&id).copied() {
                Some(c) if c.world_rev == world_rev && (c.rider_kg - load).abs() <= 1e-2 => Some(c.state),
                _ => compute_buoyancy(&phys, rest, &mut self.region, load).ok(),
            };
            let Some(buoy) = buoy else { continue };
            let (desired, rate) = match buoy {
                BuoyancyState::Floating { target_y, bob_amplitude, .. } => {
                    self.floating.insert(id, CachedBuoyancy { state: buoy, world_rev, rider_kg: load });
                    (target_y + math::sin(self.float_time * 2.2 + id as f32 * 1.13) * bob_amplitude, 6.0)
                }
                BuoyancyState::Sinking { target_y, .. } => (target_y, 4.0),
                BuoyancyState::Dry { .. } => (rest.pos[1], 6.0),
            };
            let shown = self.shown.get(&id).copied().unwrap_or(rest.pos[1]);
            let next = shown + (desired - shown) * (dt * rate).min(1.0);
            if (next - shown).abs() <= 1e-5 {
                continue;
            }
            let mut tf = rest;
            tf.pos[1] = next;
            if self.runtime.relocate_fast(id, tf, &mut self.scene) {
                self.changes.layers.push(LayerChange::Moved { owner: id, from: rest, to: tf });
            }
            self.shown.insert(id, next);
            if standing == Some(id) {
                body.feet[1] += (next - shown) as f64;
            }
        }
    }

    /// The areas' and elements' `tick` for `dt`, with the player as `ps` looking at `(hit_owner,
    /// hit_id)`.
    pub fn tick(&mut self, dt: f32, ps: &PlayerState, hit_owner: u32, hit_id: u32) -> Vec<Applied> {
        let _span = info_span!("tick").entered();
        let mut effects = self.region.region.tick_all(dt, ps, hit_owner, hit_id, &mut self.scene);
        effects.extend(self.runtime.tick_all(dt, ps, hit_owner, hit_id, &mut self.scene));
        effects
    }

    /// `F` pressed on `(hit_owner, hit_id)`.
    pub fn interact(&mut self, ps: &PlayerState, hit_owner: u32, hit_id: u32) -> Vec<Applied> {
        let mut effects = Vec::new();
        if is_area_owner(hit_owner)
            && let Some(eff) = self.region.region.interact_tile(ps, hit_owner, hit_id, &mut self.scene)
        {
            effects.push(eff);
        }
        effects.extend(self.runtime.interact(ps, hit_owner, hit_id, &mut self.scene));
        effects
    }
}

fn nlerp_quat(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let dot = a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
    let s = if dot < 0.0 { -1.0 } else { 1.0 };
    quat_normalize([0, 1, 2, 3].map(|i| a[i] + (b[i] * s - a[i]) * t))
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
            let (mut t0, mut t1) = ((lo[a] - ro[a]) * inv, (hi[a] - ro[a]) * inv);
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
