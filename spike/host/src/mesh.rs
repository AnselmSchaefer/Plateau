//! Surface nets over one chunk (POC_PLAN.md, "Meshing"): the area is asked at the corners of a
//! grid of cells; each cell the surface passes through gets one vertex, at the mean of where the
//! surface crosses its edges; each grid edge the surface crosses joins the four cells around it
//! with a quad. Seams with other chunks, and between levels of detail, are for step 2.

use crate::Area;
use crate::Result;

/// A chunk: `cells` cells along each axis, each `voxel` metres, from `origin`.
#[derive(Clone, Copy, Debug)]
pub struct Chunk {
    pub origin: [f32; 3],
    pub voxel: f32,
    pub cells: [usize; 3],
}

#[derive(Default, Debug)]
pub struct Mesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub materials: Vec<u32>,
    pub indices: Vec<u32>,
}

/// How long the parts of making a chunk took, in seconds.
#[derive(Default, Debug, Clone, Copy)]
pub struct Timing {
    pub sampling: f64,
    pub meshing: f64,
}

/// The surface of `chunk` as the area makes it for a viewer `size` tall, asked in batches of at
/// most `batch` points.
pub fn mesh(area: &mut dyn Area, chunk: Chunk, size: f32, batch: usize) -> Result<(Mesh, Timing)> {
    let started = std::time::Instant::now();
    let [nx, ny, nz] = chunk.cells.map(|c| c + 1);
    let mut points = Vec::with_capacity(nx * ny * nz);
    for x in 0..nx {
        for y in 0..ny {
            for z in 0..nz {
                let at = |o: f32, i: usize| o + i as f32 * chunk.voxel;
                points.push([at(chunk.origin[0], x), at(chunk.origin[1], y), at(chunk.origin[2], z)]);
            }
        }
    }
    let mut answers = Vec::with_capacity(points.len());
    let mut part = Vec::new();
    for batch in points.chunks(batch.max(1)) {
        area.sample(batch, size, &mut part)?;
        answers.extend_from_slice(&part);
    }
    let sampled = std::time::Instant::now();
    let mesh = net(&chunk, &answers);
    let timing = Timing { sampling: (sampled - started).as_secs_f64(), meshing: sampled.elapsed().as_secs_f64() };
    Ok((mesh, timing))
}

