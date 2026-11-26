//! How the live world keeps its graph (`WORLD_PLAN.md`, phase F): what an element rests on, found
//! from real contacts; whether it needs holding up at all; and every relation written through the
//! journal, so the action that wrote it reports it and its undo reverts it.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::World;
use crate::compose::{Composed, ComposedLog, PlacementTransform, quat_rotate};
use crate::graph::{GraphDelta, Kind, Node, Relation};
use crate::water::WaterLog;
use crate::{Answer, Description, LARGEST, Region, Result, SMALLEST, Scene};

/// The most points of an element asked about when finding what it rests on.
const CONTACT_POINTS: usize = 512;
/// A contact is within this share of the smaller object's size.
pub const CONTACT_TOLERANCE: f32 = 0.05;

/// How an element is held up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Footing {
    /// An SDF landform: it is ground.
    Ground,
    /// Rooted in what it stands on (roots or a foundation below its origin).
    Rooted,
    /// Afloat.
    Floating,
    /// Carried.
    Held,
    /// Held up only by what it rests on (`rests_on`).
    Loose,
}

/// A region with nothing in it, for asking one SDF element alone.
struct Nothing;

impl Region for Nothing {
    fn sample(&mut self, points: &[[f32; 3]], _size: f32, out: &mut Vec<Answer>) -> Result<()> {
        out.clear();
        out.resize(points.len(), Answer { distance: 1e9, material: crate::Material::Stone });
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        wasmtime::bail!("nothing to describe")
    }
}

/// Points of `scene`'s solid meshes (all its meshes if none is solid) at `tf`, in world space, at
/// most `CONTACT_POINTS`, taken evenly.
fn solid_points(scene: &Scene, tf: PlacementTransform) -> Vec<[f32; 3]> {
    let has_solid = scene.instances.iter().any(|i| i.solid);
    let mut pts = Vec::new();
    for inst in &scene.instances {
        if has_solid && !inst.solid {
            continue;
        }
        let Some(mesh) = scene.meshes.get(inst.mesh as usize) else { continue };
        for v in &mesh.vertices {
            let s = [v.pos[0] * inst.scale[0], v.pos[1] * inst.scale[1], v.pos[2] * inst.scale[2]];
            let r = quat_rotate(inst.rot, s);
            pts.push(tf.local_to_world_point([inst.pos[0] + r[0], inst.pos[1] + r[1], inst.pos[2] + r[2]]));
        }
    }
    if pts.len() > CONTACT_POINTS {
        let step = pts.len().div_ceil(CONTACT_POINTS);
        pts = pts.into_iter().step_by(step).collect();
    }
    pts
}

/// Whether distances `d` at the points, and `up` at the points raised by `tol`, show something
/// beneath one of them within `tol` (touching, or pressed into, and below rather than beside).
fn touches_from_above(d: &[f32], up: &[f32], tol: f32, depth: f32) -> bool {
    d.iter().zip(up).any(|(&d, &u)| d <= tol && d >= -depth && u - d >= 0.5 * tol)
}

impl World {
    /// Who owns the element `id` (its `owned_by`).
    pub fn owner_of(&self, id: u32) -> Option<&str> {
        self.graph.owner_of(id)
    }

    /// Adds `rel` to the graph, noted in this action's delta.
    pub fn relate(&mut self, rel: Relation) {
        self.graph.add(rel, &mut self.journal);
    }

    /// Takes `rel` from the graph, noted in this action's delta.
    pub fn unrelate(&mut self, rel: &Relation) {
        self.graph.remove(rel, &mut self.journal);
    }

    /// Undoes (or, with `again`, redoes) a change's graph delta, noted in this action's delta.
    pub(super) fn revert_graph(&mut self, delta: &GraphDelta, again: bool) {
        if again {
            self.graph.redo(delta, &mut self.journal);
        } else {
            self.graph.undo(delta, &mut self.journal);
        }
    }

    /// The relations the element `id` takes with it out of the world, taken.
    pub(super) fn unrelate_leaving(&mut self, id: u32) {
        for rel in self.graph.leaving(id) {
            self.unrelate(&rel);
        }
    }

    /// How the element `id` is held up, if it is in the world.
    pub fn footing(&self, id: u32) -> Option<Footing> {
        let pl = self.find(id)?;
        if self.held.is_some_and(|h| h.id == id) {
            return Some(Footing::Held);
        }
        if self.is_sdf_element(pl) {
            return Some(Footing::Ground);
        }
        let Some(phys) = self.physics_of(id) else { return Some(Footing::Ground) };
        Some(if phys.rooted {
            Footing::Rooted
        } else if self.floating.contains_key(&id) {
            Footing::Floating
        } else {
            Footing::Loose
        })
    }

