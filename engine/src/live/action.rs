//! What a player asks of the world, and what came of it (`WORLD_PLAN.md`, phase E).
//!
//! An `Action` says everything the change needs, already resolved by whoever asks: the edit a brush
//! makes (not the mouse), the element aimed at (not the ray), the tool in hand. So applying it again
//! to the same world gives the same world, which is what lets the action log replay.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::info_span;

use super::{ElementBrush, Invalidation, World, WorldAction};
use crate::body::Body;
use crate::compose::{PlacedElement, PlacementTransform};
use crate::mesh_sdf::MeshSdf;
use crate::cut::{local_edit_to_world, world_edit_to_local};
use crate::edit::{Edit, Op};
use crate::graph::{GraphDelta, Opening, Relation};
use crate::laws::length;
use crate::physics::{BrushCheck, BrushTarget, Tool, WorldMode, carve_yield_kg, check_carve_brush, estimate_add_mass_kg};
use crate::rules::Action as Rule;
use crate::water::WaterLog;
use crate::Material;

/// What a stroke is checked against when its tool is checked: the element as its save records it
/// (what `verify_save_edits` checks again), or the world.
fn recorded_target<'a>(region: &'a mut dyn crate::Region, brush: &'a Option<ElementBrush>, unpromoted: &'a [(PlacementTransform, Arc<MeshSdf>)]) -> BrushTarget<'a> {
    match brush {
        Some((tf, sdf, recorded, _)) => BrushTarget::Element { sdf, edits: recorded, scale: tf.scale },
        None => BrushTarget::World { region, unpromoted },
    }
}

/// Who acts: their id (for the rule book and authorship) and their body (size, where they stand).
pub struct Actor<'a> {
    pub id: &'a str,
    pub body: &'a mut Body,
}

/// Where a sculpting stroke goes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub enum SculptTarget {
    /// The terrain; `aimed` is the solid scene element the stroke was aimed at, if any, promoted
    /// along with any the stroke reaches.
    Terrain { aimed: Option<u32> },
    /// A placed scene element itself, in its own frame, so the carve travels with it.
    Element(u32),
}

/// A change asked of the world.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Action {
    /// One brush stamp, `edit` in world space, made at `hit` (where the tool meets the surface)
    /// with `tool`.
    Sculpt { edit: Edit, hit: [f32; 3], tool: Tool, target: SculptTarget },
    /// An element of `module` placed at `pos`, turned by `yaw` (or `rot`), `scale` times its size;
    /// `fitted` if it was fitted to an opening, and `opening`, where that opening was measured (it
    /// then fills it, attached to what it is in); `seed`, the one it was made and judged with, if
    /// it must be that one (else the world gives it one).
    Place {
        module: String,
        pos: [f32; 3],
        yaw: f32,
        rot: Option<[f32; 4]>,
        scale: f32,
        fitted: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        opening: Option<Opening>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        seed: Option<u64>,
        /// What it is given to fit (`host.params`): for an opening, `SiteContext::params`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        params: Vec<f32>,
    },
    /// The placed element `id` taken away.
    Remove { id: u32 },
    /// The placed element `id` chopped where `hit` is, with `tool`, which found `material` there;
    /// what stood upright topples away along `dir`.
    Cut { id: u32, hit: [f32; 3], dir: [f32; 3], tool: Tool, material: Option<Material> },
    /// The placed element `id` picked up.
    Grab { id: u32 },
    /// What is carried set down at `pos`, turned by `rot`.
    Drop { pos: [f32; 3], rot: [f32; 4] },
    /// What is carried put back where it was picked up.
    Return,
    /// The helm of the floating element `id` taken, or let go (`None`).
    Pilot { id: Option<u32> },
    /// What is piloted paddled for `dt` seconds, `forward` and `turn` from -1 to 1.
    Steer { forward: f32, turn: f32, dt: f32 },
    /// `F` on what is at `at`: allowed or not (what it does is the element's own, not the world's).
    Interact { at: [f32; 3] },
    /// Switches between `world` mode (tools & material economy) and `workshop` mode (sandbox).
    SetMode { mode: WorldMode },
    Undo,
    Redo,
}

