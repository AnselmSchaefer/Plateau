//! Cozy carved woodland arch doorway fitted to an oak hollow opening.
//!
//! Pure procedural `Scene` (`sdf_bounds() == None`):
//! - Mesh 0 (`id = 0`, `solid = true`): carved oak arch frame tracing `OPENING` (10% outer overlap
//!   into the surrounding wood wall, low walkable sill) for clean collision and sealing.
//! - Mesh 1 (`id = 1`, `solid = true` when still, `false` while swinging): arched oak door leaf
//!   overlapping the inner frame rim all around so no light or sightlines leak when shut.
//! - Mesh 2 (`id = 2`, `solid = false`): raised arched oak panels, rails, mullion, tongue-and-groove
//!   seams, forged wrought-iron strap hinges, thumb-latch, and a burnished brass ring knocker.
//! - Mesh 3 (`id = 0`, `solid = false`): mossy dressed-stone arch surround with proud keystone,
//!   moss cushions, trailing woodland ivy, carved vine molding, threshold flagstone, and interior
//!   iron lantern bracket.
//! - Mesh 4 (`id = 3`, `solid = false`): warm hanging iron-and-amber-glass lantern just inside the
//!   doorway (clear of both the door's swing arc and a walking player) with a gentle swaying draft
//!   and flickering warm point light (`id = 1`).

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

/// Solid frame reaches 0.13 m (~10% of 1.295 m mean radius) into the wall all round.
const OUTER_OVERLAP: f32 = 0.13;
const FRAME_Z0: f32 = -0.24;
const FRAME_Z1: f32 = 0.14;

/// Door leaf center plane, half-thickness, overlap into inner frame, and hinge pivot.
const DOOR_Z: f32 = -0.02;
const LEAF_HALF_Z: f32 = 0.048;
const LEAF_OVERLAP: f32 = 0.040;
const HINGE: Point = point(-1.12, 1.3285, DOOR_Z);
const SWING_RAD: f32 = 1.62;
const SWING_SECONDS: f32 = 1.10;

/// Hanging lantern pivot just inside the doorway, clear of both the player and door sweep arc.
const LANTERN_PIVOT: Point = point(0.55, 2.34, -1.35);
const LANTERN_BULB_OFFSET: Point = point(0.0, -0.265, 0.0);

const MESH_LEAF: u32 = 1;
const MESH_TRIM: u32 = 2;
const MESH_LANTERN: u32 = 4;

const ID_LEAF: u32 = 1;
const ID_TRIM: u32 = 2;
const ID_LANTERN: u32 = 3;
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

/// Frame inner inset: sturdy carved arch jambs above, tapering to a low 5.5 cm sill at the floor.
fn inner_inset(k: usize) -> f32 {
    let oy = OPENING[k % N].1;
    let bot = 1.0 - between(oy, 0.0, 0.45);
    mix(0.14, 0.055, bot)
}