/// The surface through answers at the grid's corners, x slowest and z fastest.
fn net(chunk: &Chunk, answers: &[(f32, u32)]) -> Mesh {
    let [cx, cy, cz] = chunk.cells;
    let (ny, nz) = (cy + 1, cz + 1);
    let corner = |x: usize, y: usize, z: usize| answers[(x * ny + y) * nz + z];
    let cell_index = |x: usize, y: usize, z: usize| (x * cy + y) * cz + z;
    let mut vertex_of = vec![u32::MAX; cx * cy * cz];
    let mut mesh = Mesh::default();
    const CORNERS: [[usize; 3]; 8] = [[0, 0, 0], [1, 0, 0], [0, 1, 0], [1, 1, 0], [0, 0, 1], [1, 0, 1], [0, 1, 1], [1, 1, 1]];
    const EDGES: [(usize, usize); 12] =
        [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)];
    for x in 0..cx {
        for y in 0..cy {
            for z in 0..cz {
                let d: [(f32, u32); 8] = CORNERS.map(|[i, j, k]| corner(x + i, y + j, z + k));
                let inside = d.iter().filter(|(v, _)| *v < 0.0).count();
                if inside == 0 || inside == 8 {
                    continue;
                }
                let mut sum = [0.0f32; 3];
                let mut crossings = 0.0;
                for &(a, b) in &EDGES {
                    let (da, db) = (d[a].0, d[b].0);
                    if (da < 0.0) != (db < 0.0) {
                        let t = da / (da - db);
                        for axis in 0..3 {
                            let (pa, pb) = (CORNERS[a][axis] as f32, CORNERS[b][axis] as f32);
                            sum[axis] += pa + (pb - pa) * t;
                        }
                        crossings += 1.0;
                    }
                }
                let local = sum.map(|s| s / crossings);
                let position = [0, 1, 2].map(|a| chunk.origin[a] + ([x, y, z][a] as f32 + local[a]) * chunk.voxel);
                // The trilinear gradient at the vertex, pointing out of the solid.
                let lerp4 = |axis: usize| {
                    let mut g = 0.0;
                    for (c, corner) in CORNERS.iter().enumerate() {
                        let w: f32 = (0..3)
                            .filter(|&a| a != axis)
                            .map(|a| if corner[a] == 1 { local[a] } else { 1.0 - local[a] })
                            .product();
                        g += if corner[axis] == 1 { w } else { -w } * d[c].0;
                    }
                    g
                };
                let g = [lerp4(0), lerp4(1), lerp4(2)];
                let len = (g[0] * g[0] + g[1] * g[1] + g[2] * g[2]).sqrt().max(1e-12);
                // The material is the solid's nearest the surface.
                let material = d.iter().filter(|(v, _)| *v < 0.0).max_by(|a, b| a.0.total_cmp(&b.0)).map_or(0, |c| c.1);
                vertex_of[cell_index(x, y, z)] = mesh.positions.len() as u32;
                mesh.positions.push(position);
                mesh.normals.push(g.map(|v| v / len));
                mesh.materials.push(material);
            }
        }
    }
    // Each grid edge the surface crosses, with all four cells around it in the chunk, gets a quad.
    for x in 0..=cx {
        for y in 0..=cy {
            for z in 0..=cz {
                let here = corner(x, y, z).0 < 0.0;
                for axis in 0..3 {
                    let mut next = [x, y, z];
                    next[axis] += 1;
                    if next[axis] > chunk.cells[axis] {
                        continue;
                    }
                    if here == (corner(next[0], next[1], next[2]).0 < 0.0) {
                        continue;
                    }
                    let (u, v) = ((axis + 1) % 3, (axis + 2) % 3);
                    let at = [x, y, z];
                    if at[u] == 0 || at[v] == 0 || at[u] >= chunk.cells[u] || at[v] >= chunk.cells[v] {
                        continue;
                    }
                    let cell = |du: usize, dv: usize| {
                        let mut c = at;
                        c[u] = c[u] - 1 + du;
                        c[v] = c[v] - 1 + dv;
                        vertex_of[cell_index(c[0], c[1], c[2])]
                    };
                    let quad = [cell(0, 0), cell(1, 0), cell(1, 1), cell(0, 1)];
                    if quad.contains(&u32::MAX) {
                        continue;
                    }
                    // Counter-clockwise seen from outside: the solid is where `here` is.
                    let [a, b, c, d] = if here { quad } else { [quad[0], quad[3], quad[2], quad[1]] };
                    mesh.indices.extend_from_slice(&[a, b, c, a, c, d]);
                }
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Native;

    fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }

    #[test]
    fn the_triangles_face_the_way_the_surface_does() {
        let chunk = Chunk { origin: [36.0, -2.0, 8.0], voxel: 0.25, cells: [48, 24, 48] };
        let (mesh, _) = mesh(&mut Native { seed: 1 }, chunk, 1.7, 4096).unwrap();
        assert!(mesh.indices.len() > 1000, "only {} indices", mesh.indices.len());
        let mut agree = 0;
        for t in mesh.indices.chunks_exact(3) {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
            let n = cross([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
            let g = mesh.normals[t[0] as usize];
            if n[0] * g[0] + n[1] * g[1] + n[2] * g[2] > 0.0 {
                agree += 1;
            }
        }
        let triangles = mesh.indices.len() / 3;
        assert!(agree * 100 >= triangles * 98, "{agree} of {triangles} triangles face along the gradient");
    }
}
