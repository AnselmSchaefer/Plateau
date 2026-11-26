# Plateau — Project Requirements & Architectural Guidelines

Keep these core requirements in mind across all future sessions and design decisions.

## 1. Modifiable & Composable Worlds (Core Requirement)

Plateau is not just a read-only viewer for generated maps — players must be able to **change existing maps** and **compose new Claude-made elements onto them**:

1. **Add elements to existing maps without recompiling the base map**
   - Support placing standalone elements (a tree, a hill, a bridge, a lantern, a contraption) onto an existing area as composable `.wasm` modules or SDF stamps, persisted in a save file alongside the base area + seed.
2. **Sculpt & dig landforms at every scale**
   - Players must be able to add landforms (e.g. a hill) and carve into them at their current size (e.g. a tiny person digging a 2 cm cave into a hill, or a giant carving a chasm).
   - Implement via a scale-stamped SDF edit overlay (`Edited<R>` wrapping `Region::sample`) with spatial indexing and targeted chunk invalidation, so rendering, physics (`Body::step`), and `reach` all see edits automatically.
3. **Modify, cut, carry, and relocate physical objects**
   - Players must be able to interact with and transform placed objects — e.g. **chop down a tree, carry it somewhere else, and place it down** as solid geometry (such as a bridge), with weight/carry behaviour aware of `player.size` (tiny, person, giant).
   - Design runtime `Command` verbs to stay small, orthogonal, and engine-clamped (building on `SetTransform`, `SetVisible`, `SetSolid`, `ApplyImpulse`, `SetLight`, `Say` — with future verbs like dynamic `SpawnInstance`/`DespawnInstance` and carry/drop attachment).

## 2. Hybrid Layer Ownership Rule

Keep a clean split between the two geometric representations so carving and portable objects both work naturally:

- **SDF Layer (`sample()` + SDF edit log)**: owns carvable, sculptable landforms — terrain, hills, cliffs, caves, arches, water, and multi-scale rock. Do **not** move carvable hills/landforms into static triangle meshes.
- **Procedural `Scene` Layer (`MeshBuilder` + `SceneBuilder` + `tick`/`interact`)**: owns thin/repeating details (reeds, flowers, grass blades), architectural/crafted pieces, and discrete **cuttable / portable / animated / interactive objects** (trees meant to be chopped and carried, rafts, lanterns, creatures, contraptions). Solid objects set `solid: true` and collide via `Combined` (`engine/src/scene.rs`).

## 3. Non-Negotiable Invariants

- **Strict WASM Sandboxing**: all user- and Claude-generated areas and elements compile to single `.wasm` files executed in Wasmtime with **zero host capabilities** (no filesystem, network, clock, or threads), strict `wasmparser` load checks, fuel/memory caps, and validated output caps (`meshes`, `vertices`, `indices`, `instances`, `lights`, `commands`, `impulse`).
- **Bit-Exact Determinism**: areas use only `plateau_sdk` math/noise (`nans_unseen` and chunk-hash laws must hold across machines and builds).
- **Renderer-Agnostic Engine**: `plateau-engine` must never depend on Bevy; Bevy lives strictly in `play/`.
- **Open Workshop, No Closed Prefab Ceiling**: prefer raw procedural geometry builders (`MeshBuilder`) + instancing + orthogonal physical verbs over fixed prefab enums.

## 4. Reference & Useful Commands

- Architecture overview diagram: `docs/architecture.html` and `docs/architecture.png`
- Concept & roadmap: `CONCEPT.md`, `POC_PLAN.md`
- Current plan (phases D–K: `World::apply` + action log, world graph, support & falling, materials & tools, parametric elements, brick cache, inspector): `WORLD_PLAN.md`
- Claude authoring guide: `make/GUIDE.md`

```bash
./areas/build.sh                                                            # rebuild carried areas after SDK/area edits
cargo test --workspace --exclude view-bevy --exclude view-wgpu --exclude plateau-play
cargo run --release -p plateau-play -- made/pond-sketched/area.wasm         # walk & interact (F / right-click)
cargo run --release -p plateau-make --bin plateau -- check <name>           # verify area.rs against all laws
```