impl Action {
    pub fn name(&self) -> &'static str {
        match self {
            Action::Sculpt { .. } => "sculpt",
            Action::Place { .. } => "place",
            Action::Remove { .. } => "remove",
            Action::Cut { .. } => "cut",
            Action::Grab { .. } => "grab",
            Action::Drop { .. } => "drop",
            Action::Return => "return",
            Action::Pilot { .. } => "pilot",
            Action::Steer { .. } => "steer",
            Action::Interact { .. } => "interact",
            Action::SetMode { .. } => "set_mode",
            Action::Undo => "undo",
            Action::Redo => "redo",
        }
    }
}

/// What a change did, in brief: for the log and the inspector.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Summary {
    /// Edits and placed elements the world holds afterwards.
    pub edits: usize,
    pub elements: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub removed: Vec<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub promoted: Vec<u32>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub water: bool,
    /// Elements that fell because of it (phase G), in the order they fell.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fell: Vec<u32>,
    /// What it did to the world graph.
    #[serde(default, skip_serializing_if = "GraphDelta::is_empty")]
    pub graph: GraphDelta,
    /// Net material mass deltas `(material, kg)` for the actor (phase H, `world` mode).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inventory: Vec<(Material, f32)>,
    /// What the player is told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
}

/// Why a change could not be undone or redone.
enum Refused {
    /// There was nothing to undo or redo.
    Unchanged(String),
    Denied(String),
}

/// What came of an action.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Outcome {
    Done(Summary),
    /// Refused, and why: by the rule book, the tool, or the world (nothing there).
    Denied(String),
    /// Nothing to do (the same stamp again, nothing to undo).
    Unchanged(String),
}

impl Outcome {
    /// What to tell the player, if anything.
    pub fn said(&self) -> Option<&str> {
        match self {
            Outcome::Done(s) => s.said.as_deref(),
            Outcome::Denied(r) | Outcome::Unchanged(r) => Some(r),
        }
    }

    pub fn is_done(&self) -> bool {
        matches!(self, Outcome::Done(_))
    }
}

