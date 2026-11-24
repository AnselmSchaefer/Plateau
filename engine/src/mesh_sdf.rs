//! Exact, deterministic, BVH-accelerated Signed Distance Field built from a procedural `Scene`'s
//! solid triangle meshes (`MeshSdf`).
//!
//! Enables **lazy Scene → SDF conversion ("promote on first sculpt")**:
//! - Untouched `Scene` elements (trees, bridges, cottages) stay lightweight triangle meshes with
//!   zero SDF cost.
//! - The moment a player carves or sculpts onto a solid `Scene` element, its `solid: true`
//!   instances are baked into an element-local `MeshSdf` while decorative `solid: false` instances
//!   (e.g. swaying oak canopy, bridge handrails, glowing lantern head) remain crisp in `Scene`.
//! - **Exact `|grad d| == 1`** at every scale (down to sub-millimetre tiny caves) via BVH
//!   point-to-triangle distance.
//! - **Exact union sign for overlapping closed sub-parts** (e.g. trunk + buttress roots, or
//!   overlapping bridge planks) via BVH signed ray-winding (`exits - enters > 0`), with face
//!   normals automatically aligned to outward vertex normals so every `MeshBuilder` primitive
//!   produces consistent outward orientation.

use crate::{Answer, ElementBounds, Instance, Material, Scene};

/// One world/local triangle with outward-aligned unit face normal and tight AABB.
#[derive(Clone, Copy, Debug)]
struct SdfTri {
    a: [f32; 3],
    b: [f32; 3],
    c: [f32; 3],
    normal: [f32; 3],
    lo: [f32; 3],
    hi: [f32; 3],
    centroid: [f32; 3],
    material: Material,
}

#[derive(Clone, Copy, Debug)]
enum NodeKind {
    Leaf { start: u32, count: u32 },
    Branch { left: u32, right: u32 },
}

#[derive(Clone, Copy, Debug)]
struct BvhNode {
    lo: [f32; 3],
    hi: [f32; 3],
    kind: NodeKind,
}

/// Deterministic, immutable BVH-backed Signed Distance Field over all `solid: true` triangles of a
/// procedural `Scene` (in element-local space).
#[derive(Clone, Debug)]
pub struct MeshSdf {
    tris: Vec<SdfTri>,
    nodes: Vec<BvhNode>,
    lo: [f32; 3],
    hi: [f32; 3],
}

/// Three deterministic, non-axis-aligned unit directions for signed ray-winding with grazing fallback.
const WINDING_DIRS: [[f32; 3]; 3] = [
    [0.731_292_9, 0.517_484_6, 0.444_320_3],
    [-0.413_421_6, 0.819_105_4, 0.397_679_3],
    [0.317_248_6, -0.441_125_2, 0.840_096_5],
];

impl MeshSdf {
    /// Builds a `MeshSdf` from all `solid: true` instances in `scene` (in the scene's own local
    /// space). Returns `None` if the scene has no solid triangles.
    pub fn from_solid_scene(scene: &Scene) -> Option<MeshSdf> {
        let mut tris = Vec::new();
        for inst in &scene.instances {
            if !inst.solid {
                continue;
            }
            let Some(mesh) = scene.meshes.get(inst.mesh as usize) else { continue };
            append_instance_tris(inst, mesh, &mut tris);
        }
        if tris.is_empty() {
            return None;
        }
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for t in &tris {
            for a in 0..3 {
                lo[a] = min_f32(lo[a], t.lo[a]);
                hi[a] = max_f32(hi[a], t.hi[a]);
            }
        }
        // Small padding so bounds strictly enclose vertices on all 6 faces.
        let pad = 1e-3;
        for a in 0..3 {
            lo[a] -= pad;
            hi[a] += pad;
        }
        let mut nodes = Vec::with_capacity(tris.len() * 2);
        let len = tris.len();
        build_bvh(&mut tris, 0, len, &mut nodes);
        Some(MeshSdf { tris, nodes, lo, hi })
    }

    pub fn lo(&self) -> [f32; 3] {
        self.lo
    }

    pub fn hi(&self) -> [f32; 3] {
        self.hi
    }

    pub fn bounds(&self) -> ElementBounds {
        ElementBounds { lo: self.lo, hi: self.hi, blend: 0.0 }
    }

    pub fn triangle_count(&self) -> usize {
        self.tris.len()
    }

