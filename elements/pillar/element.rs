//! Standalone carved stone cairn / pillar (`elements/pillar/element.rs`).
//!
//! Pure procedural `Scene` layer (`sdf_bounds() == None`, unrooted `y >= 0.0`): stacked mossy
//! dressed stone blocks (`Material::Stone`, `solid: true`). Demonstrates material-aware tool
//! interactions (Saw refuses Stone; Pickaxe splits it cleanly with fresh stone cross-section caps)
//! and Archimedes physics (Stone density 2200 kg/m³ sinks cleanly to the pond bed).

use plateau_sdk::*;

struct StonePillar {
    seed: u64,
}

impl Element for StonePillar {
    fn new(seed: u64) -> Self {
        StonePillar { seed }
    }

    fn name(&self) -> &str {
        "Stone Pillar"
    }

    fn scene(&self) -> Scene {
        let mut s = Seeds::of(self.seed, 31);
        let mut stone = MeshBuilder::new();

        // Five stacked chamfered stone courses rising from y = 0.0 to y = 2.05 m (unrooted).
        let courses = [
            (0.00f32, 0.36f32, 0.44f32, 0.44f32),
            (0.36, 0.42, 0.36, 0.36),
            (0.78, 0.44, 0.34, 0.34),
            (1.22, 0.44, 0.32, 0.32),
            (1.66, 0.38, 0.38, 0.38),
        ];
        for (y0, h, hx, hz) in courses {
            let shade = (188.0 + s.next(-14.0, 14.0)) as u8;
            let dx = s.next(-0.015, 0.015);
            let dz = s.next(-0.015, 0.015);
            stone.tint(shade, shade, (shade as f32 * 0.96) as u8);
            stone.cuboid(point(dx, y0 + h * 0.5, dz), point(hx, h * 0.5 + 0.01, hz), Material::Stone);
        }

        // Subtle moss cap detail on top (non-solid visual accent).
        let mut moss = MeshBuilder::new();
        moss.tint(160, 198, 128);
        moss.cuboid(point(0.04, 2.06, -0.02), point(0.24, 0.025, 0.22), Material::Moss);

        let mut sc = SceneBuilder::new();
        let m_stone = sc.add_mesh(stone.build());
        let m_moss = sc.add_mesh(moss.build());
        sc.place(m_stone, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.place(m_moss, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        sc.build()
    }
}

element!(StonePillar);
