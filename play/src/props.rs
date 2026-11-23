//! Smooth GPU-rendered props emitted once by an area's `.wasm` (`Region::props`, POC_PLAN.md step 7).
//!
//! All props are tessellated once on load into a single merged Bevy `Mesh` (1 draw call) using the
//! exact same vertex attributes and `SurfaceMaterial` as terrain chunks, so reeds, cattails,
//! meadow tussocks, flowers, sticks, and mushrooms stay razor-sharp at every distance with zero
//! voxel aliasing or LOD morphing.

use std::f32::consts::{FRAC_PI_2, TAU};

use bevy::asset::RenderAssetUsages;
use bevy::mesh::{Indices, PrimitiveTopology};
use bevy::prelude::*;
use plateau_engine::picture::colour;
use plateau_engine::{Prop, PropKind};

use crate::surface::{SurfaceMaterial, properties};

#[derive(Component)]
pub struct PropsMesh;

struct Builder {
    positions: Vec<[f32; 3]>,
    normals: Vec<[f32; 3]>,
    colors: Vec<[f32; 4]>,
    uvs: Vec<[f32; 2]>,
    indices: Vec<u32>,
}

impl Builder {
    fn new() -> Self {
        Self { positions: Vec::new(), normals: Vec::new(), colors: Vec::new(), uvs: Vec::new(), indices: Vec::new() }
    }

    fn push_vert(&mut self, p: Vec3, n: Vec3, col: [f32; 4], uv: [f32; 2]) -> u32 {
        let id = self.positions.len() as u32;
        self.positions.push(p.to_array());
        self.normals.push(n.normalize_or(Vec3::Y).to_array());
        self.colors.push(col);
        self.uvs.push(uv);
        id
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    fn add_capsule(&mut self, a: Vec3, b: Vec3, r: f32, col: [f32; 4], uv: [f32; 2]) {
        if r <= 1e-5 {
            return;
        }
        let ab = b - a;
        let len = ab.length();
        let axis = if len > 1e-6 { ab / len } else { Vec3::Y };
        let helper = if axis.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
        let u = axis.cross(helper).normalize();
        let v = axis.cross(u);

        let sides = 10usize;
        let cap_rings = 2usize;
        // Build rings from bottom pole -> bottom hemisphere -> top hemisphere -> top pole.
        let mut rings: Vec<Vec<u32>> = Vec::new();
        let bot_pole = self.push_vert(a - axis * r, -axis, col, uv);
        for k in 1..=cap_rings {
            let phi = -FRAC_PI_2 + (k as f32 / cap_rings as f32) * FRAC_PI_2;
            let (sp, cp) = (phi.sin(), phi.cos());
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let dir = (u * th.cos() + v * th.sin()) * cp + axis * sp;
                ring.push(self.push_vert(a + dir * r, dir, col, uv));
            }
            rings.push(ring);
        }
        for k in 0..cap_rings {
            let phi = (k as f32 / cap_rings as f32) * FRAC_PI_2;
            let (sp, cp) = (phi.sin(), phi.cos());
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let dir = (u * th.cos() + v * th.sin()) * cp + axis * sp;
                ring.push(self.push_vert(b + dir * r, dir, col, uv));
            }
            rings.push(ring);
        }
        let top_pole = self.push_vert(b + axis * r, axis, col, uv);

