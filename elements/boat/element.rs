//! Standalone procedural wooden rowboat (`elements/boat/element.rs`).
//!
//! Pure procedural `Scene` layer (`sdf_bounds() == None`): an open, hollow timber hull with
//! pointed bow (`-Z`), flat stern transom (`+Z`), solid walkable floor planks, curved gunwales,
//! two bench thwarts, and stowed oars. Because its solid `Material::Wood` shell encloses an open
//! cockpit (`fill_ratio < 0.45`), Archimedes buoyancy floats it high on ponds & expanded channels
//! and supports a person riding & paddling across water — with zero prefab tags!

use plateau_sdk::*;

struct WoodenBoat {
    seed: u64,
}

fn half_beam(u: f32) -> f32 {
    // u in [0, 1] from bow (-Z) to stern (+Z): pointed bow, full midship, tapered stern transom.
    let bow = clamp(u / 0.35, 0.0, 1.0);
    let stern = 1.0 - 0.32 * clamp((u - 0.65) / 0.35, 0.0, 1.0);
    0.72 * (bow * (2.0 - bow)) * stern
}

impl Element for WoodenBoat {
    fn new(seed: u64) -> Self {
        WoodenBoat { seed }
    }

    fn name(&self) -> &str {
        "Wooden Rowboat"
    }

    fn scene(&self) -> Scene {
        let mut s = Seeds::of(self.seed, 19);

        // Mesh 0: Solid walkable wooden hull bottom, overlapping side walls, bow stem, transom, & benches.
        // Local y >= 0.0 so it is unrooted; raised interior floorboard (y = 0.22 m) stays completely
        // dry above the waterline both empty and with a person aboard!
        let mut hull = MeshBuilder::new();
        let len = 3.2f32;
        let segs = 14usize;
        let floor_h = 0.22f32;
        let wall_h = 0.40f32;

        // Bottom outer hull skin (y = 0.0..0.06) + raised dry interior floorboards (y = 0.17..0.22)
        // with an air-filled bilge between them so the boat stays light and its cockpit stays dry!
        for i in 0..segs {
            let u0 = i as f32 / segs as f32;
            let u1 = (i + 1) as f32 / segs as f32;
            let um = 0.5 * (u0 + u1);
            let z = mix(-len * 0.5, len * 0.5, um);
            let dz = (len / segs as f32) * 0.62;
            let bw = max(half_beam(um) * 0.90, 0.09);
            let shade = (196.0 + s.next(-14.0, 14.0)) as u8;
            hull.tint(158, 130, 102);
            hull.cuboid(point(0.0, 0.03, z), point(bw * 0.92, 0.03, dz), Material::Wood);
            hull.tint(shade, (shade as f32 * 0.88) as u8, (shade as f32 * 0.74) as u8);
            hull.cuboid(point(0.0, floor_h - 0.025, z), point(bw, 0.025, dz), Material::Wood);

            // Side hull walls (port & starboard) spanning from outer keel (y=0) up to gunwale!
            let w0 = half_beam(u0) * 0.92;
            let w1 = half_beam(u1) * 0.92;
            let wall_x = max(0.5 * (w0 + w1), 0.09);
            let wall_hx = max(0.055, abs(w1 - w0) * 0.58 + 0.035);
            let sheer = 0.05 * (1.0 - 4.0 * (um - 0.5) * (um - 0.5));
            let total_wh = floor_h + wall_h - sheer;
            for side in [-1.0f32, 1.0] {
                hull.tint(176, 148, 118);
                hull.cuboid(point(side * wall_x, total_wh * 0.5, z), point(wall_hx, total_wh * 0.5, dz), Material::Wood);
            }
        }

        // Solid stern transom (+Z) and reinforced bow stem (-Z) full height from keel to gunwale.
        let stern_w = half_beam(1.0) * 0.96;
        let end_h = floor_h + wall_h;
        hull.tint(168, 138, 108);
        hull.cuboid(point(0.0, end_h * 0.5, len * 0.5 - 0.05), point(stern_w, end_h * 0.5, 0.06), Material::Wood);
        hull.cuboid(point(0.0, end_h * 0.52, -len * 0.5 + 0.07), point(0.11, end_h * 0.52, 0.10), Material::Wood);

        // Two wooden bench thwarts across the beam (walkable / sittable).
        for &(u, width_scale) in &[(0.42f32, 0.86f32), (0.76f32, 0.84f32)] {
            let z = mix(-len * 0.5, len * 0.5, u);
            let w = half_beam(u) * width_scale;
            hull.tint(218, 190, 154);
            hull.cuboid(point(0.0, floor_h + 0.22, z), point(w, 0.035, 0.16), Material::Wood);
        }

        // Mesh 1: Curved gunwale trim rails + stowed wooden oars along the sides (non-solid visual detail).
        let mut trim = MeshBuilder::new();
        for side in [-1.0f32, 1.0] {
            let mut gunwale = Vec::new();
            for k in 0..=14 {
                let u = k as f32 / 14.0;
                let z = mix(-len * 0.5 + 0.03, len * 0.5 - 0.03, u);
                let x = side * max(half_beam(u) * 0.95, 0.07);
                let sheer = 0.05 * (1.0 - 4.0 * (u - 0.5) * (u - 0.5));
                let y = floor_h + (wall_h - sheer) * 0.96;
                gunwale.push((point(x, y, z), 0.045));
            }
            trim.tint(232, 206, 170);
            trim.sweep_tube(&gunwale, 6, true, Material::Wood);

            // Stowed wooden oar resting along each inner gunwale.
            let oar_x = side * 0.52;
            let oar_y = floor_h + 0.34;
            trim.tint(224, 198, 162);
            trim.sweep_tube(
                &[
                    (point(oar_x * 0.85, oar_y, -0.75), 0.022),
                    (point(oar_x, oar_y + 0.02, 0.15), 0.024),
                    (point(oar_x * 0.92, oar_y - 0.01, 1.05), 0.020),
                ],
                6,
                true,
                Material::Wood,
            );
            // Oar blade near stern.
            trim.tint(236, 212, 176);
            trim.cuboid(point(oar_x * 0.90, oar_y - 0.02, 1.18), point(0.07, 0.015, 0.22), Material::Wood);
        }

        let mut sc = SceneBuilder::new();
        let m_hull = sc.add_mesh(hull.build());
        let m_trim = sc.add_mesh(trim.build());
        sc.place(m_hull, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.place(m_trim, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        sc.build()
    }
}

element!(WoodenBoat);