    /// What the element `id`, standing at `tf`, rests on: the terrain and every other element its
    /// solid geometry touches from above, within `CONTACT_TOLERANCE` of the smaller one's size.
    /// Asked of the terrain without water (a floating thing rests on nothing) and of each other
    /// element alone, at the element's own size; in order.
    pub fn contacts_at(&mut self, id: u32, tf: PlacementTransform) -> Vec<Node> {
        let Some(scene) = self.runtime.local_scene_of(id).cloned() else { return Vec::new() };
        let Some(phys) = self.physics_of(id) else { return Vec::new() };
        let pts = solid_points(&scene, tf);
        if pts.is_empty() {
            return Vec::new();
        }
        let span = phys.world_span.max(1e-4);
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in &pts {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let depth = (hi[1] - lo[1]).max(1e-4);
        let size = span.clamp(SMALLEST, LARGEST);
        let raise = |tol: f32| pts.iter().map(|p| [p[0], p[1] + tol, p[2]]).collect::<Vec<_>>();
        let mut found = BTreeSet::new();

        // The terrain: the areas and the sculpting, without water or placed elements.
        let terrain_log = ComposedLog::new_with_water(self.edits.clone(), Vec::new(), Arc::new(WaterLog::new()), 0);
        let tol = (CONTACT_TOLERANCE * span).max(1e-4);
        {
            let mut terrain = Composed::new(&mut self.region.region, terrain_log);
            let (mut d, mut u) = (Vec::new(), Vec::new());
            if terrain.sample(&pts, size, &mut d).is_ok() && terrain.sample(&raise(tol), size, &mut u).is_ok() {
                let (d, u): (Vec<f32>, Vec<f32>) = (d.iter().map(|a| a.distance).collect(), u.iter().map(|a| a.distance).collect());
                if touches_from_above(&d, &u, tol, depth) {
                    found.insert(Node::Terrain);
                }
            }
        }

        // Every other element near enough: scene elements by their solid meshes, SDF ones (native
        // or promoted) by their own field.
        let reach = tol;
        let (qlo, qhi) = ([lo[0] - reach, lo[1] - reach, lo[2] - reach], [hi[0] + reach, hi[1] + reach, hi[2] + reach]);
        let carried = self.held.map(|h| h.id);
        for other in self.scene.solid_placed_overlapping(qlo, qhi) {
            if other == id || Some(other) == carried {
                continue;
            }
            let other_span = self.physics_of(other).map_or(span, |p| p.world_span);
            let tol = (CONTACT_TOLERANCE * span.min(other_span)).max(1e-4);
            let far = 4.0 * tol + depth;
            let dist = |p: [f32; 3]| self.scene.distance_to_solids_of(other, p, far).unwrap_or(far);
            let d: Vec<f32> = pts.iter().map(|&p| dist(p)).collect();
            let u: Vec<f32> = raise(tol).into_iter().map(dist).collect();
            if touches_from_above(&d, &u, tol, depth) {
                found.insert(Node::Element(other));
            }
        }
        let entries: Vec<_> = self
            .region
            .log
            .sdf_elements
            .iter()
            .filter(|e| e.placed.id != id && (0..3).all(|a| e.world_hi[a] >= qlo[a] && e.world_lo[a] <= qhi[a]))
            .cloned()
            .collect();
        for entry in entries {
            let other = entry.placed.id;
            let ext = (0..3).map(|a| entry.world_hi[a] - entry.world_lo[a]).fold(0.0f32, f32::max);
            let tol = (CONTACT_TOLERANCE * span.min(ext)).max(1e-4);
            let log = ComposedLog::new_with_water(Arc::new(Default::default()), vec![entry], Arc::new(WaterLog::new()), 0);
            let mut alone = Composed::new(Nothing, log);
            let (mut d, mut u) = (Vec::new(), Vec::new());
            if alone.sample(&pts, size, &mut d).is_ok() && alone.sample(&raise(tol), size, &mut u).is_ok() {
                let (d, u): (Vec<f32>, Vec<f32>) = (d.iter().map(|a| a.distance).collect(), u.iter().map(|a| a.distance).collect());
                if touches_from_above(&d, &u, tol, depth) {
                    found.insert(Node::Element(other));
                }
            }
        }
        found.into_iter().collect()
    }

    /// Writes what the element `id` rests on where it now is: its real contacts if it is loose,
    /// nothing if it is rooted, afloat or ground. Returns what it rests on.
    pub fn settle_relations(&mut self, id: u32) -> Vec<Node> {
        let Some(tf) = self.find(id).map(|pl| pl.transform()) else { return Vec::new() };
        let supports = if self.footing(id) == Some(Footing::Loose) { self.contacts_at(id, tf) } else { Vec::new() };
        self.rest_on(id, &supports);
        supports
    }

    /// Sets the element `id`'s `rests_on` to exactly `supports`.
    pub fn rest_on(&mut self, id: u32, supports: &[Node]) {
        for old in self.graph.supports_of(id) {
            if !supports.contains(&old) {
                self.unrelate(&Relation::rests_on(id, old));
            }
        }
        for s in supports {
            self.relate(Relation::rests_on(id, s.clone()));
        }
    }

    /// What rests on the element `id` rests on it no longer (it moved away from under them).
    pub(super) fn unrelate_resting_on(&mut self, id: u32) {
        for other in self.graph.resting_on(id) {
            self.unrelate(&Relation::rests_on(other, Node::Element(id)));
        }
    }

    /// The graph of a save made before there was one: each element owned by whoever placed it,
    /// and every loose one resting on what it touches.
    pub(super) fn derive_graph(&mut self) {
        let ids: Vec<(u32, Option<String>)> = self.placed.iter().map(|pl| (pl.id, pl.author.clone())).collect();
        for (id, author) in ids {
            if let Some(a) = author {
                self.relate(Relation::owned_by(id, &a));
            }
            self.settle_relations(id);
        }
    }

    /// Everything noted in the graph since this was last asked.
    pub(super) fn take_journal(&mut self) -> GraphDelta {
        std::mem::take(&mut self.journal)
    }

    /// Relations of `kind` from the element `id`.
    pub fn relations_of(&self, kind: Kind, id: u32) -> Vec<Relation> {
        let node = Node::Element(id);
        self.graph.from_node(kind, &node).collect()
    }
}