fn contour_outer(extra: f32) -> [(f32, f32); N] {
    let mut out = [(0.0, 0.0); N];
    for k in 0..N {
        out[k] = offset_opening(k, OUTER_OVERLAP + extra);
    }
    out
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

/// Builds a raised oak panel with a dark shadow bevel and lighter central field (in hinge-local XY).
fn add_panel(m: &mut MeshBuilder, outer_poly: &[(f32, f32)], inner_poly: &[(f32, f32)], side: f32) {
    if outer_poly.len() < 3 {
        return;
    }
    let z_face = side * LEAF_HALF_Z;
    let (z_rim0, z_rim1) = if side > 0.0 { (z_face - 0.004, z_face + 0.011) } else { (z_face - 0.011, z_face + 0.004) };
    m.tint(112, 82, 54);
    extrude_xy(m, outer_poly, z_rim0, z_rim1, Material::Wood);

    if inner_poly.len() >= 3 {
        let (z_mid0, z_mid1) = if side > 0.0 { (z_face + 0.006, z_face + 0.023) } else { (z_face - 0.023, z_face - 0.006) };
        m.tint(188, 144, 98);
        extrude_xy(m, inner_poly, z_mid0, z_mid1, Material::Wood);
    }
}

/// Adds a forged wrought-iron strap hinge with knuckle barrel, curling woodland scroll, and rivets.
fn add_iron_hinge(m: &mut MeshBuilder, y_world: f32, side: f32, inner_poly: &[(f32, f32)]) {
    let y = y_world - HINGE.y;
    let z_base = side * (LEAF_HALF_Z + 0.014);
    let x_start_world = x_bounds_at_y(inner_poly, y_world).map(|(lo, _)| lo - 0.02).unwrap_or(-1.05);
    let x0 = x_start_world - HINGE.x;
    let strap_len = 0.90;

    // Knuckle barrel along the hinge edge (only on front side or both faces).
    m.tint(56, 60, 68);
    let kx = min(x0, 0.03);
    m.sweep_tube(
        &[(point(kx, y - 0.085, z_base), 0.024), (point(kx, y + 0.085, z_base), 0.024)],
        8,
        true,
        Material::Stone,
    );
    m.ellipsoid(point(kx, y + 0.092, z_base), point(0.028, 0.020, 0.028), 8, 6, Material::Stone);
    m.ellipsoid(point(kx, y - 0.092, z_base), point(0.028, 0.020, 0.028), 8, 6, Material::Stone);

    // Main tapered forged iron strap.
    m.tint(62, 66, 74);
    m.cuboid(point(x0 + strap_len * 0.45, y, z_base), point(strap_len * 0.45, 0.042, 0.011), Material::Stone);
    m.cuboid(point(x0 + strap_len * 0.85, y, z_base), point(strap_len * 0.18, 0.028, 0.010), Material::Stone);

    // Woodland fleur-de-lis / curled scroll finial at the strap tip.
    let tip_x = x0 + strap_len;
    m.ellipsoid(point(tip_x + 0.06, y, z_base), point(0.055, 0.024, 0.012), 8, 6, Material::Stone);
    for dir in [-1.0f32, 1.0] {
        let mut knots = Vec::with_capacity(7);
        for s in 0..7 {
            let t = s as f32 / 6.0;
            let ang = t * 4.4;
            let rad = mix(0.068, 0.022, t);
            let px = tip_x - 0.02 + sin(ang) * rad;
            let py = y + dir * (0.035 + (1.0 - cos(ang)) * rad);
            knots.push((point(px, py, z_base), mix(0.013, 0.008, t)));
        }
        m.sweep_tube(&knots, 6, true, Material::Stone);
    }

    // Forged rivet heads along the strap.
    m.tint(84, 88, 96);
    for k in 0..4 {
        let rx = x0 + 0.14 + k as f32 * 0.21;
        m.ellipsoid(point(rx, y, z_base + side * 0.011), point(0.015, 0.015, 0.009), 6, 5, Material::Stone);
    }
}

/// Adds a wrought-iron latch plate and curved handle on the right side of the oak door.
fn add_iron_latch(m: &mut MeshBuilder, side: f32) {
    let x = 0.84 - HINGE.x;
    let y = 1.14 - HINGE.y;
    let z = side * (LEAF_HALF_Z + 0.012);

    // Vertical forged backplate with pointed woodland tips.
    m.tint(58, 62, 70);
    m.cuboid(point(x, y, z), point(0.042, 0.145, 0.009), Material::Stone);
    m.ellipsoid(point(x, y + 0.155, z), point(0.036, 0.036, 0.009), 8, 5, Material::Stone);
    m.ellipsoid(point(x, y - 0.155, z), point(0.036, 0.036, 0.009), 8, 5, Material::Stone);

    // Curved vertical grip handle.
    m.tint(70, 74, 82);
    let grip = [
        (point(x, y + 0.105, z), 0.013),
        (point(x, y + 0.085, z + side * 0.068), 0.014),
        (point(x, y, z + side * 0.084), 0.016),
        (point(x, y - 0.085, z + side * 0.068), 0.014),
        (point(x, y - 0.105, z), 0.013),
    ];
    m.sweep_tube(&grip, 7, true, Material::Stone);

    // Small thumb-latch lever.
    m.tint(86, 90, 98);
    m.cuboid(point(x - 0.02, y + 0.085, z + side * 0.045), point(0.030, 0.010, 0.022), Material::Stone);
}

/// Adds a burnished brass ring knocker with scalloped escutcheon on the outer door center.
fn add_brass_knocker(m: &mut MeshBuilder) {
    let cx = 0.0 - HINGE.x;
    let cy = 1.56 - HINGE.y;
    let z = LEAF_HALF_Z + 0.018;

    // Scalloped brass escutcheon backplate.
    m.tint(232, 188, 78);
    m.ellipsoid(point(cx, cy + 0.015, z), point(0.078, 0.102, 0.014), 12, 8, Material::Sand);
    m.tint(252, 214, 102);
    m.ellipsoid(point(cx, cy + 0.032, z + 0.014), point(0.044, 0.044, 0.022), 10, 8, Material::Sand);

    // Top pivot boss & lower strike anvil.
    m.tint(215, 170, 68);
    m.sweep_tube(
        &[(point(cx, cy + 0.032, z + 0.008), 0.022), (point(cx, cy + 0.032, z + 0.042), 0.020)],
        8,
        true,
        Material::Sand,
    );
    m.ellipsoid(point(cx, cy - 0.145, z + 0.008), point(0.034, 0.024, 0.016), 10, 6, Material::Sand);

    // Heavy hanging brass knocker ring.
    m.tint(255, 222, 114);
    let ring_cy = cy - 0.058;
    let ring_r = 0.088;
    let segs = 18usize;
    let mut ring_knots = Vec::with_capacity(segs + 1);
    for s in 0..=segs {
        let a = PI * 0.5 + (s as f32 / segs as f32) * TAU;
        let weight = 0.5 * (1.0 - sin(a));
        let tube_r = mix(0.015, 0.023, weight);
        let tilt_z = z + 0.040 + weight * 0.012;
        ring_knots.push((point(cx + ring_r * cos(a), ring_cy + ring_r * sin(a), tilt_z), tube_r));
    }
    m.sweep_tube(&ring_knots, 8, false, Material::Sand);

    // Decorative brass studs on the escutcheon.
    m.tint(255, 232, 140);
    for &(dx, dy) in &[(-0.046, 0.055), (0.046, 0.055), (0.0, 0.092)] {
        m.ellipsoid(point(cx + dx, cy + dy, z + 0.014), point(0.011, 0.011, 0.009), 6, 5, Material::Sand);
    }
}

/// Builds the solid carved oak arch frame (Mesh 0, `solid = true`).
fn build_frame(seed: u64, inner: &[(f32, f32); N], outer: &[(f32, f32); N]) -> MeshData {
    let mut s = Seeds::of(seed, 11);
    let mut m = MeshBuilder::new();

    // 16 seasoned oak arch segments tracing OPENING all 360 degrees round.
    for seg in 0..16 {
        let v = s.next(-10.0, 10.0);
        m.tint(tint_u8(152.0 + v), tint_u8(116.0 + v * 0.8), tint_u8(84.0 + v * 0.6));
        contour_ring(&mut m, inner, outer, FRAME_Z0, FRAME_Z1, seg * 3, 3, false, (MID.x, MID.y), Material::Wood);
    }

    // Low flat oak threshold sill integrated cleanly within the frame's Z span and floor overlap.
    m.tint(134, 104, 74);
    m.cuboid(point(0.0, -0.032, (FRAME_Z0 + FRAME_Z1) * 0.5), point(0.36, 0.087, (FRAME_Z1 - FRAME_Z0) * 0.5), Material::Wood);

    m.build()
}

/// Builds the solid arched oak door leaf in hinge-local coordinates (Mesh 1, `solid = true`).
fn build_leaf(leaf_world: &[(f32, f32); N]) -> MeshData {
    let mut m = MeshBuilder::new();
    let leaf_local = to_hinge_poly(leaf_world);
    m.tint(166, 124, 84);
    extrude_xy(&mut m, &leaf_local, -LEAF_HALF_Z, LEAF_HALF_Z, Material::Wood);
    m.build()
}

/// Builds the decorative door joinery, raised panels, wrought-iron hinges/latch, and brass knocker (Mesh 2).
fn build_door_trim(inner_world: &[(f32, f32); N]) -> MeshData {
    let mut m = MeshBuilder::new();
    let rail_outer = contour_inner(0.015);
    let rail_inner = contour_inner(0.145);
    let rail_outer_l = to_hinge_poly(&rail_outer);
    let rail_inner_l = to_hinge_poly(&rail_inner);
    let panel_base = contour_inner(0.185);
    let panel_field = contour_inner(0.245);
    let panel_base_l = to_hinge_poly(&panel_base);
    let panel_field_l = to_hinge_poly(&panel_field);

    let mid_l = (MID.x - HINGE.x, MID.y - HINGE.y);
    let cx_l = 0.0 - HINGE.x;
    let lock_y = 1.14 - HINGE.y;
    let bot_rail_y = 0.28 - HINGE.y;

    for side in [-1.0f32, 1.0] {
        let z_face = side * LEAF_HALF_Z;

        // Vertical tongue-and-groove oak plank seams clipped cleanly inside the arch.
        m.tint(92, 66, 44);
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

        // Raised arched perimeter stile-and-rail frame around the door face.
        m.tint(148, 108, 72);
        let (zr0, zr1) = if side > 0.0 { (z_face - 0.006, z_face + 0.020) } else { (z_face - 0.020, z_face + 0.006) };
        contour_ring(&mut m, &rail_inner_l, &rail_outer_l, zr0, zr1, 0, N, false, mid_l, Material::Wood);

        // Horizontal lock rail and bottom rail.
        if let Some((xl, xr)) = x_bounds_at_y(&rail_inner, 1.14) {
            let w = 0.5 * (xr - xl) + 0.04;
            let mx = 0.5 * (xl + xr) - HINGE.x;
            m.cuboid(point(mx, lock_y, z_face + side * 0.010), point(w, 0.078, 0.012), Material::Wood);
        }
        if let Some((xl, xr)) = x_bounds_at_y(&rail_inner, 0.28) {
            let w = 0.5 * (xr - xl) + 0.04;
            let mx = 0.5 * (xl + xr) - HINGE.x;
            m.cuboid(point(mx, bot_rail_y, z_face + side * 0.010), point(w, 0.068, 0.012), Material::Wood);
        }

        // Central vertical mullion stile.
        if let Some((ymin, ymax)) = y_bounds_at_x(&rail_inner, 0.0) {
            let lo = max(ymin, 0.24);
            let hi = ymax + 0.02;
            m.cuboid(
                point(cx_l, 0.5 * (lo + hi) - HINGE.y, z_face + side * 0.010),
                point(0.062, 0.5 * (hi - lo), 0.012),
                Material::Wood,
            );
        }

        // Four sculpted raised oak panels (two tall arched upper panels + two lower panels).
        let ur_o = clip_halfplane(&clip_halfplane(&panel_base_l, 1.0, 0.0, -(cx_l + 0.095)), 0.0, 1.0, -(lock_y + 0.11));
        let ur_i = clip_halfplane(&clip_halfplane(&panel_field_l, 1.0, 0.0, -(cx_l + 0.145)), 0.0, 1.0, -(lock_y + 0.16));
        add_panel(&mut m, &ur_o, &ur_i, side);

        let ul_o = clip_halfplane(&clip_halfplane(&panel_base_l, -1.0, 0.0, cx_l - 0.095), 0.0, 1.0, -(lock_y + 0.11));
        let ul_i = clip_halfplane(&clip_halfplane(&panel_field_l, -1.0, 0.0, cx_l - 0.145), 0.0, 1.0, -(lock_y + 0.16));
        add_panel(&mut m, &ul_o, &ul_i, side);

        let lr_o = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_base_l, 1.0, 0.0, -(cx_l + 0.095)), 0.0, -1.0, lock_y - 0.11),
            0.0,
            1.0,
            -(bot_rail_y + 0.09),
        );
        let lr_i = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_field_l, 1.0, 0.0, -(cx_l + 0.145)), 0.0, -1.0, lock_y - 0.16),
            0.0,
            1.0,
            -(bot_rail_y + 0.14),
        );
        add_panel(&mut m, &lr_o, &lr_i, side);

        let ll_o = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_base_l, -1.0, 0.0, cx_l - 0.095), 0.0, -1.0, lock_y - 0.11),
            0.0,
            1.0,
            -(bot_rail_y + 0.09),
        );
        let ll_i = clip_halfplane(
            &clip_halfplane(&clip_halfplane(&panel_field_l, -1.0, 0.0, cx_l - 0.145), 0.0, -1.0, lock_y - 0.16),
            0.0,
            1.0,
            -(bot_rail_y + 0.14),
        );
        add_panel(&mut m, &ll_o, &ll_i, side);

        // Two ornate wrought-iron strap hinges & latch on both faces.
        add_iron_hinge(&mut m, 0.90, side, inner_world);
        add_iron_hinge(&mut m, 1.76, side, inner_world);
        add_iron_latch(&mut m, side);
    }

    // Burnished brass ring knocker on the outer center mullion.
    add_brass_knocker(&mut m);

    m.build()
}

