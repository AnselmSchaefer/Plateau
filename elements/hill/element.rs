//! Standalone carvable rocky mossy hill landform (`elements/hill/element.rs`).
//!
//! SDF Layer (`sdf_bounds() == Some(...)`): blends smoothly into any existing terrain at any scale
//! and can be freely sculpted or hollowed (e.g., digging a tiny 2 cm cave inside it with `B`).

use plateau_sdk::*;

struct MossyHill {
    seed: u64,
}

impl Element for MossyHill {
    fn new(seed: u64) -> Self {
        MossyHill { seed }
    }

    fn name(&self) -> &str {
        "Mossy Hill"
    }

    fn sdf_bounds(&self) -> Option<ElementBounds> {
        // Every lobe + roughness stays strictly inside [-3.6..3.6, -2.5..2.5, -3.6..3.6].
        Some(ElementBounds::new(point(-3.6, -2.5, -3.6), point(3.6, 2.5, 3.6), 0.45))
    }

    fn sample(&self, p: Point, view: &View) -> Answer {
        let s32 = (self.seed as u32).wrapping_mul(2654435761) ^ 0x51a7_3c11;
        // Three overlapping lobes forming an organic rocky mound with flat-ish grassy crown.
        let main = sphere(p - point(0.0, -0.10, 0.0), 2.05);
        let east = sphere(p - point(1.15, -0.25, 0.55), 1.32);
        let west = sphere(p - point(-1.05, -0.30, -0.50), 1.24);
        let base = smooth_union(smooth_union(main, east, 0.55), west, 0.55);

        // Multi-scale rock roughness faded smoothly by viewer size; scaled by 0.72 for strict Lipschitz safety.
        let rough = detail(p, view, 1.1, 0.055, s32);
        let d = (base + rough) * 0.72;

        // Material stratification: grassy crown over soil & stone, mossy shoulders over stone, rocky flanks.
        let n = noise(p * 0.9, s32.wrapping_add(7));
        let depth = -d;
        let mat = if p.y > 0.85 + n * 0.22 {
            strata(depth, &[(Material::Grass, 0.08), (Material::Soil, 0.35), (Material::Stone, 0.0)])
        } else if p.y > 0.15 + n * 0.28 {
            strata(depth, &[(Material::Moss, 0.04), (Material::Stone, 0.0)])
        } else {
            Material::Stone
        };
        Answer::new(d, mat)
    }
}

element!(MossyHill);
