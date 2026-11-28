//! How a sculpted `Scene` element is drawn (AGENTS.md §1.2 and §2): from its own triangles, made
//! again only where it was carved.
//!
//! A `Scene` element is promoted on its first sculpt: from then on its solid is a `MeshSdf` with
//! its edits applied, and physics, `reach` and support read that. It used to be drawn from that SDF
//! too, by the terrain's chunks, so the first carve changed how all of it looked: the terrain's
//! voxels (a twentieth of the viewer) round every edge, smooth its normals, lose its tint and
//! change with distance; and the chunks stay where the SDF is, so a carved piece did not fall or
//! travel with its scene layer. Now the element draws itself (`carve`):
//!
//! - **Kept as made.** Every solid triangle further from every edit than `KEEP` cells is kept just
//!   as the element made it (tint, normals, sharp cut caps). A triangle near an edit is divided
//!   (down to a cell) and cut where it comes within `KEEP` cells of the edit's reach.
//! - **Made again where carved.** About the edits, the SDF's surface is dual-contoured on an
//!   element-local grid fine for the edits (a sixth of the smallest, a power of two in metres):
//!   crossings found on the SDF, normals from its gradient, each cell's vertex where those planes
//!   meet, so a carve's rim and a notch's floor stay sharp. What it remakes reaches `PATCH_REACH`
//!   cells from the edits, past where the kept triangles end, so no crack shows between them.
//! - **Coloured as it was.** Where the remade surface is still the original, it takes the nearest
//!   original triangle's normal, material and tint; where it is newly cut, the fresh-cut tint a chop
//!   gives its caps (`cut::cut_face_tint`); where it was added, the added material's own colour.
//! - **Made once.** The grid is made in bricks of `BRICK` cells, each kept by the edits that reach
//!   it, so another stroke makes again only the bricks it reaches; and the whole carved mesh is kept
//!   by all its edits, so moving or dropping a carved piece makes nothing again.
//!
//! The SDF is unchanged: what the carved mesh draws is the surface physics stands on.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use crate::cut::{ClipPlane, cut_face_tint, lerp_vertex};
use crate::edit::{Edit, Shape};
use crate::mesh_sdf::{MeshSdf, point_triangle_distance};
use crate::{Answer, Instance, Material, MeshData, Scene, Vertex};

/// Original triangles are kept where every edit's reach (`Edit::distance - blend`) is at least
/// this many cells away.
pub const KEEP: f32 = 0.75;
/// The remade surface is kept where some edit's reach is within this many cells: past `KEEP`, so
/// the two overlap by a narrow band instead of leaving a crack.
pub const PATCH_REACH: f32 = 1.1;
/// Cells whose middle is within this many cells of an edit's reach get a vertex.
const DOMAIN: f32 = 3.0;
/// Cells along a brick's side: what is kept and made again at once.
const BRICK: i32 = 16;
/// Cells along the side of the smallest block asked whether it holds surface.
const LEAF: i32 = 4;
/// A cut plane is asked of a brick it comes within this many cells of (`brick`).
const CLIP_REACH: f32 = 3.0;
/// Cells across an edit's smallest extent.
const CELLS_PER_FEATURE: f32 = 6.0;
/// Cells to the height of who made an edit, at most: what they see of it (the terrain gives them
/// twenty, `chunk::VOXELS_A_HEIGHT`).
const CELLS_PER_STAMP: f32 = 40.0;
/// The grid is never finer than the element's longest side over this, nor coarser than it over
/// `COARSEST_ALONG`.
const FINEST_ALONG: f32 = 1024.0;
const COARSEST_ALONG: f32 = 24.0;
/// A coarser level of detail (`Carver::carve_at`) is no coarser than this many cells along.
const COARSEST_ALONG_FAR: f32 = 6.0;
/// The coarsest level of detail a carved element is drawn at.
pub const MAX_LEVEL: u32 = 3;
/// The most triangles a carved element's solid is drawn with: past it, a coarser level.
pub const MAX_CARVED_TRIANGLES: usize = 60_000;
/// How many times a triangle near an edit is divided (into four) before it is cut.
const MAX_DEPTH: u32 = 10;
/// A remade corner turns to its face's normal where its own differs from it more than this (cos).
const SHARP: f32 = 0.7;
/// Regularisation of each cell's vertex toward the middle of its crossings.
const LAMBDA: f32 = 0.05;
const MAX_CACHED_SOLIDS: usize = 64;
const MAX_CACHED_BRICKS: usize = 8192;
/// At most this many remade triangles for one element (the rest of a too-large carve is not drawn).
const MAX_PATCH_TRIS: usize = 600_000;
/// Changes whenever what is made changes, so nothing kept from before is reused.
const VERSION: u64 = 2;

