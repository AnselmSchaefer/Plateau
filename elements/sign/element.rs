//! Standalone wooden sign for a wall (`elements/sign/element.rs`).
//!
//! Made for a wall site: the origin is the point of the wall it is hung at, +Z out of the wall, +Y
//! up. Its board's back lies flat on the wall (z = 0, a little into it), so it meets the wall over
//! all of its back (`plateau_engine::fit::wall_fit`). Given a site's params it reads none (a wall
//! has no opening); the board is a metre wide and sixty centimetres high, about the point it is
//! hung at.
//!
//! Pure procedural `Scene` (`sdf_bounds() == None`): the board solid, its frame and the letters'
//! strokes on it only to look at.

use plateau_sdk::*;

const HALF_W: f32 = 0.5;
const HALF_H: f32 = 0.3;
/// The board's back goes this far into the wall, and its face this far out of it.
const INTO: f32 = 0.01;
const OUT: f32 = 0.05;

struct Sign {
    seed: u64,
}

impl Element for Sign {
    fn new(seed: u64) -> Self {
        Sign { seed }
    }

    fn name(&self) -> &str {
        "Wooden Sign"
    }

    fn scene(&self) -> Scene {
        let mut s = Seeds::of(self.seed, 5);
        let depth = 0.5 * (OUT + INTO);
        let middle = 0.5 * (OUT - INTO);
        // Mesh 0: the board, solid.
        let mut board = MeshBuilder::new();
        let shade = s.next(-14.0, 14.0);
        board.tint((190.0 + shade) as u8, (150.0 + shade) as u8, (104.0 + shade) as u8);
        board.cuboid(point(0.0, 0.0, middle), point(HALF_W, HALF_H, depth), Material::Wood);

        // Mesh 1: its frame and three strokes of lettering, standing a little proud of its face.
        let mut trim = MeshBuilder::new();
        trim.tint(120, 86, 56);
        let rim = 0.03;
        for (c, h) in [
            (point(0.0, HALF_H - rim, OUT + 0.006), point(HALF_W, rim, 0.006)),
            (point(0.0, -HALF_H + rim, OUT + 0.006), point(HALF_W, rim, 0.006)),
            (point(HALF_W - rim, 0.0, OUT + 0.006), point(rim, HALF_H, 0.006)),
            (point(-HALF_W + rim, 0.0, OUT + 0.006), point(rim, HALF_H, 0.006)),
        ] {
            trim.cuboid(c, h, Material::Wood);
        }
        trim.tint(48, 40, 34);
        for k in 0..3 {
            let y = 0.12 - 0.12 * k as f32;
            let w = s.next(0.22, 0.36);
            trim.cuboid(point(0.0, y, OUT + 0.004), point(w, 0.018, 0.004), Material::Stone);
        }

        let one = point(1.0, 1.0, 1.0);
        let mut sc = SceneBuilder::new();
        let board = sc.add_mesh(board.build());
        let trim = sc.add_mesh(trim.build());
        sc.place(board, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, true);
        sc.place(trim, point(0.0, 0.0, 0.0), Quat::IDENTITY, one, false);
        sc.build()
    }
}

element!(Sign);
