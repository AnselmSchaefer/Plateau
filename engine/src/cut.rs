//! Deterministic procedural `Scene` cutting, watertight cross-section capping, and scale-aware
//! object physics / carry limits (Step 8c; AGENTS.md §1.3 & §2).
//!
//! Works generically on **any** un-promoted procedural `Scene` `.wasm` element (`tree`, `bridge`,
//! `lantern`, or future Claude-made elements) with zero prefab enums and zero WIT changes:
//! - Exact deterministic Sutherland–Hodgman triangle-plane clipping in element-local space.
//! - Watertight 2D boundary-loop chaining + ear-clipping cross-section caps so cut pieces look
//!   solid and remain watertight for lazy Scene → SDF promotion (`MeshSdf`).
//! - Deterministic physical volume, mass, span, and rooting derived from solid mesh triangles and
//!   engine material densities, scaled against `player.size` (`tiny`, `person`, `giant`).

use wasmtime::bail;

use crate::compose::PlacementTransform;
use crate::edit::{Edit, Shape};
use crate::{Instance, Material, MeshData, Result, SIZES, Scene, Vertex};

const PERSON_HEIGHT: f32 = SIZES[1].1;

/// An element-local half-space plane keeping points with `dot(normal, p) >= offset`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClipPlane {
    pub normal: [f32; 3],
    pub offset: f32,
}

impl ClipPlane {
    pub fn new(normal: [f32; 3], offset: f32) -> ClipPlane {
        let l = len3(normal);
        let n = if l > 1e-8 { [normal[0] / l, normal[1] / l, normal[2] / l] } else { [0.0, 1.0, 0.0] };
        ClipPlane { normal: n, offset }
    }

    /// Returns the complementary half-space keeping `dot(normal, p) <= offset`.
    pub fn flipped(self) -> ClipPlane {
        ClipPlane { normal: [-self.normal[0], -self.normal[1], -self.normal[2]], offset: -self.offset }
    }

    pub fn validate(&self) -> Result<()> {
        if !self.normal.iter().all(|c| c.is_finite()) || !self.offset.is_finite() {
            bail!("clip plane must be finite, got {:?}", self);
        }
        let l = len3(self.normal);
        if !(0.95..=1.05).contains(&l) {
            bail!("clip plane normal must be unit length, got length {l}");
        }
        Ok(())
    }

    /// Bit-exact cache key (canonicalizing `-0.0` to `+0.0`).
    pub fn key_bits(self) -> [u32; 4] {
        let norm_zero = |x: f32| if x == 0.0 { 0.0f32.to_bits() } else { x.to_bits() };
        [norm_zero(self.normal[0]), norm_zero(self.normal[1]), norm_zero(self.normal[2]), norm_zero(self.offset)]
    }
}

/// Physical density (kg/m³) per engine material for scale-aware carry/cut checks.
pub fn material_density(mat: Material) -> f32 {
    match mat {
        Material::Wood => 600.0,
        Material::Stone => 2200.0,
        Material::Soil | Material::Mud | Material::Sand => 1400.0,
        Material::Water => 1000.0,
        Material::Grass | Material::Leaf | Material::Moss => 180.0,
    }
}

/// Fresh-cut interior RGB tint multiplier so chopped wood/stone faces look distinct and natural.
fn cut_face_tint(mat: Material) -> [u8; 3] {
    match mat {
        Material::Wood => [248, 228, 192],
        Material::Stone => [216, 212, 206],
        Material::Soil | Material::Mud => [196, 178, 160],
        Material::Sand => [240, 230, 205],
        Material::Grass | Material::Leaf | Material::Moss => [210, 232, 180],
        Material::Water => [220, 238, 248],
    }
}

/// Deterministic physical summary of a clipped `Scene` element at a given `scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ObjectPhysics {
    /// Tight element-local AABB of the element's solid geometry (or all geometry if none is solid).
    pub local_lo: [f32; 3],
    pub local_hi: [f32; 3],
    /// Local geometric center of `local_lo..local_hi`.
    pub local_center: [f32; 3],
    /// Element-local solid volume in cubic metres (at `scale = 1.0`).
    pub local_volume_m3: f32,
    /// World-space mass in kg (`local_mass_kg * scale^3`).
    pub world_mass_kg: f32,
    /// Longest world-space axis span of the element in metres.
    pub world_span: f32,
    /// True if solid geometry still extends deeply below `y = 0` (`local_lo[1] < -0.15`), indicating
    /// roots or a sunk foundation anchored into the ground.
    pub rooted: bool,
    /// Number of solid triangles remaining.
    pub solid_triangles: usize,
}