const CUBE: [[i32; 3]; 8] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]];
const CUBE_EDGES: [(usize, usize); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];

/// What a promoted element is carved by, in its own frame: its solid's SDF and every edit that
/// reaches it (its own, and the terrain's carves since it was promoted), in order.
#[derive(Clone, Debug)]
pub struct Carving {
    /// What the SDF was made from (`Carving::source_of`): the same source, the same solid.
    pub source: u64,
    /// The solid its module makes, uncut.
    pub sdf: Arc<MeshSdf>,
    /// What it is cut to (a piece of a chop or a split), applied to `sdf` before `edits`, as its
    /// physics has it. Kept apart from the solid so a piece is drawn from its original's bricks:
    /// only those a plane comes near are made again (`WORLD_PLAN.md`, phase J).
    pub clips: Vec<ClipPlane>,
    pub edits: Vec<Edit>,
}

impl Carving {
    /// The key of the solid an element's `module`, `seed` and `params` make (uncut).
    pub fn source_of(module: &str, seed: u64, params: &[f32]) -> u64 {
        let mut h = DefaultHasher::new();
        module.hash(&mut h);
        seed.hash(&mut h);
        for p in params {
            p.to_bits().hash(&mut h);
        }
        h.finish()
    }

    /// Whether `other` carves the same solid the same way.
    pub fn same_as(&self, other: &Carving) -> bool {
        self.source == other.source && self.clips == other.clips && self.edits == other.edits
    }
}

/// How far `p` is on the side `clip` takes away (negative on the side it keeps).
fn beyond(clip: &ClipPlane, p: [f32; 3]) -> f32 {
    clip.offset - dot3(clip.normal, p)
}

type Tri = [Vertex; 3];
/// Where the surface crosses a grid edge, and its normal there.
type Crossing = ([f32; 3], [f32; 3]);

/// Makes and keeps carved meshes (`carve`).
#[derive(Default)]
pub struct Carver {
    solids: HashMap<u64, Arc<MeshData>>,
    bricks: HashMap<u64, Arc<Vec<Tri>>>,
    /// Bricks made (not found kept), ever: what a carving cost.
    made: usize,
}

impl Carver {
    pub fn new() -> Carver {
        Carver::default()
    }

    /// `scene` (an element's own, in its frame) with its solid instances drawn as `carving` carves
    /// them: one static solid instance of the carved mesh (added after its meshes, which keep their
    /// indices, as `SceneWorld`'s instances of it name them), every other instance and light as it was.
    /// Bricks this carver has made (not found kept), ever.
    pub fn bricks_made(&self) -> usize {
        self.made
    }

    pub fn carve(&mut self, scene: &Scene, carving: &Carving) -> Scene {
        self.carve_at(scene, carving, 0)
    }

    /// `carve` at the detail level `level` (its grid's cell two to the `level` times as wide; 0 is
    /// the finest), or coarser if that would draw more than `MAX_CARVED_TRIANGLES`
    /// (`WORLD_PLAN.md`, phase J: a carved element is drawn in a budget, and coarser far away).
    pub fn carve_at(&mut self, scene: &Scene, carving: &Carving, level: u32) -> Scene {
        let _span = tracing::info_span!("carve", edits = carving.edits.len(), level).entered();
        if !scene.instances.iter().any(|i| i.solid) {
            return scene.clone();
        }
        let mut level = level.min(MAX_LEVEL);
        let mut solid = self.solid_mesh(scene, carving, level);
        while solid.indices.len() / 3 > MAX_CARVED_TRIANGLES && level < MAX_LEVEL {
            level += 1;
            solid = self.solid_mesh(scene, carving, level);
        }
        let mut out = Scene {
            meshes: scene.meshes.clone(),
            instances: scene.instances.iter().filter(|i| !i.solid).copied().collect(),
            lights: scene.lights.clone(),
        };
        if !solid.indices.is_empty() {
            out.meshes.push((*solid).clone());
            out.instances.push(Instance {
                id: 0,
                mesh: (out.meshes.len() - 1) as u32,
                pos: [0.0; 3],
                rot: [0.0, 0.0, 0.0, 1.0],
                scale: [1.0; 3],
                solid: true,
            });
        }
        out
    }