        for s in 0..sides {
            let n = (s + 1) % sides;
            self.tri(bot_pole, rings[0][n], rings[0][s]);
            let last = rings.len() - 1;
            self.tri(top_pole, rings[last][s], rings[last][n]);
        }
        for r_idx in 0..rings.len() - 1 {
            for s in 0..sides {
                let n = (s + 1) % sides;
                let (a0, a1) = (rings[r_idx][s], rings[r_idx][n]);
                let (b0, b1) = (rings[r_idx + 1][s], rings[r_idx + 1][n]);
                self.tri(a0, a1, b1);
                self.tri(a0, b1, b0);
            }
        }
    }

    fn add_ellipsoid(&mut self, center: Vec3, radii: Vec3, yaw: f32, col: [f32; 4], uv: [f32; 2]) {
        if radii.min_element() <= 1e-5 {
            return;
        }
        let rot = Quat::from_rotation_y(yaw);
        let inv_r = Vec3::new(1.0 / radii.x, 1.0 / radii.y, 1.0 / radii.z);
        let sides = 12usize;
        let stacks = 6usize;
        let bot = self.push_vert(center + rot * Vec3::new(0.0, -radii.y, 0.0), rot * Vec3::NEG_Y, col, uv);
        let top = self.push_vert(center + rot * Vec3::new(0.0, radii.y, 0.0), rot * Vec3::Y, col, uv);
        let mut rings: Vec<Vec<u32>> = Vec::with_capacity(stacks - 1);
        for st in 1..stacks {
            let phi = -FRAC_PI_2 + (st as f32 / stacks as f32) * std::f32::consts::PI;
            let (sp, cp) = (phi.sin(), phi.cos());
            let mut ring = Vec::with_capacity(sides);
            for s in 0..sides {
                let th = s as f32 * TAU / sides as f32;
                let unit = Vec3::new(cp * th.cos(), sp, cp * th.sin());
                let p = center + rot * (unit * radii);
                let n = (rot * (unit * inv_r)).normalize_or(Vec3::Y);
                ring.push(self.push_vert(p, n, col, uv));
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
                let (a0, a1) = (rings[r_idx][s], rings[r_idx][n]);
                let (b0, b1) = (rings[r_idx + 1][s], rings[r_idx + 1][n]);
                self.tri(a0, a1, b1);
                self.tri(a0, b1, b0);
            }
        }
    }

    fn add_cylinder(&mut self, center: Vec3, r: f32, half_h: f32, notch: Option<f32>, col: [f32; 4], uv: [f32; 2]) {
        if r <= 1e-5 || half_h <= 1e-5 {
            return;
        }
        let sides = 18usize;
        let (start_a, span) = match notch {
            Some(yaw) => (yaw + 0.18, TAU - 0.36),
            None => (0.0, TAU),
        };
        let steps = if notch.is_some() { sides + 1 } else { sides };
        let top_c = self.push_vert(center + Vec3::Y * half_h, Vec3::Y, col, uv);
        let bot_c = self.push_vert(center - Vec3::Y * half_h, Vec3::NEG_Y, col, uv);
        let mut top_ring = Vec::with_capacity(steps);
        let mut bot_ring = Vec::with_capacity(steps);
        let mut side_top = Vec::with_capacity(steps);
        let mut side_bot = Vec::with_capacity(steps);
        for i in 0..steps {
            let frac = if notch.is_some() { i as f32 / sides as f32 } else { i as f32 / sides as f32 };
            let a = start_a + frac * span;
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let pt = center + dir * r + Vec3::Y * half_h;
            let pb = center + dir * r - Vec3::Y * half_h;
            top_ring.push(self.push_vert(pt, Vec3::Y, col, uv));
            bot_ring.push(self.push_vert(pb, Vec3::NEG_Y, col, uv));
            side_top.push(self.push_vert(pt, dir, col, uv));
            side_bot.push(self.push_vert(pb, dir, col, uv));
        }
        for i in 0..sides {
            let n = if notch.is_some() { i + 1 } else { (i + 1) % sides };
            self.tri(top_c, top_ring[n], top_ring[i]);
            self.tri(bot_c, bot_ring[i], bot_ring[n]);
            self.tri(side_bot[i], side_bot[n], side_top[n]);
            self.tri(side_bot[i], side_top[n], side_top[i]);
        }
    }

    fn add_cone(&mut self, foot: Vec3, height: f32, r: f32, col: [f32; 4], uv: [f32; 2]) {
        if r <= 1e-5 || height.abs() <= 1e-5 {
            return;
        }
        let sides = 12usize;
        let tip = foot + Vec3::Y * height;
        let sign = height.signum();
        let slope_y = r / height.abs();
        let base_c = self.push_vert(foot, Vec3::Y * -sign, col, uv);
        let mut base_ring = Vec::with_capacity(sides);
        let mut side_ring = Vec::with_capacity(sides);
        for i in 0..sides {
            let a = i as f32 * TAU / sides as f32;
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let p = foot + dir * r;
            base_ring.push(self.push_vert(p, Vec3::Y * -sign, col, uv));
            side_ring.push(self.push_vert(p, (dir + Vec3::Y * (slope_y * sign)).normalize(), col, uv));
        }
        let tip_v = self.push_vert(tip, Vec3::Y * sign, col, uv);
        for i in 0..sides {
            let n = (i + 1) % sides;
            if sign > 0.0 {
                self.tri(side_ring[i], side_ring[n], tip_v);
                self.tri(base_c, base_ring[i], base_ring[n]);
            } else {
                self.tri(side_ring[n], side_ring[i], tip_v);
                self.tri(base_c, base_ring[n], base_ring[i]);
            }
        }
    }

    fn finish(self) -> Option<(Mesh, usize)> {
        if self.indices.is_empty() {
            return None;
        }
        let triangles = self.indices.len() / 3;
        let mut out = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        out.insert_attribute(Mesh::ATTRIBUTE_POSITION, self.positions);
        out.insert_attribute(Mesh::ATTRIBUTE_NORMAL, self.normals);
        out.insert_attribute(Mesh::ATTRIBUTE_COLOR, self.colors);
        out.insert_attribute(Mesh::ATTRIBUTE_UV_0, self.uvs);
        out.insert_indices(Indices::U32(self.indices));
        Some((out, triangles))
    }
}

/// Spawns one merged GPU mesh for all props emitted by the area, returning the triangle count.
pub fn spawn(commands: &mut Commands, meshes: &mut Assets<Mesh>, material: Handle<SurfaceMaterial>, props: &[Prop]) -> usize {
    let mut b = Builder::new();
    for p in props {
        let [cr, cg, cb] = colour(p.material);
        let col = [cr, cg, cb, 1.0];
        let uv = properties(p.material);
        let a = Vec3::from_array(p.a);
        let bv = Vec3::from_array(p.b);
        match p.kind {
            PropKind::Capsule => b.add_capsule(a, bv, p.r, col, uv),
            PropKind::Ellipsoid => b.add_ellipsoid(a, bv, p.yaw, col, uv),
            PropKind::Cylinder => b.add_cylinder(a, bv.x, bv.y, (p.r > 0.0).then_some(p.yaw), col, uv),
            PropKind::Cone => b.add_cone(a, bv.y, p.r, col, uv),
        }
    }
    if let Some((mesh, tris)) = b.finish() {
        commands.spawn((Mesh3d(meshes.add(mesh)), MeshMaterial3d(material), Transform::default(), Visibility::Visible, PropsMesh));
        tris
    } else {
        0
    }
}
