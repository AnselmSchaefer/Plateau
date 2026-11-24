//! Open procedural mesh & scene workshop for Plateau areas (`plateau_sdk::mesh`).
//!
//! Nothing here is a closed prefab kit: `MeshBuilder` emits raw vertices & triangles, with
//! mathematical workshop tools (`sweep_tube`, `ribbon`, `lathe`, `extrude`, `capsule`, `ellipsoid`,
//! `cylinder`, `cone`, `cuboid`) so an area can invent any plant, architecture, contraption, or
//! tool from scratch, then instance it anywhere via `SceneBuilder`.

use crate::{Instance, Light, Material, MeshData, PI, Point, Scene, TAU, Vertex, abs, cos, point, sin};

/// A unit quaternion `(x, y, z, w)` for deterministic rotations in WASM.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quat {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub w: f32,
}

impl Quat {
    pub const IDENTITY: Quat = Quat { x: 0.0, y: 0.0, z: 0.0, w: 1.0 };

    pub fn from_axis_angle(axis: Point, angle: f32) -> Quat {
        let len = axis.length();
        if len < 1e-6 {
            return Quat::IDENTITY;
        }
        let half = angle * 0.5;
        let s = sin(half) / len;
        Quat { x: axis.x * s, y: axis.y * s, z: axis.z * s, w: cos(half) }
    }

    pub fn from_y(yaw: f32) -> Quat {
        let half = yaw * 0.5;
        Quat { x: 0.0, y: sin(half), z: 0.0, w: cos(half) }
    }

    pub fn from_euler(yaw: f32, pitch: f32, roll: f32) -> Quat {
        let qy = Quat::from_y(yaw);
        let qp = Quat::from_axis_angle(point(1.0, 0.0, 0.0), pitch);
        let qr = Quat::from_axis_angle(point(0.0, 0.0, 1.0), roll);
        qy.mul(qp).mul(qr)
    }

    pub fn mul(self, o: Quat) -> Quat {
        Quat {
            x: self.w * o.x + self.x * o.w + self.y * o.z - self.z * o.y,
            y: self.w * o.y - self.x * o.z + self.y * o.w + self.z * o.x,
            z: self.w * o.z + self.x * o.y - self.y * o.x + self.z * o.w,
            w: self.w * o.w - self.x * o.x - self.y * o.y - self.z * o.z,
        }
    }

    pub fn rotate(self, p: Point) -> Point {
        let u = point(self.x, self.y, self.z);
        let t = u.cross(p) * 2.0;
        p + t * self.w + u.cross(t)
    }
}

fn norm(v: Point) -> Point {
    let l = v.length();
    if l > 1e-6 { v / l } else { point(0.0, 1.0, 0.0) }
}

/// Builds a raw procedural triangle mesh from scratch or using geometric workshop tools.
#[derive(Clone, Debug, Default)]
pub struct MeshBuilder {
    vertices: Vec<Vertex>,
    indices: Vec<u32>,
    tint: [u8; 3],
}

impl MeshBuilder {
    pub fn new() -> MeshBuilder {
        MeshBuilder { vertices: Vec::new(), indices: Vec::new(), tint: [255, 255, 255] }
    }

    /// Sets the RGB tint multiplier (0..255, default pure white 255) for subsequent vertices.
    pub fn tint(&mut self, r: u8, g: u8, b: u8) -> &mut Self {
        self.tint = [r, g, b];
        self
    }

    /// Resets the vertex tint back to pure material colour.
    pub fn clear_tint(&mut self) -> &mut Self {
        self.tint = [255, 255, 255];
        self
    }

    /// Adds a single raw vertex and returns its index.
    pub fn vertex(&mut self, pos: Point, normal: Point, material: Material) -> u32 {
        let id = self.vertices.len() as u32;
        self.vertices.push(Vertex {
            pos,
            normal: norm(normal),
            material,
            tint_r: self.tint[0],
            tint_g: self.tint[1],
            tint_b: self.tint[2],
        });
        id
    }

    /// Adds a counter-clockwise triangle.
    pub fn tri(&mut self, a: u32, b: u32, c: u32) -> &mut Self {
        self.indices.extend_from_slice(&[a, b, c]);
        self
    }