    /// Evaluates the exact signed distance and surface material at local point `p`.
    /// Returns `None` if `p` is guaranteed farther than `max_dist` from every triangle.
    pub fn sample(&self, p: [f32; 3], max_dist: f32) -> Option<Answer> {
        if self.nodes.is_empty() {
            return None;
        }
        let root_d = point_to_aabb_distance(p, self.lo, self.hi);
        if root_d >= max_dist {
            return None;
        }
        let (best_d, best_mat) = self.closest_triangle(p, max_dist)?;
        let inside = if root_d == 0.0 { self.is_inside(p) } else { false };
        let distance = if inside { -max_f32(best_d, 1e-5) } else { best_d };
        Some(Answer { distance, material: best_mat })
    }

    fn closest_triangle(&self, p: [f32; 3], max_dist: f32) -> Option<(f32, Material)> {
        let mut best_d = max_dist;
        let mut best_mat: Option<Material> = None;
        let mut stack = [0u32; 64];
        let mut sp = 1usize;
        stack[0] = 0;

        while sp > 0 {
            sp -= 1;
            let node = self.nodes[stack[sp] as usize];
            if point_to_aabb_distance(p, node.lo, node.hi) >= best_d {
                continue;
            }
            match node.kind {
                NodeKind::Leaf { start, count } => {
                    let slice = &self.tris[start as usize..(start + count) as usize];
                    for tri in slice {
                        if point_to_aabb_distance(p, tri.lo, tri.hi) >= best_d {
                            continue;
                        }
                        let d = point_triangle_distance(p, tri.a, tri.b, tri.c);
                        if d < best_d {
                            best_d = d;
                            best_mat = Some(tri.material);
                        }
                    }
                }
                NodeKind::Branch { left, right } => {
                    let dl = point_to_aabb_distance(p, self.nodes[left as usize].lo, self.nodes[left as usize].hi);
                    let dr = point_to_aabb_distance(p, self.nodes[right as usize].lo, self.nodes[right as usize].hi);
                    // Push farther child first so nearer child pops first and tightens `best_d` early.
                    if dl < dr {
                        if dr < best_d && sp < stack.len() {
                            stack[sp] = right;
                            sp += 1;
                        }
                        if dl < best_d && sp < stack.len() {
                            stack[sp] = left;
                            sp += 1;
                        }
                    } else {
                        if dl < best_d && sp < stack.len() {
                            stack[sp] = left;
                            sp += 1;
                        }
                        if dr < best_d && sp < stack.len() {
                            stack[sp] = right;
                            sp += 1;
                        }
                    }
                }
            }
        }
        best_mat.map(|m| (best_d, m))
    }

    /// Exact integer winding test (`exits - enters > 0`) for unions of closed outward-oriented
    /// sub-meshes, retrying with alternate non-axis-aligned directions if a ray grazes an edge.
    fn is_inside(&self, p: [f32; 3]) -> bool {
        let mut last = 0i32;
        for &dir in &WINDING_DIRS {
            match self.ray_winding(p, dir) {
                (w, false) => return w > 0,
                (w, true) => last = w,
            }
        }
        last > 0
    }

    /// Casts a ray from `p` along unit direction `dir` and counts `exits - enters` across all
    /// triangles intersected at `t > 0`. Returns `(winding, grazed)` where `grazed` signals an
    /// edge/vertex near-hit that warrants retrying a different ray direction.
    fn ray_winding(&self, p: [f32; 3], dir: [f32; 3]) -> (i32, bool) {
        let inv = [1.0 / dir[0], 1.0 / dir[1], 1.0 / dir[2]];
        let mut winding = 0i32;
        let mut grazed = false;
        let mut stack = [0u32; 64];
        let mut sp = 1usize;
        stack[0] = 0;

        while sp > 0 {
            sp -= 1;
            let node = self.nodes[stack[sp] as usize];
            if !ray_hits_aabb(p, inv, node.lo, node.hi) {
                continue;
            }
            match node.kind {
                NodeKind::Leaf { start, count } => {
                    let slice = &self.tris[start as usize..(start + count) as usize];
                    for tri in slice {
                        if !ray_hits_aabb(p, inv, tri.lo, tri.hi) {
                            continue;
                        }
                        if let Some((front_exit, near_edge)) = ray_triangle_signed(p, dir, tri) {
                            if near_edge {
                                grazed = true;
                            }
                            if front_exit {
                                winding += 1;
                            } else {
                                winding -= 1;
                            }
                        }
                    }
                }
                NodeKind::Branch { left, right } => {
                    if sp + 2 <= stack.len() {
                        stack[sp] = right;
                        stack[sp + 1] = left;
                        sp += 2;
                    }
                }
            }
        }
        (winding, grazed)
    }
}

