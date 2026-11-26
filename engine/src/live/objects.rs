//! Changes to placed objects (`WORLD_PLAN.md`, phase E): chopping, carrying, setting down, and
//! piloting what floats.

use super::action::{Actor, Outcome, Summary};
use super::{FallingElement, World, WorldAction};
use crate::Material;
use crate::body;
use crate::compose::{PlacementTransform, quat_rotate};
use crate::graph::Relation;
use crate::cut::{CarryDecision, ObjectPhysics, check_carry, check_cut_with_tool, choose_cut_plane, clip_scene, format_mass, toppled_transform};
use crate::math;
use crate::physics::{BuoyancyState, Tool, compute_buoyancy, player_mass_kg, step_floating_move};
use crate::rules::Action as Rule;

/// How long a chopped piece takes to topple, in seconds (shown only: it has fallen by the next
/// action).
const TOPPLE_SECONDS: f32 = 0.48;

/// Whether `rot` lies its element on its side (its own up within 60° of level).
pub fn is_rot_horizontal(rot: [f32; 4]) -> bool {
    quat_rotate(rot, [0.0, 1.0, 0.0])[1].abs() < 0.5
}

impl World {
    fn said(&self, said: impl Into<String>) -> Summary {
        Summary { edits: self.edits.len(), elements: self.placed.len(), said: Some(said.into()), ..Summary::default() }
    }

    /// Chops the element `id` where `hit` is, with `tool`, across its longest axis, into two new
    /// elements: the lower piece stays where it stood, the upper topples away along `dir` if it
    /// stood upright. Each is `derived_from` the original and owned by its owner (not the cutter),
    /// and rests on what it touches where it comes to rest.
    pub(super) fn cut(&mut self, actor: &mut Actor<'_>, id: u32, hit: [f32; 3], dir: [f32; 3], tool: Tool, material: Option<Material>) -> Outcome {
        let Some(original) = self.find(id).cloned() else { return Outcome::Denied("aim at a placed object within reach to chop (K)".into()) };
        if let Err(denial) = self.rules.check(actor.id, &Rule::CutOrCarry { at: original.pos, owner: self.owner_of(id) }) {
            return Outcome::Denied(denial.reason);
        }
        let name = self.element_name(id);
        let Some(phys) = self.physics_of(id) else { return Outcome::Denied("rooted in terrain — sculpt with B".into()) };
        if let Err(reason) = check_cut_with_tool(&phys, self.is_sdf_element(&original), actor.body.size, tool, material) {
            return Outcome::Denied(reason);
        }
        let orig_tf = original.transform();
        let Some(plane) = choose_cut_plane(&phys, orig_tf.world_to_local_point(hit)) else { return Outcome::Denied("too small to cut cleanly".into()) };
        let Some(scene) = self.runtime.local_scene_of(id).cloned() else { return Outcome::Denied(format!("{name} cannot be cut")) };
        let upper_scene = clip_scene(&scene, &[plane]);
        if clip_scene(&scene, &[plane.flipped()]).instances.is_empty() || upper_scene.instances.is_empty() {
            return Outcome::Denied("cut plane missed solid geometry — aim nearer the middle".into());
        }
        let lower_id = self.alloc_id();
        let mut lower = original.clone();
        lower.id = lower_id;
        lower.clips.push(plane.flipped());
        let upper_id = self.alloc_id();
        let mut upper = original.clone();
        upper.id = upper_id;
        upper.clips.push(plane);
        let upper_phys = ObjectPhysics::compute(&upper_scene, upper.scale);
        let upright = !is_rot_horizontal(orig_tf.rot);

        // Where the upper piece comes to rest: toppled onto the terrain (asked of the SDF alone, so
        // it never lands on the uncut tree's own crown) or nudged apart along the cut, then on water.
        let mut rest = if upright {
            let flen = (dir[0] * dir[0] + dir[2] * dir[2]).sqrt();
            let (fx, fz) = if flen > 1e-3 { (dir[0] / flen, dir[2] / flen) } else { (-math::sin(actor.body.yaw), -math::cos(actor.body.yaw)) };
            let probe = [
                orig_tf.pos[0] as f64 + (fx * upper_phys.world_span * 0.45) as f64,
                orig_tf.pos[1] as f64 + upper_phys.world_span as f64 + 1.0,
                orig_tf.pos[2] as f64 + (fz * upper_phys.world_span * 0.45) as f64,
            ];
            let ground = body::look(&mut self.region, probe, [0.0, -1.0, 0.0], actor.body.size, upper_phys.world_span * 2.0 + 10.0);
            let ground_y = ground.ok().flatten().map_or(orig_tf.pos[1], |h| h[1] as f32);
            toppled_transform(orig_tf, &upper_phys, [fx, fz], ground_y)
        } else {
            let wn = orig_tf.local_to_world_dir(plane.normal);
            let gap = (0.14 * upper.scale).max(0.01);
            PlacementTransform::with_rot([0, 1, 2].map(|a| orig_tf.pos[a] + wn[a] * gap), orig_tf.rot, orig_tf.scale)
        };
        let mut upper_floats = false;
        if let Ok(buoy) = compute_buoyancy(&upper_phys, rest, &mut self.region, 0.0) {
            upper_floats = buoy.is_floating();
            match buoy {
                BuoyancyState::Floating { target_y, .. } | BuoyancyState::Sinking { target_y, .. } => rest.pos[1] = target_y,
                BuoyancyState::Dry { floor_y } => {
                    let (min_y, _) = upper_phys.rotated_y_extents(rest.rot, rest.scale);
                    rest.pos[1] = floor_y - min_y - 0.02 * rest.scale;
                }
            }
        }
        let mut upper_settled = upper.clone();
        upper_settled.set_transform(rest);

        if upright {
            self.falling.push(FallingElement {
                id: upper_id,
                from_pos: orig_tf.pos,
                from_rot: orig_tf.rot,
                to_pos: rest.pos,
                to_rot: rest.rot,
                scale: orig_tf.scale,
                elapsed: 0.0,
                duration: TOPPLE_SECONDS,
            });
        }
        let owner = self.owner_of(id).map(str::to_string);
        self.unrelate_leaving(id);
        self.remove_placed(id);
        let spawned = self
            .spawn_placed(lower.clone(), Some(&mut *actor.body))
            .and_then(|_| self.spawn_placed(if upright { upper.clone() } else { upper_settled.clone() }, Some(&mut *actor.body)));
        if let Err(e) = spawned {
            self.falling.retain(|f| f.id != upper_id);
            return Outcome::Denied(format!("chop failed: {e}"));
        }
        for (piece, clip) in [(lower_id, plane.flipped()), (upper_id, plane)] {
            self.relate(Relation::derived_from(piece, id, clip));
            if let Some(owner) = &owner {
                self.relate(Relation::owned_by(piece, owner));
            }
        }
        self.settle_relations(lower_id);
        // The upper piece rests where its fall ends, not where it is still shown.
        let upper_rests = if upper_floats || upper_phys.rooted { Vec::new() } else { self.contacts_at(upper_id, rest) };
        self.rest_on(upper_id, &upper_rests);
        if upright && upper.is_promoted() {
            // A carved trunk falls as its crisp meshes; its SDF, tunnel and all, is back where it lands.
            self.show_solid_at(upper_id, orig_tf);
        }
        self.history.push(WorldAction::CutElement { original, lower, upper: upper_settled });
        let mut summary = self.said(format!("chopped {name} — press G to carry"));
        summary.added = vec![lower_id, upper_id];
        summary.removed = vec![id];
        Outcome::Done(summary)
    }

