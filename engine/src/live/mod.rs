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
//! a fall is decided by the action that set it off (`falls`), and the element is where it comes to
//! rest at once; an element settles on water at rest, unloaded. What only the player sees and
//! stands on (a fall under way, `tick_falling`; bobbing; a rider's weight pressing a boat down) is
//! shown between actions and moves no saved transform, so a log replays without it.
//!
//! **Support** (`falls`, phase G): after an action, what it carved or cut and no longer holds
//! together comes apart, and what nothing reaching the ground holds up falls, onto whatever is
//! beneath it; undone with the action.
//!
//! **The graph** (`graph`, phase F): what each element was cut from, rests on and is owned by,
//! written by the actions that make it so (`relations`), reported in their `Summary`, undone with
//! them, and saved.

pub mod action;
mod falls;
pub mod history;
pub mod log;
mod near;
mod objects;
mod refit;
pub mod relations;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tracing::info_span;
use wasmtime::bail;

use crate::atlas::{Atlas, CompiledAtlas, TileSpec};
use crate::compose;
use crate::body::Body;
use crate::compose::{Composed, ComposedLog, ElementCatalog, ElementRuntime, PlacedElement, PlacementTransform, SdfElementEntry, quat_normalize};
use crate::carved::Carving;
use crate::cut::{ObjectPhysics, check_carry_with_load, max_carry_mass_kg, world_edit_to_local};
use crate::edit::{Edit, EditLog, Op, SavedTile, WorldSave};
use crate::graph::{Graph, GraphDelta, Node, Relation};
use crate::modules::{ModuleCheck, SavedModule, relative_to_root};
use crate::physics::{BuoyancyState, Inventory, Tool, WorldMode, compute_buoyancy, player_mass_kg};
use crate::rules::RuleBook;
use crate::scene::{Applied, Combined, SceneWorld, is_area_owner};
use crate::water::{WaterLog, bounds_affect_chunk};
use crate::{Compiled, ElementBounds, Limits, Material, PlayerState, Result, Scene, Wasm, chunk, math};

pub use action::{Action, Actor, Outcome, SculptTarget, Summary};
pub use history::{History, WorldAction};
pub use log::{ActionLog, LOG_VERSION, Logged, replay, verify_save_edits};
pub use falls::{FALL_LIMIT, MAX_ROUNDS, SETTLE_BUDGET};

/// A scene element as a brush meets it alone (`World::element_brush`): where it stands, its exact
/// solid, the edits its save records on it, and all that carves it now.
pub(crate) type ElementBrush = (PlacementTransform, Arc<crate::mesh_sdf::MeshSdf>, Vec<Edit>, Vec<Edit>);
pub use relations::Footing;

/// Elements a carve promoted (id, edit index), their bounds, and their names.
type Promoted = (Vec<(u32, usize)>, Vec<(PlacementTransform, ElementBounds)>, Vec<String>);

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
    /// Make the layer of `owner` again from `scene` (world space): all of it is drawn (a promoted
    /// element's solid is in it carved, `carved`; the terrain's chunks do not draw it).
    Spawn { owner: u32, scene: Scene },
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

/// An element falling, as it is shown (it is already where it comes to rest): eased as if by
/// gravity, swung about `pivot` if it tipped over one.
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
    pub pivot: Option<[f32; 3]>,
}

