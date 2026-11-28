//! Support and falling in the live world (`WORLD_PLAN.md`, phase G): after every action that
//! changed the world, what no longer holds together comes apart, and what nothing holds up falls.
//!
//! An action notes what it may have shaken (`Pending`): elements it carved or cut (`reshaped`),
//! elements the world changed under (`mend`: their rests found again where they are), and pieces
//! that may not stand as they are (`unsteady`, `pushed` the way a cut pushed them). Then
//! `settle_support`, in rounds:
//!
//! 1. **Mend**: each loose element noted rests on what it now touches (terrain dug from under a log,
//!    a bridge gone from under it).
//! 2. **Come apart**: an element carved or cut is made into cells in its own frame
//!    (`support::Grid`, from the SDF the world has of it: its meshes, its own carving, the terrain's
//!    carves since it was promoted). A part not joined to what holds it up (its roots in the ground,
//!    what it rests on or hangs from) comes away along a plane between them — or, where no single
//!    plane parts them (interlocking, a ring cut round), by complementary box-carve masks in
//!    `local_edits` (`support::mask_boxes`): the original is retired, the two pieces `derived_from`
//!    it, as a cut makes them. Where it still holds together,
//!    each height a carve reached is asked whether its cross-section holds what is above it (the
//!    mass not overhanging it by more than `support::SECTION_GRIP` of its width): a notch half
//!    through holds, a deeper one breaks there.
//! 3. **Fall**: every loose element that nothing reaching the ground holds up (`World::grounded`:
//!    resting on, or hanging from, the terrain or something grounded, so two pieces resting only on
//!    each other fall), and every piece unsteady or pushed, lowest first. It drops straight down to
//!    the first thing beneath it; if its centre of mass is not over what it then stands on, it tips
//!    over the edge it overhangs, turning about it until it meets something; and so on, a few times.
//!    It meets the terrain and every other element (`obstacle_distances`), so a felled tree lands
//!    on a bridge, not through it; then water floats it. What rested on it, or hung from it, is
//!    mended and asked again in the next round.
//!
//! At most `World::fall_limit` falls and `MAX_ROUNDS` rounds an action; what is still loose falls
//! with the next action. Every split and fall is undone with the action that set it off. Only IEEE
//! arithmetic and `math`: the same falls on every machine, so the action log replays.
//!
//! What is seen of a fall is shown only (`FallingElement`, by `tick_falling`): the element is where
//! it comes to rest at once, as the save and every later action see it.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::relations::{Nothing, solid_points};
use super::{FallingElement, Footing, Invalidation, LayerChange, World, WorldAction};
use crate::compose::{Composed, ComposedLog, MAX_CLIPS, PlacedElement, PlacementTransform, SdfElementEntry, quat_mul, quat_normalize, quat_rotate};
use crate::cut::{ClipPlane, clip_scene, world_edit_to_local};
use crate::edit::{Edit, EditLog, Op};
use crate::graph::{Data, Graph, Kind, Node, Relation};
use crate::support::{self, CELLS, Grid, SECTION_GRIP};
use crate::water::WaterLog;
use crate::{LARGEST, Region, SMALLEST, math};

/// Falls one action may set off, unless the world says otherwise (`World::fall_limit`).
pub const FALL_LIMIT: usize = 24;
/// Rounds of falls one action may set off (what rested on what fell, and so on).
pub const MAX_ROUNDS: usize = 8;
/// Pieces one action may break elements into.
const MAX_SPLITS: usize = 8;
/// What a body stands on: its points within this share of its size of what is beneath.
const STAND_TOLERANCE: f32 = 0.015;
/// A fall stops this share of the body's size short of what it meets.
const STOP: f32 = 0.01;
/// The most a body tips at once before it is asked again how it stands.
const MAX_TIP: f32 = std::f32::consts::FRAC_PI_2;
const MIN_TIP_STEP: f32 = 0.004;
const MAX_TIP_STEP: f32 = 0.087;
/// The farthest a body drops at once.
const MAX_DROP: f32 = 256.0;
/// Heights a carve's reach is asked at, and the most asked of one element.
const HEIGHTS_PER_CARVE: usize = 5;
const MAX_HEIGHTS: usize = 12;
/// A cross-section is sampled on a square of this many points a side.
const SECTION_SAMPLES: usize = 24;

/// What an action may have shaken, for `settle_support`.
#[derive(Clone, Debug, Default)]
pub(super) struct Pending {
    /// Carved or cut: does it still hold together?
    pub reshaped: BTreeSet<u32>,
    /// The world changed under or round it: what does it rest on now?
    pub mend: BTreeSet<u32>,
    /// Does it stand as it is?
    pub unsteady: BTreeSet<u32>,
    /// Rooted by its shape: do its roots still hold in the terrain?
    pub roots: BTreeSet<u32>,
    /// Pushed over the way (x, z) it was cut, about the edge it broke at if that is known.
    pub pushed: BTreeMap<u32, ([f32; 2], Option<[f32; 3]>)>,
}

/// What `settle_support` did.
#[derive(Clone, Debug, Default)]
pub(super) struct Settled {
    pub added: Vec<u32>,
    pub removed: Vec<u32>,
    pub fell: Vec<u32>,
}

/// How a disconnected part comes away from the rest of an element: along a single plane where one
/// parts them, or else by complementary element-local carve masks (`support::mask_boxes`).
enum Parting {
    Plane(ClipPlane),
    Mask { for_rest: Vec<Edit>, for_part: Vec<Edit> },
}

/// A fall, as it is shown.
#[derive(Clone, Copy, Debug)]
struct Shown {
    id: u32,
    from: PlacementTransform,
    to: PlacementTransform,
    pivot: Option<[f32; 3]>,
}

/// An element's solid alone, as the world's SDF has it: its meshes (clipped), its own carving, and
/// the terrain's carves since it was promoted.
struct Solid {
    log: Arc<ComposedLog>,
    size: f32,
}

impl Solid {
    fn distances(&self, pts: &[[f32; 3]]) -> Vec<f32> {
        let mut out = Vec::new();
        match Composed::new(Nothing, self.log.clone()).sample(pts, self.size, &mut out) {
            Ok(()) => out.iter().map(|a| a.distance).collect(),
            Err(_) => vec![f32::INFINITY; pts.len()],
        }
    }
}

/// An element made into cells, in its own frame.
#[derive(Clone)]
pub(super) struct Cells {
    grid: Grid,
    /// The solid cells, in order, their centres in the element's frame, and how deep inside the
    /// surface each centre is (in the world, nought if just outside it).
    solid: Vec<usize>,
    centres: Vec<[f32; 3]>,
    inside: Vec<f32>,
}

/// The most points a falling body is asked about by.
const BODY_POINTS: usize = 1536;

impl Cells {
    fn world(&self, tf: PlacementTransform) -> Vec<[f32; 3]> {
        self.centres.iter().map(|&c| tf.local_to_world_point(c)).collect()
    }

    /// The body as balls: the centres of the solid cells on its surface (a face open), each as deep
    /// as it is inside, at most `BODY_POINTS` of them, taken evenly.
    fn body(&self) -> (Vec<[f32; 3]>, Vec<f32>) {
        let g = &self.grid;
        let open = |i: usize| {
            let c = g.coords(i);
            (0..3).any(|a| {
                [-1isize, 1].iter().any(|&s| {
                    let n = c[a] as isize + s;
                    if n < 0 || n as usize >= g.dims[a] {
                        return true;
                    }
                    let mut nc = c;
                    nc[a] = n as usize;
                    !g.solid[g.index(nc)]
                })
            })
        };
        let surface: Vec<usize> = (0..self.solid.len()).filter(|&k| open(self.solid[k])).collect();
        let step = surface.len().div_ceil(BODY_POINTS).max(1);
        let kept: Vec<usize> = surface.into_iter().step_by(step).collect();
        (kept.iter().map(|&k| self.centres[k]).collect(), kept.iter().map(|&k| self.inside[k]).collect())
    }
}