    /// Picks the element `id` up, if the player's size can carry it.
    pub(super) fn grab(&mut self, actor: &mut Actor<'_>, id: u32) -> Outcome {
        if self.held.is_some() {
            return Outcome::Denied("already carrying something".into());
        }
        let Some(pl) = self.find(id).cloned() else { return Outcome::Denied("aim at a placed object within reach to grab (G)".into()) };
        if let Err(denial) = self.rules.check(actor.id, &Rule::CutOrCarry { at: pl.pos, owner: self.owner_of(id) }) {
            return Outcome::Denied(denial.reason);
        }
        let Some(phys) = self.physics_of(id) else { return Outcome::Denied("rooted in terrain — sculpt with B".into()) };
        match check_carry(&phys, self.is_sdf_element(&pl), actor.body.size) {
            CarryDecision::Blocked(reason) => Outcome::Denied(reason),
            CarryDecision::Allowed { .. } => {
                self.take_held(id);
                let name = self.element_name(id);
                Outcome::Done(self.said(format!("carrying {name} ({}) — H flat, R/T turn, Wheel tilt, LMB/G place", format_mass(phys.world_mass_kg))))
            }
        }
    }

    /// Sets the carried element down at `pos`, turned by `rot`, as solid geometry: it rests on what
    /// it now touches; what rested on it or was fixed to it is so no longer, nor does it fill or
    /// hang from what it did.
    pub(super) fn drop_held(&mut self, actor: &mut Actor<'_>, pos: [f32; 3], rot: [f32; 4]) -> Outcome {
        let Some(held) = self.held else { return Outcome::Denied("carrying nothing".into()) };
        if !pos.iter().chain(&rot).all(|c| c.is_finite()) {
            return Outcome::Denied("cannot set it down there".into());
        }
        if let Err(denial) = self.rules.check(actor.id, &Rule::PlaceElement { at: pos, player_size: actor.body.size }) {
            return Outcome::Denied(denial.reason);
        }
        let scale = self.find(held.id).map_or(held.from.scale, |pl| pl.scale);
        self.release_held(PlacementTransform::with_rot(pos, rot, scale), true, Some(&mut *actor.body));
        self.unrelate_moved(held.id);
        self.settle_relations(held.id);
        let name = self.element_name(held.id);
        Outcome::Done(self.said(format!("placed {name} down")))
    }