impl World {
    /// Makes the change `action` asks for, as `actor`: checked by the rule book and the tool, made,
    /// and followed by what it entails. Written to the action log if one is open.
    pub fn apply(&mut self, mut actor: Actor<'_>, action: Action) -> Outcome {
        let _span = info_span!("apply", action = action.name()).entered();
        // A topple under way has ended by the time anything else is done, seen or not: so a world
        // replayed without frames between its actions is the world that was played.
        self.finish_all_falls(None);
        let before = self.composed().revision();
        let feet = actor.body.feet;
        let size = actor.body.size;
        let yaw = actor.body.yaw;
        let id = actor.id.to_string();
        self.journal = GraphDelta::default();
        self.inv_journal.clear();
        self.pending = Default::default();
        let pushed = self.history.pushed;
        let mut outcome = match &action {
            Action::Sculpt { edit, hit, tool, target } => self.sculpt(&mut actor, *edit, *hit, *tool, *target),
            Action::Place { module, pos, yaw, rot, scale, fitted, opening, seed, params } => {
                self.place(&mut actor, module, *pos, *yaw, *rot, *scale, *fitted, *opening, *seed, params)
            }
            Action::Remove { id } => self.remove(&actor, *id),
            Action::Cut { id, hit, dir, tool, material } => self.cut(&mut actor, *id, *hit, *dir, *tool, *material),
            Action::Grab { id } => self.grab(&mut actor, *id),
            Action::Drop { pos, rot } => self.drop_held(&mut actor, *pos, *rot),
            Action::Return => self.return_held(),
            Action::Pilot { id } => self.pilot(*id),
            Action::Steer { forward, turn, dt } => self.steer(&mut actor, *forward, *turn, *dt),
            Action::Interact { at } => self.may_interact(&actor, *at),
            Action::SetMode { mode } => self.set_mode(&actor, *mode),
            Action::Undo => self.undo(actor.body),
            Action::Redo => self.redo(&mut actor),
        };
        // What fills an opening it reached is fitted to it again (undone with it).
        if outcome.is_done()
            && matches!(action, Action::Sculpt { target: SculptTarget::Terrain { .. }, .. })
            && let Some(edit) = self.edits.edits().last().copied()
        {
            let told = self.refit_openings(&edit);
            if let Outcome::Done(summary) = &mut outcome
                && !told.is_empty()
            {
                let told = told.join("; ");
                summary.said = Some(match summary.said.take() {
                    Some(s) if !s.is_empty() => format!("{s}; {told}"),
                    _ => told,
                });
            }
        }
        // What it shook settles: what came apart, and what fell (undone with it).
        if outcome.is_done() && !matches!(action, Action::Undo | Action::Redo | Action::Steer { .. } | Action::SetMode { .. }) {
            let settled = self.settle_support();
            if let Outcome::Done(summary) = &mut outcome
                && !(settled.added.is_empty() && settled.fell.is_empty())
            {
                summary.added.extend(settled.added);
                summary.removed.extend(settled.removed);
                summary.edits = self.edits.len();
                summary.elements = self.placed.len();
                let mut names: Vec<String> = Vec::new();
                for &id in &settled.fell {
                    let name = self.element_name(id);
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
                if !names.is_empty() {
                    let fell = format!("{} fell", names.join(", "));
                    summary.said = Some(match summary.said.take() {
                        Some(s) if !s.is_empty() => format!("{s}; {fell}"),
                        _ => fell,
                    });
                }
                summary.fell = settled.fell;
            }
        }
        self.pending = Default::default();
        let joined = (self.history.pushed - pushed) as usize;
        self.history.join_last(joined);
        // What it did to the graph and player inventories goes with its undo, and is said in its outcome.
        let delta = self.take_journal();
        let inv_delta = self.take_inv_journal();
        if self.history.pushed != pushed
            && let Some(done) = self.history.undo.last_mut()
        {
            done.graph = delta.clone();
            done.inventory = inv_delta.clone();
        }
        if let Outcome::Done(summary) = &mut outcome {
            summary.graph = delta;
            summary.inventory = inv_delta.iter().filter(|(p, _, _)| p == &id).map(|(_, m, kg)| (*m, *kg)).collect();
        }
        if let Some(log) = &mut self.log {
            let after = self.region.log.revision();
            if let Err(e) = log.write(&id, feet, size, yaw, &action, &outcome, before, after) {
                self.changes.said.push(format!("action log failed: {e}"));
                self.log = None;
            }
        }
        outcome
    }

    fn summary(&self, said: impl Into<String>) -> Summary {
        Summary { edits: self.edits.len(), elements: self.placed.len(), said: Some(said.into()), ..Summary::default() }
    }

    fn set_mode(&mut self, actor: &Actor<'_>, mode: WorldMode) -> Outcome {
        if self.mode == mode {
            return Outcome::Unchanged(format!("already in {} mode", mode.name()));
        }
        // The mode holds for everyone in the world: only who owns all of it may change it.
        if let Err(denial) = self.rules.check_world(actor.id) {
            return Outcome::Denied(denial.reason);
        }
        self.mode = mode;
        self.dirty = true;
        Outcome::Done(self.summary(format!("mode: {} (tool: {})", mode.name(), mode.default_tool().short_name())))
    }

    /// Whether `stroke` (in `target`'s frame: the world, or the element's own) is the stamp last made there.
    fn is_duplicate_stamp(&self, target: SculptTarget, stroke: &Edit) -> bool {
        match target {
            SculptTarget::Terrain { .. } => self.edits.edits().last() == Some(stroke),
            SculptTarget::Element(id) => self.find(id).is_some_and(|pl| pl.local_edits.last() == Some(stroke)),
        }
    }

    /// Why the element `id` cannot be sculpted as itself.
    fn unsculptable(&mut self, id: u32) -> Outcome {
        let Some(pl) = self.find(id).cloned() else { return Outcome::Denied(format!("no element #{id} to sculpt")) };
        if self.is_sdf_element(&pl) {
            return Outcome::Denied("an SDF landform is sculpted as terrain".into());
        }
        match self.catalog.mesh_sdf_of(&pl) {
            Err(e) => Outcome::Denied(format!("sculpt error: {e}")),
            _ => Outcome::Denied(format!("{} has no solid to sculpt", self.element_name(id))),
        }
    }

    fn sculpt(&mut self, actor: &mut Actor<'_>, mut edit: Edit, hit: [f32; 3], tool: Tool, target: SculptTarget) -> Outcome {
        if let Err(e) = edit.validate() {
            return Outcome::Denied(format!("sculpt error: {e}"));
        }
        // Mode rules (`WORLD_PLAN.md`, phase H).
        if self.mode == WorldMode::World {
            if tool == Tool::All {
                return Outcome::Denied("Workshop tool (all) is only available in workshop mode — switch to Saw, Pickaxe, or Shovel".into());
            }
            if edit.op == Op::Carve && edit.material.is_some() {
                return Outcome::Denied("carving cannot repaint walls in world mode".into());
            }
        }
        // Record the tool that made this edit, and the mode it was made in, so saves and replays
        // can verify it again under the rules it was made by.
        edit.tool = tool;
        edit.mode = self.mode;

        // What the stroke is checked against: only what it changes. A stroke on an element carves
        // that element alone (in its own frame), so the ground under it neither yields nor refuses.
        let brush = match target {
            SculptTarget::Element(id) => match self.element_brush(id) {
                Some(b) => Some(b),
                None => return self.unsculptable(id),
            },
            SculptTarget::Terrain { .. } => None,
        };
        let (mut stroke, stroke_hit) = match &brush {
            Some((tf, ..)) => (world_edit_to_local(&edit, *tf), tf.world_to_local_point(hit)),
            None => (edit, hit),
        };
        // The same stamp as the last changes nothing but the log.
        if self.is_duplicate_stamp(target, &stroke) {
            return Outcome::Unchanged(String::new());
        }
        let unpromoted = match (target, &brush) {
            (SculptTarget::Terrain { aimed }, None) => self.unpromoted_sdfs_near(&edit, aimed),
            _ => Vec::new(),
        };

        // Whole-brush tool check and material economy (`WORLD_PLAN.md`, phase H).
        let mut carve_yield = [0.0f32; 9];
        let mut add_cost: Option<(Material, f32)> = None;
        let mut shrink_note: Option<String> = None;
        match edit.op {
            Op::Carve => {
                let _span = info_span!("tool").entered();
                let check = check_carve_brush(tool, stroke, stroke_hit, &mut recorded_target(&mut self.region, &brush, &unpromoted));
                match check {
                    Ok(BrushCheck::Ok { yield_kg }) => carve_yield = yield_kg,
                    Ok(BrushCheck::Shrunk { edit: shrunk, blocked, yield_kg }) => {
                        stroke = shrunk;
                        carve_yield = yield_kg;
                        let radius = brush.as_ref().map_or(1.0, |(tf, ..)| tf.scale) * stroke.shape.radius();
                        shrink_note = Some(format!("{} stopped at {} — shrunk brush to {}", tool.short_name(), blocked.name(), length(radius)));
                        if self.is_duplicate_stamp(target, &stroke) {
                            return Outcome::Unchanged(String::new());
                        }
                    }
                    Ok(BrushCheck::Refused { reason, .. }) => return Outcome::Denied(reason),
                    Err(e) => return Outcome::Denied(format!("tool check failed: {e}")),
                }
                // What an element yields is what it is carved to now, not only what was recorded.
                if self.mode == WorldMode::World
                    && let Some((tf, sdf, _, carving)) = &brush
                {
                    match carve_yield_kg(&stroke, &mut BrushTarget::Element { sdf, edits: carving, scale: tf.scale }) {
                        Ok(y) => carve_yield = y,
                        Err(e) => return Outcome::Denied(format!("tool check failed: {e}")),
                    }
                }
            }
            Op::Add if self.mode == WorldMode::World => {
                let _span = info_span!("tool").entered();
                let mat = edit.material.unwrap_or(Material::Soil);
                if let Err(reason) = tool.can_add(mat) {
                    return Outcome::Denied(reason);
                }
                let need = match &brush {
                    Some((tf, sdf, _, carving)) => estimate_add_mass_kg(&stroke, mat, &mut BrushTarget::Element { sdf, edits: carving, scale: tf.scale }),
                    None => estimate_add_mass_kg(&stroke, mat, &mut recorded_target(&mut self.region, &brush, &unpromoted)),
                };
                let need_kg = match need {
                    Ok(kg) => kg,
                    Err(e) => return Outcome::Denied(format!("material check failed: {e}")),
                };
                if need_kg > 0.0 {
                    let mut preview = self.inventory(actor.id);
                    if let Err(reason) = preview.take(mat, need_kg) {
                        return Outcome::Denied(reason);
                    }
                    add_cost = Some((mat, need_kg));
                }
            }
            // Workshop mode: any tool adds, for free.
            Op::Add => {}
        }

        let mut outcome = match (target, &brush) {
            (SculptTarget::Element(id), Some((tf, ..))) => {
                let world = if stroke == world_edit_to_local(&edit, *tf) { edit } else { local_edit_to_world(&stroke, *tf) };
                self.sculpt_element(actor, id, world, stroke)
            }
            (SculptTarget::Terrain { aimed }, _) => self.sculpt_terrain(actor, stroke, aimed),
            (SculptTarget::Element(id), None) => self.unsculptable(id),
        };
        if let Outcome::Done(summary) = &mut outcome {
            if self.mode == WorldMode::World {
                for m in Material::ALL {
                    let kg = carve_yield[m as usize];
                    if edit.op == Op::Carve && kg > 0.0 {
                        self.record_inventory(actor.id, m, kg);
                    }
                }
                if let Some((mat, kg)) = add_cost {
                    // Taken as far as the pack holds (the check allowed a rounding's slack).
                    self.record_inventory(actor.id, mat, -kg);
                }
            }
            if let Some(note) = shrink_note {
                summary.said = Some(match summary.said.take() {
                    Some(s) if !s.is_empty() => format!("{note}; {s}"),
                    _ => note,
                });
            }
        }
        outcome
    }

    /// Sculpts the element `id` itself: `edit` in world space, `local_edit` the same in its frame.
    fn sculpt_element(&mut self, actor: &Actor<'_>, id: u32, edit: Edit, local_edit: Edit) -> Outcome {
        let Some(pl) = self.find(id).cloned() else { return Outcome::Denied(format!("no element #{id} to sculpt")) };
        {
            let _span = info_span!("rules").entered();
            let check = Rule::SculptElement { at: edit.center, player_size: actor.body.size, owner: self.owner_of(id) };
            if let Err(denial) = self.rules.check(actor.id, &check) {
                return Outcome::Denied(denial.reason);
            }
        }
        let edit_idx = self.edits.len();
        let tf = pl.transform();
        let newly_promoted = (!pl.is_promoted()).then_some(edit_idx);
        let pos = self.placed.iter().position(|e| e.id == id).unwrap();
        self.placed[pos].local_edits.push(local_edit);
        self.dirty = true;
        if let Some(idx) = newly_promoted {
            self.set_promoted(id, Some(idx));
            let bounds = self.sdf_bounds_of(&self.placed[pos]).into_iter().map(|b| (tf, b)).collect();
            self.commit(Invalidation::Some { edits: vec![edit], bounds, water: None });
        } else {
            self.commit(Invalidation::edit(edit));
        }
        self.history.push(WorldAction::ElementSdfEdit { id, edit: local_edit, newly_promoted });
        if edit.op == Op::Carve && self.worth_asking(id, edit.shape.radius()) {
            self.pending.reshaped.insert(id);
            self.shaken(id);
        }
        let name = self.element_name(id);
        let mut summary = self.summary(if newly_promoted.is_some() {
            format!("{name} sculpted (still choppable & carriable)")
        } else {
            format!("{name} sculpted")
        });
        summary.promoted = newly_promoted.map(|_| id).into_iter().collect();
        Outcome::Done(summary)
    }

    fn sculpt_terrain(&mut self, actor: &mut Actor<'_>, edit: Edit, aimed: Option<u32>) -> Outcome {
        let size = actor.body.size;
        let (lo, hi) = edit.influence_bounds();
        {
            let _span = info_span!("rules").entered();
            let check = Rule::Sculpt { lo, hi, player_size: size, is_water: edit.material == Some(Material::Water) };
            if let Err(denial) = self.rules.check(actor.id, &check) {
                return Outcome::Denied(denial.reason);
            }
        }
        // A carve that opens onto water below its level floods (it, and any dry carve it joins).
        let water_before = self.water.clone();
        let water_bounds = if edit.op == Op::Carve && self.rules.check(actor.id, &Rule::ModifyWater { lo, hi, player_size: size }).is_ok() {
            let _span = info_span!("water").entered();
            let mut next = (*self.water).clone();
            match next.expand_from_carve(&mut self.region, &self.edits, &edit, Some(actor.id.to_string())) {
                Ok(Some(bounds)) => {
                    self.water = Arc::new(next);
                    Some(bounds)
                }
                _ => None,
            }
        } else {
            None
        };
        // Carving (never adding) beside a scene element promotes it, so the carve cuts it.
        let edit_idx = self.edits.len();
        let (promoted_ids, promo_bounds, promo_names) =
            if edit.op == Op::Carve { self.promote_overlapping_scene_elements(&edit, aimed, edit_idx) } else { (Vec::new(), Vec::new(), Vec::new()) };
        let carved = self.carved_by_edit(&edit, &promoted_ids.iter().map(|&(id, _)| id).collect::<Vec<_>>());
        let mut next = (*self.edits).clone();
        if let Err(e) = next.push(edit) {
            return Outcome::Denied(format!("sculpt error: {e}"));
        }
        self.edits = Arc::new(next);
        self.edit_authors.push(Some(actor.id.to_string()));
        self.dirty = true;
        self.history.push(WorldAction::SdfEdit { edit, promoted_ids: promoted_ids.clone() });
        for what in carved.iter().cloned() {
            self.relate(Relation::carved_into(edit_idx as u32, what));
        }
        self.shaken_by_edit(&edit, &carved);
        if water_bounds.is_some() {
            for (body, idx) in self.newly_flooded(&water_before) {
                self.relate(Relation::flooded_by(body, idx));
            }
        }
        self.commit(Invalidation::Some { edits: vec![edit], bounds: promo_bounds, water: water_bounds });
        if water_bounds.is_some() || !self.floating.is_empty() {
            self.refresh_all_buoyancy();
        }
        if edit.op == Op::Add {
            let max_lift = (hi[1] as f64 - actor.body.feet[1]).max(0.0) + size as f64 * 2.0;
            self.lift(actor.body, max_lift);
        }
        let said = if promo_names.is_empty() { String::new() } else { format!("{} converted to carvable solid", promo_names.join(", ")) };
        let mut summary = self.summary(said);
        if summary.said.as_deref() == Some("") {
            summary.said = None;
        }
        summary.promoted = promoted_ids.iter().map(|&(id, _)| id).collect();
        summary.water = water_bounds.is_some();
        Outcome::Done(summary)
    }

    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        actor: &mut Actor<'_>,
        module: &str,
        pos: [f32; 3],
        yaw: f32,
        rot: Option<[f32; 4]>,
        scale: f32,
        fitted: bool,
        opening: Option<Opening>,
        seed: Option<u64>,
        params: &[f32],
    ) -> Outcome {
        let size = actor.body.size;
        if !(scale.is_finite() && scale > 0.0)
            || !pos.iter().all(|c| c.is_finite())
            || opening.is_some_and(|o| o.validate().is_err())
            || crate::wasm::check_params(params).is_err()
        {
            return Outcome::Denied("cannot place there".into());
        }
        if let Err(denial) = self.rules.check(actor.id, &Rule::PlaceElement { at: pos, player_size: size }) {
            return Outcome::Denied(denial.reason);
        }
        if let Err(e) = self.catalog.get_or_load(module) {
            return Outcome::Denied(format!("place error: {e}"));
        }
        // What the opening is in, asked before the element is there to be mistaken for it.
        let host = opening.and_then(|o| self.host_at(o).map(|h| (o, h)));
        let id = self.alloc_id();
        let pl = PlacedElement {
            id,
            module: module.to_string(),
            seed: seed.unwrap_or_else(|| self.seed.wrapping_mul(31).wrapping_add(id as u64)),
            pos,
            yaw,
            rot,
            scale,
            stamp_size: size,
            after_edits: self.edits.len(),
            promoted: None,
            clips: Vec::new(),
            local_edits: Vec::new(),
            author: Some(actor.id.to_string()),
            params: params.to_vec(),
        };
        match self.spawn_placed(pl.clone(), Some(&mut *actor.body)) {
            Ok(name) => {
                self.relate(Relation::owned_by(id, actor.id));
                if let Some((o, host)) = host {
                    self.relate(Relation::fills(id, host.clone(), o));
                    self.relate(Relation::attached_to(id, host));
                }
                self.settle_relations(id);
                let floating = self.floating.contains_key(&id);
                self.history.push(WorldAction::PlaceElement(pl));
                let said = if fitted {
                    format!("placed fitted {name} ({scale:.3}x — press F to interact)")
                } else if floating {
                    format!("placed {name} ({scale:.2}x, floating — step aboard & press F to paddle)")
                } else {
                    format!("placed {name} ({scale:.2}x)")
                };
                let mut summary = self.summary(said);
                summary.added = vec![id];
                Outcome::Done(summary)
            }
            Err(e) => {
                self.next_id = id;
                Outcome::Denied(format!("place error: {e}"))
            }
        }
    }

