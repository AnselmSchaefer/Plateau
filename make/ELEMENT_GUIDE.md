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

## Fitting a measured site (when a site is given)

When authoring for a measured site (`plateau make-element "<words>" --site FILE.site` or `--at X,Y,Z`),
every measurement in the prompt is already converted into your element's **local metres** so the
measuring player is **1.7 m tall** in your coordinate system (`+X` right, `+Y` up, `+Z` facing out
toward whoever approaches). The engine scales, rotates, and translates your element into the world
via `site.frame`.

### Params: one element for every site like it

The numbers in the prompt are also given to your element when it runs, as `params()` (call it once,
in `new`). Write the element from them, not from numbers copied out of the prompt. Then the same
element fits every site like the one it was made for: any opening, of any shape and size. When the
player widens the opening, the engine measures it again and gives the element the new numbers.

`params()` is empty when the element is placed for no site. Then make a sensible default, such as
the shape the prompt describes. Read each number with `param(&p, index, default)`. Lengths are in
local metres, like the prompt's:

| Index | Param |
|---|---|
| 0 | 1 if there is an opening, else 0 (and every number after it is 0) |
| 1, 2 | the opening's width and height |
| 3 | how high its middle is above the sill; the radii below are measured from `(0, this, 0)` |
| 4 | how deep it goes along `-Z` |
| 5 | 1 if it goes on through, open behind |
| 6 | the least wall beyond the outline at the rim |
| 7 | roundness: 1 for a circle, less the less round it is |
| 8 | headroom: how far it is from floor to ceiling inside |
| 9 | 1 if a 1.7 m tall body walks in through it |
| 10 to 57 | 48 distances from the middle to the outline, in the rim's plane. The `k`th is at the angle `a = k / 48` of a turn, anticlockwise from straight right, so its point is `(r cos a, middle + r sin a, 0)`. Together they are the same outline as `OPENING` |

For example, `elements/hobbit-door/element.rs` (below) builds its frame, leaf and trim along these
48 radii.

### Fitting an opening

When the site has a measured cavity/opening (such as a carved cave, tunnel, or hollowed tree trunk):

- **Use procedural `Scene` meshes (`sdf_bounds() == None`)** — never an SDF landform that would fill
  the carved opening back in.
- **Origin & rim plane**: `(0, 0, 0)` lies on the opening's sill (bottom-center) in the rim plane
  `z = 0`. The surrounding wall lies at `z <= 0`; open air outside is `z > 0`.
- **Static solid frame (`id = 0`, `solid = true`)**: trace the opening's outline (from the params'
  radii; `const OPENING: [(f32, f32); 48]` in the prompt is the same outline) around the rim in `z = 0` (spanning slightly in and out, e.g. `z = -0.20..0.12`). Its outer
  edge MUST overlap into the surrounding wall by **1% to 35%** of the opening's mean radius (aim for
  **6% to 12%**) all the way around, with no floating gaps and without burying the inner edge inside
  the wall. The floor edge may be left to a sill or the bottom of a door leaf (within 5% of mean radius).
- **Non-solid decorative trim (`solid = false`)**: keep surround stones, molding, iron straps, hinges,
  handles, and small ornaments on separate `solid = false` meshes so collision stays clean.
- **Movable doors, shutters, and gates (if a leaf fills the opening)**:
  - Spawn the solid leaf with `id > 0` (`solid = true`), overlapping slightly past the frame's inner
    edge so no light or sightlines leak around the rim when shut (at most 3% keyhole opening in the
    interior, zero gaps beside the rim).
  - On `interact(player)` (`F` with crosshair aimed at the leaf or standing before it), animate the
    leaf open within **4 seconds** (temporarily setting `solid = false` while swinging so it never
    shoves the player). If the bare opening is walkable, the open doorway MUST leave clear walkable
    passage for a `1.7 m` tall (`0.51 m` wide) body. Pressing `F` again must shut it tightly again.

### Fitting open ground, slopes, and shorelines

When the site has no opening (a slope, clearing, cliff edge, or pond shore):

- Use the provided `7×7` local ground height grid (in local `y`) to seat foundations, posts, walls,
  or ramps into the terrain in every dry quarter of your element's footprint so nothing floats or is
  buried. Over water (docks, piers, bridges), structures may extend out above the surface cleanly.

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