fn bounds(pts: &[[f32; 3]]) -> ([f32; 3], [f32; 3]) {
    let (mut lo, mut hi) = ([f32::INFINITY; 3], [f32::NEG_INFINITY; 3]);
    for p in pts {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    (lo, hi)
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn len(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl World {
    /// The carved element `id`'s body as balls on its surface, at `tf` (see `Cells::body`): what it
    /// touches with, where its meshes no longer say (they do not know its carving).
    pub(super) fn carved_body(&mut self, id: u32, tf: PlacementTransform) -> Option<(Vec<[f32; 3]>, Vec<f32>)> {
        let _span = tracing::info_span!("carved_body").entered();
        if !self.find(id)?.is_promoted() {
            return None;
        }
        let (local, radii) = self.cells_of(id)?.body();
        if local.is_empty() {
            return None;
        }
        Some((local.iter().map(|&p| tf.local_to_world_point(p)).collect(), radii))
    }

    /// The elements held up by what reaches the ground: SDF landforms, rooted, afloat or carried
    /// ones, and every loose one resting on or attached to the terrain or one of these.
    pub fn grounded(&self) -> BTreeSet<u32> {
        let mut grounded = BTreeSet::new();
        let mut loose = Vec::new();
        for pl in &self.placed {
            match self.footing(pl.id) {
                Some(Footing::Loose) => loose.push(pl.id),
                Some(_) => {
                    grounded.insert(pl.id);
                }
                None => {}
            }
        }
        let holds: Vec<(u32, Node)> = self.graph.relations().filter(|r| r.kind.holds_up()).filter_map(|r| r.from.element().map(|f| (f, r.to))).collect();
        loop {
            let before = grounded.len();
            for &id in &loose {
                if !grounded.contains(&id)
                    && holds.iter().any(|(f, to)| *f == id && match to {
                        Node::Terrain => true,
                        Node::Element(x) => grounded.contains(x),
                        _ => false,
                    })
                {
                    grounded.insert(id);
                }
            }
            if grounded.len() == before {
                return grounded;
            }
        }
    }

    /// Notes that what rests on, hangs from, fills, or leans against the element `id` may have lost it.
    pub(super) fn shaken(&mut self, id: u32) {
        let node = Node::Element(id);
        let deps: Vec<u32> = self.graph.relations().filter(|r| r.to == node && Graph::rests_on_host(r.kind)).filter_map(|r| r.from.element()).collect();
        for d in deps {
            self.pending.mend.insert(d);
            self.pending.unsteady.insert(d);
        }
        // Loose neighbours touching or propped against `id` (e.g. a sibling cut piece blocked from
        // the side during tipping) also need re-checking once `id` moves away.
        if let Some((lo, hi)) = self.world_box(id) {
            let ids: Vec<u32> = self.placed.iter().map(|p| p.id).filter(|&other| other != id).collect();
            for other in ids {
                if self.footing(other) != Some(Footing::Loose) {
                    continue;
                }
                let Some((olo, ohi)) = self.world_box(other) else { continue };
                let margin = self.physics_of(other).map_or(0.0, |p| 0.05 * p.world_span);
                if (0..3).all(|a| ohi[a] + margin >= lo[a] && olo[a] - margin <= hi[a]) {
                    self.pending.mend.insert(other);
                    self.pending.unsteady.insert(other);
                }
            }
        }
    }

    /// Notes what a terrain stroke `edit` may have shaken: the elements it carved into, if it is
    /// large enough to part them, and every loose element round it.
    pub(super) fn shaken_by_edit(&mut self, edit: &Edit, carved: &[Node]) {
        for id in carved.iter().filter_map(|n| n.clone().element()) {
            if self.worth_asking(id, edit.shape.radius()) {
                self.pending.reshaped.insert(id);
                self.shaken(id);
            }
        }
        let (lo, hi) = edit.influence_bounds();
        let ids: Vec<u32> = self.placed.iter().map(|p| p.id).collect();
        for id in ids {
            let Some((blo, bhi)) = self.world_box(id) else { continue };
            let margin = self.physics_of(id).map_or(0.0, |p| 0.05 * p.world_span);
            if (0..3).all(|a| bhi[a] + margin >= lo[a] && blo[a] - margin <= hi[a]) {
                match self.footing(id) {
                    Some(Footing::Loose) => {
                        self.pending.mend.insert(id);
                    }
                    Some(Footing::Rooted) if edit.op == Op::Carve => {
                        self.pending.roots.insert(id);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Whether a carve of radius `radius` (in the world) could part the element `id`: at least half a
    /// cell of its grid. (A tiny's strokes into an oak are asked about with the next larger change.)
    pub(super) fn worth_asking(&self, id: u32, radius: f32) -> bool {
        self.physics_of(id).is_some_and(|p| radius >= 0.5 * p.world_span / CELLS as f32)
    }

    /// The element `pl`'s solid alone, if it is a scene element with solid meshes.
    fn solid_of(&mut self, pl: &PlacedElement) -> Option<Solid> {
        let _span = tracing::info_span!("solid_of").entered();
        if self.is_sdf_element(pl) {
            return None;
        }
        let mesh = self.catalog.mesh_sdf_of(pl).ok().flatten()?;
        let carves: Vec<Edit> =
            if pl.is_promoted() { self.edits.edits()[pl.sdf_after_edits().min(self.edits.len())..].iter().filter(|e| e.op == Op::Carve).copied().collect() } else { Vec::new() };
        let mut alone = pl.clone();
        alone.promoted = Some(0);
        let entry = SdfElementEntry::from_promoted_mesh(alone, mesh);
        let log = ComposedLog::new_with_water(Arc::new(EditLog::from_edits(carves).ok()?), vec![entry], Arc::new(WaterLog::new()), 0);
        Some(Solid { log: Arc::new(log), size: pl.scale.clamp(SMALLEST, LARGEST) })
    }

    /// The element `id` made into cells (kept while it and the edits are as they were).
    fn cells_of(&mut self, id: u32) -> Option<Cells> {
        let _span = tracing::info_span!("cells_of").entered();
        let pl = self.find(id)?.clone();
        let rev = self.edits.revision();
        if let Some((r, was, c)) = self.cells.get(&id)
            && *r == rev
            && *was == pl
        {
            return Some(c.clone());
        }
        let phys = self.physics_of(id)?;
        let solid = self.solid_of(&pl)?;
        let tf = pl.transform();
        let mut grid = Grid::over(phys.local_lo, phys.local_hi, CELLS)?;
        let cell_w = grid.cell * tf.scale;
        let near = 0.25 * cell_w;
        // In blocks of four cells a side: a block whose centre is clear of the solid by more than its
        // reach is left empty, unasked.
        const B: usize = 4;
        let bd = grid.dims.map(|d| d.div_ceil(B));
        let (mut centres, mut blocks) = (Vec::new(), Vec::new());
        for bz in 0..bd[2] {
            for by in 0..bd[1] {
                for bx in 0..bd[0] {
                    let lo = [bx * B, by * B, bz * B];
                    let hi = [0, 1, 2].map(|a| (lo[a] + B).min(grid.dims[a]));
                    let c = [0, 1, 2].map(|a| grid.lo[a] + (lo[a] + hi[a]) as f32 * 0.5 * grid.cell);
                    centres.push(tf.local_to_world_point(c));
                    blocks.push((lo, hi));
                }
            }
        }
        let block_d = solid.distances(&centres);
        let reach = 0.866_025_4 * B as f32 * cell_w + near;
        let (mut asked, mut pts) = (Vec::new(), Vec::new());
        for ((lo, hi), d) in blocks.iter().zip(&block_d) {
            if *d > reach {
                continue;
            }
            for z in lo[2]..hi[2] {
                for y in lo[1]..hi[1] {
                    for x in lo[0]..hi[0] {
                        let i = grid.index([x, y, z]);
                        asked.push(i);
                        pts.push(tf.local_to_world_point(grid.centre(i)));
                    }
                }
            }
        }
        let mut depth = vec![0.0f32; grid.len()];
        for (&i, d) in asked.iter().zip(solid.distances(&pts)) {
            if d <= near {
                grid.solid[i] = true;
                depth[i] = (-d).max(0.0);
            }
        }
        let solid_cells: Vec<usize> = (0..grid.len()).filter(|&i| grid.solid[i]).collect();
        let centres = solid_cells.iter().map(|&i| grid.centre(i)).collect();
        let inside = solid_cells.iter().map(|&i| depth[i]).collect();
        let cells = Cells { grid, solid: solid_cells, centres, inside };
        self.cells.insert(id, (rev, pl, cells.clone()));
        Some(cells)
    }

    /// What holds the element `id` up: the terrain if it is rooted, and what it rests on or hangs from.
    fn holders_of(&self, id: u32) -> Vec<Node> {
        let mut h = BTreeSet::new();
        if self.footing(id) == Some(Footing::Rooted) {
            h.insert(Node::Terrain);
        }
        h.extend(self.graph.supports_of(id));
        h.extend(self.graph.attached_of(id));
        h.into_iter().collect()
    }

    /// Distances from `pts` to `node` alone (the terrain, or an element), at most `far`.
    fn node_distances(&mut self, node: &Node, pts: &[[f32; 3]], size: f32, far: f32) -> Vec<f32> {
        let _span = tracing::info_span!("node_distances").entered();
        match node {
            Node::Terrain => {
                let edits = self.edits.clone();
                self.terrain_distances(edits, pts, size).unwrap_or_else(|| vec![far; pts.len()])
            }
            Node::Element(x) => {
                if let Some(entry) = self.region.log.sdf_elements.iter().find(|e| e.placed.id == *x).cloned() {
                    self.entry_distances(&entry, pts, size).unwrap_or_else(|| vec![far; pts.len()])
                } else {
                    pts.iter().map(|&p| self.scene.distance_to_solids_of(*x, p, far).unwrap_or(far)).collect()
                }
            }
            _ => vec![far; pts.len()],
        }
    }

    /// Which of the element `id`'s solid cells touch what holds it up (within a cell).
    fn anchored(&mut self, id: u32, cells: &Cells, tf: PlacementTransform) -> Vec<bool> {
        let _span = tracing::info_span!("anchored").entered();
        let world = cells.world(tf);
        let cell_w = cells.grid.cell * tf.scale;
        let size = self.physics_of(id).map_or(1.0, |p| p.world_span).clamp(SMALLEST, LARGEST);
        let mut anchored = vec![false; world.len()];
        for holder in self.holders_of(id) {
            for (a, d) in anchored.iter_mut().zip(self.node_distances(&holder, &world, size, 4.0 * cell_w)) {
                *a |= d <= cell_w;
            }
        }
        anchored
    }

    /// Distances from `pts` to everything but the element `id` (and what is carried), at most
    /// `far`: to the fields (the terrain without water, every other SDF element), signed, and to
    /// every other scene element's solid meshes, unsigned. A field carved into says less than the
    /// distance near the carve's lip (never more), so whether a point is really there is asked
    /// again further on (`meets`).
    fn obstacle_distances(&mut self, id: u32, pts: &[[f32; 3]], size: f32, far: f32) -> (Vec<f32>, Vec<f32>) {
        let _span = tracing::info_span!("obstacle_distances").entered();
        let mut field = self.node_distances(&Node::Terrain, pts, size, far);
        let mut meshes = vec![far; pts.len()];
        let (lo, hi) = bounds(pts);
        let (qlo, qhi) = (lo.map(|v| v - far), hi.map(|v| v + far));
        let carried = self.held.map(|h| h.id);
        for other in self.scene.solid_placed_overlapping(qlo, qhi) {
            if other == id || Some(other) == carried {
                continue;
            }
            for (i, p) in pts.iter().enumerate() {
                if let Some(x) = self.scene.distance_to_solids_of(other, *p, meshes[i]) {
                    meshes[i] = meshes[i].min(x);
                }
            }
        }
        for entry in self.sdf_entries_near(qlo, qhi, Some(id)) {
            if Some(entry.placed.id) == carried {
                continue;
            }
            // An element's field is never nearer than its bounds: where those are no nearer than
            // what each point has already met, it changes nothing, and is not asked (a pile of
            // carved pieces was asked whole at every step a piece tipped, played by hand 2026-11-28).
            if pts.iter().zip(&field).all(|(p, &d)| box_distance(*p, entry.world_lo, entry.world_hi) - entry.world_blend >= d) {
                continue;
            }
            if let Some(e) = self.entry_distances(&entry, pts, size) {
                for (a, b) in field.iter_mut().zip(e) {
                    *a = a.min(b);
                }
            }
        }
        (field, meshes)
    }

    /// Whether each point `pts[i]` moving along unit direction `ways[i]` is entering (not exiting
    /// or grazing away from) any other solid scene element (`!= id`, not carried), using its cached
    /// exact `MeshSdf` signed distance and Signorini's directional condition (`d(p + eps*way) < d(p)`).
    fn entering_other_mesh(&mut self, id: u32, pts: &[[f32; 3]], ways: &[[f32; 3]], eps: f32) -> Vec<bool> {
        let _span = tracing::info_span!("entering_other_mesh").entered();
        let mut hit = vec![false; pts.len()];
        if pts.is_empty() {
            return hit;
        }
        let (lo, hi) = bounds(pts);
        let pad = eps.max(1e-3);
        let qlo = lo.map(|v| v - pad);
        let qhi = hi.map(|v| v + pad);
        let carried = self.held.map(|h| h.id);
        for other in self.scene.solid_placed_overlapping(qlo, qhi) {
            if other == id || Some(other) == carried {
                continue;
            }
            let Some(pl) = self.find(other).cloned() else { continue };
            let Ok(Some(msdf)) = self.catalog.mesh_sdf_of(&pl) else {
                for (i, &p) in pts.iter().enumerate() {
                    if !hit[i] && self.scene.distance_to_solids_of(other, p, 1e-3).is_some() {
                        hit[i] = true;
                    }
                }
                continue;
            };
            let tf = pl.transform();
            let eps_local = pad / tf.scale.max(1e-4);
            for (i, (&p, &w)) in pts.iter().zip(ways).enumerate() {
                if hit[i] {
                    continue;
                }
                let lp = tf.world_to_local_point(p);
                let Some(a0) = msdf.sample(lp, 1e-3) else { continue };
                if a0.distance > 0.0 {
                    continue;
                }
                let lw = tf.world_to_local_dir(w);
                let lp1 = [lp[0] + lw[0] * eps_local, lp[1] + lw[1] * eps_local, lp[2] + lw[2] * eps_local];
                let d1 = msdf.sample(lp1, f32::INFINITY).map_or(f32::INFINITY, |a| a.distance);
                // Only block when moving along `way` goes deeper into `other` (not exiting a piece
                // resting on top of `id`, nor sliding along a vertical wall).
                if d1 < a0.distance - 0.1 * eps_local {
                    hit[i] = true;
                }
            }
        }
        hit
    }

    /// Of the points `pts` (balls of `radii`) whose distances say they are within `stop` of
    /// something, those that actually enter a solid (scene mesh or field) when moved on by `stop`
    /// along `ways` (each its own way of going). Checking that signed distance decreases along
    /// `ways` keeps a piece resting on top of `id` from blocking `id` from dropping away beneath it.
    #[allow(clippy::too_many_arguments)]
    fn meets(&mut self, id: u32, pts: &[[f32; 3]], radii: &[f32], ways: &[[f32; 3]], field: &[f32], meshes: &[f32], stop: f32, size: f32) -> Vec<bool> {
        let _span = tracing::info_span!("meets").entered();
        let mut met = vec![false; pts.len()];
        let ask_mesh: Vec<usize> = (0..pts.len()).filter(|&i| meshes[i] - radii[i] <= stop).collect();
        if !ask_mesh.is_empty() {
            let eps = (0.25 * stop).max(1e-3);
            let mways: Vec<[f32; 3]> = ask_mesh.iter().map(|&i| ways[i]).collect();
            let on: Vec<[f32; 3]> = ask_mesh.iter().map(|&i| [0, 1, 2].map(|a| pts[i][a] + ways[i][a] * (radii[i] + stop))).collect();
            let mid: Vec<[f32; 3]> = ask_mesh.iter().map(|&i| [0, 1, 2].map(|a| pts[i][a] + ways[i][a] * (radii[i] + 0.25 * stop))).collect();
            let hit_on = self.entering_other_mesh(id, &on, &mways, eps);
            let hit_mid = self.entering_other_mesh(id, &mid, &mways, eps);
            for (k, &i) in ask_mesh.iter().enumerate() {
                met[i] = hit_on[k] || hit_mid[k];
            }
        }
        let ask_field: Vec<usize> = (0..pts.len()).filter(|&i| !met[i] && field[i] - radii[i] <= stop).collect();
        if !ask_field.is_empty() {
            let on: Vec<[f32; 3]> = ask_field.iter().map(|&i| [0, 1, 2].map(|a| pts[i][a] + ways[i][a] * (radii[i] + stop))).collect();
            let (further, _) = self.obstacle_distances(id, &on, size, 4.0 * stop);
            for (&i, d) in ask_field.iter().zip(further) {
                met[i] = d <= 0.0;
            }
        }
        met
    }

    /// Whether the rooted element `id`'s roots still hold: some cell of it touches the terrain.
    fn roots_hold(&mut self, id: u32) -> bool {
        let _span = tracing::info_span!("roots_hold").entered();
        let Some(tf) = self.find(id).map(|pl| pl.transform()) else { return true };
        let Some(cells) = self.cells_of(id) else { return true };
        let world = cells.world(tf);
        let cell_w = cells.grid.cell * tf.scale;
        let size = self.physics_of(id).map_or(1.0, |p| p.world_span).clamp(SMALLEST, LARGEST);
        self.node_distances(&Node::Terrain, &world, size, 4.0 * cell_w).iter().any(|&d| d <= cell_w)
    }

    /// How a part of the element `id` no longer joined to what holds it up comes away: along a
    /// separating plane if one parts it from the rest, else by complementary element-local carve
    /// boxes (`support::mask_boxes`). `None` if it holds together.
    fn parting(&mut self, id: u32) -> Option<Parting> {
        let _span = tracing::info_span!("parting").entered();
        if !matches!(self.footing(id), Some(Footing::Rooted | Footing::Loose)) {
            return None;
        }
        let tf = self.find(id)?.transform();
        let cells = self.cells_of(id)?;
        let (labels, n) = cells.grid.parts();
        if n < 2 {
            return None;
        }
        let mut sizes = vec![0usize; n as usize + 1];
        for &i in &cells.solid {
            sizes[labels[i] as usize] += 1;
        }
        let mut held = vec![false; n as usize + 1];
        for (&i, a) in cells.solid.iter().zip(self.anchored(id, &cells, tf)) {
            held[labels[i] as usize] |= a;
        }
        // What is held up stays (each part of it held, as a root the cells see apart from its trunk);
        // held by nothing, the largest part stays and the rest come away.
        let any_held = held.iter().any(|&h| h);
        let kept = (1..=n).max_by_key(|&l| (sizes[l as usize], std::cmp::Reverse(l)))?;
        let stays = |l: u32| if any_held { held[l as usize] } else { l == kept };
        // Crumbs (a few cells) stay with it.
        let dust = (cells.solid.len() / 100).max(3);
        let up = tf.world_to_local_dir([0.0, 1.0, 0.0]);
        let loose: Vec<u32> = (1..=n).filter(|&l| !stays(l) && sizes[l as usize] >= dust).collect();
        if let Some(plane) = loose.iter().find_map(|&l| support::separating_plane(&cells.grid, &labels, l, up)) {
            return Some(Parting::Plane(plane));
        }
        // No single plane separates them (interlocking or ring-cut): fall back to a box-carve mask.
        let stamp = cells.grid.cell.max(1e-4);
        loose.into_iter().find_map(|l| {
            let for_rest = support::mask_boxes(&cells.grid, &labels, |x| x == l, stamp)?;
            let for_part = support::mask_boxes(&cells.grid, &labels, |x| x != l, stamp)?;
            Some(Parting::Mask { for_rest, for_part })
        })
    }

    /// A plane, in the element `id`'s frame, level in the world, along which it breaks because the
    /// cross-section a carve left there cannot hold what is above it (the part above on its kept
    /// side); and the way the part above tips (x, z), over which edge of the section.
    fn breaking(&mut self, id: u32) -> Option<(ClipPlane, [f32; 2], [f32; 3])> {
        let _span = tracing::info_span!("breaking").entered();
        if !matches!(self.footing(id), Some(Footing::Rooted | Footing::Loose)) {
            return None;
        }
        let pl = self.find(id)?.clone();
        if !pl.is_promoted() {
            return None;
        }
        let tf = pl.transform();
        let mut reaches: Vec<(f32, f32)> = pl
            .local_edits
            .iter()
            .filter(|e| e.op == Op::Carve)
            .map(|e| {
                let c = tf.local_to_world_point(e.center);
                let r = (e.shape.radius() + e.blend) * tf.scale;
                (c[1] - r, c[1] + r)
            })
            .collect();
        let (blo, bhi) = self.world_box(id)?;
        for e in &self.edits.edits()[pl.sdf_after_edits().min(self.edits.len())..] {
            let (lo, hi) = e.influence_bounds();
            if e.op == Op::Carve && (0..3).all(|a| hi[a] >= blo[a] && lo[a] <= bhi[a]) {
                reaches.push((lo[1], hi[1]));
            }
        }
        if reaches.is_empty() {
            return None;
        }
        let cells = self.cells_of(id)?;
        let world = cells.world(tf);
        let cell_w = cells.grid.cell * tf.scale;
        let anchored = self.anchored(id, &cells, tf);
        // Held from above or beside (hanging), it is not asked what it holds up.
        let top_anchor = world.iter().zip(&anchored).filter(|(_, a)| **a).map(|(p, _)| p[1]).fold(f32::NEG_INFINITY, f32::max);
        if !top_anchor.is_finite() {
            return None;
        }
        let (ylo, yhi) = world.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p[1]), hi.max(p[1])));
        let mut heights = Vec::new();
        for (lo, hi) in reaches {
            let (lo, hi) = (lo.max(ylo), hi.min(yhi));
            if lo >= hi {
                continue;
            }
            for k in 0..HEIGHTS_PER_CARVE {
                heights.push(lo + (hi - lo) * (k as f32 + 0.5) / HEIGHTS_PER_CARVE as f32);
            }
        }
        heights.retain(|&h| h > top_anchor + 0.5 * cell_w);
        heights.sort_by(f32::total_cmp);
        heights.dedup_by(|a, b| (*a - *b).abs() < 0.25 * cell_w);
        if heights.len() > MAX_HEIGHTS {
            let step = heights.len().div_ceil(MAX_HEIGHTS);
            heights = heights.into_iter().step_by(step).collect();
        }
        let solid = self.solid_of(&pl)?;
        let mut weakest: Option<(usize, f32, [f32; 2], [f32; 3])> = None;
        for h in heights {
            let above: Vec<[f32; 3]> = world.iter().copied().filter(|p| p[1] > h).collect();
            if above.len() < 2 || above.len() == world.len() {
                continue;
            }
            let com = support::mean(&above)?;
            let slab: Vec<[f32; 3]> = world.iter().copied().filter(|p| (p[1] - h).abs() <= cell_w).collect();
            let (slo, shi) = bounds(&slab);
            let mut grid_pts = Vec::with_capacity(SECTION_SAMPLES * SECTION_SAMPLES);
            if !slab.is_empty() {
                let (x0, z0) = (slo[0] - cell_w, slo[2] - cell_w);
                let (dx, dz) = ((shi[0] - slo[0] + 2.0 * cell_w) / SECTION_SAMPLES as f32, (shi[2] - slo[2] + 2.0 * cell_w) / SECTION_SAMPLES as f32);
                for j in 0..SECTION_SAMPLES {
                    for i in 0..SECTION_SAMPLES {
                        grid_pts.push([x0 + (i as f32 + 0.5) * dx, h, z0 + (j as f32 + 0.5) * dz]);
                    }
                }
            }
            let section: Vec<[f32; 2]> = grid_pts.iter().zip(solid.distances(&grid_pts)).filter(|(_, d)| *d <= 0.0).map(|(p, _)| [p[0], p[2]]).collect();
            // Cut right through (between the cells), it tips the way its mass leans off what was there.
            let tips = if section.is_empty() {
                let mid = support::mean(&slab).unwrap_or(com);
                let (dx, dz) = (com[0] - mid[0], com[2] - mid[2]);
                let l = (dx * dx + dz * dz).sqrt();
                let u = if l > 1e-6 { [dx / l, dz / l] } else { [1.0, 0.0] };
                Some((u, [com[0], com[2]]))
            } else {
                support::overhang(&section, [com[0], com[2]], SECTION_GRIP).map(|(u, k)| (u, section[k]))
            };
            if let Some((u, edge)) = tips
                && weakest.as_ref().is_none_or(|(n, ..)| section.len() < *n)
            {
                weakest = Some((section.len(), h, u, [edge[0], h, edge[1]]));
            }
        }
        let (_, h, u, edge) = weakest?;
        let n = tf.world_to_local_dir([0.0, 1.0, 0.0]);
        let p = tf.world_to_local_point([tf.pos[0], h, tf.pos[2]]);
        Some((ClipPlane::new(n, n[0] * p[0] + n[1] * p[1] + n[2] * p[2]), u, edge))
    }

    /// Breaks the element `id` according to `parting` (a plane or complementary carve masks) into
    /// the rest and the part, as a cut does: both `derived_from` it and owned by its owner; the
    /// rest keeps what it hung from; what rested on, hung from or filled it is found again.
    fn split(&mut self, id: u32, parting: Parting, settled: &mut Settled) -> Option<(u32, u32)> {
        let _span = tracing::info_span!("split").entered();
        let original = self.find(id)?.clone();
        let (rest_rel, part_rel, mut rest, mut part, masked) = match parting {
            Parting::Plane(plane) => {
                if original.clips.len() >= MAX_CLIPS {
                    return None;
                }
                let scene = self.runtime.local_scene_of(id)?.clone();
                if clip_scene(&scene, &[plane]).instances.is_empty() || clip_scene(&scene, &[plane.flipped()]).instances.is_empty() {
                    return None;
                }
                let mut rest = original.clone();
                rest.clips.push(plane.flipped());
                let mut part = original.clone();
                part.clips.push(plane);
                (Data::Cut(plane.flipped()), Data::Cut(plane), rest, part, false)
            }
            Parting::Mask { for_rest, for_part } => {
                if for_rest.is_empty() || for_part.is_empty() {
                    return None;
                }
                self.catalog.mesh_sdf_of(&original).ok().flatten()?;
                let promoted_at = original.promoted.unwrap_or(self.edits.len());
                let mut rest = original.clone();
                rest.promoted = Some(promoted_at);
                rest.local_edits.extend(for_rest);
                let mut part = original.clone();
                part.promoted = Some(promoted_at);
                part.local_edits.extend(for_part);
                (Data::Split, Data::Split, rest, part, true)
            }
        };
        let owner = self.owner_of(id).map(str::to_string);
        let uprooted = self.graph.contains(&Relation::uprooted(id));
        let node = Node::Element(id);
        let hung: Vec<Relation> = self.graph.relations().filter(|r| r.from == node && matches!(r.kind, Kind::AttachedTo | Kind::Fills)).collect();
        let deps: Vec<Relation> = self.graph.relations().filter(|r| r.to == node && Graph::rests_on_host(r.kind)).collect();
        let rest_id = self.alloc_id();
        let part_id = self.alloc_id();
        rest.id = rest_id;
        part.id = part_id;
        self.unrelate_leaving(id);
        // While the pieces' carvings are made, each shows the original's split along the plane.
        if let (Data::Cut(r), Data::Cut(p)) = (rest_rel, part_rel) {
            self.runtime.show_meanwhile(id, rest_id, r);
            self.runtime.show_meanwhile(id, part_id, p);
        }
        self.remove_placed(id);
        if self.spawn_placed(rest.clone(), None).and_then(|_| self.spawn_placed(part.clone(), None)).is_err() {
            self.remove_placed(rest_id);
            self.remove_placed(part_id);
            let _ = self.spawn_placed(original, None);
            return None;
        }
        for (piece, data) in [(rest_id, rest_rel), (part_id, part_rel)] {
            let rel = match data {
                Data::Cut(clip) => Relation::derived_from(piece, id, clip),
                _ => Relation::derived_from_split(piece, id),
            };
            self.relate(rel);
            if let Some(owner) = &owner {
                self.relate(Relation::owned_by(piece, owner));
            }
            // Pieces of what was uprooted have no roots either.
            if uprooted {
                self.relate(Relation::uprooted(piece));
            }
        }
        for r in hung {
            self.relate(Relation { from: Node::Element(rest_id), ..r });
        }
        // A masked split keeps the parent's mesh bounds (so `phys.rooted` may still be true): check
        // right away whether each piece's own cells still touch terrain, so detached parts fall in
        // this round.
        if masked {
            for piece in [rest_id, part_id] {
                if self.footing(piece) == Some(Footing::Rooted) && !self.roots_hold(piece) {
                    self.relate(Relation::uprooted(piece));
                }
            }
        }
        self.settle_relations(rest_id);
        self.settle_relations(part_id);
        for dep in deps {
            let Some(d) = dep.from.clone().element() else { continue };
            match (dep.kind, dep.data) {
                (Kind::Fills, Data::Opening(o)) => {
                    let host = self.host_at(o).filter(|h| matches!(h, Node::Element(x) if *x == rest_id || *x == part_id)).unwrap_or(Node::Element(rest_id));
                    self.relate(Relation::fills(d, host.clone(), o));
                    self.relate(Relation::attached_to(d, host));
                }
                (Kind::AttachedTo, _) => {
                    if !self.graph.relations().any(|r| r.kind == Kind::Fills && r.from == dep.from) {
                        self.relate(Relation::attached_to(d, Node::Element(rest_id)));
                    }
                }
                _ => {
                    self.pending.mend.insert(d);
                    self.pending.unsteady.insert(d);
                }
            }
        }
        self.history.push(WorldAction::CutElement { original, lower: rest, upper: part });
        settled.added.extend([rest_id, part_id]);
        settled.removed.push(id);
        Some((rest_id, part_id))
    }

    /// The element `pl` with the terrain's carves into it since it was promoted made its own (in its
    /// frame, so they go where it goes), entering the SDF after every edit there is now.
    fn with_carves_taken(&self, pl: &PlacedElement) -> PlacedElement {
        let mut out = pl.clone();
        if !pl.is_promoted() {
            return out;
        }
        let Some((blo, bhi)) = self.world_box(pl.id) else { return out };
        let tf = pl.transform();
        for e in &self.edits.edits()[pl.sdf_after_edits().min(self.edits.len())..] {
            let (lo, hi) = e.influence_bounds();
            if e.op == Op::Carve && (0..3).all(|a| hi[a] >= blo[a] && lo[a] <= bhi[a]) {
                out.local_edits.push(world_edit_to_local(e, tf));
            }
        }
        out.promoted = Some(self.edits.len());
        out
    }

    /// Puts the element back as `pl` says (where it is, its carving and cuts), its layer and SDF made
    /// again only if more than where it is changed; nothing settles on water.
    pub(super) fn restore_placed(&mut self, pl: PlacedElement) {
        let Some(idx) = self.placed.iter().position(|e| e.id == pl.id) else { return };
        let cur = self.placed[idx].clone();
        if cur.local_edits == pl.local_edits
            && cur.promoted == pl.promoted
            && cur.clips == pl.clips
            && cur.scale == pl.scale
            && cur.params == pl.params
            && cur.module == pl.module
            && cur.seed == pl.seed
        {
            if cur.transform() != pl.transform() {
                self.relocate_placed(pl.id, pl.transform(), None);
            }
            self.placed[idx] = pl;
            return;
        }
        let old = self.sdf_bounds_of(&cur).map(|b| (cur.transform(), b));
        self.runtime.remove(pl.id, &mut self.scene);
        self.changes.layers.push(LayerChange::Despawn { owner: pl.id });
        self.placed.remove(idx);
        if self.spawn_layer_for(pl.clone()).is_err() {
            let _ = self.spawn_layer_for(cur);
        }
        let new = self.find(pl.id).and_then(|p| self.sdf_bounds_of(p).map(|b| (p.transform(), b)));
        self.dirty = true;
        self.commit(Invalidation::Some { edits: Vec::new(), bounds: old.into_iter().chain(new).collect(), water: None });
    }

    /// Signed distance from each point in `pts` to the nearest obstacle other than `id` (and not
    /// carried), combining terrain/SDF fields with cached exact `MeshSdf`s for scene elements.
    fn signed_obstacle_distances(&mut self, id: u32, pts: &[[f32; 3]], size: f32, far: f32) -> Vec<f32> {
        let _span = tracing::info_span!("signed_obstacle_distances").entered();
        let (mut best, _) = self.obstacle_distances(id, pts, size, far);
        if pts.is_empty() {
            return best;
        }
        let (lo, hi) = bounds(pts);
        let qlo = lo.map(|v| v - far);
        let qhi = hi.map(|v| v + far);
        let carried = self.held.map(|h| h.id);
        for other in self.scene.solid_placed_overlapping(qlo, qhi) {
            if other == id || Some(other) == carried {
                continue;
            }
            let Some(pl) = self.find(other).cloned() else { continue };
            let Ok(Some(msdf)) = self.catalog.mesh_sdf_of(&pl) else { continue };
            let tf = pl.transform();
            let inv_scale = 1.0 / tf.scale.max(1e-4);
            for (i, &p) in pts.iter().enumerate() {
                let lp = tf.world_to_local_point(p);
                let limit = if best[i] > 0.0 { best[i] * inv_scale } else { f32::INFINITY };
                if let Some(a) = msdf.sample(lp, limit) {
                    best[i] = best[i].min(a.distance * tf.scale);
                }
            }
        }
        best
    }

    /// When a loose body has nothing it can stand on (`stand.is_empty()`) yet cannot drop straight
    /// down (`drop_distance == 0`) because a point is snagged on a steep slanted face (e.g. a
    /// fallen trunk's cut lip overhanging a severed stump's rim), slides the body the minimum
    /// distance down and away along the slope's tangent until it clears the snag without entering
    /// any obstacle. Returns the world-space displacement, or `None` if it cannot move safely.
    fn slide_off_snag(&mut self, id: u32, pts: &[[f32; 3]], radii: &[f32], span: f32, size: f32) -> Option<[f32; 3]> {
        let _span = tracing::info_span!("slide_off_snag").entered();
        let stop = (STOP * span).max(1e-4);
        let far = 4.0 * span + 8.0;
        let down = vec![[0.0, -1.0, 0.0]; pts.len()];
        let (field, meshes) = self.obstacle_distances(id, pts, size, far);
        let met = self.meets(id, pts, radii, &down, &field, &meshes, stop, size);
        let snags: Vec<[f32; 3]> = met
            .iter()
            .enumerate()
            .filter(|&(_, &m)| m)
            .map(|(i, _)| [pts[i][0], pts[i][1] - radii[i] - 0.5 * stop, pts[i][2]])
            .collect();
        if snags.is_empty() {
            return None;
        }
        let eps = (0.5 * stop).max(2e-3);
        let mut probes = Vec::with_capacity(snags.len() * 6);
        for p in &snags {
            probes.push([p[0] + eps, p[1], p[2]]);
            probes.push([p[0] - eps, p[1], p[2]]);
            probes.push([p[0], p[1] + eps, p[2]]);
            probes.push([p[0], p[1] - eps, p[2]]);
            probes.push([p[0], p[1], p[2] + eps]);
            probes.push([p[0], p[1], p[2] - eps]);
        }
        let d = self.signed_obstacle_distances(id, &probes, size, 4.0 * eps + stop);
        let mut n = [0.0f32; 3];
        for k in 0..snags.len() {
            let b = k * 6;
            n[0] += d[b] - d[b + 1];
            n[1] += d[b + 2] - d[b + 3];
            n[2] += d[b + 4] - d[b + 5];
        }
        let nl = len(n);
        if nl <= 1e-6 {
            return None;
        }
        let n = n.map(|c| c / nl);
        // Only slide off a steep side/slanted face, not a flat horizontal floor.
        if n[0] * n[0] + n[2] * n[2] <= 1e-4 {
            return None;
        }
        // Downward tangent to `n` plus a small outward clearance so it separates cleanly from the snag.
        let raw = [n[1] * n[0] + 0.2 * n[0], n[1] * n[1] - 1.0 + 0.2 * n[1], n[1] * n[2] + 0.2 * n[2]];
        let rl = len(raw);
        if rl <= 1e-6 {
            return None;
        }
        let dir = raw.map(|c| c / rl);
        let step = (2.0 * stop).max(5e-3);
        let max_dist = (0.35 * span).max(4.0 * step);
        let steps = ((max_dist / step).ceil() as usize).clamp(1, 32);
        let ways_dir = vec![dir; pts.len()];
        let mut moved = 0.0f32;
        for _ in 0..steps {
            let cur: Vec<[f32; 3]> = pts.iter().map(|p| [0, 1, 2].map(|a| p[a] + dir[a] * moved)).collect();
            let (cf, cm) = self.obstacle_distances(id, &cur, size, far);
            if self.meets(id, &cur, radii, &ways_dir, &cf, &cm, stop, size).iter().any(|&m| m) {
                break;
            }
            moved += step;
            let next: Vec<[f32; 3]> = pts.iter().map(|p| [0, 1, 2].map(|a| p[a] + dir[a] * moved)).collect();
            let (nf, nm) = self.obstacle_distances(id, &next, size, far);
            if !self.meets(id, &next, radii, &down, &nf, &nm, stop, size).iter().any(|&m| m) {
                break;
            }
        }
        (moved > 1e-5).then(|| dir.map(|c| c * moved))
    }

    /// How far the points `pts` of the element `id` can drop straight down before one meets
    /// something from above; nought if nothing is beneath them.
    fn drop_distance(&mut self, id: u32, pts: &[[f32; 3]], radii: &[f32], span: f32, size: f32) -> f32 {
        let _span = tracing::info_span!("drop_distance").entered();
        let stop = (STOP * span).max(1e-4);
        let far = 4.0 * span + 8.0;
        let mut t = vec![0.0f32; pts.len()];
        let mut active: Vec<usize> = (0..pts.len()).collect();
        let (mut best, mut hit) = (MAX_DROP, false);
        for _ in 0..128 {
            if active.is_empty() {
                break;
            }
            let q: Vec<[f32; 3]> = active.iter().map(|&i| [pts[i][0], pts[i][1] - t[i], pts[i][2]]).collect();
            let qr: Vec<f32> = active.iter().map(|&i| radii[i]).collect();
            let (field, meshes) = self.obstacle_distances(id, &q, size, far);
            let met = self.meets(id, &q, &qr, &vec![[0.0, -1.0, 0.0]; q.len()], &field, &meshes, stop, size);
            let mut next = Vec::with_capacity(active.len());
            for (k, &i) in active.iter().enumerate() {
                if met[k] {
                    best = best.min(t[i]);
                    hit = true;
                    continue;
                }
                let d = field[k].min(meshes[k]) - radii[i];
                t[i] += (0.9 * d).max(0.5 * stop);
                if t[i] < best {
                    next.push(i);
                }
            }
            active = next;
        }
        if hit { best } else { 0.0 }
    }

    /// The element `id` (its points `local`, in its frame) at `tf`, tipped about `pivot` the way `u`
    /// (x, z) until a point going down meets something, or a quarter turn: where it ends, and how far
    /// it turned.
    #[allow(clippy::too_many_arguments)]
    fn tip(&mut self, id: u32, local: &[[f32; 3]], radii: &[f32], tf: PlacementTransform, pivot: [f32; 3], u: [f32; 2], span: f32, size: f32) -> (PlacementTransform, f32) {
        let _span = tracing::info_span!("tip").entered();
        // Turning about (u.z, 0, -u.x) carries up towards u.
        let k = [u[1], 0.0, -u[0]];
        let stop = (STOP * span).max(1e-4);
        let far = 4.0 * span + 8.0;
        let near = 0.05 * span;
        let at = |angle: f32| {
            let (s, c) = (math::sin(0.5 * angle), math::cos(0.5 * angle));
            let q = [k[0] * s, k[1] * s, k[2] * s, c];
            let r = quat_rotate(q, sub(tf.pos, pivot));
            PlacementTransform::with_rot([pivot[0] + r[0], pivot[1] + r[1], pivot[2] + r[2]], quat_normalize(quat_mul(q, tf.rot)), tf.scale)
        };
        let (mut angle, mut prev) = (0.0f32, 0.0f32);
        for _ in 0..256 {
            let now = at(angle);
            let pts: Vec<[f32; 3]> = local.iter().map(|&p| now.local_to_world_point(p)).collect();
            let (field, meshes) = self.obstacle_distances(id, &pts, size, far);
            let (mut step, mut met, mut pressed) = (MAX_TIP_STEP, false, false);
            // Only what goes down as it turns can meet anything; what is on the line it turns
            // about stays where it is.
            let (mut ask, mut ways, mut arms) = (Vec::new(), Vec::new(), Vec::new());
            for (i, p) in pts.iter().enumerate() {
                let r = sub(*p, pivot);
                let along = r[0] * k[0] + r[1] * k[1] + r[2] * k[2];
                let rl = len([r[0] - along * k[0], r[1] - along * k[1], r[2] - along * k[2]]);
                let v = cross(k, r);
                let vl = len(v);
                if rl < near || v[1] > -0.2 * vl {
                    continue;
                }
                ask.push(i);
                ways.push(v.map(|c| c / vl.max(1e-9)));
                arms.push(rl);
            }
            let sel = |v: &[f32]| ask.iter().map(|&i| v[i]).collect::<Vec<f32>>();
            let (apts, aradii, afield, ameshes) = (ask.iter().map(|&i| pts[i]).collect::<Vec<_>>(), sel(radii), sel(&field), sel(&meshes));
            let hits = self.meets(id, &apts, &aradii, &ways, &afield, &ameshes, stop, size);
            for k in 0..ask.len() {
                let d = afield[k].min(ameshes[k]) - aradii[k];
                if hits[k] {
                    met = true;
                    pressed |= d < -stop;
                } else {
                    step = step.min(0.9 * d.max(0.0) / arms[k]);
                }
            }
            if met {
                if pressed {
                    angle = prev;
                }
                break;
            }
            if angle >= MAX_TIP {
                break;
            }
            prev = angle;
            angle = (angle + step.max(MIN_TIP_STEP)).min(MAX_TIP);
        }
        (at(angle), angle)
    }

    /// Lets the element `id` fall, if it is loose, to where it comes to rest: down to what is beneath
    /// it, then over the edge its centre of mass overhangs (first the way `push` says, if given),
    /// until it stands; then onto water. What rested on it is shaken; it rests on what it now
    /// touches. Whether it moved.
    fn fall(&mut self, id: u32, push: Option<([f32; 2], Option<[f32; 3]>)>, shown: &mut Vec<Shown>) -> bool {
        let _span = tracing::info_span!("fall").entered();
        if self.footing(id) != Some(Footing::Loose) {
            return false;
        }
        let Some(before) = self.find(id).cloned() else { return false };
        let (Some(phys), Some(scene)) = (self.physics_of(id), self.runtime.local_scene_of(id).cloned()) else { return false };
        // A piece that moves takes the terrain's carves into it along.
        let taken = self.with_carves_taken(&before);
        if taken != before {
            self.restore_placed(taken);
        }
        let start = before.transform();
        let span = phys.world_span.max(1e-4);
        let size = span.clamp(SMALLEST, LARGEST);
        let cells = self.cells_of(id);
        // Its body as balls on its surface (its carving, too); else its meshes' points.
        let (local, radii) = match &cells {
            Some(c) if !c.solid.is_empty() => c.body(),
            _ => {
                let pts = solid_points(&scene, PlacementTransform::with_rot([0.0; 3], [0.0, 0.0, 0.0, 1.0], 1.0));
                let n = pts.len();
                (pts, vec![0.0; n])
            }
        };
        if local.is_empty() {
            return false;
        }
        let com_local = cells.as_ref().and_then(|c| support::mean(&c.centres)).unwrap_or(phys.local_center);
        let (mut tf, mut pivot, mut push) = (start, None, push);
        for _ in 0..8 {
            // Broken at a known edge, it first tips over that edge.
            if let Some((u, Some(edge))) = push {
                push = None;
                let (next, angle) = self.tip(id, &local, &radii, tf, edge, u, span, size);
                if angle >= 0.5 * MIN_TIP_STEP {
                    pivot.get_or_insert(edge);
                    tf = next;
                }
                continue;
            }
            let pts: Vec<[f32; 3]> = local.iter().map(|&p| tf.local_to_world_point(p)).collect();
            let (_, touch) = self.touching(id, &pts, Some(&radii), STAND_TOLERANCE, STAND_TOLERANCE * span);
            let stand: Vec<[f32; 3]> = pts.iter().zip(&touch).filter(|(_, t)| **t).map(|(p, _)| *p).collect();
            if stand.is_empty() {
                let dy = self.drop_distance(id, &pts, &radii, span, size);
                if dy <= 1e-5 {
                    if let Some(delta) = self.slide_off_snag(id, &pts, &radii, span, size) {
                        tf.pos = [tf.pos[0] + delta[0], tf.pos[1] + delta[1], tf.pos[2] + delta[2]];
                        continue;
                    }
                    break;
                }
                tf.pos[1] -= dy;
                continue;
            }
            let com = tf.local_to_world_point(com_local);
            let xz: Vec<[f32; 2]> = stand.iter().map(|p| [p[0], p[2]]).collect();
            let tip = match push.take().map(|(u, _)| u) {
                Some(u) => {
                    let far = (0..xz.len()).max_by(|&a, &b| (xz[a][0] * u[0] + xz[a][1] * u[1]).total_cmp(&(xz[b][0] * u[0] + xz[b][1] * u[1]))).unwrap_or(0);
                    Some((u, far))
                }
                None => support::overhang(&xz, [com[0], com[2]], 0.0),
            };
            let Some((u, k)) = tip else { break };
            let (next, angle) = self.tip(id, &local, &radii, tf, stand[k], u, span, size);
            if angle < 0.5 * MIN_TIP_STEP {
                break;
            }
            pivot.get_or_insert(stand[k]);
            tf = next;
        }
        if tf == start {
            if self.find(id) != Some(&before) {
                self.restore_placed(before);
            }
            return false;
        }
        self.shaken(id);
        let mut after = self.find(id).cloned().unwrap_or(before.clone());
        after.set_transform(tf);
        self.restore_placed(after);
        let _ = self.settle_buoyancy_for(id, 0.0);
        self.unrelate_moved(id);
        self.settle_relations(id);
        let after = self.find(id).cloned().unwrap_or(before.clone());
        shown.push(Shown { id, from: start, to: after.transform(), pivot });
        self.history.push(WorldAction::Fell { before, after });
        true
    }

    /// What the action shook settles: mended, come apart, fallen (see the module).
    pub(super) fn settle_support(&mut self) -> Settled {
        let _span = tracing::info_span!("support").entered();
        let mut settled = Settled::default();
        let mut shown = Vec::new();
        let (mut falls, mut splits) = (0usize, 0usize);
        for _ in 0..MAX_ROUNDS {
            let pending = std::mem::take(&mut self.pending);
            let mut changed = false;
            let mut unsteady = pending.unsteady.clone();
            // Rooted by its shape, but its roots dug round or carved away: it is loose now.
            for &id in pending.roots.iter().chain(&pending.reshaped) {
                if self.footing(id) == Some(Footing::Rooted) && !self.roots_hold(id) {
                    self.relate(Relation::uprooted(id));
                    unsteady.insert(id);
                    self.settle_relations(id);
                    changed = true;
                }
            }
            for id in &pending.mend {
                if self.find(*id).is_some() && self.footing(*id) == Some(Footing::Loose) {
                    self.settle_relations(*id);
                }
            }
            let mut pushed = pending.pushed.clone();
            for &id in &pending.reshaped {
                if splits >= MAX_SPLITS || self.find(id).is_none() {
                    continue;
                }
                let (parting, broke) = match self.parting(id) {
                    Some(p) => (Some(p), None),
                    None => match self.breaking(id) {
                        Some((p, u, edge)) => (Some(Parting::Plane(p)), Some((u, Some(edge)))),
                        None => (None, None),
                    },
                };
                let Some(parting) = parting else { continue };
                if let Some((rest, part)) = self.split(id, parting, &mut settled) {
                    splits += 1;
                    changed = true;
                    self.pending.roots.extend([rest, part]);
                    if broke.is_none() {
                        // Come apart, the rest may still be in more parts.
                        self.pending.reshaped.insert(rest);
                    }
                    unsteady.extend([rest, part]);
                    unsteady.remove(&id);
                    if let Some(p) = broke.or_else(|| pushed.remove(&id)) {
                        pushed.insert(part, p);
                    }
                }
            }
            let grounded = self.grounded();
            let mut movers: Vec<u32> = self.placed.iter().map(|p| p.id).filter(|id| (!grounded.contains(id) && self.footing(*id) == Some(Footing::Loose)) || unsteady.contains(id) || pushed.contains_key(id)).collect();
            let low = |w: &World, id: u32| w.world_box(id).map_or(0.0, |(lo, _)| lo[1]);
            movers.sort_by(|&a, &b| low(self, a).total_cmp(&low(self, b)).then(a.cmp(&b)));
            for id in movers {
                if falls >= self.fall_limit {
                    break;
                }
                // Held up by nothing that reaches the ground (resting only on each other, say), it
                // rests on what it touches now, which may be the ground after all.
                if !grounded.contains(&id) && self.footing(id) == Some(Footing::Loose) {
                    self.settle_relations(id);
                }
                if self.fall(id, pushed.get(&id).copied(), &mut shown) {
                    falls += 1;
                    changed = true;
                    if !settled.fell.contains(&id) {
                        settled.fell.push(id);
                    }
                }
            }
            let more = !self.pending.reshaped.is_empty() || !self.pending.mend.is_empty() || !self.pending.unsteady.is_empty() || !self.pending.roots.is_empty();
            if !changed && !more {
                break;
            }
            if falls >= self.fall_limit {
                break;
            }
        }
        self.pending = Pending::default();
        // The cells and fields asked are kept by what they were made of, but not past the action.
        self.cells.clear();
        self.alone.clear();
        self.show_falls(shown);
        settled
    }

    /// Shows each fall from where it began (its first) to where it ended (its last); the element is
    /// already there.
    fn show_falls(&mut self, shown: Vec<Shown>) {
        let mut merged: Vec<Shown> = Vec::new();
        for s in shown {
            match merged.iter_mut().find(|m| m.id == s.id) {
                Some(m) => m.to = s.to,
                None => merged.push(s),
            }
        }
        let mut stale = Vec::new();
        for s in merged {
            // A piece split by a component mask has its geometry in the SDF (not its unmasked
            // parent mesh), so leave it committed at its resting place rather than flashing the
            // parent's uncarved mesh during easing.
            if self.graph.parent_of(s.id).is_some_and(|(_, d)| d == Data::Split) {
                continue;
            }
            let drop = (s.from.pos[1] - s.to.pos[1]).abs();
            let duration = 0.25 + (2.0 * drop / 9.81).sqrt() + if s.pivot.is_some() { 0.35 } else { 0.0 };
            self.falling.push(FallingElement {
                id: s.id,
                from_pos: s.from.pos,
                from_rot: s.from.rot,
                to_pos: s.to.pos,
                to_rot: s.to.rot,
                scale: s.to.scale,
                elapsed: 0.0,
                duration,
                pivot: s.pivot,
            });
            // Shown from where it began, as its meshes (a carved piece too: its SDF is back where
            // it lands).
            if let Some(bounds) = self.find(s.id).and_then(|pl| self.sdf_bounds_of(pl)) {
                stale.push((s.to, bounds));
            }
            self.show_solid_at(s.id, s.from);
        }
        if !stale.is_empty() {
            self.commit(Invalidation::Some { edits: Vec::new(), bounds: stale, water: None });
        }
    }
}

/// How far `p` is from the box `lo..hi` (0 within it).
fn box_distance(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let d = [0, 1, 2].map(|a| (lo[a] - p[a]).max(p[a] - hi[a]).max(0.0));
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}