    /// The solid triangles of `scene`, carved.
    fn solid_mesh(&mut self, scene: &Scene, carving: &Carving, level: u32) -> Arc<MeshData> {
        let mut key = DefaultHasher::new();
        (VERSION, carving.source, level).hash(&mut key);
        for c in &carving.clips {
            c.key_bits().hash(&mut key);
        }
        for e in &carving.edits {
            hash_edit(e, &mut key);
        }
        let key = key.finish();
        if let Some(m) = self.solids.get(&key) {
            return m.clone();
        }
        let tris = solid_tris(scene);
        let mesh = if carving.edits.is_empty() {
            to_mesh(&tris)
        } else {
            let h = cell_size(&carving.edits, &carving.sdf, level);
            let reach: Vec<Reach> = carving.edits.iter().map(|&e| Reach::new(e)).collect();
            let mut out = Vec::with_capacity(tris.len() * 3);
            {
                let _span = tracing::info_span!("carve_keep").entered();
                for &t in &tris {
                    keep_outside(t, &reach, h, 0, &mut out);
                }
            }
            let field = Field { sdf: &carving.sdf, clips: &carving.clips, edits: &carving.edits };
            let mut bricks = Vec::new();
            if let Some((lo, size)) = root(&reach, h) {
                let _span = tracing::info_span!("carve_find_bricks").entered();
                collect_bricks(&field, &reach, h, lo, size, &mut bricks);
            }
            let _span = tracing::info_span!("carve_bricks", n = bricks.len()).entered();
            let mut patch = 0usize;
            for b in bricks {
                let made = self.brick(carving, &field, &reach, h, b);
                patch += made.len();
                if patch > MAX_PATCH_TRIS {
                    break;
                }
                out.extend(made.iter().copied());
            }
            to_mesh(&out)
        };
        let mesh = Arc::new(mesh);
        if self.solids.len() >= MAX_CACHED_SOLIDS {
            self.solids.clear();
        }
        self.solids.insert(key, mesh.clone());
        mesh
    }

    /// The remade surface in the brick at cell `lo`, kept by what reaches it.
    fn brick(&mut self, carving: &Carving, field: &Field, reach: &[Reach], h: f32, lo: [i32; 3]) -> Arc<Vec<Tri>> {
        let pad = 8.0 * h;
        let blo = lo.map(|c| (c - 1) as f32 * h - pad);
        let bhi = lo.map(|c| (c + BRICK + 1) as f32 * h + pad);
        let mut key = DefaultHasher::new();
        (VERSION, carving.source, h.to_bits(), lo).hash(&mut key);
        // What the brick is made of, and kept by: the edits that reach it (and its pad), in order.
        // Those beyond change no sign in it, nor any distance near its surface, so the brick is
        // asked of these alone: one stroke beside a much-carved element asks its few, not all.
        let near: Vec<Reach> = reach.iter().filter(|r| (0..3).all(|a| r.hi[a] >= blo[a] && r.lo[a] <= bhi[a])).copied().collect();
        for r in &near {
            hash_edit(&r.edit, &mut key);
        }
        // And the cut planes that come within `CLIP_REACH` cells of its own cells (and the one about
        // them it asks): further, a plane changes no sign the brick contours, nor any crossing on
        // its edges (only values deeper inside than the plane is near, and the colours come from the
        // uncut solid). A brick no plane comes near is its original's, made once for every piece.
        let (clo, chi) = (lo.map(|c| (c - 1) as f32 * h), lo.map(|c| (c + BRICK + 1) as f32 * h));
        let corners: Vec<[f32; 3]> = (0..8).map(|k| [0, 1, 2].map(|a| if (k >> a) & 1 == 0 { clo[a] } else { chi[a] })).collect();
        let clips: Vec<ClipPlane> = field.clips.iter().filter(|c| corners.iter().any(|&p| beyond(c, p) > -CLIP_REACH * h)).copied().collect();
        "clips".hash(&mut key);
        for c in &clips {
            c.key_bits().hash(&mut key);
        }
        let key = key.finish();
        if let Some(b) = self.bricks.get(&key) {
            return b.clone();
        }
        let _span = tracing::info_span!("carve_brick_made", edits = near.len()).entered();
        self.made += 1;
        let edits: Vec<Edit> = near.iter().map(|r| r.edit).collect();
        let field = Field { sdf: field.sdf, clips: &clips, edits: &edits };
        let mut contour = Contour::new(&field, &near, h);
        contour.visit(lo, BRICK);
        let made = Arc::new(contour.finish());
        if self.bricks.len() >= MAX_CACHED_BRICKS {
            self.bricks.clear();
        }
        self.bricks.insert(key, made.clone());
        made
    }
}

/// One edit and how far it reaches.
#[derive(Clone, Copy)]
struct Reach {
    edit: Edit,
    /// From its centre, the furthest it changes anything.
    radius: f32,
    lo: [f32; 3],
    hi: [f32; 3],
}

