//! How the live world keeps its graph (`WORLD_PLAN.md`, phase F): what an element rests on, found
//! from real contacts; what an opening is in; what a carve cut and what water came in through it;
//! whether an element needs holding up at all; and every relation written through the journal, so
//! the action that wrote it reports it and its undo reverts it.
//!
//! Each question is asked of one thing alone: the terrain (the areas and their sculpting, without
//! water or elements), one SDF element (a landform, or a promoted scene element with its own
//! carving), or one scene element's solid meshes.

use std::collections::BTreeSet;
use std::sync::Arc;

use super::World;
use crate::compose::{Composed, ComposedLog, PlacementTransform, SdfElementEntry, quat_rotate};
use crate::edit::{Edit, EditLog, Op};
use crate::graph::{GraphDelta, Kind, Node, Opening, Relation};
use crate::water::{WaterBasin, WaterLog};
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
pub(super) struct Nothing;

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
pub(super) fn solid_points(scene: &Scene, tf: PlacementTransform) -> Vec<[f32; 3]> {
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

/// Which points distances `d` at them, and `up` at them raised by `tol`, show something beneath
/// within `tol` (touching, or pressed into, and below rather than beside).
fn touching_from_above(d: &[f32], up: &[f32], tol: f32, depth: f32) -> Vec<bool> {
    d.iter().zip(up).map(|(&d, &u)| d <= tol && d >= -depth && u - d >= 0.5 * tol).collect()
}


fn overlaps(lo: [f32; 3], hi: [f32; 3], blo: [f32; 3], bhi: [f32; 3]) -> bool {
    (0..3).all(|a| hi[a] >= blo[a] && lo[a] <= bhi[a])
}

impl World {
    /// Distances from `pts` to the SDF element `entry` alone, at `size`, as the world has it: its own
    /// field with the terrain's carves made since it entered the SDF (a stump the terrain was carved
    /// through is carved). Kept while the edits and the element are as they were.
    pub(super) fn entry_distances(&mut self, entry: &SdfElementEntry, pts: &[[f32; 3]], size: f32) -> Option<Vec<f32>> {
        let _span = tracing::info_span!("entry_distances").entered();
        let rev = self.edits.revision();
        let log = match self.alone.get(&entry.placed.id) {
            Some((r, pl, log)) if *r == rev && *pl == entry.placed => log.clone(),
            _ => {
                let from = entry.placed.sdf_after_edits().min(self.edits.len());
                let carves: Vec<Edit> = self.edits.edits()[from..].iter().filter(|e| e.op == Op::Carve).copied().collect();
                let mut first = entry.clone();
                first.placed.after_edits = 0;
                if first.placed.promoted.is_some() {
                    first.placed.promoted = Some(0);
                }
                let log = Arc::new(ComposedLog::new_with_water(Arc::new(EditLog::from_edits(carves).ok()?), vec![first], Arc::new(WaterLog::new()), 0));
                self.alone.insert(entry.placed.id, (rev, entry.placed.clone(), log.clone()));
                log
            }
        };
        let mut out = Vec::new();
        Composed::new(Nothing, log).sample(pts, size, &mut out).ok()?;
        Some(out.iter().map(|a| a.distance).collect())
    }

    /// Distances from `pts` to the terrain alone (the areas with `edits`, no water, no elements), at
    /// `size`.
    pub(super) fn terrain_distances(&mut self, edits: Arc<EditLog>, pts: &[[f32; 3]], size: f32) -> Option<Vec<f32>> {
        let _span = tracing::info_span!("terrain_distances").entered();
        let log = ComposedLog::new_with_water(edits, Vec::new(), Arc::new(WaterLog::new()), 0);
        let mut out = Vec::new();
        Composed::new(&mut self.region.region, log).sample(pts, size, &mut out).ok()?;
        Some(out.iter().map(|a| a.distance).collect())
    }

    /// The SDF elements (landforms and promoted ones) whose bounds come within `lo..hi`, but `except`.
    pub(super) fn sdf_entries_near(&self, lo: [f32; 3], hi: [f32; 3], except: Option<u32>) -> Vec<SdfElementEntry> {
        self.region.log.sdf_elements.iter().filter(|e| Some(e.placed.id) != except && overlaps(e.world_lo, e.world_hi, lo, hi)).cloned().collect()
    }

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
        // Rooted by its shape, unless its roots were found to hold in nothing (`uprooted`).
        Some(if phys.rooted && !self.graph.contains(&Relation::uprooted(id)) {
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
        let _span = tracing::info_span!("contacts").entered();
        if let Some((pts, radii)) = self.carved_body(id, tf) {
            return self.touching(id, &pts, Some(&radii), CONTACT_TOLERANCE, 1e-4).0;
        }
        let Some(scene) = self.runtime.local_scene_of(id).cloned() else { return Vec::new() };
        let pts = solid_points(&scene, tf);
        self.touching(id, &pts, None, CONTACT_TOLERANCE, 1e-4).0
    }

    /// What the points `pts` of the element `id` (each a ball of its `radii`, if given) touch from
    /// above, within `share` of the smaller one's size (and at least `least`), and which of them
    /// touch anything.
    pub(super) fn touching(&mut self, id: u32, pts: &[[f32; 3]], radii: Option<&[f32]>, share: f32, least: f32) -> (Vec<Node>, Vec<bool>) {
        let _span = tracing::info_span!("touching").entered();
        let mut touched = vec![false; pts.len()];
        let Some(phys) = self.physics_of(id) else { return (Vec::new(), touched) };
        if pts.is_empty() {
            return (Vec::new(), touched);
        }
        let span = phys.world_span.max(1e-4);
        let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
        for p in pts {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let depth = (hi[1] - lo[1]).max(1e-4);
        let size = span.clamp(SMALLEST, LARGEST);
        let raise = |tol: f32| pts.iter().map(|p| [p[0], p[1] + tol, p[2]]).collect::<Vec<_>>();
        // A field (signed) is touched from above where a point within reach of it is in it once let
        // down by that reach (its ball and the tolerance): beside a wall it is not, and a carved
        // field, which says less than the distance near the carve's lip, is not taken at its word.
        let lower = |tol: f32| pts.iter().enumerate().map(|(i, p)| [p[0], p[1] - tol - radii.map_or(0.0, |r| r[i]), p[2]]).collect::<Vec<_>>();
        let on_field = |d: Vec<f32>, low: Vec<f32>, tol: f32| -> Vec<bool> { d.into_iter().zip(low).map(|(d, l)| d <= tol && d >= -depth && l <= 0.0).collect() };
        let mut found = BTreeSet::new();
        let less = |d: Vec<f32>| match radii {
            Some(r) => d.into_iter().zip(r).map(|(d, r)| d - r).collect(),
            None => d,
        };
        let mut note = |node: Node, mask: Vec<bool>, touched: &mut Vec<bool>| {
            if mask.iter().any(|&m| m) {
                found.insert(node);
                for (t, m) in touched.iter_mut().zip(mask) {
                    *t |= m;
                }
            }
        };

        // The terrain: the areas and the sculpting, without water or placed elements.
        let tol = (share * span).max(least);
        let edits = self.edits.clone();
        if let Some(d) = self.terrain_distances(edits.clone(), pts, size) {
            let d = less(d);
            if d.iter().any(|&d| d <= tol)
                && let Some(low) = self.terrain_distances(edits, &lower(tol), size)
            {
                note(Node::Terrain, on_field(d, low, tol), &mut touched);
            }
        }

        // Every other element near enough: scene elements by their solid meshes, SDF ones (native
        // or promoted) by their own field.
        let reach = (CONTACT_TOLERANCE * span).max(1e-4).max(tol);
        let (qlo, qhi) = ([lo[0] - reach, lo[1] - reach, lo[2] - reach], [hi[0] + reach, hi[1] + reach, hi[2] + reach]);
        let carried = self.held.map(|h| h.id);
        for other in self.scene.solid_placed_overlapping(qlo, qhi) {
            if other == id || Some(other) == carried {
                continue;
            }
            let other_span = self.physics_of(other).map_or(span, |p| p.world_span);
            let tol = (share * span.min(other_span)).max(least);
            let far = 4.0 * tol + depth;
            let dist = |p: [f32; 3]| self.scene.distance_to_solids_of(other, p, far).unwrap_or(far);
            let d: Vec<f32> = pts.iter().map(|&p| dist(p)).collect();
            let u: Vec<f32> = raise(tol).into_iter().map(dist).collect();
            note(Node::Element(other), touching_from_above(&less(d), &less(u), tol, depth), &mut touched);
        }
        for entry in self.sdf_entries_near(qlo, qhi, Some(id)) {
            if Some(entry.placed.id) == carried {
                continue;
            }
            let ext = (0..3).map(|a| entry.world_hi[a] - entry.world_lo[a]).fold(0.0f32, f32::max);
            let tol = (share * span.min(ext)).max(least);
            if let Some(d) = self.entry_distances(&entry, pts, size) {
                let d = less(d);
                if d.iter().any(|&d| d <= tol)
                    && let Some(low) = self.entry_distances(&entry, &lower(tol), size)
                {
                    note(Node::Element(entry.placed.id), on_field(d, low, tol), &mut touched);
                }
            }
        }
        (found.into_iter().collect(), touched)
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

    /// The element `id` has moved: what rested on it, or was fixed to it, is so no longer, and it
    /// no longer fills or is fixed to what it was.
    pub(super) fn unrelate_moved(&mut self, id: u32) {
        let node = Node::Element(id);
        let gone: Vec<Relation> = self
            .graph
            .relations()
            .filter(|r| (r.to == node && crate::graph::Graph::rests_on_host(r.kind)) || (r.from == node && matches!(r.kind, Kind::AttachedTo | Kind::Fills)))
            .collect();
        for r in gone {
            self.unrelate(&r);
        }
    }

    /// The placed scene element (unpromoted or promoted, never an SDF landform or carried element)
    /// whose actual solid surface lies at `p` (within `5 %` of `size`, at least `2 mm`), the
    /// nearest. Used by aiming so overlapping AABBs of neighbouring pieces (such as a fallen trunk
    /// lying behind its stump) never hijack strokes aimed at another piece or at the terrain.
    pub fn scene_element_at(&mut self, p: [f32; 3], size: f32) -> Option<u32> {
        let tol = (0.05 * size).max(2e-3);
        let lo = [p[0] - tol, p[1] - tol, p[2] - tol];
        let hi = [p[0] + tol, p[1] + tol, p[2] + tol];
        let held = self.held.map(|h| h.id);
        let mut best: Option<(f32, u32)> = None;
        let mut consider = |d: f32, id: u32| {
            if d <= tol && best.is_none_or(|(bd, bi)| d < bd || (d == bd && id < bi)) {
                best = Some((d, id));
            }
        };
        for id in self.scene.solid_placed_overlapping(lo, hi) {
            if Some(id) == held || self.find(id).is_none_or(|pl| self.is_sdf_element(pl)) {
                continue;
            }
            if let Some(d) = self.scene.distance_to_solids_of(id, p, tol) {
                consider(d, id);
            }
        }
        let sample_size = size.clamp(SMALLEST, LARGEST);
        for entry in self.sdf_entries_near(lo, hi, held) {
            if !entry.placed.is_promoted() || self.is_sdf_element(&entry.placed) {
                continue;
            }
            if let Some(d) = self.entry_distances(&entry, &[p], sample_size) {
                consider(d[0].abs(), entry.placed.id);
            }
        }
        best.map(|(_, id)| id)
    }

    /// What the opening pointed at by `o` is in: the thing solid just behind its wall (the terrain,
    /// or an SDF element, most deeply), or else the scene element whose solid the wall is.
    pub fn host_at(&mut self, o: Opening) -> Option<Node> {
        let eps = (0.05 * o.size).max(1e-3);
        let n = o.normal;
        let p = [o.target[0] - n[0] * eps, o.target[1] - n[1] * eps, o.target[2] - n[2] * eps];
        let size = o.size.clamp(SMALLEST, LARGEST);
        let mut best: Option<(f32, Node)> = None;
        let mut consider = |d: f32, node: Node| {
            if d < 0.0 && best.as_ref().is_none_or(|(b, _)| d < *b) {
                best = Some((d, node));
            }
        };
        let edits = self.edits.clone();
        if let Some(d) = self.terrain_distances(edits, &[p], size) {
            consider(d[0], Node::Terrain);
        }
        for entry in self.sdf_entries_near(p, p, None) {
            if let Some(d) = self.entry_distances(&entry, &[p], size) {
                consider(d[0], Node::Element(entry.placed.id));
            }
        }
        if let Some((_, node)) = best {
            return Some(node);
        }
        // A scene element's meshes say how far, not which side: the nearest within the wall's reach.
        let reach = 2.0 * eps;
        let mut nearest: Option<(f32, u32)> = None;
        for owner in self.scene.solid_placed_overlapping([p[0] - reach, p[1] - reach, p[2] - reach], [p[0] + reach, p[1] + reach, p[2] + reach]) {
            if let Some(d) = self.scene.distance_to_solids_of(owner, p, reach)
                && nearest.is_none_or(|(b, _)| d < b)
            {
                nearest = Some((d, owner));
            }
        }
        nearest.map(|(_, id)| Node::Element(id))
    }

    /// What the terrain carve `edit`, about to be the edit log's `index`th, carves into: the terrain
    /// if there was any within it, and every SDF element (landform or promoted) it reaches inside,
    /// with `promoted` (those it promotes now, not yet in the SDF).
    pub(super) fn carved_by_edit(&mut self, edit: &Edit, promoted: &[u32]) -> Vec<Node> {
        if edit.op != Op::Carve {
            return Vec::new();
        }
        let size = edit.stamp_size.clamp(SMALLEST, LARGEST);
        let reach = edit.shape.radius();
        let mut found = BTreeSet::new();
        let edits = self.edits.clone();
        if self.terrain_distances(edits, &[edit.center], size).is_some_and(|d| d[0] < reach) {
            found.insert(Node::Terrain);
        }
        let (lo, hi) = edit.influence_bounds();
        for entry in self.sdf_entries_near(lo, hi, None) {
            if self.entry_distances(&entry, &[edit.center], size).is_some_and(|d| d[0] < reach) {
                found.insert(Node::Element(entry.placed.id));
            }
        }
        found.extend(promoted.iter().map(|&id| Node::Element(id)));
        found.into_iter().collect()
    }

    /// The basins water has come into since `before`, as (water body, the terrain edit whose carve
    /// it fills: the last carve of that shape there).
    pub(super) fn newly_flooded(&self, before: &WaterLog) -> Vec<(u32, u32)> {
        let had = |basin: &WaterBasin| before.bodies().iter().any(|b| b.basins.contains(basin));
        let mut out = BTreeSet::new();
        for body in self.water.bodies() {
            for basin in body.basins.iter().filter(|b| !had(b)) {
                if let Some(i) = self.edits.edits().iter().rposition(|e| e.op == Op::Carve && WaterBasin::from_edit(e) == *basin) {
                    out.insert((body.id, i as u32));
                }
            }
        }
        out.into_iter().collect()
    }

    /// Whether the element `id` is held up by the graph: it rests on, or is attached to, something.
    pub fn is_held_up(&self, id: u32) -> bool {
        let node = Node::Element(id);
        self.graph.relations().any(|r| r.from == node && r.kind.holds_up())
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
