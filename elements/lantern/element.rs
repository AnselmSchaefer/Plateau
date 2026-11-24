//! Standalone interactive swaying post lantern (`elements/lantern/element.rs`).
//!
//! Pure procedural `Scene` + runtime `tick` / `interact` + dynamic `PointLight`:
//! sways gently in the wind in element-local space and toggles its warm light on/off with `F`.

use plateau_sdk::*;

struct PostLantern {
    seed: u64,
    lit: bool,
}

const PIVOT: Point = Point { x: 0.0, y: 2.02, z: 0.46 };

impl Element for PostLantern {
    fn new(seed: u64) -> Self {
        PostLantern { seed, lit: true }
    }

    fn name(&self) -> &str {
        "Post Lantern"
    }

    fn scene(&self) -> Scene {
        // Mesh 0: sturdy timber post, stone plinth, and overhanging curved arm (static solid).
        let mut post = MeshBuilder::new();
        post.tint(195, 192, 185);
        post.sweep_tube(&[(point(0.0, -0.18, 0.0), 0.22), (point(0.0, 0.22, 0.0), 0.16)], 8, true, Material::Stone);
        post.tint(175, 152, 130);
        post.cuboid(point(0.0, 1.15, 0.0), point(0.085, 1.10, 0.085), Material::Wood);
        post.tint(150, 138, 128);
        post.sweep_tube(
            &[
                (point(0.0, 1.98, 0.0), 0.055),
                (point(0.0, 2.22, 0.20), 0.045),
                (point(0.0, 2.18, 0.44), 0.036),
                (PIVOT, 0.025),
            ],
            7,
            true,
            Material::Stone,
        );
        // Diagonal brace.
        post.tint(165, 145, 125);
        post.sweep_tube(&[(point(0.0, 1.65, 0.05), 0.038), (point(0.0, 2.12, 0.26), 0.034)], 6, true, Material::Wood);

        // Mesh 1: hanging lantern head centered at PIVOT (local origin = pivot so rotation swings naturally).
        let mut head = MeshBuilder::new();
        // Chain link from pivot down to cap.
        head.tint(120, 120, 125);
        head.sweep_tube(&[(point(0.0, 0.0, 0.0), 0.014), (point(0.0, -0.13, 0.0), 0.014)], 6, true, Material::Stone);
        // Pointed dark hood.
        head.tint(110, 112, 120);
        head.cone(point(0.0, -0.16, 0.0), 0.12, 0.16, 6, Material::Stone);
        // Warm glowing lantern glass core.
        head.tint(255, 220, 120);
        head.sweep_tube(&[(point(0.0, -0.16, 0.0), 0.105), (point(0.0, -0.38, 0.0), 0.095)], 6, true, Material::Sand);
        // Bottom metal tray.
        head.tint(110, 112, 120);
        head.sweep_tube(&[(point(0.0, -0.38, 0.0), 0.13), (point(0.0, -0.43, 0.0), 0.11)], 6, true, Material::Stone);

        let mut sc = SceneBuilder::new();
        let m_post = sc.add_mesh(post.build());
        let m_head = sc.add_mesh(head.build());
        sc.place(m_post, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.spawn(1, m_head, PIVOT, Quat::IDENTITY, point(1.0, 1.0, 1.0), false);
        sc.light(1, point(PIVOT.x, PIVOT.y - 0.27, PIVOT.z), [1.0, 0.78, 0.42], 85.0, 9.0);
        sc.build()
    }

    fn tick(&mut self, _dt: f32, p: &PlayerState) -> Vec<Command> {
        let phase = (self.seed % 89) as f32 * 0.17;
        let sway_z = 0.09 * sin(p.time * 1.7 + phase);
        let sway_x = 0.05 * cos(p.time * 1.2 + phase * 1.3);
        let rot = Quat::from_euler(0.0, sway_x, sway_z);
        let bulb = PIVOT + rot.rotate(point(0.0, -0.27, 0.0));
        let mut cmds = vec![Command::SetTransform(instance(1, 1, PIVOT, rot, point(1.0, 1.0, 1.0), false))];
        if self.lit {
            let flicker = 85.0 + 7.0 * sin(p.time * 9.3 + phase) + 4.0 * cos(p.time * 15.1);
            cmds.push(Command::SetLight(light(1, bulb, [1.0, 0.78, 0.42], flicker, 9.0)));
        }
        cmds
    }

    fn interact(&mut self, p: &PlayerState) -> Vec<Command> {
        let near = (p.feet - point(0.0, 0.0, 0.0)).length() < 4.0;
        if p.target == 1 || near {
            self.lit = !self.lit;
            let intensity = if self.lit { 85.0 } else { 0.0 };
            let bulb = point(PIVOT.x, PIVOT.y - 0.27, PIVOT.z);
            return vec![
                Command::SetLight(light(1, bulb, [1.0, 0.78, 0.42], intensity, 9.0)),
                Command::Say(if self.lit { "Lantern lit.".into() } else { "Lantern extinguished.".into() }),
            ];
        }
        Vec::new()
    }
}

element!(PostLantern);