impl ObjectPhysics {
    /// Computes deterministic physical properties from a (possibly clipped) element-local `Scene`
    /// and its uniform world placement `scale`.
    pub fn compute(scene: &Scene, scale: f32) -> ObjectPhysics {
        let has_solid = scene.instances.iter().any(|inst| {
            inst.solid && scene.meshes.get(inst.mesh as usize).is_some_and(|m| !m.indices.is_empty())
        });
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut total_vol = 0.0f32;
        let mut total_mass_unit = 0.0f32;
        let mut solid_triangles = 0usize;

        for inst in &scene.instances {
            if has_solid && !inst.solid {
                continue;
            }
            let Some(mesh) = scene.meshes.get(inst.mesh as usize) else { continue };
            if mesh.vertices.is_empty() || mesh.indices.is_empty() {
                continue;
            }
            let mut mlo = [f32::INFINITY; 3];
            let mut mhi = [f32::NEG_INFINITY; 3];
            let mut signed_vol = 0.0f32;
            let mut weighted_vol = 0.0f32;
            let mut dominant_mat = Material::Wood;
            let mut tri_count = 0usize;

            let sc = inst.scale;
            let inv_sc = [
                if abs_f32(sc[0]) > 1e-5 { 1.0 / sc[0] } else { 1.0 },
                if abs_f32(sc[1]) > 1e-5 { 1.0 / sc[1] } else { 1.0 },
                if abs_f32(sc[2]) > 1e-5 { 1.0 / sc[2] } else { 1.0 },
            ];
            let xform_p = |p: [f32; 3]| {
                let r = quat_rotate(inst.rot, [p[0] * sc[0], p[1] * sc[1], p[2] * sc[2]]);
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
                for p in [a, b, c] {
                    for axis in 0..3 {
                        lo[axis] = min_f32(lo[axis], p[axis]);
                        hi[axis] = max_f32(hi[axis], p[axis]);
                        mlo[axis] = min_f32(mlo[axis], p[axis]);
                        mhi[axis] = max_f32(mhi[axis], p[axis]);
                    }
                }
                let g = cross3(sub3(b, a), sub3(c, a));
                let na = xform_n(va.normal);
                let nb = xform_n(vb.normal);
                let nc = xform_n(vc.normal);
                let vn = [na[0] + nb[0] + nc[0], na[1] + nb[1] + nc[1], na[2] + nb[2] + nc[2]];
                // Align signed tetrahedron volume to outward vertex normals so MeshBuilder
                // primitives always contribute positive volume.
                let v6 = if dot3(g, vn) < 0.0 {
                    dot3(a, cross3(c, b))
                } else {
                    dot3(a, cross3(b, c))
                };
                let dv = v6 * (1.0 / 6.0);
                signed_vol += dv;
                weighted_vol += dv * material_density(va.material);
                dominant_mat = va.material;
                tri_count += 1;
            }
            if inst.solid {
                solid_triangles += tri_count;
            }
            let vol = abs_f32(signed_vol);
            if vol > 1e-6 {
                total_vol += vol;
                total_mass_unit += abs_f32(weighted_vol);
            } else if tri_count > 0 {
                let aabb_v = max_f32(mhi[0] - mlo[0], 0.02) * max_f32(mhi[1] - mlo[1], 0.02) * max_f32(mhi[2] - mlo[2], 0.02) * 0.15;
                total_vol += aabb_v;
                total_mass_unit += aabb_v * material_density(dominant_mat);
            }
        }

        if !lo[0].is_finite() {
            lo = [0.0; 3];
            hi = [0.0; 3];
        }
        let local_center = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
        let ext = [hi[0] - lo[0], hi[1] - lo[1], hi[2] - lo[2]];
        let local_span = max_f32(ext[0], max_f32(ext[1], ext[2]));
        let s3 = scale * scale * scale;
        ObjectPhysics {
            local_lo: lo,
            local_hi: hi,
            local_center,
            local_volume_m3: total_vol,
            world_mass_kg: total_mass_unit * s3,
            world_span: local_span * scale,
            rooted: lo[1] < -0.15,
            solid_triangles,
        }
    }

    /// Computes the world-space vertical bounds `(min_y, max_y)` of this object's local AABB
    /// rotated by unit quaternion `rot` and scaled by `scale` around `pos = [0, 0, 0]`.
    pub fn rotated_y_extents(&self, rot: [f32; 4], scale: f32) -> (f32, f32) {
        let mut min_y = f32::INFINITY;
        let mut max_y = f32::NEG_INFINITY;
        for sx in [self.local_lo[0], self.local_hi[0]] {
            for sy in [self.local_lo[1], self.local_hi[1]] {
                for sz in [self.local_lo[2], self.local_hi[2]] {
                    let p = quat_rotate(rot, [sx * scale, sy * scale, sz * scale]);
                    min_y = min_f32(min_y, p[1]);
                    max_y = max_f32(max_y, p[1]);
                }
            }
        }
        (min_y, max_y)
    }