    fn remove(&mut self, actor: &Actor<'_>, id: u32) -> Outcome {
        let Some(pl) = self.find(id) else { return Outcome::Denied(format!("no element #{id} to remove")) };
        if let Err(denial) = self.rules.check(actor.id, &Rule::RemoveElement { at: pl.pos, owner: self.owner_of(id) }) {
            return Outcome::Denied(denial.reason);
        }
        self.shaken(id);
        self.unrelate_leaving(id);
        match self.remove_placed(id) {
            Some(removed) => {
                self.history.push(WorldAction::RemoveElement(removed));
                let mut summary = self.summary(format!("removed element #{id}"));
                summary.removed = vec![id];
                Outcome::Done(summary)
            }
            None => Outcome::Denied(format!("no element #{id} to remove")),
        }
    }

    /// Water made again from the base areas and the edits, after an edit that may have flooded
    /// was undone or redone: the bounds that changed.
    fn rebuild_water(&mut self) -> Option<([f32; 3], [f32; 3])> {
        let rebuilt = WaterLog::recompute_from_edits(&mut self.region.region, &self.edits).ok()?;
        let diff = self.water.diff_bounds(&rebuilt);
        if rebuilt.bodies() != self.water.bodies() {
            self.water = Arc::new(rebuilt);
        }
        diff
    }