/// Builds the mossy dressed-stone surround, carved woodland vine molding, moss cushions,
/// trailing ivy, threshold flagstone, and interior lantern wall bracket (Mesh 3, `solid = false`).
fn build_surround_and_bracket(seed: u64, inner: &[(f32, f32); N], outer: &[(f32, f32); N]) -> MeshData {
    let mut s = Seeds::of(seed, 37);
    let mut m = MeshBuilder::new();

    // 1. Recessed dark mortar collar behind the stone voussoirs so joints read cleanly.
    let mortar_in = contour_outer(-0.04);
    let mortar_out = contour_outer(0.20);
    m.tint(104, 108, 96);
    contour_ring(&mut m, &mortar_in, &mortar_out, -0.06, 0.145, 39, 42, true, (MID.x, MID.y), Material::Stone);

    // 2. Individual dressed woodland arch stones (voussoirs) from right plinth (k=39) over crown to left plinth (k=33).
    // Indices wrap smoothly around 48: 39..48 (9 segs), 0..11 (11 segs), keystone 11..13 (2 segs), 13..33 (20 segs).
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

    for (first, count, is_keystone) in stone_spans {
        let shade = s.next(-18.0, 18.0);
        let moss_wash = s.next(0.0, 12.0);
        if is_keystone {
            m.tint(tint_u8(208.0 + shade * 0.4), tint_u8(204.0 + shade * 0.4), tint_u8(190.0 + shade * 0.4));
        } else {
            m.tint(
                tint_u8(186.0 + shade - moss_wash * 0.5),
                tint_u8(188.0 + shade + moss_wash * 0.4),
                tint_u8(174.0 + shade - moss_wash * 0.7),
            );
        }
        let inner_r = if is_keystone { -0.055 } else { -0.03 };
        let outer_r = if is_keystone { 0.36 } else { s.next(0.22, 0.30) };
        let z_front = if is_keystone { 0.255 } else { s.next(0.175, 0.225) };
        let st_in = contour_outer(inner_r);
        let st_out = contour_outer(outer_r);
        contour_ring(&mut m, &st_in, &st_out, -0.09, z_front, first, count, true, (MID.x, MID.y), Material::Stone);

        // Soft mossy cushion tucked into stone joints and along the upper arch.
        let joint_k = first % N;
        let (jx, jy) = offset_opening(joint_k, OUTER_OVERLAP + outer_r * 0.55);
        if jy > 0.35 || is_keystone {
            let puffs = if is_keystone { 3 } else { 2 };
            for _ in 0..puffs {
                let mv = s.next(-16.0, 16.0);
                m.tint(tint_u8(122.0 + mv), tint_u8(198.0 + mv), tint_u8(94.0 + mv * 0.6));
                let px = jx + s.next(-0.08, 0.08);
                let py = jy + s.next(-0.05, 0.07);
                let pz = z_front + s.next(-0.03, 0.035);
                let rx = s.next(0.06, 0.13);
                let ry = s.next(0.04, 0.09);
                let rz = s.next(0.04, 0.08);
                m.ellipsoid(point(px, py, pz), point(rx, ry, rz), 8, 6, Material::Moss);
            }
        }
    }

    // Extra lush moss pillow perched right atop the keystone and base plinth stones.
    for &(mx, my, mz, rx, ry, rz) in &[
        (0.00, 2.98, 0.20, 0.18, 0.08, 0.11),
        (-0.22, 2.91, 0.19, 0.14, 0.07, 0.09),
        (0.24, 2.90, 0.18, 0.13, 0.06, 0.09),
        (-0.76, 0.20, 0.20, 0.17, 0.10, 0.12),
        (0.76, 0.18, 0.20, 0.16, 0.09, 0.11),
    ] {
        let mv = s.next(-14.0, 14.0);
        m.tint(tint_u8(116.0 + mv), tint_u8(204.0 + mv), tint_u8(92.0 + mv * 0.6));
        m.ellipsoid(point(mx, my, mz), point(rx, ry, rz), 9, 6, Material::Moss);
    }

    // 3. Sturdy base plinth stones anchoring both jambs into the grassy slope, plus low threshold flagstone.
    m.tint(176, 178, 168);
    m.cuboid(point(-0.70, 0.04, 0.06), point(0.20, 0.20, 0.18), Material::Stone);
    m.cuboid(point(0.72, 0.02, 0.06), point(0.20, 0.20, 0.18), Material::Stone);
    m.tint(188, 186, 178);
    m.cuboid(point(0.0, -0.045, 0.22), point(0.48, 0.088, 0.17), Material::Stone);

    // 4. Carved woodland arch timber moldings (front beaded rim + interior oak architrave collar).
    m.tint(128, 94, 64);
    let mut front_bead = Vec::with_capacity(N + 1);
    let mut inner_bead = Vec::with_capacity(N + 1);
    for k in 0..=N {
        let idx = k % N;
        let (ix, iy) = inner[idx];
        let (ox, oy) = outer[idx];
        let bx = mix(ix, ox, 0.22);
        let by = mix(iy, oy, 0.22);
        front_bead.push((point(bx, by, FRAME_Z1 + 0.015), 0.024));
        inner_bead.push((point(bx, by, FRAME_Z0 - 0.012), 0.022));
    }
    m.sweep_tube(&front_bead, 6, false, Material::Wood);
    m.sweep_tube(&inner_bead, 6, false, Material::Wood);

    // Carved woodland twisting vine relief creeping up both front oak jambs.
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
            let wave = 0.5 + 0.24 * sin(t * TAU * 3.5 * dir);
            let vx = mix(ix, ox, wave);
            let vy = mix(iy, oy, wave);
            vine.push((point(vx, vy, FRAME_Z1 + 0.014), mix(0.017, 0.011, t)));
        }
        m.sweep_tube(&vine, 5, true, Material::Wood);
    }

    // Trailing woodland ivy fronds draping gently off the upper mossy arch stones.
    for &(sx, sy, sz, sway_x, len) in &[
        (-0.88, 2.52, 0.24, -0.10, 0.56),
        (-0.62, 2.74, 0.25, -0.06, 0.44),
        (0.72, 2.66, 0.24, 0.08, 0.52),
        (0.96, 2.38, 0.23, 0.11, 0.48),
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

    // 5. Forged wrought-iron wall bracket mounted inside the right hollow wall for the hanging lantern.
    // Positioned at z = -1.35, completely outside the door's swing arc!
    m.tint(58, 62, 70);
    let bz = LANTERN_PIVOT.z;
    // Wall plate seated into the right oak wall.
    m.cuboid(point(1.16, 1.94, bz), point(0.055, 0.19, 0.055), Material::Stone);
    // Graceful overhanging scroll arm to LANTERN_PIVOT.
    m.sweep_tube(
        &[
            (point(1.15, 2.02, bz), 0.024),
            (point(0.98, 2.26, bz), 0.020),
            (point(0.76, 2.42, bz), 0.017),
            (point(LANTERN_PIVOT.x + 0.02, LANTERN_PIVOT.y + 0.06, bz), 0.015),
            (LANTERN_PIVOT, 0.012),
        ],
        7,
        true,
        Material::Stone,
    );
    // Decorative S-scroll brace beneath the arm.
    m.sweep_tube(
        &[
            (point(1.14, 1.80, bz), 0.014),
            (point(0.99, 1.92, bz), 0.013),
            (point(0.92, 2.12, bz), 0.012),
            (point(0.81, 2.34, bz), 0.012),
        ],
        6,
        true,
        Material::Stone,
    );

    m.build()
}