    /// World-space bounding radius around `local_center` scaled by `scale`.
    pub fn world_radius(&self, scale: f32) -> f32 {
        let hx = (self.local_hi[0] - self.local_lo[0]) * 0.5 * scale;
        let hy = (self.local_hi[1] - self.local_lo[1]) * 0.5 * scale;
        let hz = (self.local_hi[2] - self.local_lo[2]) * 0.5 * scale;
        (hx * hx + hy * hy + hz * hz).sqrt()
    }
}

/// Outcome of checking whether a player of `player_size` can pick up & carry an element.
#[derive(Clone, Debug, PartialEq)]
pub enum CarryDecision {
    /// Allowed; `speed_factor` in `(0.6..=1.0]` scales walk/run pace for weighty heft.
    Allowed { speed_factor: f32 },
    /// Refused with a concise player-facing reason.
    Blocked(String),
}

impl CarryDecision {
    pub fn is_allowed(&self) -> bool {
        matches!(self, CarryDecision::Allowed { .. })
    }

    pub fn speed_factor(&self) -> f32 {
        match *self {
            CarryDecision::Allowed { speed_factor } => speed_factor,
            CarryDecision::Blocked(_) => 1.0,
        }
    }
}

/// Maximum mass (kg) a player of height `player_size` can carry.
/// Scales with `size^3` so tiny/person/giant experience identical physics on scale-matched objects.
pub fn max_carry_mass_kg(player_size: f32) -> f32 {
    let r = max_f32(player_size / PERSON_HEIGHT, 1e-4);
    1400.0 * r * r * r
}

/// Maximum rooted mass (kg) a player can uproot straight out of the ground without chopping first.
pub fn max_uproot_mass_kg(player_size: f32) -> f32 {
    let r = max_f32(player_size / PERSON_HEIGHT, 1e-4);
    260.0 * r * r * r
}

/// Maximum object span (metres) a player of height `player_size` can carry.
pub fn max_carry_span(player_size: f32) -> f32 {
    let r = max_f32(player_size / PERSON_HEIGHT, 1e-4);
    8.5 * r
}

/// Formats a physical mass nicely across tiny (mg/g), person (kg), and giant (t) scales.
pub fn format_mass(kg: f32) -> String {
    if kg < 0.001 {
        format!("{:.1} mg", kg * 1_000_000.0)
    } else if kg < 1.0 {
        format!("{:.1} g", kg * 1000.0)
    } else if kg < 1000.0 {
        format!("{:.0} kg", kg)
    } else {
        format!("{:.1} t", kg / 1000.0)
    }
}

/// Checks whether a player of height `player_size` can pick up and carry this element.
/// Native SDF landforms (`is_sdf == true`) are part of the terrain; discrete procedural `Scene`
/// elements (including ones with carved local tunnels) remain portable within weight/rooting limits.
pub fn check_carry(phys: &ObjectPhysics, is_sdf: bool, player_size: f32) -> CarryDecision {
    if is_sdf {
        return CarryDecision::Blocked("Rooted in terrain — sculpt with B".into());
    }
    let max_mass = max_carry_mass_kg(player_size);
    let max_span = max_carry_span(player_size);
    if phys.world_mass_kg > max_mass || phys.world_span > max_span {
        return CarryDecision::Blocked(format!(
            "Too heavy at this size ({} > {}) — grow larger or chop smaller (K)",
            format_mass(phys.world_mass_kg),
            format_mass(max_mass)
        ));
    }
    let uproot = max_uproot_mass_kg(player_size);
    if phys.rooted && phys.world_mass_kg > uproot {
        return CarryDecision::Blocked("Rooted in the ground — press K to chop free (or grow larger)".into());
    }
    let load = clamp_f32(phys.world_mass_kg / max_f32(max_mass, 1e-9), 0.0, 1.0);
    let speed_factor = 1.0 - 0.36 * load;
    CarryDecision::Allowed { speed_factor }
}

/// Checks whether a player of height `player_size` can chop/cut this element.
/// Native SDF landforms (`is_sdf == true`) are carved with `B`; discrete `Scene` elements
/// (including lazily promoted ones with carved tunnels) can be chopped cleanly at scale.
pub fn check_cut(phys: &ObjectPhysics, is_sdf: bool, player_size: f32) -> std::result::Result<(), String> {
    if is_sdf {
        return Err("Terrain landform — carve with B".into());
    }
    let r = max_f32(player_size / PERSON_HEIGHT, 1e-4);
    let max_cut_mass = 5000.0 * r * r * r;
    let max_cut_span = 14.0 * r;
    if phys.world_mass_kg > max_cut_mass || phys.world_span > max_cut_span {
        return Err(format!(
            "Too massive to chop at this size ({}) — grow larger or carve with B",
            format_mass(phys.world_mass_kg)
        ));
    }
    if phys.world_span < player_size * 0.12 {
        return Err("Too small to chop cleanly — shrink down first".into());
    }
    Ok(())
}