impl Reach {
    fn new(edit: Edit) -> Reach {
        let bound = match edit.shape {
            Shape::Sphere { radius } => radius,
            Shape::Box { half, .. } => len3(half),
            Shape::Cylinder { radius, half_height, .. } => (radius * radius + half_height * half_height).sqrt(),
        };
        let (lo, hi) = edit.influence_bounds();
        Reach { edit, radius: bound + edit.blend, lo, hi }
    }

    /// How far `p` is outside what the edit changes (negative within it).
    fn g(&self, p: [f32; 3]) -> f32 {
        self.edit.distance(p) - self.edit.blend
    }
}

fn gmin(reach: &[Reach], p: [f32; 3]) -> f32 {
    reach.iter().fold(f32::INFINITY, |m, r| m.min(r.g(p)))
}

/// The carved solid's SDF.
struct Field<'a> {
    sdf: &'a MeshSdf,
    clips: &'a [ClipPlane],
    edits: &'a [Edit],
}

impl Field<'_> {
    /// The cut planes, then the edits, on `a` (the uncut solid at `p`).
    fn apply(&self, p: [f32; 3], mut a: Answer) -> Answer {
        for c in self.clips {
            a.distance = a.distance.max(beyond(c, p));
        }
        for e in self.edits {
            a = e.apply(p, a);
        }
        a
    }

    fn answer(&self, p: [f32; 3]) -> Answer {
        let a = self.sdf.sample(p, f32::INFINITY).unwrap_or(Answer { distance: 1e4, material: Material::Wood });
        self.apply(p, a)
    }

    fn value(&self, p: [f32; 3]) -> f32 {
        self.answer(p).distance
    }

    /// The distance at `p` if the solid is nearer than `limit` (exact), or a value no greater
    /// than it (not exact).
    fn bounded(&self, p: [f32; 3], limit: f32) -> (f32, bool) {
        match self.sdf.sample(p, limit) {
            Some(a) => (self.apply(p, a).distance, true),
            None => (self.apply(p, Answer { distance: limit, material: Material::Wood }).distance, false),
        }
    }

    /// The unit gradient at `p` (out of the solid), by a tetrahedron of samples `eps` away.
    fn gradient(&self, p: [f32; 3], eps: f32) -> [f32; 3] {
        const K: [[f32; 3]; 4] = [[1.0, -1.0, -1.0], [-1.0, -1.0, 1.0], [-1.0, 1.0, -1.0], [1.0, 1.0, 1.0]];
        let mut g = [0.0f32; 3];
        for k in K {
            let f = self.value([p[0] + k[0] * eps, p[1] + k[1] * eps, p[2] + k[2] * eps]);
            for a in 0..3 {
                g[a] += k[a] * f;
            }
        }
        normalize(g).unwrap_or([0.0, 1.0, 0.0])
    }
}

/// Whether the block of `s` cells from cell `lo` can hold no remade surface: too far from every
/// edit, or wholly inside or outside the solid.
fn pruned(field: &Field, reach: &[Reach], h: f32, lo: [i32; 3], s: i32) -> bool {
    let c = lo.map(|i| (i as f32 + s as f32 * 0.5) * h);
    let r = s as f32 * h * 0.866_025_4;
    if gmin(reach, c) > DOMAIN * h + r {
        return true;
    }
    let (d, exact) = field.bounded(c, r + 2.0 * h);
    d > r + h || (exact && d < -(r + h))
}

/// The block of cells (aligned to bricks, a power of two of them along) holding every cell an
/// edit reaches.
fn root(reach: &[Reach], h: f32) -> Option<([i32; 3], i32)> {
    let pad = (DOMAIN + 2.0) * h;
    let mut lo = [i32::MAX; 3];
    let mut hi = [i32::MIN; 3];
    for r in reach {
        for a in 0..3 {
            lo[a] = lo[a].min(((r.lo[a] - pad) / h).floor() as i32);
            hi[a] = hi[a].max(((r.hi[a] + pad) / h).ceil() as i32);
        }
    }
    if reach.is_empty() {
        return None;
    }
    let lo = lo.map(|c| c.div_euclid(BRICK) * BRICK);
    let ext = (0..3).map(|a| hi[a] - lo[a]).max().unwrap_or(0);
    let mut size = BRICK;
    while size < ext && size < (1 << 24) {
        size *= 2;
    }
    Some((lo, size))
}

fn collect_bricks(field: &Field, reach: &[Reach], h: f32, lo: [i32; 3], s: i32, out: &mut Vec<[i32; 3]>) {
    if pruned(field, reach, h, lo, s) {
        return;
    }
    if s <= BRICK {
        out.push(lo);
        return;
    }
    let half = s / 2;
    for k in 0..8 {
        let child = [0, 1, 2].map(|a| lo[a] + ((k >> a) & 1) * half);
        collect_bricks(field, reach, h, child, half, out);
    }
}

