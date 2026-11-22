//! Chunks of an area's surface (POC_PLAN.md, step 2, "Meshing"): surface nets over a cube of
//! `CELLS` cells a side, asked of the area at the size the chunk's voxel is detail for.
//!
//! - **Levels.** A chunk's voxel is a power of two metres (`Key::level`), so the grids of every
//!   level line up: a chunk is eight of the level below it. It is asked for the viewer twenty
//!   voxels tall (`Key::size`), the one whose detail it is near them, so the same chunk serves any
//!   viewer who needs that level, and what is far off is asked only coarsely. Except in a hollow
//!   only smaller viewers see (the tiny in a cave): there the larger viewers of the coarser levels
//!   would see solid all about the eye, so those levels are asked for a smaller one (`Key::asked`,
//!   `rings::open`).
//! - **No seams within a level.** Each chunk asks one cell beyond its own on every side, makes the
//!   vertices of the cells along its edges as its neighbours make them, and makes the quads only of
//!   the grid edges it owns (those whose first corner is its own): neighbours meet exactly.
//! - **Seams between levels hidden by skirts**: every edge the mesh ends at is hung with a strip
//!   two voxels deep, into the solid, so where a finer chunk meets a coarser one the gap is closed
//!   from outside.
//! - **Only near the surface.** A distance says nothing is nearer than it (it is never too long),
//!   so a block of corners whose middle is further from any surface than its corners are from the
//!   middle is all inside or all outside, and is not asked: first the chunk, then its eighths, down
//!   to blocks of four corners. The surface is two-dimensional and the chunk three, so most of a
//!   chunk is never asked.

use crate::{LARGEST, Material, Region, Result, SMALLEST};

/// Cells along each side of a chunk.
pub const CELLS: usize = 32;
/// Corners along each side asked: the chunk's own, and one cell's beyond it on each side (grid
/// coordinates -1 to `CELLS`).
const CORNERS: usize = CELLS + 2;
/// Cells along each side that get a vertex: the chunk's own and the one before it on each axis.
const NET: usize = CELLS + 1;
/// The smallest block of corners the coarse pass proves empty or full before asking every corner.
const SMALLEST_BLOCK: usize = 4;
/// A voxel is a twentieth of the height of the viewer the chunk is asked for (POC_PLAN.md,
/// "Detail per size").
pub const VOXELS_A_HEIGHT: f32 = 20.0;

/// Where a chunk is: its level (its voxel is `2^level` metres) and its place on that level's grid;
/// and the level whose viewer it is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub level: i32,
    pub at: [i32; 3],
    /// Its own level, or a finer one, when the viewer is in a hollow that the viewer of its own
    /// level does not see (`capped`).
    pub asked: i32,
}

impl Key {
    /// The chunk at `at` on the grid of `level`, asked for its own level's viewer.
    pub fn new(level: i32, at: [i32; 3]) -> Key {
        Key { level, at, asked: level }
    }

    /// The same chunk asked for no larger a viewer than level `open`'s.
    pub fn capped(self, open: i32) -> Key {
        Key { asked: self.level.min(open), ..self }
    }

    /// A voxel's width, in metres: a power of two, so every level's grid lines up with the next.
    pub fn voxel(self) -> f64 {
        2f64.powi(self.level)
    }

    /// The chunk's side, in metres.
    pub fn side(self) -> f64 {
        self.voxel() * CELLS as f64
    }

    /// Its corner nearest the area's origin.
    pub fn origin(self) -> [f64; 3] {
        let side = self.side();
        self.at.map(|i| i as f64 * side)
    }

    pub fn middle(self) -> [f64; 3] {
        let (o, half) = (self.origin(), self.side() * 0.5);
        o.map(|c| c + half)
    }

    /// The chunk of the next level up that holds this one, asked for its own level's viewer.
    pub fn parent(self) -> Key {
        Key::new(self.level + 1, self.at.map(|i| i.div_euclid(2)))
    }

    /// The eight chunks of the next level down within this one, asked for their own level's viewer.
    pub fn children(self) -> [Key; 8] {
        std::array::from_fn(|k| Key::new(self.level - 1, [0, 1, 2].map(|a| self.at[a] * 2 + (k as i32 >> a & 1))))
    }

    /// The viewer the chunk is asked for: twenty voxels of level `asked` tall, within the sizes an
    /// area is asked at.
    pub fn size(self) -> f32 {
        level_size(self.asked)
    }
}