/// Converts a world-space SDF `Edit` into an element's local coordinate frame so carved tunnels
/// and notches travel and rotate with the element when it is chopped, toppled, or carried.
pub fn world_edit_to_local(edit: &Edit, tf: PlacementTransform) -> Edit {
    let inv = 1.0 / max_f32(tf.scale, 1e-5);
    let center = tf.world_to_local_point(edit.center);
    let shape = match edit.shape {
        Shape::Sphere { radius } => Shape::Sphere { radius: radius * inv },
        Shape::Box { half, round } => Shape::Box { half: [half[0] * inv, half[1] * inv, half[2] * inv], round: round * inv },
        Shape::Cylinder { radius, half_height, round } => {
            Shape::Cylinder { radius: radius * inv, half_height: half_height * inv, round: round * inv }
        }
    };
    Edit {
        op: edit.op,
        shape,
        center,
        blend: edit.blend * inv,
        material: edit.material,
        stamp_size: edit.stamp_size * inv,
    }
}

/// Chooses a clean element-local cut plane perpendicular to the object's longest structural axis
/// through `hit_local`, clamped away from the extreme ends so both halves remain substantial.
///
/// The returned plane's normal points toward the **upper / detached** half (for an upright tree
/// whose longest axis is +Y, the plane keeps `y >= offset` as the felled trunk and its `.flipped()`
/// keeps `y <= offset` as the stump).
pub fn choose_cut_plane(phys: &ObjectPhysics, hit_local: [f32; 3]) -> Option<ClipPlane> {
    let ext = [
        phys.local_hi[0] - phys.local_lo[0],
        phys.local_hi[1] - phys.local_lo[1],
        phys.local_hi[2] - phys.local_lo[2],
    ];
    let axis = if ext[1] >= ext[0] && ext[1] >= ext[2] {
        1
    } else if ext[2] >= ext[0] {
        2
    } else {
        0
    };
    let span = ext[axis];
    if span < 0.05 {
        return None;
    }
    let margin = span * 0.14;
    let min_off = phys.local_lo[axis] + margin;
    let max_off = phys.local_hi[axis] - margin;
    if min_off >= max_off {
        return None;
    }
    let offset = clamp_f32(hit_local[axis], min_off, max_off);
    let mut normal = [0.0; 3];
    normal[axis] = 1.0;
    Some(ClipPlane { normal, offset })
}

/// Computes the world-space resting `(pos, rot)` when a newly cut upright piece topples 90° onto
/// its side along horizontal direction `fall_dir` (unit XZ vector away from the player) onto
/// ground height `ground_y`.
pub fn toppled_transform(
    origin_tf: crate::compose::PlacementTransform,
    upper_phys: &ObjectPhysics,
    fall_dir_xz: [f32; 2],
    ground_y: f32,
) -> crate::compose::PlacementTransform {
    let len = (fall_dir_xz[0] * fall_dir_xz[0] + fall_dir_xz[1] * fall_dir_xz[1]).sqrt();
    let (fx, fz) = if len > 1e-5 { (fall_dir_xz[0] / len, fall_dir_xz[1] / len) } else { (0.0, -1.0) };
    // Horizontal hinge axis perpendicular to +Y and fall_dir: `axis = UP x fall_dir = (fz, 0, -fx)`.
    // Rotating +90° around `(fz, 0, -fx)` maps world +Y `(0, 1, 0)` onto `(fx, 0, fz)`!
    let hinge = [fz, 0.0, -fx];
    let tip_q = quat_from_axis_angle(hinge, std::f32::consts::FRAC_PI_2);
    let final_rot = quat_mul(tip_q, origin_tf.rot);

    // Pivot the fall around the cut base `(local_center.x, local_lo.y, local_center.z)` nudged
    // slightly outward along `fall_dir` so the felled trunk clears the stump cleanly.
    let pivot_local = [upper_phys.local_center[0], upper_phys.local_lo[1], upper_phys.local_center[2]];
    let pivot_world = origin_tf.local_to_world_point(pivot_local);
    let rotated_pivot = quat_rotate(final_rot, [
        pivot_local[0] * origin_tf.scale,
        pivot_local[1] * origin_tf.scale,
        pivot_local[2] * origin_tf.scale,
    ]);
    let nudge = max_f32(0.35 * origin_tf.scale, 0.02);
    let mut pos = [
        pivot_world[0] - rotated_pivot[0] + fx * nudge,
        pivot_world[1] - rotated_pivot[1],
        pivot_world[2] - rotated_pivot[2] + fz * nudge,
    ];
    let (min_y, _) = upper_phys.rotated_y_extents(final_rot, origin_tf.scale);
    // Rest lowest vertex cleanly on ground_y (sunk 2% of scale so it seats without floating).
    pos[1] = ground_y - min_y - 0.02 * origin_tf.scale;
    crate::compose::PlacementTransform::with_rot(pos, final_rot, origin_tf.scale)
}