    fn lift(&mut self, body: &mut Body, max_lift: f64) {
        let _ = body.surface_from_solid(&mut self.combined(), max_lift);
    }

    fn undo(&mut self, body: &mut Body) -> Outcome {
        self.finish_all_falls(Some(&mut *body));
        let Some(done) = self.history.undo.pop() else { return Outcome::Unchanged("nothing to undo".into()) };
        let said = match self.undo_one(done.action.clone(), body) {
            Ok(said) => said,
            Err(Refused::Unchanged(why)) => {
                self.history.undo.push(done);
                return Outcome::Unchanged(why);
            }
            Err(Refused::Denied(why)) => return Outcome::Denied(why),
        };
        self.revert_graph(&done.graph, false);
        self.revert_inventory(&done.inventory, false);
        self.history.redo.push(done);
        Outcome::Done(self.summary(said))
    }

    /// Undoes one change (its graph is reverted by `undo`): what to say, or why it could not be.
    fn undo_one(&mut self, action: WorldAction, body: &mut Body) -> std::result::Result<String, Refused> {
        Ok(match action {
            WorldAction::SdfEdit { edit, promoted_ids } => {
                let mut next = (*self.edits).clone();
                if next.pop().is_none() {
                    return Err(Refused::Unchanged("nothing to undo".into()));
                }
                self.edits = Arc::new(next);
                self.edit_authors.pop();
                self.dirty = true;
                let water = if edit.op == Op::Carve && !self.water.is_empty() { self.rebuild_water() } else { None };
                let demote: Vec<_> = promoted_ids.iter().map(|&(id, _)| (id, None)).collect();
                let bounds = self.set_elements_promoted(&demote);
                self.commit(Invalidation::Some { edits: vec![edit], bounds, water });
                format!("undo sculpt ({} edits)", self.edits.len())
            }
            WorldAction::ElementSdfEdit { id, newly_promoted, .. } => {
                let Some(pos) = self.placed.iter().position(|e| e.id == id) else {
                    return Err(Refused::Unchanged("nothing to undo".into()));
                };
                let old_bounds = self.sdf_bounds_of(&self.placed[pos]);
                let tf = self.placed[pos].transform();
                self.placed[pos].local_edits.pop();
                if newly_promoted.is_some() {
                    self.set_promoted(id, None);
                }
                self.dirty = true;
                self.commit(old_bounds.map_or(Invalidation::All, |b| Invalidation::element(tf, b)));
                "undo object sculpt".into()
            }
            WorldAction::PlaceElement(pl) => {
                self.remove_placed(pl.id);
                format!("undo place ({} elements)", self.placed.len())
            }
            WorldAction::RemoveElement(pl) => {
                if let Err(e) = self.spawn_placed(pl, Some(&mut *body)) {
                    return Err(Refused::Denied(format!("undo failed: {e}")));
                }
                format!("restored element ({} elements)", self.placed.len())
            }
            WorldAction::CutElement { original, lower, upper } => {
                self.remove_placed(upper.id);
                self.remove_placed(lower.id);
                if let Err(e) = self.spawn_placed(original, Some(&mut *body)) {
                    return Err(Refused::Denied(format!("undo failed: {e}")));
                }
                "undo chop".into()
            }
            WorldAction::RelocateElement { id, before, .. } => {
                self.relocate_placed(id, before, Some(&mut *body));
                "undo move".into()
            }
            WorldAction::Fell { before, .. } => {
                let id = before.id;
                self.restore_placed(before);
                self.floating.remove(&id);
                "undo fall".into()
            }
            WorldAction::Refit { before, .. } => {
                self.restore_placed(before);
                "undo refit".into()
            }
            WorldAction::Many(actions) => {
                let mut said = String::new();
                for a in actions.into_iter().rev() {
                    said = self.undo_one(a, body)?;
                }
                said
            }
        })
    }

