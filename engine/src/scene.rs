//! Runtime procedural scene state, dynamic instance collision, and crosshair targeting.
//!
//! Keeps track of an area's raw meshes, solid/dynamic instances, and dynamic lights, applies
//! validated `Command`s emitted by `tick` / `interact`, and wraps any `Region` (`Combined`) so
//! `Body::step` and `body::look` automatically walk on and collide with solid procedural meshes
//! (stairs, moving platforms, bridges, doors) alongside the SDF terrain!

use std::collections::HashMap;

use crate::{Answer, Command, Description, Instance, Light, Material, MeshData, Region, Result, Scene};

/// Precomputed local AABB and triangle data for one mesh, for fast collision and raycasting.
#[derive(Clone, Debug)]
struct MeshBounds {
    lo: [f32; 3],
    hi: [f32; 3],
    /// Triangles as `(a, b, c, material)`.
    tris: Vec<([f32; 3], [f32; 3], [f32; 3], Material)>,
}

impl MeshBounds {
    fn from_mesh(m: &MeshData) -> MeshBounds {
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        for v in &m.vertices {
            for a in 0..3 {
                lo[a] = lo[a].min(v.pos[a]);
                hi[a] = hi[a].max(v.pos[a]);
            }
        }
        if m.vertices.is_empty() {
            lo = [0.0; 3];
            hi = [0.0; 3];
        }
        let mut tris = Vec::with_capacity(m.indices.len() / 3);
        for t in m.indices.as_chunks::<3>().0 {
            let va = &m.vertices[t[0] as usize];
            let vb = &m.vertices[t[1] as usize];
            let vc = &m.vertices[t[2] as usize];
            tris.push((va.pos, vb.pos, vc.pos, va.material));
        }
        MeshBounds { lo, hi, tris }
    }
}

/// Live state of one placed instance (static or dynamic).
#[derive(Clone, Debug)]
pub struct LiveInstance {
    pub instance: Instance,
    pub visible: bool,
    pub world_lo: [f32; 3],
    pub world_hi: [f32; 3],
    /// Conservative minimum uniform scale for transforming local distances to world space.
    pub min_scale: f32,
}

impl LiveInstance {
    fn new(instance: Instance, bounds: &MeshBounds) -> LiveInstance {
        let mut live = LiveInstance { instance, visible: true, world_lo: [0.0; 3], world_hi: [0.0; 3], min_scale: 1.0 };
        live.recompute_aabb(bounds);
        live
    }

    fn recompute_aabb(&mut self, bounds: &MeshBounds) {
        let mut wlo = [f32::INFINITY; 3];
        let mut whi = [f32::NEG_INFINITY; 3];
        for sx in [bounds.lo[0], bounds.hi[0]] {
            for sy in [bounds.lo[1], bounds.hi[1]] {
                for sz in [bounds.lo[2], bounds.hi[2]] {
                    let scaled = [sx * self.instance.scale[0], sy * self.instance.scale[1], sz * self.instance.scale[2]];
                    let rot = quat_rotate(self.instance.rot, scaled);
                    for a in 0..3 {
                        let c = rot[a] + self.instance.pos[a];
                        wlo[a] = wlo[a].min(c);
                        whi[a] = whi[a].max(c);
                    }
                }
            }
        }
        self.world_lo = wlo;
        self.world_hi = whi;
        self.min_scale = self.instance.scale[0].abs().min(self.instance.scale[1].abs()).min(self.instance.scale[2].abs()).max(1e-4);
    }
}

/// Live procedural scene state + collision & raycast queries.
#[derive(Clone, Debug, Default)]
pub struct SceneWorld {
    bounds: Vec<MeshBounds>,
    /// All solid static instances (checked for physics collision).
    solid_static: Vec<LiveInstance>,
    /// All dynamic instances (`id > 0`), keyed by id.
    pub dynamic: HashMap<u32, LiveInstance>,
    /// Active point lights, keyed by id.
    pub lights: HashMap<u32, Light>,
}

/// Side effects from applying a batch of runtime WASM `Command`s that the player/renderer should act on.
#[derive(Clone, Debug, Default)]
pub struct Applied {
    pub impulse: [f32; 3],
    pub said: Option<String>,
    pub dirty_instances: Vec<u32>,
    pub dirty_lights: Vec<u32>,
}