/// Computes the world position for placing a carried object at `aim_world` with rotation `rot` and
/// uniform `scale` so its rotated horizontal center aligns with `(aim_world.x, aim_world.z)` and
/// its lowest world vertex rests cleanly on `aim_world.y`.
pub fn placement_pos_for_aim(phys: &ObjectPhysics, aim_world: [f32; 3], rot: [f32; 4], scale: f32) -> [f32; 3] {
    let rc = quat_rotate(rot, [
        phys.local_center[0] * scale,
        phys.local_center[1] * scale,
        phys.local_center[2] * scale,
    ]);
    let (min_y, _) = phys.rotated_y_extents(rot, scale);
    [
        aim_world[0] - rc[0],
        aim_world[1] - min_y - 0.02 * scale,
        aim_world[2] - rc[2],
    ]
}

/// Clips all meshes, instances, and lights of an element-local `Scene` against `planes`
/// deterministically, capping cut cross-sections so solid meshes remain watertight.
///
/// Preserves original mesh indices for dynamic instances (`id > 0`) so runtime `tick`/`interact`
/// commands referencing mesh indices continue to match seamlessly; instances whose clipped mesh
/// becomes empty are omitted.
pub fn clip_scene(scene: &Scene, planes: &[ClipPlane]) -> Scene {
    if planes.is_empty() {
        return scene.clone();
    }

    // Track whether a mesh index has already been written into `out_meshes` at its original index
    // with the same instance transform. For distinct transforms sharing a mesh, allocate a new slot.
    type TransformKey = ([u32; 3], [u32; 4], [u32; 3]);
    let mut out_meshes: Vec<MeshData> = vec![MeshData::default(); scene.meshes.len()];
    let mut mesh_used_by: Vec<Option<TransformKey>> = vec![None; scene.meshes.len()];
    let mut out_instances = Vec::with_capacity(scene.instances.len());

    let tf_key = |inst: &Instance| {
        (
            [inst.pos[0].to_bits(), inst.pos[1].to_bits(), inst.pos[2].to_bits()],
            [inst.rot[0].to_bits(), inst.rot[1].to_bits(), inst.rot[2].to_bits(), inst.rot[3].to_bits()],
            [inst.scale[0].to_bits(), inst.scale[1].to_bits(), inst.scale[2].to_bits()],
        )
    };

    for &inst in &scene.instances {
        let Some(orig_mesh) = scene.meshes.get(inst.mesh as usize) else { continue };
        let local_planes: Vec<ClipPlane> = planes.iter().map(|&p| plane_to_instance_local(p, &inst)).collect();
        let clipped = clip_mesh_planes(orig_mesh, &local_planes, inst.solid);
        if clipped.indices.is_empty() {
            continue;
        }
        let key = tf_key(&inst);
        let target_idx = match mesh_used_by[inst.mesh as usize] {
            None => {
                out_meshes[inst.mesh as usize] = clipped;
                mesh_used_by[inst.mesh as usize] = Some(key);
                inst.mesh
            }
            Some(prev) if prev == key => inst.mesh,
            Some(_) => {
                let idx = out_meshes.len() as u32;
                out_meshes.push(clipped);
                idx
            }
        };
        out_instances.push(Instance { mesh: target_idx, ..inst });
    }

    let out_lights = scene
        .lights
        .iter()
        .copied()
        .filter(|l| planes.iter().all(|p| dot3(p.normal, l.pos) >= p.offset - 1e-4))
        .collect();

    Scene { meshes: out_meshes, instances: out_instances, lights: out_lights }
}

/// Transforms a `ClipPlane` from element-local space into an instance's mesh-local space.
fn plane_to_instance_local(plane: ClipPlane, inst: &Instance) -> ClipPlane {
    // Point `p` in mesh-local maps to element-local `w = inst.pos + R * (S * p)`.
    // Condition `dot(n, w) >= offset` becomes:
    // `dot(n, R * (S * p)) >= offset - dot(n, inst.pos)`
    // `dot(S * (R^-1 * n), p) >= offset - dot(n, inst.pos)`.
    let inv_q = [-inst.rot[0], -inst.rot[1], -inst.rot[2], inst.rot[3]];
    let rn = quat_rotate(inv_q, plane.normal);
    let sn = [rn[0] * inst.scale[0], rn[1] * inst.scale[1], rn[2] * inst.scale[2]];
    let l = len3(sn);
    if l <= 1e-8 {
        return plane;
    }
    let rhs = plane.offset - dot3(plane.normal, inst.pos);
    ClipPlane { normal: [sn[0] / l, sn[1] / l, sn[2] / l], offset: rhs / l }
}