/// The viewer the chunks of `level` are asked for: twenty of its voxels tall, within the sizes an
/// area is asked at.
pub fn level_size(level: i32) -> f32 {
    (2f32.powi(level) * VOXELS_A_HEIGHT).clamp(SMALLEST, LARGEST)
}

/// A chunk's surface: positions relative to its origin, normals pointing out of the solid, each
/// vertex's material and how open it is to the sky (1 in the open, less in hollows and corners).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub materials: Vec<Material>,
    pub open: Vec<f32>,
    pub indices: Vec<u32>,
}

/// A chunk made: its surface, if it has one, and how many points the area was asked.
#[derive(Clone, Debug)]
pub struct Made {
    pub mesh: Option<Mesh>,
    pub samples: usize,
}

/// How a chunk is made: the coarse pass and skirts can be turned off, to compare.
#[derive(Clone, Copy, Debug)]
pub struct Making {
    pub coarse: bool,
    pub skirts: bool,
    pub shading: bool,
}

impl Default for Making {
    fn default() -> Self {
        Making { coarse: true, skirts: true, shading: true }
    }
}

/// The chunk `key` of `area`, made as `Making::default()`.
pub fn make(area: &mut dyn Region, key: Key) -> Result<Made> {
    make_with(area, key, Making::default())
}

pub fn make_with(area: &mut dyn Region, key: Key, making: Making) -> Result<Made> {
    let size = key.size();
    let origin = key.origin();
    let voxel = key.voxel();
    let point = |g: [f64; 3]| -> [f32; 3] { [0, 1, 2].map(|a| (origin[a] + g[a] * voxel) as f32) };
    // Grid coordinate of corner `i` along an axis is `i - 1`.
    let corner_point = |i: [usize; 3]| point(i.map(|c| c as f64 - 1.0));
    let index = |i: [usize; 3]| (i[0] * CORNERS + i[1]) * CORNERS + i[2];
    let mut distance = vec![0f32; CORNERS * CORNERS * CORNERS];
    let mut material = vec![Material::Soil; CORNERS * CORNERS * CORNERS];
    let mut samples = 0;
    let mut out = Vec::new();

    // The corners to ask one by one, after the coarse pass has set aside the blocks it proves.
    let mut ask: Vec<[usize; 3]> = Vec::new();
    if making.coarse {
        let mut blocks = vec![([0usize; 3], [CORNERS; 3])];
        while !blocks.is_empty() {
            let middles: Vec<[f32; 3]> =
                blocks.iter().map(|(lo, hi)| point([0, 1, 2].map(|a| (lo[a] + hi[a] - 1) as f64 * 0.5 - 1.0))).collect();
            sample(area, &middles, size, &mut out)?;
            samples += middles.len();
            let mut finer = Vec::new();
            for (k, &(lo, hi)) in blocks.iter().enumerate() {
                let d = out[k].distance;
                let half = [0, 1, 2].map(|a| (hi[a] - 1 - lo[a]) as f64 * 0.5);
                let reach = (half[0] * half[0] + half[1] * half[1] + half[2] * half[2]).sqrt() * voxel;
                // Proved all one side, and a little beyond (a cell's diagonal is enough for distances
                // never too long; two cells allow for an area whose distances are a little too long
                // in places), so no cell with a corner here is crossed.
                if (d.abs() as f64) > reach + 2.0 * voxel {
                    let fill = d.signum() * (d.abs() - reach as f32);
                    for x in lo[0]..hi[0] {
                        for y in lo[1]..hi[1] {
                            for z in lo[2]..hi[2] {
                                distance[index([x, y, z])] = fill;
                                material[index([x, y, z])] = out[k].material;
                            }
                        }
                    }
                } else if (0..3).any(|a| hi[a] - lo[a] > SMALLEST_BLOCK) {
                    let mid = [0, 1, 2].map(|a| lo[a] + (hi[a] - lo[a]).div_ceil(2));
                    for octant in 0..8 {
                        let pick = |a: usize| if octant >> a & 1 == 0 { (lo[a], mid[a]) } else { (mid[a], hi[a]) };
                        let (x, y, z) = (pick(0), pick(1), pick(2));
                        if x.0 < x.1 && y.0 < y.1 && z.0 < z.1 {
                            finer.push(([x.0, y.0, z.0], [x.1, y.1, z.1]));
                        }
                    }
                } else {
                    for x in lo[0]..hi[0] {
                        for y in lo[1]..hi[1] {
                            for z in lo[2]..hi[2] {
                                ask.push([x, y, z]);
                            }
                        }
                    }
                }
            }
            blocks = finer;
        }
    } else {
        for x in 0..CORNERS {
            for y in 0..CORNERS {
                for z in 0..CORNERS {
                    ask.push([x, y, z]);
                }
            }
        }
    }
    let points: Vec<[f32; 3]> = ask.iter().map(|&i| corner_point(i)).collect();
    sample(area, &points, size, &mut out)?;
    samples += points.len();
    for (k, &i) in ask.iter().enumerate() {
        distance[index(i)] = out[k].distance;
        material[index(i)] = out[k].material;
    }

    let (mut mesh, slopes) = net(&distance, &material, voxel as f32);
    if mesh.indices.is_empty() {
        return Ok(Made { mesh: None, samples });
    }
    if making.shading {
        samples += shade(area, &mut mesh, &slopes, origin, size)?;
    }
    if making.skirts {
        skirt(&mut mesh, 2.0 * voxel as f32);
    }
    Ok(Made { mesh: Some(mesh), samples })
}