    /// Adds a counter-clockwise quad split into two triangles.
    pub fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) -> &mut Self {
        self.indices.extend_from_slice(&[a, b, c, a, c, d]);
        self
    }

    /// Adds a flat-shaded triangle, computing its face normal automatically.
    pub fn flat_tri(&mut self, a: Point, b: Point, c: Point, material: Material) -> &mut Self {
        let n = norm((b - a).cross(c - a));
        let ia = self.vertex(a, n, material);
        let ib = self.vertex(b, n, material);
        let ic = self.vertex(c, n, material);
        self.tri(ia, ib, ic)
    }

    /// Sweeps a smooth circular tube along a 3D path of `(point, radius)` knots — great for curved
    /// reed stalks, branches, vines, arches, roots, handles, or horns.
    pub fn sweep_tube(&mut self, knots: &[(Point, f32)], sides: usize, cap_ends: bool, material: Material) -> &mut Self {
        if knots.len() < 2 || sides < 3 {
            return self;
        }
        let mut rings: Vec<Vec<u32>> = Vec::with_capacity(knots.len());
        let mut prev_u = point(1.0, 0.0, 0.0);
        for (i, &(p, r)) in knots.iter().enumerate() {
            let tangent = if i == 0 {
                norm(knots[1].0 - p)
            } else if i + 1 == knots.len() {
                norm(p - knots[i - 1].0)
            } else {
                norm(knots[i + 1].0 - knots[i - 1].0)
            };
            // Parallel-transport frame so curved tubes never twist.
            let mut u = prev_u - tangent * prev_u.dot(tangent);
            if u.length() < 1e-4 {
                let helper = if abs(tangent.y) < 0.9 { point(0.0, 1.0, 0.0) } else { point(1.0, 0.0, 0.0) };
                u = tangent.cross(helper);
            }
            u = norm(u);
            prev_u = u;
            let v = norm(tangent.cross(u));
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let dir = u * cos(th) + v * sin(th);
                ring.push(self.vertex(p + dir * r, dir, material));
            }
            rings.push(ring);
        }
        for i in 0..rings.len() - 1 {
            for s in 0..sides {
                let n = (s + 1) % sides;
                self.quad(rings[i][s], rings[i][n], rings[i + 1][n], rings[i + 1][s]);
            }
        }
        if cap_ends {
            let t0 = norm(knots[1].0 - knots[0].0);
            let c0 = self.vertex(knots[0].0, -t0, material);
            let last = knots.len() - 1;
            let t1 = norm(knots[last].0 - knots[last - 1].0);
            let c1 = self.vertex(knots[last].0, t1, material);
            for s in 0..sides {
                let n = (s + 1) % sides;
                self.tri(c0, rings[0][n], rings[0][s]);
                self.tri(c1, rings[last][s], rings[last][n]);
            }
        }
        self
    }

    /// Sweeps a curved two-sided ribbon along a path of `(center, half_width, normal_hint)` — great
    /// for grass blades, reed leaves, fern fronds, flower petals, sails, or flags.
    pub fn ribbon(&mut self, knots: &[(Point, f32, Point)], material: Material) -> &mut Self {
        if knots.len() < 2 {
            return self;
        }
        let mut lefts = Vec::with_capacity(knots.len());
        let mut rights = Vec::with_capacity(knots.len());
        for (i, &(p, hw, n_hint)) in knots.iter().enumerate() {
            let tangent = if i == 0 {
                norm(knots[1].0 - p)
            } else if i + 1 == knots.len() {
                norm(p - knots[i - 1].0)
            } else {
                norm(knots[i + 1].0 - knots[i - 1].0)
            };
            let n = norm(n_hint);
            let side = norm(tangent.cross(n));
            let real_n = norm(side.cross(tangent));
            lefts.push(self.vertex(p - side * hw, real_n, material));
            rights.push(self.vertex(p + side * hw, real_n, material));
        }
        for i in 0..knots.len() - 1 {
            self.quad(lefts[i], rights[i], rights[i + 1], lefts[i + 1]);
        }
        self
    }

    /// Spins a 2D `(radius, y)` profile around the Y axis — great for mushroom caps, pottery,
    /// bells, lantern domes, pillars, or lighthouse towers.
    pub fn lathe(&mut self, origin: Point, profile: &[(f32, f32)], sides: usize, material: Material) -> &mut Self {
        if profile.len() < 2 || sides < 3 {
            return self;
        }
        let mut rings: Vec<Vec<u32>> = Vec::with_capacity(profile.len());
        for (i, &(r, y)) in profile.iter().enumerate() {
            let (dr, dy) = if i == 0 {
                (profile[1].0 - r, profile[1].1 - y)
            } else if i + 1 == profile.len() {
                (r - profile[i - 1].0, y - profile[i - 1].1)
            } else {
                (profile[i + 1].0 - profile[i - 1].0, profile[i + 1].1 - profile[i - 1].1)
            };
            // Outward normal in (r, y) plane: (dy, -dr).
            let n_len = (dy * dy + dr * dr).sqrt();
            let (nr, ny) = if n_len > 1e-6 { (dy / n_len, -dr / n_len) } else { (1.0, 0.0) };
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let (c, sn) = (cos(th), sin(th));
                let pos = origin + point(r * c, y, r * sn);
                let n = point(nr * c, ny, nr * sn);
                ring.push(self.vertex(pos, n, material));
            }
            rings.push(ring);
        }
        for i in 0..rings.len() - 1 {
            for s in 0..sides {
                let n = (s + 1) % sides;
                self.quad(rings[i][s], rings[i][n], rings[i + 1][n], rings[i + 1][s]);
            }
        }
        self
    }

    /// Extrudes a closed convex/star 2D `(x, z)` polygon vertically from `y0` to `y1` — great for
    /// planks, steps, stone slabs, roofs, and architectural trim.
    pub fn extrude(&mut self, poly: &[(f32, f32)], y0: f32, y1: f32, material: Material) -> &mut Self {
        let n = poly.len();
        if n < 3 {
            return self;
        }
        let (mut cx, mut cz) = (0.0, 0.0);
        for &(x, z) in poly {
            cx += x;
            cz += z;
        }
        cx /= n as f32;
        cz /= n as f32;
        let top_c = self.vertex(point(cx, y1, cz), point(0.0, 1.0, 0.0), material);
        let bot_c = self.vertex(point(cx, y0, cz), point(0.0, -1.0, 0.0), material);
        let mut top = Vec::with_capacity(n);
        let mut bot = Vec::with_capacity(n);
        for &(x, z) in poly {
            top.push(self.vertex(point(x, y1, z), point(0.0, 1.0, 0.0), material));
            bot.push(self.vertex(point(x, y0, z), point(0.0, -1.0, 0.0), material));
        }
        for i in 0..n {
            let j = (i + 1) % n;
            self.tri(top_c, top[j], top[i]);
            self.tri(bot_c, bot[i], bot[j]);
            let (ax, az) = poly[i];
            let (bx, bz) = poly[j];
            let fn_dir = norm(point(bz - az, 0.0, -(bx - ax)));
            let a0 = self.vertex(point(ax, y0, az), fn_dir, material);
            let b0 = self.vertex(point(bx, y0, bz), fn_dir, material);
            let b1 = self.vertex(point(bx, y1, bz), fn_dir, material);
            let a1 = self.vertex(point(ax, y1, az), fn_dir, material);
            self.quad(a0, b0, b1, a1);
        }
        self
    }

    /// Adds a smooth capsule from `a` to `b` of radius `r`.
    pub fn capsule(&mut self, a: Point, b: Point, r: f32, sides: usize, material: Material) -> &mut Self {
        if r <= 1e-5 || sides < 3 {
            return self;
        }
        let axis = norm(b - a);
        let helper = if abs(axis.y) < 0.9 { point(0.0, 1.0, 0.0) } else { point(1.0, 0.0, 0.0) };
        let u = norm(axis.cross(helper));
        let v = axis.cross(u);
        let half_pi = PI * 0.5;
        let cap = 2usize;
        let mut rings: Vec<Vec<u32>> = Vec::new();
        let bot_pole = self.vertex(a - axis * r, -axis, material);
        for k in 1..=cap {
            let phi = -half_pi + (k as f32 / cap as f32) * half_pi;
            let (sp, cp) = (sin(phi), cos(phi));
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let dir = (u * cos(th) + v * sin(th)) * cp + axis * sp;
                ring.push(self.vertex(a + dir * r, dir, material));
            }
            rings.push(ring);
        }
        for k in 0..cap {
            let phi = (k as f32 / cap as f32) * half_pi;
            let (sp, cp) = (sin(phi), cos(phi));
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let dir = (u * cos(th) + v * sin(th)) * cp + axis * sp;
                ring.push(self.vertex(b + dir * r, dir, material));
            }
            rings.push(ring);
        }
        let top_pole = self.vertex(b + axis * r, axis, material);
        for s in 0..sides {
            let n = (s + 1) % sides;
            self.tri(bot_pole, rings[0][n], rings[0][s]);
            let last = rings.len() - 1;
            self.tri(top_pole, rings[last][s], rings[last][n]);
        }
        for r_idx in 0..rings.len() - 1 {
            for s in 0..sides {
                let n = (s + 1) % sides;
                self.quad(rings[r_idx][s], rings[r_idx][n], rings[r_idx + 1][n], rings[r_idx + 1][s]);
            }
        }
        self
    }

    /// Adds a smooth ellipsoid centred at `center` with half-sizes `radii`.
    pub fn ellipsoid(&mut self, center: Point, radii: Point, sides: usize, stacks: usize, material: Material) -> &mut Self {
        if radii.x <= 1e-5 || radii.y <= 1e-5 || radii.z <= 1e-5 || sides < 3 || stacks < 2 {
            return self;
        }
        let half_pi = PI * 0.5;
        let bot = self.vertex(center - point(0.0, radii.y, 0.0), point(0.0, -1.0, 0.0), material);
        let top = self.vertex(center + point(0.0, radii.y, 0.0), point(0.0, 1.0, 0.0), material);
        let mut rings: Vec<Vec<u32>> = Vec::with_capacity(stacks - 1);
        for st in 1..stacks {
            let phi = -half_pi + (st as f32 / stacks as f32) * PI;
            let (sp, cp) = (sin(phi), cos(phi));
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let (ux, uy, uz) = (cp * cos(th), sp, cp * sin(th));
                let p = center + point(ux * radii.x, uy * radii.y, uz * radii.z);
                let n = point(ux / radii.x, uy / radii.y, uz / radii.z);
                ring.push(self.vertex(p, n, material));
            }
            rings.push(ring);
        }
        for s in 0..sides {
            let n = (s + 1) % sides;
            self.tri(bot, rings[0][n], rings[0][s]);
            let last = rings.len() - 1;
            self.tri(top, rings[last][s], rings[last][n]);
        }
        for r_idx in 0..rings.len() - 1 {
            for s in 0..sides {
                let n = (s + 1) % sides;
                self.quad(rings[r_idx][s], rings[r_idx][n], rings[r_idx + 1][n], rings[r_idx + 1][s]);
            }
        }
        self
    }

    /// Adds a smooth cone with base centred at `foot`, radius `r`, and `height` (negative points down).
    pub fn cone(&mut self, foot: Point, height: f32, r: f32, sides: usize, material: Material) -> &mut Self {
        if r <= 1e-5 || abs(height) <= 1e-5 || sides < 3 {
            return self;
        }
        let sign = if height >= 0.0 { 1.0 } else { -1.0 };
        let slope_y = r / abs(height);
        let base_c = self.vertex(foot, point(0.0, -sign, 0.0), material);
        let mut base_ring = Vec::with_capacity(sides);
        let mut side_ring = Vec::with_capacity(sides);
        for s in 0..sides {
            let a = s as f32 * TAU / sides as f32;
            let dir = point(cos(a), 0.0, sin(a));
            let p = foot + dir * r;
            base_ring.push(self.vertex(p, point(0.0, -sign, 0.0), material));
            side_ring.push(self.vertex(p, dir + point(0.0, slope_y * sign, 0.0), material));
        }
        let tip = self.vertex(foot + point(0.0, height, 0.0), point(0.0, sign, 0.0), material);
        for s in 0..sides {
            let n = (s + 1) % sides;
            if sign > 0.0 {
                self.tri(side_ring[s], side_ring[n], tip);
                self.tri(base_c, base_ring[s], base_ring[n]);
            } else {
                self.tri(side_ring[n], side_ring[s], tip);
                self.tri(base_c, base_ring[n], base_ring[s]);
            }
        }
        self
    }

    /// Adds an axis-aligned box centred at `center` with half-sizes `half`.
    pub fn cuboid(&mut self, center: Point, half: Point, material: Material) -> &mut Self {
        let faces = [
            (point(1.0, 0.0, 0.0), point(0.0, 1.0, 0.0), point(0.0, 0.0, 1.0)),
            (point(-1.0, 0.0, 0.0), point(0.0, 1.0, 0.0), point(0.0, 0.0, -1.0)),
            (point(0.0, 1.0, 0.0), point(0.0, 0.0, -1.0), point(1.0, 0.0, 0.0)),
            (point(0.0, -1.0, 0.0), point(0.0, 0.0, 1.0), point(1.0, 0.0, 0.0)),
            (point(0.0, 0.0, 1.0), point(0.0, 1.0, 0.0), point(-1.0, 0.0, 0.0)),
            (point(0.0, 0.0, -1.0), point(0.0, 1.0, 0.0), point(1.0, 0.0, 0.0)),
        ];
        let sc = |v: Point| point(v.x * half.x, v.y * half.y, v.z * half.z);
        for (n, u, v) in faces {
            let c = center + sc(n);
            let du = sc(u);
            let dv = sc(v);
            let a = self.vertex(c - du - dv, n, material);
            let b = self.vertex(c + du - dv, n, material);
            let d = self.vertex(c + du + dv, n, material);
            let e = self.vertex(c - du + dv, n, material);
            self.quad(a, b, d, e);
        }
        self
    }

    pub fn build(self) -> MeshData {
        MeshData { vertices: self.vertices, indices: self.indices }
    }
}