fn clip_mesh_planes(mesh: &MeshData, planes: &[ClipPlane], cap: bool) -> MeshData {
    let mut cur = mesh.clone();
    for &plane in planes {
        if cur.indices.is_empty() {
            break;
        }
        cur = clip_mesh_one_plane(&cur, plane, cap);
    }
    cur
}

/// Clips a triangle mesh against one plane `dot(n, p) >= offset` using deterministic
/// Sutherland–Hodgman clipping, and when `cap` is true triangulates closed cut cross-sections
/// with outward normal `-plane.normal`.
fn clip_mesh_one_plane(mesh: &MeshData, plane: ClipPlane, cap: bool) -> MeshData {
    let mut out_verts: Vec<Vertex> = Vec::new();
    let mut out_indices: Vec<u32> = Vec::new();
    let mut cut_edges: Vec<([f32; 3], [f32; 3], Material)> = Vec::new();

    let push_tri = |verts: &mut Vec<Vertex>, indices: &mut Vec<u32>, a: Vertex, b: Vertex, c: Vertex| {
        let g = cross3(sub3(b.pos, a.pos), sub3(c.pos, a.pos));
        if len3(g) <= 1e-10 {
            return;
        }
        let base = verts.len() as u32;
        verts.push(a);
        verts.push(b);
        verts.push(c);
        indices.extend_from_slice(&[base, base + 1, base + 2]);
    };

    for tri in mesh.indices.as_chunks::<3>().0 {
        let v0 = mesh.vertices[tri[0] as usize];
        let v1 = mesh.vertices[tri[1] as usize];
        let v2 = mesh.vertices[tri[2] as usize];
        let d0 = dot3(plane.normal, v0.pos) - plane.offset;
        let d1 = dot3(plane.normal, v1.pos) - plane.offset;
        let d2 = dot3(plane.normal, v2.pos) - plane.offset;

        let in0 = d0 >= 0.0;
        let in1 = d1 >= 0.0;
        let in2 = d2 >= 0.0;
        let count = in0 as u8 + in1 as u8 + in2 as u8;

        match count {
            0 => continue,
            3 => push_tri(&mut out_verts, &mut out_indices, v0, v1, v2),
            1 => {
                // Rotate so `a` is inside and `b, c` are outside (preserving winding!).
                let (a, da, b, db, c, dc) = if in0 {
                    (v0, d0, v1, d1, v2, d2)
                } else if in1 {
                    (v1, d1, v2, d2, v0, d0)
                } else {
                    (v2, d2, v0, d0, v1, d1)
                };
                let ab = lerp_vertex(a, b, da / (da - db));
                let ac = lerp_vertex(a, c, da / (da - dc));
                push_tri(&mut out_verts, &mut out_indices, a, ab, ac);
                if cap && len3(sub3(ab.pos, ac.pos)) > 1e-6 {
                    let g = cross3(sub3(b.pos, a.pos), sub3(c.pos, a.pos));
                    let vn = [
                        a.normal[0] + b.normal[0] + c.normal[0],
                        a.normal[1] + b.normal[1] + c.normal[1],
                        a.normal[2] + b.normal[2] + c.normal[2],
                    ];
                    if dot3(g, vn) < 0.0 {
                        cut_edges.push((ab.pos, ac.pos, a.material));
                    } else {
                        cut_edges.push((ac.pos, ab.pos, a.material));
                    }
                }
            }
            2 => {
                // Rotate so `a` is outside and `b, c` are inside (preserving winding!).
                let (a, da, b, db, c, dc) = if !in0 {
                    (v0, d0, v1, d1, v2, d2)
                } else if !in1 {
                    (v1, d1, v2, d2, v0, d0)
                } else {
                    (v2, d2, v0, d0, v1, d1)
                };
                let ab = lerp_vertex(a, b, (-da) / (db - da));
                let ac = lerp_vertex(a, c, (-da) / (dc - da));
                push_tri(&mut out_verts, &mut out_indices, b, c, ac);
                push_tri(&mut out_verts, &mut out_indices, b, ac, ab);
                if cap && len3(sub3(ab.pos, ac.pos)) > 1e-6 {
                    let g = cross3(sub3(b.pos, a.pos), sub3(c.pos, a.pos));
                    let vn = [
                        a.normal[0] + b.normal[0] + c.normal[0],
                        a.normal[1] + b.normal[1] + c.normal[1],
                        a.normal[2] + b.normal[2] + c.normal[2],
                    ];
                    if dot3(g, vn) < 0.0 {
                        cut_edges.push((ac.pos, ab.pos, b.material));
                    } else {
                        cut_edges.push((ab.pos, ac.pos, b.material));
                    }
                }
            }
            _ => unreachable!(),
        }
    }

    if cap && cut_edges.len() >= 3 {
        append_cap_polygons(&cut_edges, plane, &mut out_verts, &mut out_indices);
    }

    MeshData { vertices: out_verts, indices: out_indices }
}