fn append_instance_tris(inst: &Instance, mesh: &crate::MeshData, out: &mut Vec<SdfTri>) {
    let sc = inst.scale;
    let inv_sc = [
        if abs_f32(sc[0]) > 1e-5 { 1.0 / sc[0] } else { 1.0 },
        if abs_f32(sc[1]) > 1e-5 { 1.0 / sc[1] } else { 1.0 },
        if abs_f32(sc[2]) > 1e-5 { 1.0 / sc[2] } else { 1.0 },
    ];
    let xform_p = |v: [f32; 3]| {
        let r = quat_rotate(inst.rot, [v[0] * sc[0], v[1] * sc[1], v[2] * sc[2]]);
        [inst.pos[0] + r[0], inst.pos[1] + r[1], inst.pos[2] + r[2]]
    };
    let xform_n = |n: [f32; 3]| quat_rotate(inst.rot, [n[0] * inv_sc[0], n[1] * inv_sc[1], n[2] * inv_sc[2]]);

    for tri in mesh.indices.as_chunks::<3>().0 {
        let va = &mesh.vertices[tri[0] as usize];
        let vb = &mesh.vertices[tri[1] as usize];
        let vc = &mesh.vertices[tri[2] as usize];
        let a = xform_p(va.pos);
        let b = xform_p(vb.pos);
        let c = xform_p(vc.pos);

        let e1 = sub3(b, a);
        let e2 = sub3(c, a);
        let mut g = cross3(e1, e2);
        let glen = len3(g);
        if glen <= 1e-10 {
            continue; // skip degenerate zero-area triangles
        }
        g = [g[0] / glen, g[1] / glen, g[2] / glen];

        // Align geometric face normal to the outward vertex normals emitted by MeshBuilder
        // (guarantees consistent outward orientation across cuboid, sweep_tube, ellipsoid, cone, etc.).
        let na = xform_n(va.normal);
        let nb = xform_n(vb.normal);
        let nc = xform_n(vc.normal);
        let vn = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
        if dot3(g, vn) < 0.0 {
            g = [-g[0], -g[1], -g[2]];
        }

        let lo = [min_f32(a[0], min_f32(b[0], c[0])), min_f32(a[1], min_f32(b[1], c[1])), min_f32(a[2], min_f32(b[2], c[2]))];
        let hi = [max_f32(a[0], max_f32(b[0], c[0])), max_f32(a[1], max_f32(b[1], c[1])), max_f32(a[2], max_f32(b[2], c[2]))];
        let centroid = [(a[0] + b[0] + c[0]) * (1.0 / 3.0), (a[1] + b[1] + c[1]) * (1.0 / 3.0), (a[2] + b[2] + c[2]) * (1.0 / 3.0)];
        out.push(SdfTri { a, b, c, normal: g, lo, hi, centroid, material: va.material });
    }
}

const LEAF_TRIS: usize = 6;

fn build_bvh(tris: &mut [SdfTri], start: usize, end: usize, nodes: &mut Vec<BvhNode>) -> u32 {
    let mut lo = [f32::INFINITY; 3];
    let mut hi = [f32::NEG_INFINITY; 3];
    let mut clo = [f32::INFINITY; 3];
    let mut chi = [f32::NEG_INFINITY; 3];
    for t in &tris[start..end] {
        for a in 0..3 {
            lo[a] = min_f32(lo[a], t.lo[a]);
            hi[a] = max_f32(hi[a], t.hi[a]);
            clo[a] = min_f32(clo[a], t.centroid[a]);
            chi[a] = max_f32(chi[a], t.centroid[a]);
        }
    }
    let idx = nodes.len() as u32;
    let count = end - start;
    if count <= LEAF_TRIS {
        nodes.push(BvhNode { lo, hi, kind: NodeKind::Leaf { start: start as u32, count: count as u32 } });
        return idx;
    }
    // Longest centroid extent axis.
    let ext = [chi[0] - clo[0], chi[1] - clo[1], chi[2] - clo[2]];
    let axis = if ext[0] >= ext[1] && ext[0] >= ext[2] {
        0
    } else if ext[1] >= ext[2] {
        1
    } else {
        2
    };
    let mid = start + count / 2;
    tris[start..end].sort_by(|a, b| a.centroid[axis].total_cmp(&b.centroid[axis]));
    nodes.push(BvhNode { lo, hi, kind: NodeKind::Leaf { start: 0, count: 0 } });
    let left = build_bvh(tris, start, mid, nodes);
    let right = build_bvh(tris, mid, end, nodes);
    nodes[idx as usize].kind = NodeKind::Branch { left, right };
    idx
}