    /// Puts the carried element back where it was picked up.
    pub(super) fn return_held(&mut self) -> Outcome {
        let Some(held) = self.held else { return Outcome::Unchanged(String::new()) };
        self.release_held(held.from, false, None);
        let name = self.element_name(held.id);
        Outcome::Done(self.said(format!("returned {name}")))
    }

    /// Takes the helm of the floating element `id` (`Some`), or lets it go (`None`).
    pub(super) fn pilot(&mut self, id: Option<u32>) -> Outcome {
        match id {
            Some(id) => {
                if !self.floating.contains_key(&id) {
                    return Outcome::Denied("stand on something floating to paddle it".into());
                }
                self.piloting = Some(id);
                let name = self.element_name(id);
                Outcome::Done(self.said(format!("paddling {name} — W/S row, A/D steer, F/Space dismount")))
            }
            None => {
                let Some(id) = self.piloting.take() else { return Outcome::Unchanged(String::new()) };
                // Unloaded, it rests where it was left.
                let _ = self.settle_buoyancy_for(id, 0.0);
                Outcome::Done(self.said("released helm"))
            }
        }
    }

    /// Paddles the piloted element for `dt`, `forward` and `turn` from -1 to 1; the player aboard is
    /// carried along.
    pub(super) fn steer(&mut self, actor: &mut Actor<'_>, forward: f32, turn: f32, dt: f32) -> Outcome {
        let Some(id) = self.piloting else { return Outcome::Denied("not paddling anything".into()) };
        let (Some(pl), Some(phys)) = (self.find(id).cloned(), self.runtime.physics_of(id)) else {
            self.piloting = None;
            return Outcome::Denied("nothing to paddle".into());
        };
        let body = &mut *actor.body;
        let old_tf = pl.transform();
        let speed = forward.clamp(-1.0, 1.0) * (2.8 * old_tf.scale.max(0.01));
        let turn_rate = turn.clamp(-1.0, 1.0) * 1.65;
        if speed.abs() <= 1e-4 && turn_rate.abs() <= 1e-4 {
            return Outcome::Unchanged(String::new());
        }
        let rider_kg = player_mass_kg(body.size);
        let (new_tf, buoy) = match step_floating_move(&phys, old_tf, speed, turn_rate, dt, &mut self.region, rider_kg, 0.0) {
            Ok(stepped) => stepped,
            Err(e) => return Outcome::Denied(format!("paddling failed: {e}")),
        };
        if let BuoyancyState::Sinking { .. } = buoy {
            self.piloting = None;
            self.floating.remove(&id);
            self.relocate_placed_fast(id, new_tf);
            self.dirty = true;
            let name = self.element_name(id);
            return Outcome::Done(self.said(format!("too heavy — {name} is sinking!")));
        }
        let dyaw = turn_rate * dt.clamp(0.0, 0.1);
        let d = [0, 1, 2].map(|a| (new_tf.pos[a] - old_tf.pos[a]) as f64);
        let rx = body.feet[0] - old_tf.pos[0] as f64;
        let rz = body.feet[2] - old_tf.pos[2] as f64;
        let (s, c) = (math::sin(dyaw) as f64, math::cos(dyaw) as f64);
        body.feet[0] = old_tf.pos[0] as f64 + (rx * c + rz * s) + d[0];
        body.feet[1] += d[1];
        body.feet[2] = old_tf.pos[2] as f64 + (-rx * s + rz * c) + d[2];
        body.yaw += dyaw;
        self.relocate_placed_fast(id, new_tf);
        self.dirty = true;
        Outcome::Done(Summary { edits: self.edits.len(), elements: self.placed.len(), ..Summary::default() })
    }

    /// Whether the player may use (`F`) what is at `at`.
    pub(super) fn may_interact(&self, actor: &Actor<'_>, at: [f32; 3]) -> Outcome {
        match self.rules.check(actor.id, &Rule::Interact { at }) {
            Ok(()) => Outcome::Done(Summary { edits: self.edits.len(), elements: self.placed.len(), ..Summary::default() }),
            Err(denial) => Outcome::Denied(denial.reason),
        }
    }
}