impl SceneWorld {
    pub fn from_scene(scene: &Scene) -> SceneWorld {
        let bounds: Vec<MeshBounds> = scene.meshes.iter().map(MeshBounds::from_mesh).collect();
        let mut solid_static = Vec::new();
        let mut dynamic = HashMap::new();
        for &inst in &scene.instances {
            let Some(b) = bounds.get(inst.mesh as usize) else { continue };
            let live = LiveInstance::new(inst, b);
            if inst.id > 0 {
                dynamic.insert(inst.id, live);
            } else if inst.solid {
                solid_static.push(live);
            }
        }
        let mut lights = HashMap::new();
        for &l in &scene.lights {
            if l.id > 0 && l.intensity > 0.0 {
                lights.insert(l.id, l);
            }
        }
        SceneWorld { bounds, solid_static, dynamic, lights }
    }

    /// Applies validated runtime `Command`s from `tick` / `interact` to the live scene state.
    pub fn apply(&mut self, commands: &[Command]) -> Applied {
        let mut applied = Applied::default();
        for cmd in commands {
            match cmd {
                Command::SetTransform(inst) => {
                    if inst.id == 0 {
                        continue;
                    }
                    if let Some(live) = self.dynamic.get_mut(&inst.id) {
                        let mesh_idx = if (inst.mesh as usize) < self.bounds.len() { inst.mesh } else { live.instance.mesh };
                        live.instance = Instance { mesh: mesh_idx, ..*inst };
                        if let Some(b) = self.bounds.get(mesh_idx as usize) {
                            live.recompute_aabb(b);
                        }
                        applied.dirty_instances.push(inst.id);
                    }
                }
                Command::SetVisible(id, vis) => {
                    if let Some(live) = self.dynamic.get_mut(id) {
                        live.visible = *vis;
                        applied.dirty_instances.push(*id);
                    }
                }
                Command::SetSolid(id, sol) => {
                    if let Some(live) = self.dynamic.get_mut(id) {
                        live.instance.solid = *sol;
                    }
                }
                Command::ApplyImpulse(dv) => {
                    for a in 0..3 {
                        applied.impulse[a] += dv[a];
                    }
                }
                Command::SetLight(l) => {
                    if l.id > 0 {
                        if l.intensity <= 0.0 {
                            self.lights.remove(&l.id);
                        } else {
                            self.lights.insert(l.id, *l);
                        }
                        applied.dirty_lights.push(l.id);
                    }
                }
                Command::Say(msg) => {
                    applied.said = Some(msg.clone());
                }
            }
        }
        applied
    }

    /// Whether there are any solid procedural instances to collide with.
    pub fn has_solids(&self) -> bool {
        !self.solid_static.is_empty() || self.dynamic.values().any(|l| l.visible && l.instance.solid)
    }

    /// Unsigned distance from `p` to the nearest solid procedural mesh triangle (fast AABB reject).
    pub fn distance_to_solids(&self, p: [f32; 3], best: f32) -> Option<Answer> {
        let mut d_best = best;
        let mut hit: Option<Answer> = None;
        for live in self.solid_static.iter().chain(self.dynamic.values().filter(|l| l.visible && l.instance.solid)) {
            let box_d = aabb_distance(p, live.world_lo, live.world_hi);
            if box_d >= d_best {
                continue;
            }
            let Some(b) = self.bounds.get(live.instance.mesh as usize) else { continue };
            let local = world_to_local(p, &live.instance);
            for &(ta, tb, tc, mat) in &b.tris {
                let d_local = point_triangle_distance(local, ta, tb, tc);
                let d_world = d_local * live.min_scale;
                if d_world < d_best {
                    d_best = d_world;
                    hit = Some(Answer { distance: d_world, material: mat });
                }
            }
        }
        hit
    }