/// Möller–Trumbore ray-triangle intersection for `t > 0`.
/// Returns `Some((is_exit, near_edge))` where `is_exit = dot(dir, tri.normal) > 0`.
#[inline]
fn ray_triangle_signed(ro: [f32; 3], rd: [f32; 3], tri: &SdfTri) -> Option<(bool, bool)> {
    let e1 = sub3(tri.b, tri.a);
    let e2 = sub3(tri.c, tri.a);
    let h = cross3(rd, e2);
    let det = dot3(e1, h);
    if abs_f32(det) < 1e-8 {
        return None;
    }
    let inv_det = 1.0 / det;
    let s = sub3(ro, tri.a);
    let u = dot3(s, h) * inv_det;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = cross3(s, e1);
    let v = dot3(rd, q) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = dot3(e2, q) * inv_det;
    if t <= 0.0 {
        return None;
    }
    let nd = dot3(rd, tri.normal);
    let w = 1.0 - u - v;
    let eps = 1e-4;
    let near_edge = u < eps || v < eps || w < eps || abs_f32(nd) < 1e-4 || t < 1e-5;
    Some((nd > 0.0, near_edge))
}

#[inline]
fn ray_hits_aabb(ro: [f32; 3], inv: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> bool {
    let mut tmin = 0.0f32;
    let mut tmax = f32::INFINITY;
    for a in 0..3 {
        let t0 = (lo[a] - ro[a]) * inv[a];
        let t1 = (hi[a] - ro[a]) * inv[a];
        let lo_t = min_f32(t0, t1);
        let hi_t = max_f32(t0, t1);
        tmin = max_f32(tmin, lo_t);
        tmax = min_f32(tmax, hi_t);
        if tmin > tmax {
            return false;
        }
    }
    tmax >= 0.0
}

// Deterministic math helpers.

#[inline]
fn min_f32(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn max_f32(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

#[inline]
fn abs_f32(a: f32) -> f32 {
    if a < 0.0 { -a } else { a }
}

#[inline]
fn sub3(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [u[0] - v[0], u[1] - v[1], u[2] - v[2]]
}

#[inline]
fn dot3(u: [f32; 3], v: [f32; 3]) -> f32 {
    u[0] * v[0] + u[1] * v[1] + u[2] * v[2]
}

#[inline]
fn cross3(u: [f32; 3], v: [f32; 3]) -> [f32; 3] {
    [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]]
}

#[inline]
fn len3(v: [f32; 3]) -> f32 {
    dot3(v, v).sqrt()
}

#[inline]
fn quat_rotate(q: [f32; 4], v: [f32; 3]) -> [f32; 3] {
    let [qx, qy, qz, qw] = q;
    let tx = 2.0 * (qy * v[2] - qz * v[1]);
    let ty = 2.0 * (qz * v[0] - qx * v[2]);
    let tz = 2.0 * (qx * v[1] - qy * v[0]);
    [
        v[0] + qw * tx + (qy * tz - qz * ty),
        v[1] + qw * ty + (qz * tx - qx * tz),
        v[2] + qw * tz + (qx * ty - qy * tx),
    ]
}

#[inline]
fn point_to_aabb_distance(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let dx = max_f32(max_f32(lo[0] - p[0], 0.0), p[0] - hi[0]);
    let dy = max_f32(max_f32(lo[1] - p[1], 0.0), p[1] - hi[1]);
    let dz = max_f32(max_f32(lo[2] - p[2], 0.0), p[2] - hi[2]);
    (dx * dx + dy * dy + dz * dz).sqrt()
}

/// Exact unsigned Euclidean distance from point `p` to triangle `(a, b, c)` (Ericson, RTCD).
fn point_triangle_distance(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let dist = |u: [f32; 3], v: [f32; 3]| len3(sub3(u, v));
    let ab = sub3(b, a);
    let ac = sub3(c, a);
    let ap = sub3(p, a);
    let d1 = dot3(ab, ap);
    let d2 = dot3(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return dist(p, a);
    }
    let bp = sub3(p, b);
    let d3 = dot3(ab, bp);
    let d4 = dot3(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return dist(p, b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return dist(p, [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]]);
    }
    let cp = sub3(p, c);
    let d5 = dot3(ab, cp);
    let d6 = dot3(ac, cp);
    if d6 >= 0.0 && d5 <= d6 {
        return dist(p, c);
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return dist(p, [a[0] + w * ac[0], a[1] + w * ac[1], a[2] + w * ac[2]]);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return dist(p, [b[0] + w * (c[0] - b[0]), b[1] + w * (c[1] - b[1]), b[2] + w * (c[2] - b[2])]);
    }
    let denom = 1.0 / (va + vb + vc);
    let v = vb * denom;
    let w = vc * denom;
    dist(p, [a[0] + ab[0] * v + ac[0] * w, a[1] + ab[1] * v + ac[1] * w, a[2] + ab[2] * v + ac[2] * w])
}
