//! Standalone procedural oak tree element (`elements/tree/element.rs`).
//!
//! Pure procedural `Scene` layer (`sdf_bounds() == None`): sturdy flared trunk & branches (`id = 1`,
//! `solid: true`) and layered leafy canopy (`id = 2`), authored around `(0, 0, 0)` with base at
//! `y = 0` so it composes cleanly at tiny, person, or giant scale and is ready for cutting/carrying.

use plateau_sdk::*;

struct OakTree {
    seed: u64,
    rustle: f32,
}

impl Element for OakTree {
    fn new(seed: u64) -> Self {
        OakTree { seed, rustle: 0.0 }
    }

    fn name(&self) -> &str {
        "Oak Tree"
    }

    fn scene(&self) -> Scene {
        let mut s = Seeds::of(self.seed, 1);
        let lean_x = s.next(-0.18, 0.18);
        let lean_z = s.next(-0.18, 0.18);
        let height = s.next(4.2, 5.2);

        // Mesh 0: flared root buttresses, slightly curved trunk, and main limbs (solid wood).
        let mut trunk = MeshBuilder::new();
        trunk.tint(215, 198, 178);
        trunk.sweep_tube(
            &[
                (point(0.0, -0.25, 0.0), 0.44),
                (point(0.0, 0.12, 0.0), 0.34),
                (point(lean_x * 0.3, height * 0.32, lean_z * 0.3), 0.25),
                (point(lean_x * 0.7, height * 0.62, lean_z * 0.7), 0.20),
                (point(lean_x, height * 0.84, lean_z), 0.14),
            ],
            10,
            true,
            Material::Wood,
        );
        // Four buttress roots anchoring into uneven ground.
        for i in 0..4 {
            let a = i as f32 * (TAU * 0.25) + s.next(-0.25, 0.25);
            let (ca, sa) = (cos(a), sin(a));
            trunk.tint(195, 180, 160);
            trunk.sweep_tube(
                &[
                    (point(ca * 0.16, 0.45, sa * 0.16), 0.14),
                    (point(ca * 0.46, 0.10, sa * 0.46), 0.11),
                    (point(ca * 0.78, -0.22, sa * 0.78), 0.06),
                ],
                6,
                true,
                Material::Wood,
            );
        }
        // Five spreading boughs.
        let mut crown_pts = Vec::new();
        for i in 0..5 {
            let a = i as f32 * (TAU / 5.0) + s.next(-0.22, 0.22);
            let (ca, sa) = (cos(a), sin(a));
            let fork_y = height * s.next(0.56, 0.74);
            let reach = s.next(1.15, 1.65);
            let tip_y = fork_y + s.next(0.75, 1.35);
            let base_p = point(lean_x * 0.65, fork_y, lean_z * 0.65);
            let mid_p = point(base_p.x + ca * reach * 0.55, fork_y + (tip_y - fork_y) * 0.55, base_p.z + sa * reach * 0.55);
            let tip_p = point(base_p.x + ca * reach, tip_y, base_p.z + sa * reach);
            trunk.tint(210, 195, 175);
            trunk.sweep_tube(&[(base_p, 0.14), (mid_p, 0.095), (tip_p, 0.055)], 7, true, Material::Wood);
            crown_pts.push(tip_p);
        }

        // Mesh 1: lush layered oak foliage clusters in sunlit & shaded greens.
        let mut canopy = MeshBuilder::new();
        canopy.tint(225, 245, 200);
        canopy.ellipsoid(point(lean_x, height + 0.35, lean_z), point(1.45, 1.05, 1.45), 10, 7, Material::Leaf);
        for (i, &bp) in crown_pts.iter().enumerate() {
            let bright = if i % 2 == 0 { (245, 255, 210) } else { (195, 225, 175) };
            canopy.tint(bright.0, bright.1, bright.2);
            let rx = s.next(0.95, 1.35);
            let ry = s.next(0.72, 1.02);
            let rz = s.next(0.95, 1.35);
            canopy.ellipsoid(bp + point(0.0, 0.35, 0.0), point(rx, ry, rz), 9, 6, Material::Leaf);
            canopy.tint(175, 210, 160);
            canopy.ellipsoid(bp + point(0.0, -0.05, 0.0), point(rx * 0.78, ry * 0.65, rz * 0.78), 8, 5, Material::Leaf);
        }

        let mut sc = SceneBuilder::new();
        let m_trunk = sc.add_mesh(trunk.build());
        let m_canopy = sc.add_mesh(canopy.build());
        sc.spawn(1, m_trunk, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.spawn(2, m_canopy, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        sc.build()
    }

    fn tick(&mut self, dt: f32, p: &PlayerState) -> Vec<Command> {
        let phase = (self.seed % 97) as f32 * 0.13;
        let wind = 0.014 * sin(p.time * 1.1 + phase);
        let kick = if self.rustle > 0.001 {
            self.rustle = max(self.rustle - dt * 1.4, 0.0);
            self.rustle * 0.09 * sin(p.time * 14.0)
        } else {
            0.0
        };
        let rot = Quat::from_euler(0.0, (wind + kick) * 0.6, wind + kick);
        vec![Command::SetTransform(instance(2, 1, point(0.0, 0.0, 0.0), rot, point(1.0, 1.0, 1.0), false))]
    }

    fn interact(&mut self, p: &PlayerState) -> Vec<Command> {
        if p.target == 1 || p.target == 2 {
            self.rustle = 1.0;
            return vec![Command::Say("The oak leaves rustle overhead.".into())];
        }
        Vec::new()
    }
}

element!(OakTree);