/// Builds the warm hanging wrought-iron & amber-glass lantern in pivot-local coords (Mesh 4).
fn build_lantern() -> MeshData {
    let mut m = MeshBuilder::new();

    // Iron hanging ring & short chain links down from pivot (0, 0, 0).
    m.tint(62, 66, 74);
    m.sweep_tube(&[(point(0.0, 0.0, 0.0), 0.011), (point(0.0, -0.095, 0.0), 0.011)], 6, true, Material::Stone);
    m.ellipsoid(point(0.0, -0.02, 0.0), point(0.024, 0.024, 0.012), 8, 5, Material::Stone);

    // Flared dark iron canopy roof & chimney cap.
    m.tint(54, 58, 66);
    m.cone(point(0.0, -0.175, 0.0), 0.095, 0.145, 6, Material::Stone);
    m.sweep_tube(&[(point(0.0, -0.175, 0.0), 0.142), (point(0.0, -0.192, 0.0), 0.125)], 6, true, Material::Stone);

    // Warm glowing amber glass lantern body.
    m.tint(255, 222, 118);
    m.sweep_tube(&[(point(0.0, -0.188, 0.0), 0.092), (point(0.0, -0.345, 0.0), 0.076)], 6, true, Material::Sand);

    // Six slender wrought-iron corner bars framing the glowing glass panes.
    m.tint(56, 60, 68);
    for s in 0..6 {
        let a = s as f32 * TAU / 6.0;
        let (ca, sa) = (cos(a), sin(a));
        m.sweep_tube(
            &[(point(ca * 0.098, -0.184, sa * 0.098), 0.008), (point(ca * 0.082, -0.348, sa * 0.082), 0.008)],
            5,
            true,
            Material::Stone,
        );
    }

    // Bottom iron dish & pointed finial drop (total height 0.40 m -> y_world = 1.94 m).
    m.tint(54, 58, 66);
    m.sweep_tube(&[(point(0.0, -0.345, 0.0), 0.104), (point(0.0, -0.372, 0.0), 0.072)], 6, true, Material::Stone);
    m.cone(point(0.0, -0.370, 0.0), -0.036, 0.032, 6, Material::Stone);

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
            68.0 + 5.5 * sin(time * 8.7 + phase) + 3.0 * cos(time * 14.3 + phase)
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
        let m_leaf = build_leaf(&leaf);
        let m_trim = build_door_trim(&inner);
        let m_surround = build_surround_and_bracket(self.seed, &inner, &outer);
        let m_lantern = build_lantern();

        let one = point(1.0, 1.0, 1.0);
        let mut sc = SceneBuilder::new();
        let frame_id = sc.add_mesh(m_frame);       // 0
        let leaf_id = sc.add_mesh(m_leaf);         // 1 (MESH_LEAF)
        let trim_id = sc.add_mesh(m_trim);         // 2 (MESH_TRIM)
        let surround_id = sc.add_mesh(m_surround); // 3
        let lantern_id = sc.add_mesh(m_lantern);   // 4 (MESH_LANTERN)

        sc.place(frame_id, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, true);
        sc.place(surround_id, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, false);
        sc.spawn(ID_LEAF, leaf_id, HINGE, Quat::IDENTITY, one, true);
        sc.spawn(ID_TRIM, trim_id, HINGE, Quat::IDENTITY, one, false);
        sc.spawn(ID_LANTERN, lantern_id, LANTERN_PIVOT, Quat::IDENTITY, one, false);
        sc.light(ID_LIGHT, LANTERN_PIVOT + LANTERN_BULB_OFFSET, [1.0, 0.74, 0.36], 68.0, 8.5);
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
            false,
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