    /// Raycasts against visible dynamic instances (`id > 0`) up to `max_dist`, returning `(id, hit_t)`.
    pub fn raycast_dynamic(&self, origin: [f32; 3], dir: [f32; 3], max_dist: f32) -> Option<(u32, f32)> {
        let mut best_t = max_dist;
        let mut best_id = None;
        for (&id, live) in &self.dynamic {
            if !live.visible {
                continue;
            }
            if let Some(t) = ray_aabb(origin, dir, live.world_lo, live.world_hi, best_t) {
                best_t = t;
                best_id = Some(id);
            }
        }
        best_id.map(|id| (id, best_t))
    }
}

/// Combines an SDF `Region` with `SceneWorld`'s solid procedural instances so `Body::step` and
/// `body::look` walk on and collide with solid procedural meshes automatically.
pub struct Combined<'a, R: Region + ?Sized> {
    pub region: &'a mut R,
    pub world: &'a SceneWorld,
}

impl<R: Region + ?Sized> Region for Combined<'_, R> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        self.region.sample(points, size, out)?;
        if !self.world.has_solids() {
            return Ok(());
        }
        for (&p, ans) in points.iter().zip(out.iter_mut()) {
            if ans.distance > 0.0
                && let Some(hit) = self.world.distance_to_solids(p, ans.distance)
            {
                *ans = hit;
            }
        }
        Ok(())
    }

    fn describe(&mut self) -> Result<Description> {
        self.region.describe()
    }
}

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

fn world_to_local(p: [f32; 3], inst: &Instance) -> [f32; 3] {
    let d = [p[0] - inst.pos[0], p[1] - inst.pos[1], p[2] - inst.pos[2]];
    let inv_q = [-inst.rot[0], -inst.rot[1], -inst.rot[2], inst.rot[3]];
    let r = quat_rotate(inv_q, d);
    [
        r[0] / inst.scale[0].abs().max(1e-5).copysign(inst.scale[0]),
        r[1] / inst.scale[1].abs().max(1e-5).copysign(inst.scale[1]),
        r[2] / inst.scale[2].abs().max(1e-5).copysign(inst.scale[2]),
    ]
}

fn aabb_distance(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
    let mut s = 0.0;
    for a in 0..3 {
        let d = (lo[a] - p[a]).max(0.0).max(p[a] - hi[a]);
        s += d * d;
    }
    s.sqrt()
}

fn ray_aabb(ro: [f32; 3], rd: [f32; 3], lo: [f32; 3], hi: [f32; 3], max_t: f32) -> Option<f32> {
    let (mut tmin, mut tmax) = (0.0f32, max_t);
    for a in 0..3 {
        if rd[a].abs() < 1e-6 {
            if ro[a] < lo[a] || ro[a] > hi[a] {
                return None;
            }
        } else {
            let inv = 1.0 / rd[a];
            let mut t0 = (lo[a] - ro[a]) * inv;
            let mut t1 = (hi[a] - ro[a]) * inv;
            if t0 > t1 {
                std::mem::swap(&mut t0, &mut t1);
            }
            tmin = tmin.max(t0);
            tmax = tmax.min(t1);
            if tmin > tmax {
                return None;
            }
        }
    }
    Some(tmin)
}

/// Exact unsigned Euclidean distance from point `p` to triangle `(a, b, c)` (Ericson, RTCD).
fn point_triangle_distance(p: [f32; 3], a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let sub = |u: [f32; 3], v: [f32; 3]| [u[0] - v[0], u[1] - v[1], u[2] - v[2]];
    let dot = |u: [f32; 3], v: [f32; 3]| u[0] * v[0] + u[1] * v[1] + u[2] * v[2];
    let dist = |u: [f32; 3], v: [f32; 3]| {
        let d = sub(u, v);
        dot(d, d).sqrt()
    };
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let d1 = dot(ab, ap);
    let d2 = dot(ac, ap);
    if d1 <= 0.0 && d2 <= 0.0 {
        return dist(p, a);
    }
    let bp = sub(p, b);
    let d3 = dot(ab, bp);
    let d4 = dot(ac, bp);
    if d3 >= 0.0 && d4 <= d3 {
        return dist(p, b);
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return dist(p, [a[0] + v * ab[0], a[1] + v * ab[1], a[2] + v * ab[2]]);
    }
    let cp = sub(p, c);
    let d5 = dot(ab, cp);
    let d6 = dot(ac, cp);
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