const CUBE: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]];
const CUBE_EDGES: [(usize, usize); 12] = [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];

/// Surface nets through the corners' answers: a vertex in each cell the surface crosses, at the
/// mean of where it crosses the cell's edges; a quad across each owned grid edge it crosses. With
/// the mesh, how steeply the distance grows at each vertex (1 for a true distance, less for one
/// the area made shorter).
fn net(distance: &[f32], material: &[Material], voxel: f32) -> (Mesh, Vec<f32>) {
    let index = |i: [usize; 3]| (i[0] * CORNERS + i[1]) * CORNERS + i[2];
    let inside = |i: [usize; 3]| distance[index(i)] < 0.0;
    let cell_index = |c: [usize; 3]| (c[0] * NET + c[1]) * NET + c[2];
    let mut vertex_of = vec![u32::MAX; NET * NET * NET];
    let mut mesh = Mesh::default();
    let mut slopes = Vec::new();
    for x in 0..NET {
        for y in 0..NET {
            for z in 0..NET {
                let c = [x, y, z];
                let d: [f32; 8] = CUBE.map(|o| distance[index([x + o[0], y + o[1], z + o[2]])]);
                let solid = d.iter().filter(|v| **v < 0.0).count();
                if solid == 0 || solid == 8 {
                    continue;
                }
                let mut sum = [0f32; 3];
                let mut crossings = 0.0;
                for &(a, b) in &CUBE_EDGES {
                    if (d[a] < 0.0) != (d[b] < 0.0) {
                        let t = d[a] / (d[a] - d[b]);
                        for axis in 0..3 {
                            let (pa, pb) = (CUBE[a][axis] as f32, CUBE[b][axis] as f32);
                            sum[axis] += pa + (pb - pa) * t;
                        }
                        crossings += 1.0;
                    }
                }
                let local = sum.map(|s| s / crossings);
                // The trilinear gradient at the vertex, pointing out of the solid.
                let along = |axis: usize| -> f32 {
                    let mut g = 0.0;
                    for (k, o) in CUBE.iter().enumerate() {
                        let w: f32 = (0..3).filter(|&a| a != axis).map(|a| if o[a] == 1 { local[a] } else { 1.0 - local[a] }).product();
                        g += if o[axis] == 1 { w } else { -w } * d[k];
                    }
                    g
                };
                let g = [along(0), along(1), along(2)];
                let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-20);
                // The material of the solid corner nearest the surface.
                let nearest = (0..8).filter(|&k| d[k] < 0.0).max_by(|&a, &b| d[a].total_cmp(&d[b])).unwrap();
                let o = CUBE[nearest];
                vertex_of[cell_index(c)] = mesh.positions.len() as u32;
                mesh.positions.push([0, 1, 2].map(|a| (c[a] as f32 - 1.0 + local[a]) * voxel));
                mesh.normals.push(g.map(|v| v / len));
                mesh.materials.push(material[index([x + o[0], y + o[1], z + o[2]])]);
                mesh.open.push(1.0);
                slopes.push(len / voxel);
            }
        }
    }
    // The grid edges this chunk owns: from each of its own corners (grid 0 to CELLS - 1, corner
    // index 1 to CELLS) along each axis.
    for x in 1..=CELLS {
        for y in 1..=CELLS {
            for z in 1..=CELLS {
                let i = [x, y, z];
                let here = inside(i);
                for axis in 0..3 {
                    let mut next = i;
                    next[axis] += 1;
                    if here == inside(next) {
                        continue;
                    }
                    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                    // The four cells about the edge; cell `c` spans corners `c` to `c + 1`.
                    let cell = |du: usize, dv: usize| {
                        let mut c = i;
                        c[u] = c[u] - 1 + du;
                        c[v] = c[v] - 1 + dv;
                        vertex_of[cell_index(c)]
                    };
                    let quad = [cell(0, 0), cell(1, 0), cell(1, 1), cell(0, 1)];
                    debug_assert!(!quad.contains(&u32::MAX), "a crossed edge's cells all have vertices");
                    // Counter-clockwise seen from outside.
                    let [a, b, c, d] = if here { quad } else { [quad[0], quad[3], quad[2], quad[1]] };
                    mesh.indices.extend_from_slice(&[a, b, c, a, c, d]);
                }
            }
        }
    }
    (mesh, slopes)
}

