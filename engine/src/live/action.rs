//! What a player asks of the world, and what came of it (`WORLD_PLAN.md`, phase E).
//!
//! An `Action` says everything the change needs, already resolved by whoever asks: the edit a brush
//! makes (not the mouse), the element aimed at (not the ray), the tool in hand. So applying it again
//! to the same world gives the same world, which is what lets the action log replay.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tracing::info_span;

use super::{Invalidation, World, WorldAction};
use crate::body::Body;
use crate::compose::PlacedElement;
use crate::cut::world_edit_to_local;
use crate::edit::{Edit, Op};
use crate::physics::Tool;
use crate::rules::Action as Rule;
use crate::water::WaterLog;
use crate::{Material, Region};

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
    /// `fitted` if it was fitted to an opening.
    Place { module: String, pos: [f32; 3], yaw: f32, rot: Option<[f32; 4]>, scale: f32, fitted: bool },
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
    /// What the player is told.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub said: Option<String>,
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
        let outcome = match &action {
            Action::Sculpt { edit, hit, tool, target } => self.sculpt(&mut actor, *edit, *hit, *tool, *target),
            Action::Place { module, pos, yaw, rot, scale, fitted } => self.place(&mut actor, module, *pos, *yaw, *rot, *scale, *fitted),
            Action::Remove { id } => self.remove(&actor, *id),
            Action::Cut { id, hit, dir, tool, material } => self.cut(&mut actor, *id, *hit, *dir, *tool, *material),
            Action::Grab { id } => self.grab(&mut actor, *id),
            Action::Drop { pos, rot } => self.drop_held(&mut actor, *pos, *rot),
            Action::Return => self.return_held(),
            Action::Pilot { id } => self.pilot(*id),
            Action::Steer { forward, turn, dt } => self.steer(&mut actor, *forward, *turn, *dt),
            Action::Interact { at } => self.may_interact(&actor, *at),
            Action::Undo => self.undo(actor.body),
            Action::Redo => self.redo(&mut actor),
        };
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

    fn sculpt(&mut self, actor: &mut Actor<'_>, edit: Edit, hit: [f32; 3], tool: Tool, target: SculptTarget) -> Outcome {
        if let Err(e) = edit.validate() {
            return Outcome::Denied(format!("sculpt error: {e}"));
        }
        let size = actor.body.size;
        // The same stamp as the last changes nothing but the log.
        let duplicate = match target {
            SculptTarget::Terrain { .. } => self.edits.edits().last() == Some(&edit),
            SculptTarget::Element(id) => self.find(id).is_some_and(|pl| pl.local_edits.last() == Some(&world_edit_to_local(&edit, pl.transform()))),
        };
        if duplicate {
            return Outcome::Unchanged(String::new());
        }
        // A tool carves only what it is for, by what it meets where it is aimed.
        if edit.op == Op::Carve {
            let _span = info_span!("tool").entered();
            let mut out = Vec::new();
            let material = self.combined().sample(&[hit], size, &mut out).ok().and_then(|_| out.first().map(|a| a.material));
            if let Some(reason) = material.and_then(|m| tool.can_carve(m).err()) {
                return Outcome::Denied(reason);
            }
        }
        match target {
            SculptTarget::Element(id) => self.sculpt_element(actor, id, edit),
            SculptTarget::Terrain { aimed } => self.sculpt_terrain(actor, edit, aimed),
        }
    }

    fn sculpt_element(&mut self, actor: &Actor<'_>, id: u32, edit: Edit) -> Outcome {
        let Some(pl) = self.find(id).cloned() else { return Outcome::Denied(format!("no element #{id} to sculpt")) };
        if self.is_sdf_element(&pl) {
            return Outcome::Denied("an SDF landform is sculpted as terrain".into());
        }
        {
            let _span = info_span!("rules").entered();
            let check = Rule::SculptElement { at: edit.center, player_size: actor.body.size, author: pl.author.as_deref() };
            if let Err(denial) = self.rules.check(actor.id, &check) {
                return Outcome::Denied(denial.reason);
            }
        }
        match self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips) {
            Ok(Some(_)) => {}
            Ok(None) => return Outcome::Denied(format!("{} has no solid to sculpt", self.element_name(id))),
            Err(e) => return Outcome::Denied(format!("sculpt error: {e}")),
        }
        let edit_idx = self.edits.len();
        let tf = pl.transform();
        let local_edit = world_edit_to_local(&edit, tf);
        let newly_promoted = (!pl.is_promoted()).then_some(edit_idx);
        let pos = self.placed.iter().position(|e| e.id == id).unwrap();
        self.placed[pos].local_edits.push(local_edit);
        self.dirty = true;
        if let Some(idx) = newly_promoted {
            self.set_promoted(id, Some(idx), None);
            let bounds = self.sdf_bounds_of(&self.placed[pos]).into_iter().map(|b| (tf, b)).collect();
            self.commit(Invalidation::Some { edits: vec![edit], bounds, water: None });
        } else {
            self.commit(Invalidation::edit(edit));
        }
        self.history.push(WorldAction::ElementSdfEdit { id, edit: local_edit, newly_promoted });
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
        // A carve that opens onto water below its level floods.
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
        let mut next = (*self.edits).clone();
        if let Err(e) = next.push(edit) {
            return Outcome::Denied(format!("sculpt error: {e}"));
        }
        self.edits = Arc::new(next);
        self.edit_authors.push(Some(actor.id.to_string()));
        self.dirty = true;
        self.history.push(WorldAction::SdfEdit { edit, promoted_ids: promoted_ids.clone() });
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
    fn place(&mut self, actor: &mut Actor<'_>, module: &str, pos: [f32; 3], yaw: f32, rot: Option<[f32; 4]>, scale: f32, fitted: bool) -> Outcome {
        let size = actor.body.size;
        if !(scale.is_finite() && scale > 0.0) || !pos.iter().all(|c| c.is_finite()) {
            return Outcome::Denied("cannot place there".into());
        }
        if let Err(denial) = self.rules.check(actor.id, &Rule::PlaceElement { at: pos, player_size: size }) {
            return Outcome::Denied(denial.reason);
        }
        if let Err(e) = self.catalog.get_or_load(module) {
            return Outcome::Denied(format!("place error: {e}"));
        }
        let id = self.alloc_id();
        let pl = PlacedElement {
            id,
            module: module.to_string(),
            seed: self.seed.wrapping_mul(31).wrapping_add(id as u64),
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
        };
        match self.spawn_placed(pl.clone(), Some(&mut *actor.body)) {
            Ok(name) => {
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
        if let Err(denial) = self.rules.check(actor.id, &Rule::RemoveElement { at: pl.pos, author: pl.author.as_deref() }) {
            return Outcome::Denied(denial.reason);
        }
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
        let Some(action) = self.history.undo.pop() else { return Outcome::Unchanged("nothing to undo".into()) };
        let said = match action.clone() {
            WorldAction::SdfEdit { edit, promoted_ids } => {
                let mut next = (*self.edits).clone();
                if next.pop().is_none() {
                    self.history.undo.push(action);
                    return Outcome::Unchanged("nothing to undo".into());
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
                    return Outcome::Unchanged("nothing to undo".into());
                };
                let old_bounds = self.sdf_bounds_of(&self.placed[pos]);
                let tf = self.placed[pos].transform();
                self.placed[pos].local_edits.pop();
                if newly_promoted.is_some() {
                    self.set_promoted(id, None, Some(false));
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
                    return Outcome::Denied(format!("undo failed: {e}"));
                }
                format!("restored element ({} elements)", self.placed.len())
            }
            WorldAction::CutElement { original, lower, upper } => {
                self.remove_placed(upper.id);
                self.remove_placed(lower.id);
                if let Err(e) = self.spawn_placed(original, Some(&mut *body)) {
                    return Outcome::Denied(format!("undo failed: {e}"));
                }
                "undo chop".into()
            }
            WorldAction::RelocateElement { id, before, .. } => {
                self.relocate_placed(id, before, Some(&mut *body));
                "undo move".into()
            }
        };
        self.history.redo.push(action);
        Outcome::Done(self.summary(said))
    }

    fn redo(&mut self, actor: &mut Actor<'_>) -> Outcome {
        let author = actor.id.to_string();
        let body = &mut *actor.body;
        self.finish_all_falls(Some(&mut *body));
        let Some(action) = self.history.redo.pop() else { return Outcome::Unchanged("nothing to redo".into()) };
        let said = match action.clone() {
            WorldAction::SdfEdit { edit, promoted_ids } => {
                let repromote: Vec<_> = promoted_ids.iter().map(|&(id, idx)| (id, Some(idx))).collect();
                let bounds = self.set_elements_promoted(&repromote);
                let mut next = (*self.edits).clone();
                if let Err(e) = next.push(edit) {
                    return Outcome::Denied(format!("redo failed: {e}"));
                }
                self.edits = Arc::new(next);
                self.edit_authors.push(Some(author));
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
                    return Outcome::Unchanged("nothing to redo".into());
                };
                let pl = self.placed[pos].clone();
                let _ = self.catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips);
                self.placed[pos].local_edits.push(edit);
                if let Some(idx) = newly_promoted {
                    self.set_promoted(id, Some(idx), None);
                }
                self.dirty = true;
                let tf = self.placed[pos].transform();
                let bounds = self.sdf_bounds_of(&self.placed[pos]);
                self.commit(bounds.map_or(Invalidation::All, |b| Invalidation::element(tf, b)));
                "redo object sculpt".into()
            }
            WorldAction::PlaceElement(pl) => {
                if let Err(e) = self.spawn_placed(pl, Some(&mut *body)) {
                    return Outcome::Denied(format!("redo failed: {e}"));
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
                    return Outcome::Denied(format!("redo failed: {e}"));
                }
                "redo chop".into()
            }
            WorldAction::RelocateElement { id, after, .. } => {
                self.relocate_placed(id, after, Some(&mut *body));
                "redo move".into()
            }
        };
        self.history.undo.push(action);
        Outcome::Done(self.summary(said))
    }
}
