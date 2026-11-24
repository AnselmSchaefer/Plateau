# Writing a standalone Plateau element

A Plateau **element** is a single Rust file (`element.rs`) that compiles to a standalone sandboxed
`.wasm` module and can be placed onto **any** existing Plateau area at **any** player size
(`tiny`, `person`, or `giant`) without recompiling the base area.

## Hybrid Layer Ownership (choose the right layer!)

- **Procedural `Scene` layer (`sdf_bounds() == None`)** — use this for discrete, crafted,
  interactive, or portable objects: trees, bridges, lanterns, cottages, benches, rafts, signs,
  contraptions, flowers, arches built of timber/metal, creatures.
  - Build raw custom meshes with `MeshBuilder` (`sweep_tube`, `ribbon`, `lathe`, `extrude`,
    `capsule`, `ellipsoid`, `cone`, `cuboid`, per-vertex RGB `tint`) and place/spawn them via
    `SceneBuilder`.
  - Set `solid: true` on walkable/collidable meshes (bridge decks, trunks, stairs, roofs). Keep
    collision meshes clean and moderate in triangle count; put thin decorative trim (`rails`,
    `leaves`, `chains`) on separate `solid: false` meshes.
  - Use `sc.spawn(id, ...)` (`id > 0`) for parts that move, animate, sway, toggle, or should be
    interactive (`F`) or cuttable/relocatable later.
- **Carvable SDF landform layer (`sdf_bounds() == Some(ElementBounds::new(lo, hi, blend))`)** — use
  this for organic earth/rock landforms meant to blend into terrain and be sculpted or hollowed
  (hills, boulders, mossy outcrops, rocky arches, caves).
  - Return tight local `lo..hi` bounds (within `[-64, 64]`, `lo < hi`) and an optional smooth blend
    width `blend` (`0.0..=32.0`, typically `0.2..=0.6` m).
  - **Every solid surface (plus `detail` roughness) MUST stay strictly inside `lo..hi`** so the SDF
    is positive on all six faces of `lo..hi` and never clips against its bounding box!
  - Distances must be **never too long** (`|grad d| <= 1.0`, scale down by `0.7..0.85` when adding
    `detail` roughness) and every surface must have an inside.
  - An SDF landform element may *also* emit a procedural `Scene` (e.g., ferns or a lantern perched
    on a rocky mound).

## Coordinate System & Multi-Scale Design

- Author in **element-local metres** centered around `(0.0, 0.0, 0.0)` with the base resting at
  `y = 0.0` (sink foundations/roots/ramps slightly below `y = 0.0`, e.g. down to `-0.20`, so the
  element seats cleanly on sloped or uneven terrain without floating!).
- Write at natural **person scale** (e.g. a lantern ~2.2 m tall, a bridge ~4.5 m long, a tree ~5 m
  tall, a hill ~4–6 m wide). The engine automatically scales, rotates (`yaw`), and translates your
  element when placed by a tiny player (`0.01x`), a person (`1.0x`), or a giant (`10.0x`).
- In `tick(dt, player)` and `interact(player)`, `player` (`feet`, `eye`, `look`, `velocity`, `aim`,
  `size`, `target`) is **already transformed into your element's local coordinates**, and any
  `SetTransform`, `SetLight`, or `ApplyImpulse` commands you emit are automatically transformed back
  into world space and scoped to your element alone.

## Determinism & Sandbox Rules

- Use **only** `plateau_sdk` math (`sin`, `cos`, `atan2`, `min`, `max`, `abs`, `clamp`, `mix`,
  `noise`, `detail`, `Seeds`, `Quat`). Never call `f32::sin`, `f32::min`, `f32::max`, `f32::clamp`,
  `powf`, `exp`, or `to_bits`.
- Zero host capabilities except `seed` (passed to `Element::new(seed)`). Use `Seeds::of(seed, k)`
  for subtle organic variation between placements.
- Stay well within engine caps (`<= 256` meshes, `<= 200_000` vertices, `<= 600_000` indices,
  `<= 16_384` instances, `<= 64` lights, `<= 256` commands per tick).

## Skeleton

```rust
use plateau_sdk::*;

struct MyElement {
    seed: u64,
    active: bool,
}

impl Element for MyElement {
    fn new(seed: u64) -> Self {
        MyElement { seed, active: false }
    }

    fn name(&self) -> &str {
        "My Element"
    }

    fn scene(&self) -> Scene {
        let mut m = MeshBuilder::new();
        m.cuboid(point(0.0, 0.5, 0.0), point(0.5, 0.5, 0.5), Material::Wood);
        let mut sc = SceneBuilder::new();
        let mesh = sc.add_mesh(m.build());
        sc.spawn(1, mesh, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.build()
    }

    fn interact(&mut self, p: &PlayerState) -> Vec<Command> {
        if p.target == 1 {
            self.active = !self.active;
            return vec![Command::Say("Activated!".into())];
        }
        Vec::new()
    }
}

element!(MyElement);
```