/// Dual contouring of one brick (with what it asks of the cells about it).
struct Contour<'a> {
    field: &'a Field<'a>,
    reach: &'a [Reach],
    h: f32,
    corners: HashMap<[i32; 3], f32>,
    crossings: HashMap<([i32; 3], u8), Crossing>,
    cells: HashMap<[i32; 3], Option<u32>>,
    verts: Vec<Vertex>,
    tris: Vec<[u32; 3]>,
}

impl<'a> Contour<'a> {
    fn new(field: &'a Field<'a>, reach: &'a [Reach], h: f32) -> Contour<'a> {
        Contour { field, reach, h, corners: HashMap::new(), crossings: HashMap::new(), cells: HashMap::new(), verts: Vec::new(), tris: Vec::new() }
    }

    fn at(&self, i: [i32; 3]) -> [f32; 3] {
        i.map(|c| c as f32 * self.h)
    }

    fn corner(&mut self, i: [i32; 3]) -> f32 {
        if let Some(&d) = self.corners.get(&i) {
            return d;
        }
        let d = self.field.value(self.at(i));
        self.corners.insert(i, d);
        d
    }

    fn visit(&mut self, lo: [i32; 3], s: i32) {
        if pruned(self.field, self.reach, self.h, lo, s) {
            return;
        }
        if s <= LEAF {
            self.leaf(lo, s);
            return;
        }
        let half = s / 2;
        for k in 0..8 {
            let child = [0, 1, 2].map(|a| lo[a] + ((k >> a) & 1) * half);
            self.visit(child, half);
        }
    }

    /// A quad for every grid edge in the block (from its corners in it) the surface crosses, joining
    /// the vertices of the four cells about it.
    fn leaf(&mut self, lo: [i32; 3], s: i32) {
        for x in 0..s {
            for y in 0..s {
                for z in 0..s {
                    let i = [lo[0] + x, lo[1] + y, lo[2] + z];
                    let di = self.corner(i);
                    for axis in 0..3 {
                        let mut j = i;
                        j[axis] += 1;
                        let dj = self.corner(j);
                        if (di < 0.0) == (dj < 0.0) {
                            continue;
                        }
                        let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                        let mut quad = [0u32; 4];
                        let mut whole = true;
                        for (q, (du, dv)) in [(0, 0), (1, 0), (1, 1), (0, 1)].into_iter().enumerate() {
                            let mut c = i;
                            c[u] += du - 1;
                            c[v] += dv - 1;
                            match self.vertex(c) {
                                Some(k) => quad[q] = k,
                                None => {
                                    whole = false;
                                    break;
                                }
                            }
                        }
                        if !whole {
                            continue;
                        }
                        // Counter-clockwise seen from outside (as `chunk::net` makes them).
                        let [a, b, c, d] = if di < 0.0 { quad } else { [quad[0], quad[3], quad[2], quad[1]] };
                        let [pa, pb, pc, pd] = [a, b, c, d].map(|k| self.verts[k as usize].pos);
                        if dist2(pa, pc) <= dist2(pb, pd) {
                            self.tris.push([a, b, c]);
                            self.tris.push([a, c, d]);
                        } else {
                            self.tris.push([a, b, d]);
                            self.tris.push([b, c, d]);
                        }
                    }
                }
            }
        }
    }

    fn vertex(&mut self, c: [i32; 3]) -> Option<u32> {
        if let Some(&v) = self.cells.get(&c) {
            return v;
        }
        let v = self.make_vertex(c);
        self.cells.insert(c, v);
        v
    }

    /// Where the surface crosses the grid edge from corner `i` along `axis`, and its normal there.
    fn crossing(&mut self, i: [i32; 3], axis: usize) -> Crossing {
        if let Some(&x) = self.crossings.get(&(i, axis as u8)) {
            return x;
        }
        let mut j = i;
        j[axis] += 1;
        let (p0, p1) = (self.at(i), self.at(j));
        let (mut t0, mut f0, mut t1, mut f1) = (0.0f32, self.corner(i), 1.0f32, self.corner(j));
        let mut t = f0 / (f0 - f1);
        // Regula falsi (Illinois) on the SDF along the edge.
        let mut side = 0i8;
        for _ in 0..6 {
            if f1 == f0 {
                break;
            }
            t = ((t0 * f1 - t1 * f0) / (f1 - f0)).clamp(0.0, 1.0);
            let f = self.field.value(lerp3(p0, p1, t));
            if f == 0.0 {
                break;
            }
            if (f < 0.0) == (f0 < 0.0) {
                t0 = t;
                f0 = f;
                if side == -1 {
                    f1 *= 0.5;
                }
                side = -1;
            } else {
                t1 = t;
                f1 = f;
                if side == 1 {
                    f0 *= 0.5;
                }
                side = 1;
            }
        }
        let x = lerp3(p0, p1, t);
        let n = self.field.gradient(x, 0.05 * self.h);
        self.crossings.insert((i, axis as u8), (x, n));
        (x, n)
    }

    /// The vertex of cell `c` (from corner `c` to `c + 1`), if the surface crosses it within reach
    /// of an edit: where the planes of its crossings meet (kept within the cell), drawn as the
    /// original where it is the original and as cut wood where it was cut.
    fn make_vertex(&mut self, c: [i32; 3]) -> Option<u32> {
        let h = self.h;
        let middle = c.map(|i| (i as f32 + 0.5) * h);
        if gmin(self.reach, middle) > DOMAIN * h {
            return None;
        }
        let mut d = [0.0f32; 8];
        for (k, o) in CUBE.iter().enumerate() {
            d[k] = self.corner([c[0] + o[0], c[1] + o[1], c[2] + o[2]]);
        }
        let solid = d.iter().filter(|v| **v < 0.0).count();
        if solid == 0 || solid == 8 {
            return None;
        }
        let mut planes: Vec<Crossing> = Vec::with_capacity(12);
        for &(a, b) in &CUBE_EDGES {
            if (d[a] < 0.0) == (d[b] < 0.0) {
                continue;
            }
            let o = CUBE[a];
            let axis = (0..3).find(|&k| CUBE[a][k] != CUBE[b][k]).unwrap_or(0);
            planes.push(self.crossing([c[0] + o[0], c[1] + o[1], c[2] + o[2]], axis));
        }
        let n = planes.len() as f32;
        let mut mass = [0.0f32; 3];
        for (p, _) in &planes {
            for a in 0..3 {
                mass[a] += p[a] / n;
            }
        }
        let mut ata = [[0.0f32; 3]; 3];
        let mut atb = [0.0f32; 3];
        for (p, nrm) in &planes {
            let b = dot3(*nrm, sub3(*p, mass));
            for r in 0..3 {
                for k in 0..3 {
                    ata[r][k] += nrm[r] * nrm[k];
                }
                atb[r] += nrm[r] * b;
            }
        }
        for (r, row) in ata.iter_mut().enumerate() {
            row[r] += LAMBDA;
        }
        let off = solve3(ata, atb).unwrap_or([0.0; 3]);
        let pos = [0, 1, 2].map(|a| (mass[a] + off[a]).clamp(c[a] as f32 * h, (c[a] + 1) as f32 * h));

        let field = self.field;
        let ans = field.answer(pos);
        let grad = field.gradient(pos, 0.05 * h);
        // How far inside the original solid (the union of its parts: not a part's surface buried
        // in another) the vertex is.
        let depth = -field.sdf.sample(pos, f32::INFINITY).map_or(f32::NEG_INFINITY, |a| a.distance);
        let original = if depth.abs() <= 0.25 * h { field.sdf.surface_at(pos, 0.3 * h).filter(|s| dot3(s.normal, grad) > 0.5) } else { None };
        let vertex = match original {
            Some(s) => Vertex { pos, normal: s.normal, material: s.material, tint: s.tint },
            None if depth > 0.0 => {
                // Cut: fresh wood, shading from the bark's tint at the rim over a cell or so.
                let cut = cut_face_tint(ans.material);
                let rim = field.sdf.surface_at(pos, 1.5 * h).map_or(cut, |s| s.tint);
                let w = ((depth - 0.25 * h) / (1.25 * h)).clamp(0.0, 1.0);
                let tint = [0, 1, 2].map(|k| (rim[k] as f32 + (cut[k] as f32 - rim[k] as f32) * w).round().clamp(0.0, 255.0) as u8);
                Vertex { pos, normal: grad, material: ans.material, tint }
            }
            None => Vertex { pos, normal: grad, material: ans.material, tint: [255; 3] },
        };
        self.verts.push(vertex);
        Some((self.verts.len() - 1) as u32)
    }

    /// The brick's triangles, each corner turned to its face's normal where the face turns sharply
    /// from it (a carve's rim), cut to within `PATCH_REACH` cells of an edit's reach.
    fn finish(self) -> Vec<Tri> {
        let mut out = Vec::with_capacity(self.tris.len());
        let reach_h = PATCH_REACH * self.h;
        for t in &self.tris {
            let mut v = t.map(|k| self.verts[k as usize]);
            let Some(face) = normalize(cross3(sub3(v[1].pos, v[0].pos), sub3(v[2].pos, v[0].pos))) else { continue };
            for x in &mut v {
                if dot3(x.normal, face) < SHARP {
                    x.normal = face;
                }
            }
            let s = v.map(|x| reach_h - gmin(self.reach, x.pos));
            clip_keep(v, s, &mut out);
        }
        out
    }
}

/// `t`, or the parts of it, outside every edit's reach by `KEEP` cells.
///
/// It is divided only where the edge of what is kept may cross it: each reach's `g` is a distance
/// (it grows no faster than the point moves), so a triangle whose every point lies within `rad` of
/// its middle is wholly on one side of a reach's edge when `g` at the middle is further than `rad`
/// from `keep`; nor does a reach cross a triangle beyond its bounding ball. Dividing every triangle near an edit down to a cell, as before, made a few dozen
/// plank triangles of a carved and cut bridge into 1.2 million (played by hand, 2026-11-28).
fn keep_outside(t: Tri, reach: &[Reach], h: f32, depth: u32, out: &mut Vec<Tri>) {
    let keep = KEEP * h;
    let mid = [0, 1, 2].map(|a| (t[0].pos[a] + t[1].pos[a] + t[2].pos[a]) / 3.0);
    let rad = t.iter().map(|v| dist2(v.pos, mid)).fold(0.0f32, f32::max).sqrt();
    let mut crossed = false;
    for r in reach {
        // Each reach is convex: a triangle whose corners are all within one is wholly within it.
        if t.iter().all(|v| r.g(v.pos) < keep) {
            return;
        }
        let at_mid = r.g(mid) - keep;
        if at_mid < -rad {
            return;
        }
        // Both must allow it: near the middle (tight for small triangles), and within the reach's
        // bounding ball of the triangle itself (tight for a long one passing by).
        if !crossed && at_mid <= rad && point_triangle_distance(r.edit.center, t[0].pos, t[1].pos, t[2].pos) <= r.radius + keep {
            crossed = true;
        }
    }
    if !crossed {
        out.push(t);
        return;
    }
    let longest = dist2(t[0].pos, t[1].pos).max(dist2(t[1].pos, t[2].pos)).max(dist2(t[2].pos, t[0].pos)).sqrt();
    if longest <= h || depth >= MAX_DEPTH {
        let s = t.map(|v| gmin(reach, v.pos) - keep);
        clip_keep(t, s, out);
        return;
    }
    let ab = lerp_vertex(t[0], t[1], 0.5);
    let bc = lerp_vertex(t[1], t[2], 0.5);
    let ca = lerp_vertex(t[2], t[0], 0.5);
    for child in [[t[0], ab, ca], [ab, t[1], bc], [ca, bc, t[2]], [ab, bc, ca]] {
        keep_outside(child, reach, h, depth + 1, out);
    }
}

/// The part of `t` where `s` (given at its corners, linear between) is not negative, as triangles
/// wound as `t` is.
fn clip_keep(t: Tri, s: [f32; 3], out: &mut Vec<Tri>) {
    let inside = s.map(|x| x >= 0.0);
    let at = |i: usize, j: usize| lerp_vertex(t[i], t[j], s[i] / (s[i] - s[j]));
    match inside.iter().filter(|b| **b).count() {
        3 => out.push(t),
        2 => {
            let o = inside.iter().position(|b| !b).unwrap_or(0);
            let (i, k) = ((o + 1) % 3, (o + 2) % 3);
            let q = at(k, o);
            let p = at(i, o);
            out.push([t[i], t[k], q]);
            out.push([t[i], q, p]);
        }
        1 => {
            let i = inside.iter().position(|b| *b).unwrap_or(0);
            let (j, k) = ((i + 1) % 3, (i + 2) % 3);
            out.push([t[i], at(i, j), at(i, k)]);
        }
        _ => {}
    }
}

/// The grid's cell: a sixth of the smallest edit, but no finer than its maker's size allows
/// (`CELLS_PER_STAMP` to their height), a power of two (in the element's units), within what the
/// element's size allows.
///
/// Bound to the maker's size, the grid stays the same while one size carves: a small stroke among
/// a person's strokes used to make the whole grid finer, and every brick was made again (a third
/// of a second to three seconds a stroke, played by hand 2026-11-28).
fn cell_size(edits: &[Edit], sdf: &MeshSdf, level: u32) -> f32 {
    let mut h = f32::INFINITY;
    for e in edits {
        let feature = match e.shape {
            Shape::Sphere { radius } => radius,
            Shape::Box { half, .. } => half[0].min(half[1]).min(half[2]),
            Shape::Cylinder { radius, half_height, .. } => radius.min(half_height),
        };
        // `stamp_size` is the maker's height, in the element's units as the edit is.
        let finest = e.stamp_size / CELLS_PER_STAMP;
        h = h.min((feature / CELLS_PER_FEATURE).max(finest));
    }
    let (lo, hi) = (sdf.lo(), sdf.hi());
    let longest = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(hi[2] - lo[2]).max(1e-3);
    let h = h.clamp(longest / FINEST_ALONG, longest / COARSEST_ALONG).max(1e-4);
    // The power of two at or below (a positive, normal float's exponent alone); a coarser level
    // doubles it each time, to no coarser than its longest side over `COARSEST_ALONG_FAR`.
    let h = f32::from_bits(h.to_bits() & 0x7f80_0000);
    let mut out = h;
    for _ in 0..level {
        if out * 2.0 > longest / COARSEST_ALONG_FAR {
            break;
        }
        out *= 2.0;
    }
    out
}

/// Every solid triangle of `scene`, in its frame.
fn solid_tris(scene: &Scene) -> Vec<Tri> {
    let mut out = Vec::new();
    for inst in &scene.instances {
        if !inst.solid {
            continue;
        }
        let Some(mesh) = scene.meshes.get(inst.mesh as usize) else { continue };
        let sc = inst.scale;
        let inv = sc.map(|s| if s.abs() > 1e-5 { 1.0 / s } else { 1.0 });
        let place = |v: &Vertex| {
            let r = quat_rotate(inst.rot, [v.pos[0] * sc[0], v.pos[1] * sc[1], v.pos[2] * sc[2]]);
            let n = quat_rotate(inst.rot, [v.normal[0] * inv[0], v.normal[1] * inv[1], v.normal[2] * inv[2]]);
            Vertex { pos: [inst.pos[0] + r[0], inst.pos[1] + r[1], inst.pos[2] + r[2]], normal: normalize(n).unwrap_or([0.0, 1.0, 0.0]), ..*v }
        };
        for tri in mesh.indices.as_chunks::<3>().0 {
            let (Some(a), Some(b), Some(c)) = (mesh.vertices.get(tri[0] as usize), mesh.vertices.get(tri[1] as usize), mesh.vertices.get(tri[2] as usize)) else { continue };
            out.push([place(a), place(b), place(c)]);
        }
    }
    out
}

fn to_mesh(tris: &[Tri]) -> MeshData {
    let mut m = MeshData { vertices: Vec::with_capacity(tris.len() * 3), indices: Vec::with_capacity(tris.len() * 3) };
    for t in tris {
        for v in t {
            m.indices.push(m.vertices.len() as u32);
            m.vertices.push(*v);
        }
    }
    m
}

fn hash_edit(e: &Edit, h: &mut DefaultHasher) {
    e.op.hash(h);
    match e.shape {
        Shape::Sphere { radius } => (0u8, radius.to_bits()).hash(h),
        Shape::Box { half, round } => (1u8, half.map(f32::to_bits), round.to_bits()).hash(h),
        Shape::Cylinder { radius, half_height, round } => (2u8, radius.to_bits(), half_height.to_bits(), round.to_bits()).hash(h),
    }
    e.center.map(f32::to_bits).hash(h);
    e.blend.to_bits().hash(h);
    e.material.hash(h);
}

fn solve3(m: [[f32; 3]; 3], b: [f32; 3]) -> Option<[f32; 3]> {
    let det3 = |m: [[f32; 3]; 3]| {
        m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1]) - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0]) + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0])
    };
    let det = det3(m);
    if det.abs() < 1e-12 {
        return None;
    }
    let mut x = [0.0f32; 3];
    for (col, xc) in x.iter_mut().enumerate() {
        let mut mc = m;
        for r in 0..3 {
            mc[r][col] = b[r];
        }
        *xc = det3(mc) / det;
    }
    Some(x)
}

fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let tx = 2.0 * (qy * v[2] - qz * v[1]);
    let ty = 2.0 * (qz * v[0] - qx * v[2]);
    let tz = 2.0 * (qx * v[1] - qy * v[0]);
    [v[0] + qw * tx + (qy * tz - qz * ty), v[1] + qw * ty + (qz * tx - qx * tz), v[2] + qw * tz + (qx * ty - qy * tx)]
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn sub3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn len3(a: [f32; 3]) -> f32 {
    dot3(a, a).sqrt()
}

fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub3(a, b);
    dot3(d, d)
}

fn normalize(a: [f32; 3]) -> Option<[f32; 3]> {
    let l = len3(a);
    (l > 1e-20).then(|| a.map(|c| c / l))
}