    fn redo(&mut self, actor: &mut Actor<'_>) -> Outcome {
        let author = actor.id.to_string();
        let body = &mut *actor.body;
        self.finish_all_falls(Some(&mut *body));
        let Some(done) = self.history.redo.pop() else { return Outcome::Unchanged("nothing to redo".into()) };
        let said = match self.redo_one(done.action.clone(), &author, body) {
            Ok(said) => said,
            Err(Refused::Unchanged(why)) => return Outcome::Unchanged(why),
            Err(Refused::Denied(why)) => return Outcome::Denied(why),
        };
        self.revert_graph(&done.graph, true);
        self.revert_inventory(&done.inventory, true);
        self.history.undo.push(done);
        Outcome::Done(self.summary(said))
    }

    /// Redoes one change (its graph is redone by `redo`): what to say, or why it could not be.
    fn redo_one(&mut self, action: WorldAction, author: &str, body: &mut Body) -> std::result::Result<String, Refused> {
        Ok(match action {
            WorldAction::SdfEdit { edit, promoted_ids } => {
                let repromote: Vec<_> = promoted_ids.iter().map(|&(id, idx)| (id, Some(idx))).collect();
                let bounds = self.set_elements_promoted(&repromote);
                let mut next = (*self.edits).clone();
                if let Err(e) = next.push(edit) {
                    return Err(Refused::Denied(format!("redo failed: {e}")));
                }
                self.edits = Arc::new(next);
                self.edit_authors.push(Some(author.to_string()));
                self.dirty = true;
                let water = if edit.op == Op::Carve { self.rebuild_water() } else { None };
                self.commit(Invalidation::Some { edits: vec![edit], bounds, water });
                if edit.op == Op::Add {
                    let (_, hi) = edit.influence_bounds();
                    let max_lift = (hi[1] as f64 - body.feet[1]).max(0.0) + body.size as f64 * 2.0;
                    self.lift(body, max_lift);
                }
                format!("redo sculpt ({} edits)", self.edits.len())
            }
            WorldAction::ElementSdfEdit { id, edit, newly_promoted } => {
                let Some(pos) = self.placed.iter().position(|e| e.id == id) else {
                    return Err(Refused::Unchanged("nothing to redo".into()));
                };
                let pl = self.placed[pos].clone();
                let _ = self.catalog.mesh_sdf_of(&pl);
                self.placed[pos].local_edits.push(edit);
                if let Some(idx) = newly_promoted {
                    self.set_promoted(id, Some(idx));
                }
                self.dirty = true;
                let tf = self.placed[pos].transform();
                let bounds = self.sdf_bounds_of(&self.placed[pos]);
                self.commit(bounds.map_or(Invalidation::All, |b| Invalidation::element(tf, b)));
                "redo object sculpt".into()
            }
            WorldAction::PlaceElement(pl) => {
                if let Err(e) = self.spawn_placed(pl, Some(&mut *body)) {
                    return Err(Refused::Denied(format!("redo failed: {e}")));
                }
                format!("redo place ({} elements)", self.placed.len())
            }
            WorldAction::RemoveElement(pl) => {
                self.remove_placed(pl.id);
                format!("redo remove ({} elements)", self.placed.len())
            }
            WorldAction::CutElement { original, lower, upper } => {
                self.remove_placed(original.id);
                if let Err(e) = self.spawn_placed(lower, Some(&mut *body)).and_then(|_| self.spawn_placed(upper, Some(&mut *body))) {
                    return Err(Refused::Denied(format!("redo failed: {e}")));
                }
                "redo chop".into()
            }
            WorldAction::RelocateElement { id, after, .. } => {
                self.relocate_placed(id, after, Some(&mut *body));
                "redo move".into()
            }
            WorldAction::Fell { after, .. } => {
                let id = after.id;
                self.restore_placed(after);
                let _ = self.settle_buoyancy_for(id, 0.0);
                "redo fall".into()
            }
            WorldAction::Refit { after, .. } => {
                self.restore_placed(after);
                "redo refit".into()
            }
            WorldAction::Many(actions) => {
                let mut said = String::new();
                for (i, a) in actions.into_iter().enumerate() {
                    let s = self.redo_one(a, author, body)?;
                    if i == 0 {
                        said = s;
                    }
                }
                said
            }
        })
    }
}