fn lerp_vertex(a: Vertex, b: Vertex, t: f32) -> Vertex {
    let t = clamp_f32(t, 0.0, 1.0);
    let mix = |x: f32, y: f32| x + (y - x) * t;
    let nx = mix(a.normal[0], b.normal[0]);
    let ny = mix(a.normal[1], b.normal[1]);
    let nz = mix(a.normal[2], b.normal[2]);
    let nl = len3([nx, ny, nz]);
    let normal = if nl > 1e-8 { [nx / nl, ny / nl, nz / nl] } else { a.normal };
    let tint = [
        clamp_f32(mix(a.tint[0] as f32, b.tint[0] as f32), 0.0, 255.0) as u8,
        clamp_f32(mix(a.tint[1] as f32, b.tint[1] as f32), 0.0, 255.0) as u8,
        clamp_f32(mix(a.tint[2] as f32, b.tint[2] as f32), 0.0, 255.0) as u8,
    ];
    Vertex { pos: [mix(a.pos[0], b.pos[0]), mix(a.pos[1], b.pos[1]), mix(a.pos[2], b.pos[2])], normal, material: a.material, tint }
}

/// Chains directed cut-plane boundary segments into closed 2D loops and triangulates each loop
/// with outward normal `-plane.normal`.
fn append_cap_polygons(
    edges: &[([f32; 3], [f32; 3], Material)],
    plane: ClipPlane,
    verts: &mut Vec<Vertex>,
    indices: &mut Vec<u32>,
) {
    let cap_n = [-plane.normal[0], -plane.normal[1], -plane.normal[2]];
    let (u, v) = plane_basis(cap_n); // u x v == cap_n

    let mut used = vec![false; edges.len()];
    const SNAP_D2: f32 = 1e-5; // ~3 mm endpoint weld tolerance for robust loop closure

    for start_idx in 0..edges.len() {
        if used[start_idx] {
            continue;
        }
        used[start_idx] = true;
        let (p0, p1, mat) = edges[start_idx];
        let mut ring: Vec<[f32; 3]> = vec![p0, p1];

        loop {
            let cur = *ring.last().unwrap();
            // Check if loop closed back to ring[0] (once we have >= 3 vertices).
            if ring.len() >= 3 && dist2(cur, ring[0]) <= SNAP_D2 {
                ring.pop();
                break;
            }
            // Find closest unused segment whose start matches `cur`.
            let mut best: Option<(usize, f32, bool)> = None;
            for (i, &(a, b, _)) in edges.iter().enumerate() {
                if used[i] {
                    continue;
                }
                let d_fwd = dist2(cur, a);
                if d_fwd <= SNAP_D2 && best.is_none_or(|(_, bd, _)| d_fwd < bd) {
                    best = Some((i, d_fwd, false));
                }
                let d_rev = dist2(cur, b);
                if d_rev <= SNAP_D2 && best.is_none_or(|(_, bd, _)| d_rev < bd) {
                    best = Some((i, d_rev, true));
                }
            }
            match best {
                Some((idx, _, rev)) => {
                    used[idx] = true;
                    let next_p = if rev { edges[idx].0 } else { edges[idx].1 };
                    if dist2(next_p, cur) > 1e-10 {
                        ring.push(next_p);
                    }
                    if ring.len() > edges.len() + 2 {
                        break;
                    }
                }
                None => break,
            }
        }

        if ring.len() < 3 {
            continue;
        }
        // Only cap genuinely closed (or nearly closed) contours so open decorative ribbons don't get spurious fans.
        let gap2 = dist2(*ring.last().unwrap(), ring[0]);
        let seg_scale2 = dist2(ring[0], ring[1]).max(1e-4);
        if gap2 > max_f32(SNAP_D2 * 16.0, seg_scale2 * 4.0) {
            continue;
        }

        // Project onto `(u, v)` where CCW triangles have normal `+cap_n`.
        let origin = ring[0];
        let mut pts2: Vec<[f32; 2]> = ring
            .iter()
            .map(|&p| {
                let d = sub3(p, origin);
                [dot3(d, u), dot3(d, v)]
            })
            .collect();

        // Ensure CCW winding in (u, v) so triangle cross products align with `cap_n = -plane.normal`.
        if signed_area_2d(&pts2) < 0.0 {
            ring.reverse();
            pts2.reverse();
        }
        if abs_f32(signed_area_2d(&pts2)) < 1e-8 {
            continue;
        }

        let tint = cut_face_tint(mat);
        let base = verts.len() as u32;
        for &p in &ring {
            verts.push(Vertex { pos: p, normal: cap_n, material: mat, tint });
        }

        let tris = ear_clip_2d(&pts2);
        if !tris.is_empty() {
            for [i0, i1, i2] in tris {
                indices.extend_from_slice(&[base + i0 as u32, base + i1 as u32, base + i2 as u32]);
            }
        } else {
            // Centroid fan fallback for degenerate polygons.
            let n = ring.len() as f32;
            let mut c = [0.0f32; 3];
            for &p in &ring {
                c[0] += p[0];
                c[1] += p[1];
                c[2] += p[2];
            }
            let c_idx = verts.len() as u32;
            verts.push(Vertex { pos: [c[0] / n, c[1] / n, c[2] / n], normal: cap_n, material: mat, tint });
            for i in 0..ring.len() {
                let j = (i + 1) % ring.len();
                indices.extend_from_slice(&[c_idx, base + i as u32, base + j as u32]);
            }
        }
    }
}