impl FallingElement {
    pub fn sample(&self) -> PlacementTransform {
        let u = (self.elapsed / self.duration.max(1e-3)).clamp(0.0, 1.0);
        // Quadratic ease-in: tipping over, faster and faster.
        let t = u * u;
        let rot = nlerp_quat(self.from_rot, self.to_rot, t);
        let pos = match self.pivot {
            // Swung about the edge it tipped over, and moved the rest of the way as it goes.
            Some(pv) => {
                let turn = compose::quat_mul(self.to_rot, compose::quat_conj(self.from_rot));
                let arm = [0, 1, 2].map(|a| self.from_pos[a] - pv[a]);
                let swung = compose::quat_rotate(nlerp_quat([0.0, 0.0, 0.0, 1.0], turn, t), arm);
                let ends = compose::quat_rotate(turn, arm);
                [0, 1, 2].map(|a| pv[a] + swung[a] + (self.to_pos[a] - (pv[a] + ends[a])) * t)
            }
            None => [0, 1, 2].map(|a| self.from_pos[a] + (self.to_pos[a] - self.from_pos[a]) * t),
        };
        PlacementTransform::with_rot(pos, rot, self.scale)
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
    /// Play vs workshop mode (`WORLD_PLAN.md`, phase H).
    pub mode: WorldMode,
    /// Per-player carried bulk materials in kilograms (`world` mode economy).
    pub inventories: BTreeMap<String, Inventory>,
    /// What each element was cut from, rests on, and is owned by.
    pub graph: Graph,
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
    /// The graph's changes during the action being applied.
    journal: GraphDelta,
    /// Inventory deltas `(player_id, material, delta_kg)` during the action being applied.
    inv_journal: Vec<(String, Material, f32)>,
    /// The most falls one action may set off (`FALL_LIMIT`); what is still loose falls with the next.
    /// What settling left for later, out of budget (`falls::SETTLE_BUDGET`): settled with the
    /// next action, or an `Action::Settle`.
    unsettled: Option<falls::Pending>,
    /// Points settling has asked in this action.
    settle_spent: usize,
    pub fall_limit: usize,
    /// What the action being applied may have shaken (`falls`).
    pending: falls::Pending,
    /// Elements made into cells, by id, with the edits' revision and the element as they were.
    cells: BTreeMap<u32, (PlacedElement, Vec<Edit>, falls::Cells)>,
    /// SDF elements asked alone (`entry_distances`), by id, with the edits' revision and the element.
    alone: BTreeMap<u32, (u64, PlacedElement, Arc<ComposedLog>)>,
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
        let graph = Graph::from_relations(save.relations.iter().cloned())?;
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
            mode: save.mode,
            inventories: save.inventories.clone(),
            graph,
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
            journal: GraphDelta::default(),
            inv_journal: Vec::new(),
            unsettled: None,
            settle_spent: 0,
            fall_limit: FALL_LIMIT,
            pending: falls::Pending::default(),
            cells: BTreeMap::new(),
            alone: BTreeMap::new(),
        };
        // Pieces cut from elements no longer in the world keep their ids, so none is used again.
        world.next_id = world.next_id.max(world.graph.relations().filter_map(|r| r.to.element()).max().map_or(0, |m| m + 1));
        for pl in save.elements {
            world.spawn_layer_for(pl)?;
        }
        world.commit(Invalidation::All);
        world.refresh_all_buoyancy();
        if save.relations.is_empty() {
            world.derive_graph();
        }
        world.journal = GraphDelta::default();
        world.changes.said.clear();
        world.dirty = false;
        Ok((world, area_scenes))
    }

    /// The world as it would be saved now.
    pub fn to_save(&self) -> WorldSave {
        let mut save = WorldSave::with_elements(self.area_name.clone(), self.seed, self.edits.edits().to_vec(), self.placed.clone());
        save.mode = self.mode;
        save.tiles = self.tiles.clone();
        save.modules = self.modules.clone();
        save.rules = self.rules.clone();
        save.water = self.water.bodies().to_vec();
        save.edit_authors = self.edit_authors.clone();
        save.relations = self.graph.relations().collect();
        save.inventories = self.inventories.iter().filter(|(_, inv)| !inv.is_empty()).map(|(k, v)| (k.clone(), *v)).collect();
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
        self.mode = saved.mode;
        self.inventories = saved.inventories;
        self.graph = Graph::from_relations(saved.relations.iter().cloned())?;
        self.runtime.clear(&mut self.scene);
        self.changes.layers.push(LayerChange::DespawnAllPlaced);
        self.placed.clear();
        self.history = History::default();
        self.unsettled = None;
        let edit_count = self.edits.len();
        for pl in saved.elements {
            let id = pl.id;
            self.spawn_layer_for(pl)?;
            self.next_id = self.next_id.max(id + 1);
        }
        self.next_id = self.next_id.max(self.graph.relations().filter_map(|r| r.to.element()).max().map_or(0, |m| m + 1));
        self.commit(Invalidation::All);
        self.refresh_all_buoyancy();
        if saved.relations.is_empty() {
            self.derive_graph();
        }
        self.journal = GraphDelta::default();
        self.dirty = false;
        Ok((edit_count, self.placed.len()))
    }

    /// Opens an action log at `path` (written as actions are applied), or closes it with `None`.
    pub fn set_log(&mut self, log: Option<ActionLog>) {
        self.log = log;
    }

    /// What changed since this was last asked, for the window to show.
    pub fn take_changes(&mut self) -> Changes {
        for (owner, scene) in self.runtime.finished_carvings() {
            self.changes.layers.push(LayerChange::Spawn { owner, scene });
        }
        std::mem::take(&mut self.changes)
    }

    /// Whether settling left something for later (out of its budget): the window sends
    /// `Action::Settle` while it does.
    pub fn unsettled(&self) -> bool {
        self.unsettled.is_some()
    }

    /// Draws carved elements on a thread of their own (`ElementRuntime::carve_in_background`): an
    /// action returns without them, and each is among the changes (`take_changes`) once made.
    pub fn carve_in_background(&mut self, on: bool) {
        self.runtime.carve_in_background(on);
    }

    /// Waits until every carving asked of the background is made (they are then among the changes).
    pub fn wait_for_carvings(&mut self) {
        for (owner, scene) in self.runtime.wait_for_carvings() {
            self.changes.layers.push(LayerChange::Spawn { owner, scene });
        }
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

    /// Carried bulk materials for `player` (empty if none).
    pub fn inventory(&self, player: &str) -> Inventory {
        self.inventories.get(player).copied().unwrap_or_default()
    }

    /// Total carried bulk material mass (kg) for `player`.
    pub fn inventory_mass_kg(&self, player: &str) -> f32 {
        self.inventories.get(player).map_or(0.0, Inventory::total_mass_kg)
    }

    /// Whether material is counted: in `world` mode, where the rule book's `economy` is on.
    pub fn counts_material(&self) -> bool {
        self.mode == WorldMode::World && self.rules.economy
    }

    /// Extra carried pack mass counted toward carry limits & speed (`0.0` unless material is counted).
    pub fn carried_pack_kg(&self, player: &str) -> f32 {
        if self.counts_material() { self.inventory_mass_kg(player) } else { 0.0 }
    }

    /// Movement speed multiplier in `[0.35..=1.0]` accounting for both any held object and carried
    /// bulk materials in `world` mode (`WORLD_PLAN.md`, phase H).
    pub fn carry_speed_factor(&self, player: &str, player_size: f32) -> f32 {
        let obj_kg = self.held.and_then(|h| self.physics_of(h.id)).map_or(0.0, |p| p.world_mass_kg);
        let total = obj_kg + self.carried_pack_kg(player);
        if total <= 0.0 {
            return 1.0;
        }
        let max_m = max_carry_mass_kg(player_size).max(1e-9);
        if total <= max_m {
            1.0 - 0.36 * (total / max_m).clamp(0.0, 1.0)
        } else {
            let over = ((total - max_m) / max_m).clamp(0.0, 1.0);
            (0.64 - 0.29 * over).max(0.35)
        }
    }

    /// Whether `player` at `player_size` can still hold whatever object they are currently carrying
    /// (counting their carried material pack in `world` mode).
    pub fn can_still_hold(&self, player: &str, player_size: f32) -> bool {
        let Some(held) = self.held else { return true };
        let Some(phys) = self.carry_physics(held.id) else { return false };
        let is_sdf = self.find(held.id).is_some_and(|pl| self.is_sdf_element(pl));
        check_carry_with_load(&phys, is_sdf, player_size, self.carried_pack_kg(player)).is_allowed()
    }

    /// The physics `id` is carried by: its own, but not rooted if its roots were found to hold in
    /// nothing (`uprooted`), as grabbing it judged.
    pub(super) fn carry_physics(&self, id: u32) -> Option<ObjectPhysics> {
        let mut phys = self.physics_of(id)?;
        if self.graph.contains(&Relation::uprooted(id)) {
            phys.rooted = false;
        }
        Some(phys)
    }

    /// Default tool for the world's current mode (`Hands` in `world`, `All` in `workshop`).
    pub fn default_tool(&self) -> Tool {
        self.mode.default_tool()
    }

    /// Next tool to cycle to for the world's current mode (so `play` never inspects `Tool` variants).
    pub fn next_tool(&self, cur: Tool) -> Tool {
        let tools = self.mode.tools();
        let i = tools.iter().position(|&t| t == cur).map_or(0, |p| (p + 1) % tools.len());
        tools[i]
    }

    pub(super) fn record_inventory(&mut self, player: &str, mat: Material, delta_kg: f32) {
        if !delta_kg.is_finite() || delta_kg == 0.0 {
            return;
        }
        let inv = self.inventories.entry(player.to_string()).or_default();
        let before = inv.get(mat);
        inv.apply_delta(mat, delta_kg);
        // What was actually applied (a debit is clamped at empty), so undo gives back no more.
        let applied = inv.get(mat) - before;
        if inv.is_empty() {
            self.inventories.remove(player);
        }
        if applied != 0.0 {
            self.inv_journal.push((player.to_string(), mat, applied));
        }
    }

    pub(super) fn take_inv_journal(&mut self) -> Vec<(String, Material, f32)> {
        std::mem::take(&mut self.inv_journal)
    }

    pub(super) fn revert_inventory(&mut self, deltas: &[(String, Material, f32)], redo: bool) {
        let iter: Box<dyn Iterator<Item = &(String, Material, f32)>> =
            if redo { Box::new(deltas.iter()) } else { Box::new(deltas.iter().rev()) };
        for (player, mat, delta) in iter {
            let signed = if redo { *delta } else { -*delta };
            let inv = self.inventories.entry(player.clone()).or_default();
            inv.apply_delta(*mat, signed);
            if inv.is_empty() {
                self.inventories.remove(player);
            }
        }
    }

    /// Exact `MeshSdf`s of unpromoted solid scene elements that `edit` (or `target_id`) overlaps,
    /// so whole-brush tool checks and material yields see exact signed distance and per-triangle
    /// materials inside unpromoted objects without promoting them.
    pub(super) fn unpromoted_sdfs_near(&mut self, edit: &Edit, target_id: Option<u32>) -> Vec<(PlacementTransform, Arc<crate::mesh_sdf::MeshSdf>)> {
        let (elo, ehi) = edit.influence_bounds();
        let mut ids = self.scene.solid_placed_overlapping(elo, ehi);
        if let Some(id) = target_id
            && !ids.contains(&id)
        {
            ids.push(id);
        }
        let mut out = Vec::new();
        for id in ids {
            let Some(pl) = self.find(id).cloned() else { continue };
            if pl.is_promoted() || self.is_sdf_element(&pl) {
                continue;
            }
            if let Ok(Some(msdf)) = self.catalog.mesh_sdf_of(&pl) {
                out.push((pl.transform(), msdf));
            }
        }
        out
    }

    /// What a stroke on the scene element `id` alone is checked against: where it stands, its exact
    /// solid, the edits recorded on it (`local_edits`: what a save can check again) and all that
    /// carves it now (`carving_of`: those and the terrain's carves into it since it was promoted).
    pub(super) fn element_brush(&mut self, id: u32) -> Option<ElementBrush> {
        let pl = self.find(id)?.clone();
        if self.is_sdf_element(&pl) {
            return None;
        }
        let sdf = self.catalog.mesh_sdf_of(&pl).ok().flatten()?;
        let carving = self.carving_of(&pl).map_or_else(|| pl.local_edits.clone(), |c| c.edits);
        Some((pl.transform(), sdf, pl.local_edits.clone(), carving))
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
            && let Some(bounds) = el.sdf_bounds_for(&pl.params)
        {
            return Some(bounds);
        }
        if pl.is_promoted()
            && let Some(mesh_sdf) = self.catalog.cached_mesh_sdf(&pl.module, pl.seed, &pl.params, &pl.clips)
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

    /// The SDF as it is (the edits, water and every placed SDF element not carried or falling), but
    /// for the elements `left_out`: a new snapshot, under a revision of its own.
    pub(super) fn composed_without(&mut self, left_out: &[u32]) -> Arc<ComposedLog> {
        let mut exclude: Vec<u32> = self.falling.iter().map(|f| f.id).collect();
        exclude.extend(self.held.map(|h| h.id));
        exclude.extend_from_slice(left_out);
        self.composed_revision += 1;
        let mut sdf_elements = Vec::new();
        for pl in &self.placed {
            if exclude.contains(&pl.id) {
                continue;
            }
            if let Ok(compiled) = self.catalog.get_or_load(&pl.module) {
                if let Some(bounds) = compiled.sdf_bounds_for(&pl.params) {
                    sdf_elements.push(SdfElementEntry::new(pl.clone(), compiled.compiled.clone(), bounds));
                } else if pl.is_promoted()
                    && let Ok(Some(mesh_sdf)) = self.catalog.mesh_sdf_of(pl)
                {
                    sdf_elements.push(SdfElementEntry::from_promoted_mesh(pl.clone(), mesh_sdf));
                }
            }
        }
        Arc::new(ComposedLog::new_with_water(self.edits.clone(), sdf_elements, self.water.clone(), self.composed_revision))
    }

    /// A new SDF snapshot from the edits, water and every placed SDF element not carried or falling,
    /// given to the region here and, with the chunks `inv` makes stale, to the window.
    pub fn commit(&mut self, inv: Invalidation) {
        let _span = info_span!("commit").entered();
        let composed = self.composed_without(&[]);
        self.region.log = composed.clone();
        self.region.prune_instances();
        self.changes.composed.push((composed, inv));
        let carved: Vec<u32> = self.placed.iter().filter(|pl| pl.is_promoted() || self.runtime.is_carved(pl.id)).map(|pl| pl.id).collect();
        for id in carved {
            self.sync_carving(id);
        }
    }

    /// What the promoted element `pl` is carved by, in its frame: its own edits, then the terrain's
    /// carves since it was promoted that reach where it is (as `Composed` applies them to it).
    pub fn carving_of(&mut self, pl: &PlacedElement) -> Option<Carving> {
        if !pl.is_promoted() || self.is_sdf_element(pl) {
            return None;
        }
        // Drawn from its module's uncut solid and its cut planes, so a piece shares its original's
        // bricks (`carved::Carving::clips`).
        let sdf = self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.params, &[]).ok().flatten()?;
        let mut edits = pl.local_edits.clone();
        if let Some(bounds) = self.sdf_bounds_of(pl) {
            let tf = pl.transform();
            let (blo, bhi) = tf.world_bounds(bounds);
            for e in &self.edits.edits()[pl.sdf_after_edits().min(self.edits.len())..] {
                let (lo, hi) = e.influence_bounds();
                if e.op == Op::Carve && (0..3).all(|a| hi[a] >= blo[a] && lo[a] <= bhi[a]) {
                    edits.push(world_edit_to_local(e, tf));
                }
            }
        }
        Some(Carving { source: Carving::source_of(&pl.module, pl.seed, &pl.params), sdf, clips: pl.clips.clone(), edits })
    }

    /// Draws the element `id` as it is now carved (its layer made again if that changed).
    fn sync_carving(&mut self, id: u32) {
        let Some(pl) = self.find(id).cloned() else { return };
        let carving = self.carving_of(&pl);
        if let Some(scene) = self.runtime.set_carving(id, carving) {
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene });
        }
    }

    /// Promoted elements keep a solid proxy (in collisions) for viewers far larger than they are.
    pub fn sync_viewer_size(&mut self, viewer_size: f32) {
        self.viewer_size = viewer_size;
        self.runtime.sync_macro_proxies(viewer_size, &mut self.scene);
    }

    /// Spawns `pl`'s runtime and scene layer, adds it to the placed elements in order of id (no SDF
    /// commit).
    fn spawn_layer_for(&mut self, pl: PlacedElement) -> Result<Arc<crate::compose::CompiledElement>> {
        let compiled = self.catalog.get_or_load(&pl.module)?;
        if pl.is_promoted() {
            self.catalog.mesh_sdf_of(&pl)?;
        }
        let carving = self.carving_of(&pl);
        let scene = self.runtime.spawn_carved(pl.clone(), &compiled, &mut self.scene, self.viewer_size, carving)?;
        self.changes.layers.push(LayerChange::Spawn { owner: pl.id, scene });
        // In order of id, so a world undone to where it was saves as it did.
        let at = self.placed.partition_point(|e| e.id < pl.id);
        self.placed.insert(at, pl);
        Ok(compiled)
    }

    /// Places `pl` (its id already allocated): spawned, in the SDF if it is an SDF element or
    /// promoted, settled on water if it floats; `body`, if given, lifted out of it. Its name.
    pub fn spawn_placed(&mut self, pl: PlacedElement, body: Option<&mut Body>) -> Result<String> {
        let _span = info_span!("spawn_placed", id = pl.id).entered();
        let compiled = self.spawn_layer_for(pl.clone())?;
        self.dirty = true;
        let sdf_bounds = compiled.sdf_bounds_for(&pl.params).or_else(|| if pl.is_promoted() { self.sdf_bounds_of(&pl) } else { None });
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
        if let Some(scene) = self.runtime.relocate(id, tf, &mut self.scene) {
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene });
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

    /// Promotes or demotes the element `id` (`Some(edit index)` / `None`), its layer made again
    /// (carved as it now is).
    pub fn set_promoted(&mut self, id: u32, state: Option<usize>) {
        if let Some(pos) = self.index_of(id) {
            self.placed[pos].promoted = state;
        }
        let was_carved = self.runtime.is_carved(id);
        let Some(scene) = self.runtime.set_promoted(id, state, &mut self.scene) else { return };
        let Some(pl) = self.find(id).cloned() else { return };
        let carving = self.carving_of(&pl);
        match self.runtime.set_carving(id, carving) {
            Some(scene) => self.changes.layers.push(LayerChange::Spawn { owner: id, scene }),
            None if was_carved => self.changes.layers.push(LayerChange::Spawn { owner: id, scene }),
            None => {}
        }
    }

    /// Sets promotion on several elements (for undo and redo): their bounds, for invalidation.
    pub fn set_elements_promoted(&mut self, ids: &[(u32, Option<usize>)]) -> Vec<(PlacementTransform, ElementBounds)> {
        let mut bounds_list = Vec::new();
        for &(id, next_state) in ids {
            let Some(pos) = self.index_of(id) else { continue };
            let pl = self.placed[pos].clone();
            if let Ok(Some(mesh_sdf)) = self.catalog.mesh_sdf_of(&pl) {
                bounds_list.push((pl.transform(), mesh_sdf.bounds()));
            }
            self.set_promoted(id, next_state);
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
    ) -> Promoted {
        let _span = info_span!("promote").entered();
        let (elo, ehi) = edit.influence_bounds();
        let mut candidates = self.scene.solid_placed_overlapping(elo, ehi);
        if let Some(owner) = aimed
            && !candidates.contains(&owner)
        {
            candidates.push(owner);
        }
        let (mut promoted, mut bounds_list, mut names) = (Vec::new(), Vec::new(), Vec::new());
        // Conservative bounding-sphere radius of `edit` around `edit.center`: an element with no
        // solid triangle within this reach only overlapped by AABB (e.g. a fallen tree's boughs
        // behind a stump) and is not touched by the carve.
        let reach = match edit.shape {
            crate::edit::Shape::Sphere { radius } => radius,
            crate::edit::Shape::Box { half, .. } => (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt(),
            crate::edit::Shape::Cylinder { radius, half_height, .. } => (radius * radius + half_height * half_height).sqrt(),
        } + edit.blend
            + 1e-3;
        for id in candidates {
            // A floating boat is not turned to voxels by sculpting beside it.
            if self.floating.contains_key(&id) && aimed != Some(id) {
                continue;
            }
            if aimed != Some(id) && self.scene.distance_to_solids_of(id, edit.center, reach).is_none() {
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
            let Ok(Some(mesh_sdf)) = self.catalog.mesh_sdf_of(&pl) else { continue };
            self.set_promoted(id, Some(edit_idx));
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
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene });
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

    /// Shows the element `id` at `tf` (as it starts to fall: a carved one carved, as it lands).
    pub fn show_solid_at(&mut self, id: u32, tf: PlacementTransform) {
        if let Some(scene) = self.runtime.relocate(id, tf, &mut self.scene) {
            self.changes.layers.push(LayerChange::Spawn { owner: id, scene });
        }
    }

    /// Shows every fall under way where it ends (where the element already is).
    pub fn finish_all_falls(&mut self, body: Option<&mut Body>) {
        let pending = std::mem::take(&mut self.falling);
        for fall in pending {
            self.finish_fall(fall.id);
        }
        if let Some(body) = body {
            let max_lift = body.size as f64 * 3.0;
            let _ = body.surface_from_solid(&mut self.combined(), max_lift);
        }
    }

    /// The element `id`, no longer shown falling, shown (and in the SDF) where it is.
    fn finish_fall(&mut self, id: u32) {
        if let Some(tf) = self.find(id).map(|pl| pl.transform()) {
            let pl = self.find(id).cloned();
            self.relocate_placed(id, tf, None);
            // `relocate_placed` writes the transform back; keep the record exactly as it was.
            if let (Some(pl), Some(i)) = (pl, self.index_of(id)) {
                self.placed[i] = pl;
            }
        }
    }

    /// Carries the falls under way on by `dt` (shown only: nothing kept moves).
    pub fn tick_falling(&mut self, dt: f32, body: &mut Body) {
        if self.falling.is_empty() {
            return;
        }
        let mut updates = Vec::new();
        for fall in &mut self.falling {
            fall.elapsed += dt;
            let from = PlacementTransform::with_rot(fall.from_pos, fall.from_rot, fall.scale);
            updates.push((fall.id, from, fall.sample(), fall.elapsed >= fall.duration));
        }
        self.falling.retain(|f| f.elapsed < f.duration);
        for (id, from, tf, finished) in updates {
            if finished {
                self.finish_fall(id);
                let max_lift = body.size as f64 * 3.0;
                let _ = body.surface_from_solid(&mut self.combined(), max_lift);
            } else if self.runtime.relocate_fast(id, tf, &mut self.scene) {
                // Its layer was made where the fall began.
                self.changes.layers.push(LayerChange::Moved { owner: id, from, to: tf });
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
        let mut buoy = compute_buoyancy(&phys, old_tf, &mut self.region, extra_load_kg).ok()?;
        // Water knows only the terrain: what rests on another element (a bridge over a channel, a
        // pier) is held up by it, and does not sink or float down through it. (Lifted off it by
        // water rising, it floats.)
        if let BuoyancyState::Floating { target_y, .. } | BuoyancyState::Sinking { target_y, .. } = buoy
            && target_y < old_tf.pos[1]
            && self.contacts_at(id, old_tf).iter().any(|n| matches!(n, Node::Element(_)))
        {
            buoy = BuoyancyState::Dry { floor_y: old_tf.pos[1] };
        }
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
            // Resting on another element (a log on a bridge over water), a rider's weight does not
            // press it down through what holds it.
            if !self.floating.contains_key(&id) && self.graph.supports_of(id).iter().any(|n| matches!(n, Node::Element(_))) {
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