/// Darkens each vertex as nearby surfaces close it in: the area asked a little way out along the
/// normal, three times as far each time (up to a third of the viewer's height), and any surface
/// nearer than the
/// vertex's own would be there takes away some of the sky. Hollows, cracks and the feet of walls
/// are darker (spike: "hollows need shading to be seen"). Gives how many points it asked.
fn shade(area: &mut dyn Region, mesh: &mut Mesh, slopes: &[f32], origin: [f64; 3], size: f32) -> Result<usize> {
    let mut asked = 0;
    let mut out = Vec::new();
    for (k, reach) in [0.04, 0.12, 0.36].into_iter().enumerate() {
        let reach = size * reach;
        let points: Vec<[f32; 3]> = mesh
            .positions
            .iter()
            .zip(&mesh.normals)
            .map(|(p, n)| [0, 1, 2].map(|a| (origin[a] + p[a] as f64) as f32 + n[a] * reach))
            .collect();
        sample(area, &points, size, &mut out)?;
        asked += points.len();
        for ((open, a), slope) in mesh.open.iter_mut().zip(&out).zip(slopes) {
            let alone = reach * slope.clamp(0.3, 1.0);
            *open -= 0.9 * (alone - a.distance).max(0.0) / alone / (1 << k) as f32;
        }
    }
    for open in &mut mesh.open {
        *open = open.clamp(0.15, 1.0);
    }
    Ok(asked)
}

/// Hangs a strip `depth` deep from every edge the mesh ends at, into the solid, facing away from
/// the mesh: where a finer chunk meets a coarser one, the coarser one's skirt closes the gap.
fn skirt(mesh: &mut Mesh, depth: f32) {
    let mut edges: Vec<(u32, u32, u32, u32)> = Vec::with_capacity(mesh.indices.len());
    for t in mesh.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            edges.push((a.min(b), a.max(b), a, b));
        }
    }
    edges.sort_unstable();
    let mut k = 0;
    while k < edges.len() {
        let mut n = 1;
        while k + n < edges.len() && edges[k + n].0 == edges[k].0 && edges[k + n].1 == edges[k].1 {
            n += 1;
        }
        if n == 1 {
            let (_, _, a, b) = edges[k];
            let low = |v: u32, mesh: &mut Mesh| -> u32 {
                let (p, n) = (mesh.positions[v as usize], mesh.normals[v as usize]);
                mesh.positions.push([0, 1, 2].map(|i| p[i] - n[i] * depth));
                mesh.normals.push(n);
                mesh.materials.push(mesh.materials[v as usize]);
                mesh.open.push(mesh.open[v as usize]);
                mesh.positions.len() as u32 - 1
            };
            let (a2, b2) = (low(a, mesh), low(b, mesh));
            mesh.indices.extend_from_slice(&[a, a2, b2, a, b2, b]);
        }
        k += n;
    }
}

/// The area asked at `points`, in batches it takes.
pub(crate) fn sample(area: &mut dyn Region, points: &[[f32; 3]], size: f32, out: &mut Vec<crate::Answer>) -> Result<()> {
    out.clear();
    let mut batch = Vec::new();
    for chunk in points.chunks(4096) {
        area.sample(chunk, size, &mut batch)?;
        out.extend_from_slice(&batch);
    }
    Ok(())
}