/// Deterministic ear-clipping triangulation for a CCW simple 2D polygon.
fn ear_clip_2d(pts: &[[f32; 2]]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::with_capacity(n - 2);
    let mut guard = n * n + 4;

    while idx.len() > 3 && guard > 0 {
        guard -= 1;
        let m = idx.len();
        let mut clipped = false;
        for k in 0..m {
            let i0 = idx[(k + m - 1) % m];
            let i1 = idx[k];
            let i2 = idx[(k + 1) % m];
            let (a, b, c) = (pts[i0], pts[i1], pts[i2]);
            if cross2(a, b, c) <= 1e-10 {
                continue; // reflex or collinear
            }
            let mut any_inside = false;
            for &other in &idx {
                if other == i0 || other == i1 || other == i2 {
                    continue;
                }
                if point_in_tri_2d(pts[other], a, b, c) {
                    any_inside = true;
                    break;
                }
            }
            if !any_inside {
                out.push([i0, i1, i2]);
                idx.remove(k);
                clipped = true;
                break;
            }
        }
        if !clipped {
            return Vec::new(); // trigger centroid fan fallback
        }
    }
    if idx.len() == 3 {
        out.push([idx[0], idx[1], idx[2]]);
    }
    out
}

fn signed_area_2d(pts: &[[f32; 2]]) -> f32 {
    let mut a = 0.0f32;
    let n = pts.len();
    for i in 0..n {
        let p = pts[i];
        let q = pts[(i + 1) % n];
        a += p[0] * q[1] - q[0] * p[1];
    }
    a * 0.5
}

#[inline]
fn cross2(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn point_in_tri_2d(p: [f32; 2], a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> bool {
    cross2(a, b, p) >= -1e-7 && cross2(b, c, p) >= -1e-7 && cross2(c, a, p) >= -1e-7
}

/// Builds an orthonormal right-handed basis `(u, v)` perpendicular to unit `n` with `u × v = n`.
fn plane_basis(n: [f32; 3]) -> ([f32; 3], [f32; 3]) {
    let helper = if abs_f32(n[1]) < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
    let u_raw = cross3(helper, n);
    let ul = len3(u_raw);
    let u = [u_raw[0] / ul, u_raw[1] / ul, u_raw[2] / ul];
    let v = cross3(n, u);
    (u, v)
}

// Deterministic math helpers (no libm sin/cos/min/max/clamp).

const PI: f32 = std::f32::consts::PI;
const TAU: f32 = std::f32::consts::TAU;

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
fn clamp_f32(x: f32, lo: f32, hi: f32) -> f32 {
    min_f32(max_f32(x, lo), hi)
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
fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = sub3(a, b);
    dot3(d, d)
}

#[inline]
fn sin_f32(x: f32) -> f32 {
    let turns = x * (1.0 / TAU);
    let mut r = (turns - (turns + 0.5).floor()) * TAU;
    if r > PI / 2.0 {
        r = PI - r;
    } else if r < -PI / 2.0 {
        r = -PI - r;
    }
    let r2 = r * r;
    r * (1.0 + r2 * (-1.0 / 6.0 + r2 * (1.0 / 120.0 + r2 * (-1.0 / 5040.0 + r2 * (1.0 / 362_880.0 + r2 * (-1.0 / 39_916_800.0))))))
}

#[inline]
fn cos_f32(x: f32) -> f32 {
    sin_f32(x + PI / 2.0)
}

#[inline]
fn quat_from_axis_angle(axis: [f32; 3], angle: f32) -> [f32; 4] {
    let l = len3(axis);
    if l <= 1e-8 {
        return [0.0, 0.0, 0.0, 1.0];
    }
    let h = angle * 0.5;
    let s = sin_f32(h) / l;
    [axis[0] * s, axis[1] * s, axis[2] * s, cos_f32(h)]
}

#[inline]
fn quat_mul(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [
        a[3] * b[0] + a[0] * b[3] + a[1] * b[2] - a[2] * b[1],
        a[3] * b[1] - a[0] * b[2] + a[1] * b[3] + a[2] * b[0],
        a[3] * b[2] + a[0] * b[1] - a[1] * b[0] + a[2] * b[3],
        a[3] * b[3] - a[0] * b[0] - a[1] * b[1] - a[2] * b[2],
    ]
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