/// High-level scene builder: registers reusable procedural meshes once and places static or
/// dynamic instances and point lights.
#[derive(Clone, Debug, Default)]
pub struct SceneBuilder {
    meshes: Vec<MeshData>,
    instances: Vec<Instance>,
    lights: Vec<Light>,
}

impl SceneBuilder {
    pub fn new() -> SceneBuilder {
        SceneBuilder::default()
    }

    /// Registers a procedural mesh and returns its handle (`u32`) for instancing.
    pub fn add_mesh(&mut self, mesh: MeshData) -> u32 {
        let id = self.meshes.len() as u32;
        self.meshes.push(mesh);
        id
    }

    /// Places a static instance (`id = 0`) of `mesh` (batched on the GPU).
    pub fn place(&mut self, mesh: u32, pos: Point, rot: Quat, scale: Point, solid: bool) -> &mut Self {
        self.instances.push(Instance {
            id: 0,
            mesh,
            pos,
            rot_x: rot.x,
            rot_y: rot.y,
            rot_z: rot.z,
            rot_w: rot.w,
            scale,
            solid,
        });
        self
    }

    /// Spawns a dynamic instance (`id > 0`) that `tick` and `interact` can animate or toggle.
    pub fn spawn(&mut self, id: u32, mesh: u32, pos: Point, rot: Quat, scale: Point, solid: bool) -> &mut Self {
        self.instances.push(Instance {
            id,
            mesh,
            pos,
            rot_x: rot.x,
            rot_y: rot.y,
            rot_z: rot.z,
            rot_w: rot.w,
            scale,
            solid,
        });
        self
    }

    /// Adds a point light (`id > 0` so `tick` / `interact` can update or toggle it).
    pub fn light(&mut self, id: u32, pos: Point, color: [f32; 3], intensity: f32, radius: f32) -> &mut Self {
        self.lights.push(Light { id, pos, r: color[0], g: color[1], b: color[2], intensity, radius });
        self
    }

    pub fn build(self) -> Scene {
        Scene { meshes: self.meshes, instances: self.instances, lights: self.lights }
    }
}

/// Convenience constructor for `Instance` (useful inside `Command::SetTransform`).
pub fn instance(id: u32, mesh: u32, pos: Point, rot: Quat, scale: Point, solid: bool) -> Instance {
    Instance {
        id,
        mesh,
        pos,
        rot_x: rot.x,
        rot_y: rot.y,
        rot_z: rot.z,
        rot_w: rot.w,
        scale,
        solid,
    }
}

/// Convenience constructor for `Light` (useful inside `Command::SetLight`).
pub fn light(id: u32, pos: Point, color: [f32; 3], intensity: f32, radius: f32) -> Light {
    Light { id, pos, r: color[0], g: color[1], b: color[2], intensity, radius }
}
