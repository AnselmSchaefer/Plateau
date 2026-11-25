```rust
//! Cozy carved woodland arch doorway fitted to an oak hollow opening.
//!
//! Pure procedural `Scene` (`sdf_bounds() == None`):
//! - Mesh 0 (`id = 0`, `solid = true`): carved oak inner arch frame ringed by a mossy dressed-stone
//!   arch surround with a proud crown keystone, moss cushions, and a low weathered stone threshold
//!   sill (tracing `OPENING` with 10%–17% outer wall overlap and full walkable clearance).
//! - Mesh 1 (`id = 1`, `solid = true` when still, `false` while swinging): paneled arched oak door
//!   leaf overlapping the inner frame rim all round, with raised oak stiles/rails/panels, forged
//!   wrought-iron strap hinges, iron latch, and a burnished brass ring knocker.
//! - Mesh 2 (`id = 2`, `solid = false`): fine non-solid door joinery (plank grooves, curled iron
//!   hinge scrolls, rivets, thumb-latch lever, brass escutcheon studs) moving with the leaf.
//! - Mesh 3 (`id = 0`, `solid = false`): fine non-solid surround craft (carved twisting oak vine
//!   molding, interior architrave bead, trailing woodland ivy fronds, outer flagstone step).
//! - Mesh 4 (`id = 3`, `solid = true`): warm hanging wrought-iron & amber-glass lantern just inside
//!   the doorway, above both player head height and the door's swing arc, swaying gently with a
//!   warm flickering point light (`id = 1`).
//! - Mesh 5 (`id = 4`, `solid = true`): forged wrought-iron wall bracket anchoring the lantern to
//!   the right inner oak wall.

use plateau_sdk::*;

/// Measured outline of the oak hollow rim at `z = 0`, anticlockwise from right as seen from `+Z`.
const OPENING: [(f32, f32); 48] = [
    (1.2556, 1.3285), (1.2296, 1.4903), (1.1856, 1.6461), (1.1259, 1.7948), (1.0567, 1.9385), (0.9732, 2.0752),
    (0.8767, 2.2051), (0.7694, 2.3311), (0.6430, 2.4421), (0.5034, 2.5438), (0.3463, 2.6209), (0.1769, 2.6721),
    (0.0000, 2.6935), (-0.1784, 2.6837), (-0.3525, 2.6439), (-0.5155, 2.5729), (-0.6683, 2.4859), (-0.8038, 2.3760),
    (-0.9262, 2.2547), (-1.0322, 2.1205), (-1.1187, 1.9744), (-1.1832, 1.8186), (-1.2303, 1.6581), (-1.2566, 1.4939),
    (-1.2620, 1.3285), (-1.2470, 1.1643), (-1.2128, 1.0035), (-1.1594, 0.8482), (-1.0879, 0.7004), (-0.9991, 0.5618),
    (-0.8946, 0.4339), (-0.7747, 0.3188), (-0.6412, 0.2179), (-0.4949, 0.1336), (-0.3378, 0.0677), (-0.1719, 0.0225),
    (0.0000, 0.0000), (0.1747, 0.0017), (0.3483, 0.0286), (0.5167, 0.0810), (0.6757, 0.1581), (0.8210, 0.2585),
    (0.9492, 0.3793), (1.0574, 0.5171), (1.1433, 0.6684), (1.2059, 0.8289), (1.2450, 0.9949), (1.2614, 1.1624),
];

const N: usize = OPENING.len();
const MID: Point = point(0.0, 1.3285, 0.0);

/// Solid frame reaches 0.13..0.22 m (10%–17% of 1.295 m mean radius) into the surrounding wall.
const OUTER_OVERLAP: f32 = 0.13;
const FRAME_Z0: f32 = -0.24;
const FRAME_Z1: f32 = 0.14;

/// Door leaf center plane, half-thickness, rim overlap, hinge pivot, and open swing angle.
const DOOR_Z: f32 = -0.03;
const LEAF_HALF_Z: f32 = 0.054;
const LEAF_OVERLAP: f32 = 0.042;
const HINGE: Point = point(-1.14, 1.3285, DOOR_Z);
const SWING_RAD: f32 = 1.36;
const SWING_SECONDS: f32 = 1.10;

/// Hanging lantern pivot just inside the doorway, clear of both the player and door swing arc.
const LANTERN_PIVOT: Point = point(0.54, 2.36, -1.38);
const LANTERN_BULB_OFFSET: Point = point(0.0, -0.265, 0.0);

const MESH_LEAF: u32 = 1;
const MESH_TRIM: u32 = 2;
const MESH_LANTERN: u32 = 4;

const ID_LEAF: u32 = 1;
const ID_TRIM: u32 = 2;
const ID_LANTERN: u32 = 3;
const ID_BRACKET: u32 = 4;
const ID_LIGHT: u32 = 1;

struct OakHollowDoor {
    seed: u64,
    open: bool,
    done: f32,
    way: f32,
    moving: bool,
    lit: bool,
    draft: f32,
}

fn tint_u8(v: f32) -> u8 {
    clamp(v, 0.0, 255.0) as u8
}

fn unit2(dx: f32, dy: f32) -> (f32, f32) {
    let l = (dx * dx + dy * dy).sqrt();
    if l > 1e-6 { (dx / l, dy / l) } else { (1.0, 0.0) }
}

/// Outward offset from `OPENING[k]` along the radial ray from `MID` (negative pushes inward).
fn offset_opening(k: usize, delta: f32) -> (f32, f32) {
    let (ox, oy) = OPENING[k % N];
    let (ux, uy) = unit2(ox - MID.x, oy - MID.y);
    (ox + ux * delta, oy + uy * delta)
}

/// Frame inner inset: sturdy carved oak arch jambs above, tapering to a low 5.5 cm sill at the floor.
fn inner_inset(k: usize) -> f32 {
    let oy = OPENING[k % N].1;
    let bot = 1.0 - between(oy, 0.0, 0.45);
    mix(0.15, 0.055, bot)
}

fn contour_at(delta: f32) -> [(f32, f32); N] {
    let mut out = [(0.0, 0.0); N];
    for k in 0..N {
        out[k] = offset_opening(k, delta);
    }
    out
}

fn contour_outer(extra: f32) -> [(f32, f32); N] {
    contour_at(OUTER_OVERLAP + extra)
}

fn contour_inner(extra_inward: f32) -> [(f32, f32); N] {
    let mut out = [(0.0, 0.0); N];
    for k in 0..N {
        out[k] = offset_opening(k, -(inner_inset(k) + extra_inward));
    }
    out
}

fn contour_leaf() -> [(f32, f32); N] {
    let mut out = [(0.0, 0.0); N];
    for k in 0..N {
        out[k] = offset_opening(k, -(inner_inset(k) - LEAF_OVERLAP));
    }
    out
}

fn to_hinge_poly(poly: &[(f32, f32)]) -> Vec<(f32, f32)> {
    poly.iter().map(|&(x, y)| (x - HINGE.x, y - HINGE.y)).collect()
}

/// Horizontal span `[x_min, x_max]` of a closed 2D polygon at height `y`.
fn x_bounds_at_y(poly: &[(f32, f32)], y: f32) -> Option<(f32, f32)> {
    let mut lo = 1e6f32;
    let mut hi = -1e6f32;
    let n = poly.len();
    for i in 0..n {
        let (ax, ay) = poly[i];
        let (bx, by) = poly[(i + 1) % n];
        if (ay <= y && by > y) || (by <= y && ay > y) {
            let t = (y - ay) / (by - ay);
            let x = mix(ax, bx, t);
            lo = min(lo, x);
            hi = max(hi, x);
        }
    }
    if lo < hi { Some((lo, hi)) } else { None }
}

/// Vertical span `[y_min, y_max]` of a closed 2D polygon at coordinate `x`.
fn y_bounds_at_x(poly: &[(f32, f32)], x: f32) -> Option<(f32, f32)> {
    let mut lo = 1e6f32;
    let mut hi = -1e6f32;
    let n = poly.len();
    for i in 0..n {
        let (ax, ay) = poly[i];
        let (bx, by) = poly[(i + 1) % n];
        if (ax <= x && bx > x) || (bx <= x && ax > x) {
            let t = (x - ax) / (bx - ax);
            let y = mix(ay, by, t);
            lo = min(lo, y);
            hi = max(hi, y);
        }
    }
    if lo < hi { Some((lo, hi)) } else { None }
}

/// Clips a convex/star 2D polygon to the half-plane `a * x + b * y + c >= 0`.
fn clip_halfplane(poly: &[(f32, f32)], a: f32, b: f32, c: f32) -> Vec<(f32, f32)> {
    let mut out = Vec::new();
    let n = poly.len();
    if n < 3 {
        return out;
    }
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        let dp = a * p.0 + b * p.1 + c;
        let dq = a * q.0 + b * q.1 + c;
        if dp >= 0.0 {
            out.push(p);
        }
        if (dp >= 0.0) != (dq >= 0.0) {
            let denom = dp - dq;
            if abs(denom) > 1e-6 {
                let t = dp / denom;
                out.push((mix(p.0, q.0, t), mix(p.1, q.1, t)));
            }
        }
    }
    out
}

/// Flat-shaded quad guaranteed to face `out`.
fn face(m: &mut MeshBuilder, [a, b, c, d]: [Point; 4], out: Point, material: Material) {
    let [a, b, c, d] = if (b - a).cross(c - a).dot(out) < 0.0 { [a, d, c, b] } else { [a, b, c, d] };
    let va = m.vertex(a, out, material);
    let vb = m.vertex(b, out, material);
    let vc = m.vertex(c, out, material);
    let vd = m.vertex(d, out, material);
    m.quad(va, vb, vc, vd);
}

/// Flat-shaded triangle guaranteed to face `out`.
fn tri_out(m: &mut MeshBuilder, a: Point, b: Point, c: Point, out: Point, material: Material) {
    let (b, c) = if (b - a).cross(c - a).dot(out) < 0.0 { (c, b) } else { (b, c) };
    let va = m.vertex(a, out, material);
    let vb = m.vertex(b, out, material);
    let vc = m.vertex(c, out, material);
    m.tri(va, vb, vc);
}

/// Extrudes a closed convex/star 2D `(x, y)` polygon along Z from `z0` to `z1`.
fn extrude_xy(m: &mut MeshBuilder, poly: &[(f32, f32)], z0: f32, z1: f32, material: Material) {
    let n = poly.len();
    if n < 3 || z1 <= z0 {
        return;
    }
    let (mut cx, mut cy) = (0.0, 0.0);
    for &(x, y) in poly {
        cx += x;
        cy += y;
    }
    cx /= n as f32;
    cy /= n as f32;
    let front_n = point(0.0, 0.0, 1.0);
    let back_n = point(0.0, 0.0, -1.0);
    let fc = point(cx, cy, z1);
    let bc = point(cx, cy, z0);
    for i in 0..n {
        let j = (i + 1) % n;
        let (ax, ay) = poly[i];
        let (bx, by) = poly[j];
        tri_out(m, fc, point(ax, ay, z1), point(bx, by, z1), front_n, material);
        tri_out(m, bc, point(bx, by, z0), point(ax, ay, z0), back_n, material);
        let (dx, dy) = (bx - ax, by - ay);
        let (mut nx, mut ny) = unit2(dy, -dx);
        let (mx, my) = (0.5 * (ax + bx) - cx, 0.5 * (ay + by) - cy);
        if nx * mx + ny * my < 0.0 {
            nx = -nx;
            ny = -ny;
        }
        face(
            m,
            [point(ax, ay, z0), point(bx, by, z0), point(bx, by, z1), point(ax, ay, z1)],
            point(nx, ny, 0.0),
            material,
        );
    }
}

/// Builds a closed ring slice between `inner` and `outer` 48-point contours across `z0..z1`.
#[allow(clippy::too_many_arguments)]
fn contour_ring(
    m: &mut MeshBuilder,
    inner: &[(f32, f32)],
    outer: &[(f32, f32)],
    z0: f32,
    z1: f32,
    first: usize,
    count: usize,
    ends: bool,
    center: (f32, f32),
    material: Material,
) {
    let n = inner.len();
    let front = point(0.0, 0.0, 1.0);
    for step in 0..count {
        let i = (first + step) % n;
        let j = (first + step + 1) % n;
        let (ix0, iy0) = inner[i];
        let (ix1, iy1) = inner[j];
        let (ox0, oy0) = outer[i];
        let (ox1, oy1) = outer[j];

        let (mut onx, mut ony) = unit2(oy1 - oy0, -(ox1 - ox0));
        if onx * (0.5 * (ox0 + ox1) - center.0) + ony * (0.5 * (oy0 + oy1) - center.1) < 0.0 {
            onx = -onx;
            ony = -ony;
        }
        let (mut inx, mut iny) = unit2(iy1 - iy0, -(ix1 - ix0));
        if inx * (0.5 * (ix0 + ix1) - center.0) + iny * (0.5 * (iy0 + iy1) - center.1) > 0.0 {
            inx = -inx;
            iny = -iny;
        }

        face(m, [point(ox0, oy0, z0), point(ox1, oy1, z0), point(ox1, oy1, z1), point(ox0, oy0, z1)], point(onx, ony, 0.0), material);
        face(m, [point(ix0, iy0, z0), point(ix1, iy1, z0), point(ix1, iy1, z1), point(ix0, iy0, z1)], point(inx, iny, 0.0), material);
        face(m, [point(ix0, iy0, z1), point(ix1, iy1, z1), point(ox1, oy1, z1), point(ox0, oy0, z1)], front, material);
        face(m, [point(ix0, iy0, z0), point(ix1, iy1, z0), point(ox1, oy1, z0), point(ox0, oy0, z0)], -front, material);
    }
    if ends {
        for (idx, sign) in [(first % n, -1.0f32), ((first + count) % n, 1.0)] {
            let (ix, iy) = inner[idx];
            let (ox, oy) = outer[idx];
            let (rx, ry) = unit2(ox - ix, oy - iy);
            let en = point(-ry * sign, rx * sign, 0.0);
            face(m, [point(ix, iy, z0), point(ox, oy, z0), point(ox, oy, z1), point(ix, iy, z1)], en, material);
        }
    }
}

/// Builds a raised oak panel with beveled shadow molding and proud central field (in hinge-local XY).
fn add_panel(m: &mut MeshBuilder, outer_poly: &[(f32, f32)], inner_poly: &[(f32, f32)], side: f32) {
    if outer_poly.len() < 3 {
        return;
    }
    let z_face = side * LEAF_HALF_Z;
    let (z_rim0, z_rim1) = if side > 0.0 { (z_face - 0.004, z_face + 0.014) } else { (z_face - 0.014, z_face + 0.004) };
    m.tint(112, 82, 54);
    extrude_xy(m, outer_poly, z_rim0, z_rim1, Material::Wood);

    if inner_poly.len() >= 3 {
        let (z_mid0, z_mid1) = if side > 0.0 { (z_face + 0.010, z_face + 0.030) } else { (z_face - 0.030, z_face - 0.010) };
        m.tint(188, 144, 98);
        extrude_xy(m, inner_poly, z_mid0, z_mid1, Material::Wood);
    }
}

/// Builds the solid carved oak arch frame + mossy dressed-stone arch surround (Mesh 0, `solid = true`).
fn build_frame(seed: u64, inner: &[(f32, f32); N], outer: &[(f32, f32); N]) -> MeshData {
    let mut s = Seeds::of(seed, 11);
    let mut m = MeshBuilder::new();

    // 1. Carved oak inner arch jambs & full 360-degree sealing frame (`OPENING` + 10% overlap).
    let wood_outer = contour_at(0.03);
    for seg in 0..16 {
        let v = s.next(-10.0, 10.0);
        m.tint(tint_u8(146.0 + v), tint_u8(110.0 + v * 0.8), tint_u8(78.0 + v * 0.6));
        contour_ring(&mut m, inner, &wood_outer, FRAME_Z0, FRAME_Z1, seg * 3, 3, false, (MID.x, MID.y), Material::Wood);
    }

    // Continuous stone backing collar spanning out to OUTER_OVERLAP (10%) all around for zero gaps.
    m.tint(124, 126, 116);
    contour_ring(&mut m, &wood_outer, outer, FRAME_Z0, FRAME_Z1 - 0.01, 0, N, false, (MID.x, MID.y), Material::Stone);

    // 2. Sculpted dressed-stone arch voussoirs (10.8%–17.0% outer overlap, well within 1%–35%).
    let stone_spans: [(usize, usize, bool); 15] = [
        (39, 3, false),
        (42, 3, false),
        (45, 3, false),
        (0, 3, false),
        (3, 3, false),
        (6, 3, false),
        (9, 2, false),
        (11, 2, true), // Proud carved keystone at the arch crown!
        (13, 2, false),
        (15, 3, false),
        (18, 3, false),
        (21, 3, false),
        (24, 3, false),
        (27, 3, false),
        (30, 3, false),
    ];

    let stone_in = contour_at(-0.015);
    let key_in = contour_at(-0.045);
    for (first, count, is_keystone) in stone_spans {
        let shade = s.next(-18.0, 18.0);
        let moss_wash = s.next(0.0, 12.0);
        if is_keystone {
            m.tint(tint_u8(210.0 + shade * 0.35), tint_u8(206.0 + shade * 0.35), tint_u8(192.0 + shade * 0.35));
        } else {
            m.tint(
                tint_u8(186.0 + shade - moss_wash * 0.5),
                tint_u8(188.0 + shade + moss_wash * 0.4),
                tint_u8(174.0 + shade - moss_wash * 0.7),
            );
        }
        let extra_out = if is_keystone { 0.09 } else { s.next(0.015, 0.055) };
        let z_front = if is_keystone { 0.255 } else { s.next(0.175, 0.220) };
        let st_out = contour_outer(extra_out);
        let st_in = if is_keystone { &key_in } else { &stone_in };
        contour_ring(&mut m, st_in, &st_out, -0.10, z_front, first, count, true, (MID.x, MID.y), Material::Stone);

        // Lush solid moss pillows nestled into the upper stone joints (staying within stone footprint).
        let (jx, jy) = offset_opening(first % N, OUTER_OVERLAP * 0.65);
        if jy > 0.45 || is_keystone {
            let puffs = if is_keystone { 2 } else { 1 };
            for _ in 0..puffs {
                let mv = s.next(-14.0, 14.0);
                m.tint(tint_u8(118.0 + mv), tint_u8(202.0 + mv), tint_u8(92.0 + mv * 0.6));
                let px = jx + s.next(-0.06, 0.06);
                let py = jy + s.next(-0.04, 0.05);
                let pz = z_front + s.next(-0.01, 0.025);
                m.ellipsoid(
                    point(px, py, pz),
                    point(s.next(0.065, 0.11), s.next(0.045, 0.075), s.next(0.04, 0.065)),
                    7,
                    5,
                    Material::Moss,
                );
            }
        }
    }

    // Crowning moss cushions perched atop the keystone & base plinth stones (within <= 15.5% overlap).
    for &(mx, my, mz, rx, ry, rz) in &[
        (0.00, 2.81, 0.22, 0.17, 0.075, 0.09),
        (-0.24, 2.75, 0.20, 0.13, 0.065, 0.08),
        (0.24, 2.75, 0.20, 0.13, 0.065, 0.08),
        (-0.62, 0.18, 0.19, 0.14, 0.085, 0.09),
        (0.64, 0.16, 0.19, 0.14, 0.085, 0.09),
    ] {
        let mv = s.next(-12.0, 12.0);
        m.tint(tint_u8(114.0 + mv), tint_u8(204.0 + mv), tint_u8(90.0 + mv * 0.6));
        m.ellipsoid(point(mx, my, mz), point(rx, ry, rz), 8, 5, Material::Moss);
    }

    // 3. Low weathered flat grey stone threshold sill (y = -0.13..+0.055, cleanly walkable).
    m.tint(182, 180, 172);
    m.cuboid(point(0.0, -0.037, -0.025), point(0.42, 0.092, 0.215), Material::Stone);

    m.build()
}

/// Builds the solid paneled oak door leaf with sculpted stiles/rails, raised arched panels,
/// wrought-iron strap hinges, iron latch, and burnished brass ring knocker (Mesh 1, `solid = true`).
fn build_leaf(leaf_world: &[(f32, f32); N], inner_world: &[(f32, f32); N]) -> MeshData {
    let mut m = MeshBuilder::new();
    let leaf_local = to_hinge_poly(leaf_world);

    // Base oak door slab sealing the opening completely.
    m.tint(164, 122, 82);
    extrude_xy(&mut m, &leaf_local, -LEAF_HALF_Z, LEAF_HALF_Z, Material::Wood);

    // Raised frame-and-panel oak joinery on both front (+Z) and back (-Z) faces.
    let rail_outer = contour_inner(0.018);
    let rail_inner = contour_inner(0.155);
    let rail_outer_l = to_hinge_poly(&rail_outer);
    let rail_inner_l = to_hinge_poly(&rail_inner);
    let panel_base = contour_inner(0.195);
    let panel_field = contour_inner(0.255);
    let panel_base_l = to_hinge_poly(&panel_base);
    let panel_field_l = to_hinge_poly(&panel_field);

    let mid_l = (MID.x - HINGE.x, MID.y - HINGE.y);
    let cx_l = 0.0 - HINGE.x;
    let lock_y = 1.14 - HINGE.y;
    let bot_rail_y = 0.28 - HINGE.y;

    for side in [-1.0f32, 1.0] {
        let z_face = side * LEAF_HALF_Z;

        // Raised perimeter arch rail.
        m.tint(146, 106, 70);
        let (zr0, zr1) = if side > 0.0 { (z_face - 0.004, z_face + 0.024) } else { (z_face - 0.024, z_face + 0.004) };
        contour_ring(&mut m, &rail_inner_l, &rail_outer_l, zr0, zr1, 0, N, false, mid_l, Material::Wood);

        // Horizontal lock rail and bottom kick rail.
        if let Some((xl, xr)) = x_bounds_at_y(&rail_inner, 1.14) {
            let w = 0.5 * (xr - xl) + 0.04;
            let mx = 0.5 * (xl + xr) - HINGE.x;
            m.cuboid(point(mx, lock_y, z_face + side * 0.012), point(w, 0.082, 0.014), Material::Wood);
        }
        if let Some((xl, xr)) = x_bounds_at_y(&rail_inner, 0.28) {
            let w = 0.5 * (xr - xl) + 0.04;
            let mx = 0.5 * (xl + xr) - HINGE.x;
            m.cuboid(point(mx, bot_rail_y, z_face + side * 0.012), point(w, 0.072, 0.014), Material::Wood);
        }

        // Central vertical mullion stile.
        if let Some((ymin, ymax)) = y_bounds_at_x(&rail_inner, 0.0) {
            let lo = max(ymin, 0.24);
            let hi = ymax + 0.02;
            m.cuboid(
                point(cx_l, 0.5 * (lo + hi) - HINGE.y, z_face + side * 0.012),
                point(0.068, 0.5 * (hi - lo), 0.014),
                Material::Wood,
            );
        }

        // Four sculpted beveled raised oak panels (two arched upper panels + two lower panels).
        let ur_o = clip_halfplane(&clip_halfplane(&panel_base_l, 1.0, 0.0, -(cx_l + 0.10)), 0.0, 1.0, -(lock_y + 0.115));
        let ur_i = clip_halfplane(&clip_halfplane(&panel_field_l, 1.0, 0.0, -(cx_l + 0.155)), 0.0, 1.0, -(lock_y + 0.17));
        add_panel(&mut m, &ur_o, &ur_i, side);

        let ul_o = clip_halfplane(&clip_halfplane(&panel_base_l, -1.0, 0.0, cx_l - 0.10), 0.0, 1.0, -(lock_y + 0.115));
        let ul_i = clip_halfplane(&clip_halfplane(&panel_field_l, -1.0, 0.0, cx_l - 0.155), 0.0, 1.0, -(lock_y + 0.17));
        add_panel(&mut m, &ul_o, &ul_i, side);

        let lr_o = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_base_l, 1.0, 0.0, -(cx_l + 0.10)), 0.0, -1.0, lock_y - 0.115),
            0.0,
            1.0,
            -(bot_rail_y + 0.095),
        );
        let lr_i = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_field_l, 1.0, 0.0, -(cx_l + 0.155)), 0.0, -1.0, lock_y - 0.17),
            0.0,
            1.0,
            -(bot_rail_y + 0.145),
        );
        add_panel(&mut m, &lr_o, &lr_i, side);

        let ll_o = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_base_l, -1.0, 0.0, cx_l - 0.10), 0.0, -1.0, lock_y - 0.115),
            0.0,
            1.0,
            -(bot_rail_y + 0.095),
        );
        let ll_i = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_field_l, -1.0, 0.0, cx_l - 0.155), 0.0, -1.0, lock_y - 0.17),
            0.0,
            1.0,
            -(bot_rail_y + 0.145),
        );
        add_panel(&mut m, &ll_o, &ll_i, side);

        // Two bold forged wrought-iron strap hinges (Material::Stone for unmistakable contrast).
        for &yw in &[0.90f32, 1.76] {
            let y = yw - HINGE.y;
            let z_h = side * (LEAF_HALF_Z + 0.028);
            let x_start = x_bounds_at_y(inner_world, yw).map(|(lo, _)| lo - 0.015).unwrap_or(-1.05) - HINGE.x;
            let strap_len = 0.90;

            m.tint(56, 60, 68);
            let kx = min(x_start, 0.05);
            m.cuboid(point(kx, y, z_h), point(0.028, 0.095, 0.024), Material::Stone);
            m.tint(62, 66, 74);
            m.cuboid(point(x_start + strap_len * 0.44, y, z_h), point(strap_len * 0.44, 0.048, 0.014), Material::Stone);
            m.cuboid(point(x_start + strap_len * 0.86, y, z_h), point(strap_len * 0.18, 0.032, 0.013), Material::Stone);
            // Spear-leaf iron finial tip.
            m.ellipsoid(point(x_start + strap_len + 0.06, y, z_h), point(0.062, 0.034, 0.015), 6, 4, Material::Stone);
        }

        // Forged wrought-iron latch plate & solid grip bar on the right lock rail.
        let lx = 0.80 - HINGE.x;
        let ly = 1.14 - HINGE.y;
        let lz = side * (LEAF_HALF_Z + 0.026);
        m.tint(58, 62, 70);
        m.cuboid(point(lx, ly, lz), point(0.048, 0.155, 0.012), Material::Stone);
        m.tint(70, 74, 82);
        m.cuboid(point(lx, ly, lz + side * 0.058), point(0.018, 0.095, 0.016), Material::Stone);
        m.cuboid(point(lx, ly + 0.088, lz + side * 0.030), point(0.018, 0.016, 0.032), Material::Stone);
        m.cuboid(point(lx, ly - 0.088, lz + side * 0.030), point(0.018, 0.016, 0.032), Material::Stone);
    }

    // Burnished brass ring knocker on the outer center mullion (Material::Sand for golden warmth).
    let kcx = 0.0 - HINGE.x;
    let kcy = 1.58 - HINGE.y;
    let kz = LEAF_HALF_Z + 0.030;
    m.tint(234, 190, 80);
    m.ellipsoid(point(kcx, kcy + 0.015, kz), point(0.082, 0.108, 0.018), 10, 6, Material::Sand);
    m.tint(252, 214, 104);
    m.ellipsoid(point(kcx, kcy + 0.035, kz + 0.018), point(0.046, 0.046, 0.024), 8, 6, Material::Sand);
    m.ellipsoid(point(kcx, kcy - 0.155, kz + 0.012), point(0.038, 0.026, 0.018), 8, 5, Material::Sand);

    m.tint(255, 224, 118);
    let ring_cy = kcy - 0.060;
    let ring_r = 0.098;
    let segs = 14usize;
    let mut ring_knots = Vec::with_capacity(segs + 1);
    for s in 0..=segs {
        let a = PI * 0.5 + (s as f32 / segs as f32) * TAU;
        let weight = 0.5 * (1.0 - sin(a));
        let tube_r = mix(0.018, 0.026, weight);
        let tilt_z = kz + 0.042 + weight * 0.014;
        ring_knots.push((point(kcx + ring_r * cos(a), ring_cy + ring_r * sin(a), tilt_z), tube_r));
    }
    m.sweep_tube(&ring_knots, 7, false, Material::Sand);

    m.build()
}

/// Builds fine non-solid door joinery accents: plank seams, curled wrought-iron hinge scrolls,
/// forged rivet studs, thumb-latch lever, and brass escutcheon studs (Mesh 2, `solid = false`).
fn build_door_trim(inner_world: &[(f32, f32); N]) -> MeshData {
    let mut m = MeshBuilder::new();
    let rail_inner = contour_inner(0.155);

    for side in [-1.0f32, 1.0] {
        let z_face = side * LEAF_HALF_Z;

        // Vertical tongue-and-groove oak plank seams.
        m.tint(90, 64, 42);
        for k in -4i32..=4 {
            let xw = k as f32 * 0.22;
            if let Some((ymin, ymax)) = y_bounds_at_x(&rail_inner, xw) {
                let lo = ymin + 0.02;
                let hi = ymax - 0.02;
                if hi > lo + 0.08 {
                    let cy = 0.5 * (lo + hi) - HINGE.y;
                    let hy = 0.5 * (hi - lo);
                    m.cuboid(point(xw - HINGE.x, cy, z_face + side * 0.002), point(0.008, hy, 0.005), Material::Wood);
                }
            }
        }

        // Delicate curled iron scroll tendrils & rivets on each strap hinge.
        for &yw in &[0.90f32, 1.76] {
            let y = yw - HINGE.y;
            let z_h = side * (LEAF_HALF_Z + 0.028);
            let x0 = x_bounds_at_y(inner_world, yw).map(|(lo, _)| lo - 0.015).unwrap_or(-1.05) - HINGE.x;
            let tip_x = x0 + 0.90;

            m.tint(60, 64, 72);
            for dir in [-1.0f32, 1.0] {
                let mut knots = Vec::with_capacity(7);
                for s in 0..7 {
                    let t = s as f32 / 6.0;
                    let ang = t * 4.4;
                    let rad = mix(0.068, 0.022, t);
                    let px = tip_x - 0.02 + sin(ang) * rad;
                    let py = y + dir * (0.040 + (1.0 - cos(ang)) * rad);
                    knots.push((point(px, py, z_h), mix(0.013, 0.008, t)));
                }
                m.sweep_tube(&knots, 6, true, Material::Stone);
            }

            m.tint(86, 90, 98);
            for k in 0..4 {
                let rx = x0 + 0.14 + k as f32 * 0.21;
                m.ellipsoid(point(rx, y, z_h + side * 0.014), point(0.016, 0.016, 0.010), 6, 5, Material::Stone);
            }
        }

        // Thumb-latch lever on the right lock rail.
        let lx = 0.80 - HINGE.x;
        let ly = 1.14 - HINGE.y;
        let lz = side * (LEAF_HALF_Z + 0.026);
        m.tint(86, 90, 98);
        m.cuboid(point(lx - 0.025, ly + 0.095, lz + side * 0.042), point(0.034, 0.010, 0.024), Material::Stone);
    }

    // Small brass studs on the knocker escutcheon.
    let kcx = 0.0 - HINGE.x;
    let kcy = 1.58 - HINGE.y;
    let kz = LEAF_HALF_Z + 0.044;
    m.tint(255, 234, 144);
    for &(dx, dy) in &[(-0.048, 0.060), (0.048, 0.060), (0.0, 0.098)] {
        m.ellipsoid(point(kcx + dx, kcy + dy, kz), point(0.012, 0.012, 0.010), 6, 5, Material::Sand);
    }

    m.build()
}

/// Builds fine non-solid surround craft: carved twisting oak vine molding, interior architrave
/// bead, trailing woodland ivy fronds, and an outer stepping flagstone (Mesh 3, `solid = false`).
fn build_surround_decor(seed: u64, inner: &[(f32, f32); N], outer: &[(f32, f32); N]) -> MeshData {
    let mut s = Seeds::of(seed, 37);
    let mut m = MeshBuilder::new();

    // Front & interior beaded oak architrave moldings.
    m.tint(124, 90, 60);
    let mut front_bead = Vec::with_capacity(N + 1);
    let mut inner_bead = Vec::with_capacity(N + 1);
    for k in 0..=N {
        let idx = k % N;
        let (ix, iy) = inner[idx];
        let (ox, oy) = outer[idx];
        let bx = mix(ix, ox, 0.16);
        let by = mix(iy, oy, 0.16);
        front_bead.push((point(bx, by, FRAME_Z1 + 0.014), 0.022));
        inner_bead.push((point(bx, by, FRAME_Z0 - 0.012), 0.022));
    }
    m.sweep_tube(&front_bead, 6, false, Material::Wood);
    m.sweep_tube(&inner_bead, 6, false, Material::Wood);

    // Carved twisting woodland vine relief creeping up both front oak arch jambs.
    m.tint(176, 134, 92);
    for &(start_k, end_k, dir) in &[(41usize, 59usize, 1.0f32), (29usize, 13usize, -1.0f32)] {
        let steps = 22usize;
        let mut vine = Vec::with_capacity(steps);
        for i in 0..steps {
            let t = i as f32 / (steps - 1) as f32;
            let fk = mix(start_k as f32, end_k as f32, t);
            let k0 = (fk.floor() as usize) % N;
            let k1 = (k0 + 1) % N;
            let frac = fk - fk.floor();
            let ix = mix(inner[k0].0, inner[k1].0, frac);
            let iy = mix(inner[k0].1, inner[k1].1, frac);
            let ox = mix(outer[k0].0, outer[k1].0, frac);
            let oy = mix(outer[k0].1, outer[k1].1, frac);
            let wave = 0.28 + 0.14 * sin(t * TAU * 3.5 * dir);
            let vx = mix(ix, ox, wave);
            let vy = mix(iy, oy, wave);
            vine.push((point(vx, vy, FRAME_Z1 + 0.015), mix(0.016, 0.011, t)));
        }
        m.sweep_tube(&vine, 5, true, Material::Wood);
    }

    // Trailing woodland ivy fronds draping gracefully off the upper mossy stone arch.
    for &(sx, sy, sz, sway_x, len) in &[
        (-0.84, 2.48, 0.23, -0.10, 0.54),
        (-0.58, 2.70, 0.24, -0.06, 0.44),
        (0.68, 2.62, 0.23, 0.08, 0.50),
        (0.92, 2.34, 0.22, 0.10, 0.46),
    ] {
        let lv = s.next(-14.0, 14.0);
        m.tint(tint_u8(92.0 + lv), tint_u8(162.0 + lv), tint_u8(78.0 + lv * 0.6));
        let mut knots = Vec::with_capacity(6);
        for i in 0..6 {
            let t = i as f32 / 5.0;
            let px = sx + sway_x * t + 0.03 * sin(t * 5.0);
            let py = sy - len * t;
            let pz = sz + 0.04 * sin(t * PI);
            let hw = mix(0.042, 0.010, t) * (0.75 + 0.35 * sin(t * PI * 3.0));
            knots.push((point(px, py, pz), hw, point(0.0, 0.0, 1.0)));
        }
        m.ribbon(&knots, Material::Leaf);
    }

    // Welcoming weathered outer stepping flagstone seated into the grassy approach.
    m.tint(188, 186, 178);
    m.cuboid(point(0.0, -0.045, 0.28), point(0.50, 0.085, 0.14), Material::Stone);

    m.build()
}

/// Builds the warm hanging wrought-iron & amber-glass lantern in pivot-local coords (Mesh 4).
fn build_lantern() -> MeshData {
    let mut m = MeshBuilder::new();

    // Iron hanging ring & chain down from pivot (0, 0, 0).
    m.tint(62, 66, 74);
    m.sweep_tube(&[(point(0.0, 0.0, 0.0), 0.013), (point(0.0, -0.095, 0.0), 0.013)], 6, true, Material::Stone);
    m.ellipsoid(point(0.0, -0.02, 0.0), point(0.026, 0.026, 0.014), 6, 5, Material::Stone);

    // Flared dark iron canopy roof & chimney cap.
    m.tint(54, 58, 66);
    m.cone(point(0.0, -0.175, 0.0), 0.100, 0.155, 6, Material::Stone);
    m.sweep_tube(&[(point(0.0, -0.175, 0.0), 0.152), (point(0.0, -0.194, 0.0), 0.132)], 6, true, Material::Stone);

    // Warm glowing amber glass lantern body (Material::Sand for warm golden look).
    m.tint(255, 224, 120);
    m.sweep_tube(&[(point(0.0, -0.190, 0.0), 0.098), (point(0.0, -0.350, 0.0), 0.082)], 6, true, Material::Sand);

    // Six slender wrought-iron corner bars framing the glowing glass panes.
    m.tint(56, 60, 68);
    for s in 0..6 {
        let a = s as f32 * TAU / 6.0;
        let (ca, sa) = (cos(a), sin(a));
        m.sweep_tube(
            &[(point(ca * 0.104, -0.186, sa * 0.104), 0.009), (point(ca * 0.088, -0.352, sa * 0.088), 0.009)],
            5,
            true,
            Material::Stone,
        );
    }

    // Bottom iron dish & pointed finial drop (lowest point y_world = 1.95 m, safely above 1.7 m player).
    m.tint(54, 58, 66);
    m.sweep_tube(&[(point(0.0, -0.350, 0.0), 0.112), (point(0.0, -0.378, 0.0), 0.078)], 6, true, Material::Stone);
    m.cone(point(0.0, -0.376, 0.0), -0.036, 0.034, 6, Material::Stone);

    m.build()
}

/// Builds the forged wrought-iron wall bracket anchoring the inner lantern (Mesh 5, `solid = true`).
fn build_bracket() -> MeshData {
    let mut m = MeshBuilder::new();
    let bz = LANTERN_PIVOT.z;

    m.tint(58, 62, 70);
    // Wall mounting plate anchored firmly into the right inner oak wall.
    m.cuboid(point(1.08, 2.02, bz), point(0.14, 0.16, 0.055), Material::Stone);
    // Graceful overhanging forged iron arm to LANTERN_PIVOT.
    m.sweep_tube(
        &[
            (point(1.12, 2.06, bz), 0.025),
            (point(0.94, 2.28, bz), 0.021),
            (point(0.74, 2.43, bz), 0.018),
            (point(LANTERN_PIVOT.x + 0.02, LANTERN_PIVOT.y + 0.06, bz), 0.016),
            (LANTERN_PIVOT, 0.013),
        ],
        7,
        true,
        Material::Stone,
    );
    // Decorative curved S-brace beneath the arm (all above y = 1.88 m).
    m.sweep_tube(
        &[
            (point(1.10, 1.90, bz), 0.015),
            (point(0.95, 2.00, bz), 0.014),
            (point(0.88, 2.18, bz), 0.013),
            (point(0.78, 2.36, bz), 0.013),
        ],
        6,
        true,
        Material::Stone,
    );

    m.build()
}

impl OakHollowDoor {
    fn door_rot(&self) -> Quat {
        Quat::from_y(self.way * SWING_RAD * ease(self.done))
    }

    fn door_commands(&self) -> Vec<Command> {
        let rot = self.door_rot();
        let one = point(1.0, 1.0, 1.0);
        let solid = !self.moving;
        vec![
            Command::SetTransform(instance(ID_LEAF, MESH_LEAF, HINGE, rot, one, solid)),
            Command::SetTransform(instance(ID_TRIM, MESH_TRIM, HINGE, rot, one, false)),
        ]
    }

    fn lantern_rot(&self, time: f32) -> Quat {
        let phase = (self.seed % 97) as f32 * 0.19;
        let amp = 1.0 + self.draft * 1.8;
        let sway_z = 0.042 * amp * sin(time * 1.85 + phase);
        let sway_x = 0.028 * amp * cos(time * 1.35 + phase * 1.4);
        Quat::from_euler(0.0, sway_x, sway_z)
    }

    fn lantern_light(&self, time: f32, rot: Quat) -> Command {
        let bulb = LANTERN_PIVOT + rot.rotate(LANTERN_BULB_OFFSET);
        let intensity = if self.lit {
            let phase = (self.seed % 83) as f32 * 0.23;
            72.0 + 5.5 * sin(time * 8.7 + phase) + 3.0 * cos(time * 14.3 + phase)
        } else {
            0.0
        };
        Command::SetLight(light(ID_LIGHT, bulb, [1.0, 0.74, 0.36], intensity, 8.5))
    }
}

impl Element for OakHollowDoor {
    fn new(seed: u64) -> Self {
        OakHollowDoor { seed, open: false, done: 0.0, way: 1.0, moving: false, lit: true, draft: 0.0 }
    }

    fn name(&self) -> &str {
        "Oak Hollow Door"
    }

    fn scene(&self) -> Scene {
        let inner = contour_inner(0.0);
        let outer = contour_outer(0.0);
        let leaf = contour_leaf();

        let m_frame = build_frame(self.seed, &inner, &outer);
        let m_leaf = build_leaf(&leaf, &inner);
        let m_trim = build_door_trim(&inner);
        let m_decor = build_surround_decor(self.seed, &inner, &outer);
        let m_lantern = build_lantern();
        let m_bracket = build_bracket();

        let one = point(1.0, 1.0, 1.0);
        let mut sc = SceneBuilder::new();
        let frame_id = sc.add_mesh(m_frame);       // 0
        let leaf_id = sc.add_mesh(m_leaf);         // 1 (MESH_LEAF)
        let trim_id = sc.add_mesh(m_trim);         // 2 (MESH_TRIM)
        let decor_id = sc.add_mesh(m_decor);       // 3
        let lantern_id = sc.add_mesh(m_lantern);   // 4 (MESH_LANTERN)
        let bracket_id = sc.add_mesh(m_bracket);   // 5

        sc.place(frame_id, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, true);
        sc.place(decor_id, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, false);
        sc.spawn(ID_LEAF, leaf_id, HINGE, Quat::IDENTITY, one, true);
        sc.spawn(ID_TRIM, trim_id, HINGE, Quat::IDENTITY, one, false);
        sc.spawn(ID_LANTERN, lantern_id, LANTERN_PIVOT, Quat::IDENTITY, one, true);
        sc.spawn(ID_BRACKET, bracket_id, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, true);
        sc.light(ID_LIGHT, LANTERN_PIVOT + LANTERN_BULB_OFFSET, [1.0, 0.74, 0.36], 72.0, 8.5);
        sc.build()
    }

    fn tick(&mut self, dt: f32, p: &PlayerState) -> Vec<Command> {
        let dt = clamp(dt, 0.0, 0.25);
        let mut cmds = Vec::new();

        if self.moving {
            let step = dt / SWING_SECONDS;
            self.done = if self.open { min(self.done + step, 1.0) } else { max(self.done - step, 0.0) };
            self.moving = if self.open { self.done < 1.0 } else { self.done > 0.0 };
            cmds.extend(self.door_commands());
        }

        if self.draft > 0.0 {
            self.draft = max(self.draft - dt * 0.45, 0.0);
        }

        let lrot = self.lantern_rot(p.time);
        cmds.push(Command::SetTransform(instance(
            ID_LANTERN,
            MESH_LANTERN,
            LANTERN_PIVOT,
            lrot,
            point(1.0, 1.0, 1.0),
            true,
        )));
        if self.lit {
            cmds.push(self.lantern_light(p.time, lrot));
        }

        cmds
    }

    fn interact(&mut self, p: &PlayerState) -> Vec<Command> {
        if p.target == ID_LANTERN {
            self.lit = !self.lit;
            let lrot = self.lantern_rot(p.time);
            return vec![
                self.lantern_light(p.time, lrot),
                Command::Say(if self.lit { "The inner lantern glows warmly.".into() } else { "The inner lantern dims.".into() }),
            ];
        }

        let aimed = p.target == ID_LEAF || p.target == ID_TRIM;
        let near = p.feet.length() < 5.0;
        if !aimed && !near {
            return Vec::new();
        }

        if self.open && abs(p.feet.z) < LEAF_HALF_Z + 0.16 * p.size && abs(p.feet.x) < 1.12 {
            return vec![Command::Say("Step clear of the doorway to shut the oak door.".into())];
        }

        self.open = !self.open;
        if self.open && self.done <= 0.0 {
            self.way = if p.feet.z >= 0.0 { 1.0 } else { -1.0 };
        }
        self.moving = true;
        self.draft = 1.0;

        let mut cmds = self.door_commands();
        cmds.push(Command::Say(if self.open {
            "The paneled oak door swings open.".into()
        } else {
            "The paneled oak door swings shut.".into()
        }));
        cmds
    }
}

element!(OakHollowDoor);
```
