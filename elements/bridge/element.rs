//! Standalone walkable arched timber footbridge (`elements/bridge/element.rs`).
//!
//! Pure procedural `Scene` layer (`sdf_bounds() == None`): spans 4.8 m along Z with a gentle
//! walkable arch (`solid: true` planks & stringers) plus decorative posts and rope/timber rails.
//! Works at tiny (spanning a puddle), person (spanning a stream), or giant scale.

use plateau_sdk::*;

struct TimberBridge {
    seed: u64,
}

fn arch_y(z: f32) -> f32 {
    let t = clamp(z / 2.4, -1.0, 1.0);
    0.42 * (1.0 - t * t)
}

impl Element for TimberBridge {
    fn new(seed: u64) -> Self {
        TimberBridge { seed }
    }

    fn name(&self) -> &str {
        "Timber Bridge"
    }

    fn scene(&self) -> Scene {
        let mut s = Seeds::of(self.seed, 2);

        // Mesh 0: solid walkable deck planks + end ramps + support beams.
        let mut deck = MeshBuilder::new();
        let planks = 16;
        for i in 0..planks {
            let u = (i as f32 + 0.5) / planks as f32;
            let z = mix(-2.25, 2.25, u);
            let y = arch_y(z);
            let shade = (205.0 + s.next(-18.0, 18.0)) as u8;
            deck.tint(shade, (shade as f32 * 0.92) as u8, (shade as f32 * 0.82) as u8);
            // Overlapping planks keep collision completely continuous across the arch.
            deck.cuboid(point(0.0, y, z), point(0.84, 0.06, 0.165), Material::Wood);
        }
        // Solid end abutments sinking down into uneven ground so entry is smooth.
        deck.tint(185, 175, 162);
        deck.cuboid(point(0.0, -0.14, -2.42), point(0.88, 0.18, 0.24), Material::Stone);
        deck.cuboid(point(0.0, -0.14, 2.42), point(0.88, 0.18, 0.24), Material::Stone);

        // Mesh 1: structural arched beams, handrail posts, and curved top rails (visual detail).
        let mut trim = MeshBuilder::new();
        for side in [-1.0f32, 1.0] {
            let x = side * 0.76;
            // Curved lower stringer beam.
            let mut stringer = Vec::new();
            let mut rail = Vec::new();
            for k in 0..=10 {
                let z = mix(-2.35, 2.35, k as f32 / 10.0);
                let y = arch_y(z);
                stringer.push((point(x, y - 0.10, z), 0.085));
                rail.push((point(x, y + 0.86, z), 0.048));
            }
            trim.tint(175, 155, 135);
            trim.sweep_tube(&stringer, 6, true, Material::Wood);
            trim.tint(220, 200, 178);
            trim.sweep_tube(&rail, 6, true, Material::Wood);

            // Five upright posts per side.
            for p in 0..5 {
                let z = mix(-2.20, 2.20, p as f32 / 4.0);
                let y = arch_y(z);
                trim.tint(190, 172, 152);
                trim.cuboid(point(x, y + 0.44, z), point(0.055, 0.50, 0.055), Material::Wood);
                trim.tint(230, 215, 195);
                trim.cuboid(point(x, y + 0.96, z), point(0.075, 0.035, 0.075), Material::Wood);
            }
        }

        let mut sc = SceneBuilder::new();
        let m_deck = sc.add_mesh(deck.build());
        let m_trim = sc.add_mesh(trim.build());
        sc.place(m_deck, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.place(m_trim, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        sc.build()
    }
}

element!(TimberBridge);
