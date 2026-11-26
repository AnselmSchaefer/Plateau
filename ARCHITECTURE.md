# Plateau's architecture

How the parts work together, what the words mean, and where each thing lives. The module list
itself is made from the code: [`docs/MODULES.md`](docs/MODULES.md) (each module, what it says it
is, and what it uses). The picture is `docs/architecture.html`. The rules are in `AGENTS.md`; the
plan is `WORLD_PLAN.md`.

## The crates

| Crate | What it is | Knows of |
|---|---|---|
| `sdk/` (`plateau-sdk`) | What an area's or element's program is written with: `sample` answers, shapes and their sums, noise, `MeshBuilder`/`SceneBuilder`, the deterministic maths. Compiled into each `.wasm` | Nothing of the engine |
| `engine/` (`plateau-engine`) | Everything a world is and does, without a window: modules checked and run in a sandbox, the SDF made into chunks, bodies, the laws, pictures, and the live world (`live::World`) | The SDK's interface (`sdk/wit/region.wit`), never Bevy |
| `play/` (`plateau-play`) | The window (Bevy): input, aiming, ghosts, the surface drawn, what changed shown | The engine |
| `make/` (`plateau-make`, the `plateau` command) | Claude makes areas and elements: writes the source, builds it with the SDK, judges it by the laws, pictures it, asks again | The engine (to judge and picture) |
| `areas/`, `elements/`, `made/` | Hand-written and Claude-made areas and elements, as source and `.wasm` | The SDK |

## Two layers of geometry

- **The SDF layer** owns what can be carved: terrain, hills, caves, water, rock. An area answers
  `sample(points, size) -> (distance, material)`; the engine lays edits (`edit::EditLog`), water
  (`water::WaterLog`) and placed SDF elements over it (`compose::Composed`), in the order they were
  made. Chunks (`chunk`), bodies (`body`), reach (`reach`), the laws and pictures all ask the same
  `Region`, so they all see every change.
- **The scene layer** owns what is thin, repeated, built, or moved: grass, reeds, trees, bridges,
  boats, lanterns. Meshes from `MeshBuilder`, instanced; solid ones collide through
  `scene::Combined`. A scene element carved into is **promoted**: its solid meshes become an exact
  SDF (`mesh_sdf::MeshSdf`) so the carve cuts it, and it can still be chopped and carried.

## How a change is made

```
play (input, aim)          engine (live::World)                                play (shown)
──────────────────         ─────────────────────────────────────────────       ─────────────────
key, ray, brush  ──Action──▶ apply ─▶ fall under way finished                   take_changes:
                            ├─ rule book (rules::RuleBook) ─▶ Denied           ├─ SDF snapshot +
                            ├─ tool (physics::Tool)        ─▶ Denied           │  Invalidation ─▶ chunks
                            ├─ the change (edit, place, cut, carry, steer)     ├─ LayerChange ─▶ scene meshes
                            ├─ what follows (promotion, water, buoyancy)       └─ words ─▶ HUD
                            ├─ the graph (derived_from, rests_on, owned_by, attached_to, fills, carved_into, flooded_by)
                            ├─ commit: a new ComposedLog snapshot
                            ├─ history (undo/redo, with the graph's delta)
                            └─ action log line (--log) ─▶ Outcome
```

- Everything a save keeps changes only through `World::apply`; `play` asks and shows. What is only
  seen (a topple under way, bobbing, a rider pressing a boat down) is `World::animate`d between
  actions and moves nothing saved, so a log replays (`live::replay`) onto the save it began from.
- **The world graph** (`graph::Graph`, kept by `live::relations`): typed relations between
  elements, the terrain and players, in an ordered map. A cut makes two new pieces, each
  `derived_from` the original along its side of the cut and `owned_by` the original's owner (not
  the cutter); `Place`, `Drop` and a cut piece's landing write `rests_on` from real contacts
  (`World::contacts_at`: the element's solid points touching something from above, within 5 % of
  the smaller one's size). A fitted `Place` (given where its opening was measured) `fills` and is
  `attached_to` what the opening is in (`World::host_at`: what is solid just behind its wall); a
  terrain carve is `carved_into` the terrain and the elements it cuts; a water body is
  `flooded_by` the carves it came in through. Each action's `GraphDelta` is in its `Summary` and
  kept with its undo. The rule book asks the graph who owns what. Its laws (`laws::world_graph`)
  are kept by every test world.
- **A site's neighbourhood** (`World::neighbourhood`, `live::near`): what a measured site is in and
  who owns it, what that was cut from, carves and water there, what already fills it, and the
  elements near by, in words, saved in the `.site` file (`near` lines) and given to Claude in
  `describe_for_prompt`. `play`'s `I` fills it; `make --place` places through `World::apply`.
- `--log actions.jsonl` writes every action and what came of it; `--trace trace.json` writes the
  engine's spans for the Perfetto UI.

## Invariants (from `AGENTS.md`)

- **Sandbox**: areas and elements are single `.wasm` modules with no host capability but their seed;
  checked on every load (`check`), run with fuel and memory caps (`wasm`), their outputs capped.
- **Determinism**: the same inputs give the same bits on every machine. Areas use the SDK's maths;
  the engine uses `math` (no platform maths library outside `picture`, enforced by a test); saves name
  modules by hash (`modules`); chunk hashes are kept (`laws`, `areas/*/hashes.txt`).
- **No Bevy in the engine.**

## Words

| Word | Meaning | Where |
|---|---|---|
| Area | A 64 m square of world: one `.wasm` module answering `sample` and `describe` | `sdk`, `engine::wasm` |
| Atlas | Areas side by side, their borders matched | `engine::atlas` |
| Size | The player's height: tiny 1.7 cm, person 1.7 m, giant 17 m, or between | `engine::SIZES` |
| Sample | An area's answer at a point for a viewer of a size: distance to the surface and material | `Region::sample` |
| Chunk | A cube of surface made by surface nets, at a level of detail for the viewer | `engine::chunk`, `rings` |
| Edit, stamp | One brush stroke: add or carve a shape, made at the player's size (`stamp_size`) | `engine::edit` |
| Element | A standalone `.wasm` placed onto any area: an SDF landform or a procedural scene | `engine::compose` |
| Placed element | An element in a world: module, seed, transform, scale, cut planes, its own edits | `compose::PlacedElement` |
| After-edits | How many edits existed when an element was placed: edits before it are under it, after it cut it | `PlacedElement::after_edits` |
| Promoted | A scene element turned to an exact SDF by the first carve into it | `mesh_sdf`, `PlacedElement::promoted` |
| Macro proxy | A promoted element's crisp mesh kept for viewers far larger than it (whose voxels would lose it) | `compose::PROMOTED_SDF_MAX_SIZE_RATIO` |
| Clip | A plane an element was chopped along: a piece is its scene clipped and capped | `cut::ClipPlane` |
| Water body, basin | Engine-owned water flooding a carve that opens onto it, at its level | `engine::water` |
| Rule book | Who may do what where, up to which size | `engine::rules` |
| Tool | What the player holds; each carves and cuts some materials | `physics::Tool` |
| Site, cavity, opening | A place measured at the player's size; an opening's rim, outline, depth, floor | `engine::site` |
| Fit | Whether an element meets its site (rim fit, ground fit), and where it goes to | `engine::fit` |
| Law | A property every area must keep, searched for, never assumed (sizes agree, gates are real, …) | `engine::laws` |
| World save | `plateau-world 2`: tiles, module hashes, rules, water, edits, elements, relations (`rel`) | `edit::WorldSave`, `modules` |
| World graph, relation | What each element was cut from (`derived_from`), rests on (`rests_on`), is owned by (`owned_by`), is attached to (`attached_to`) and fills (`fills`); what each carve cut (`carved_into`) and let water into (`flooded_by`) | `engine::graph` |
| Host | What an opening is in: the terrain or an element, solid just behind its wall | `World::host_at` |
| Neighbourhood | A site's surroundings in the world, in words, for Claude | `live::near`, `SiteContext::near` |
| Footing | How an element is held up: ground (an SDF landform), rooted, afloat, carried, or loose (resting on something) | `live::relations::Footing` |
| Contact | An element's solid touching something from above, within 5 % of the smaller one's size: what `rests_on` is made of | `World::contacts_at` |
| Piece, original | The two elements a cut makes, and the element they were cut from (gone, its id kept) | `live::objects`, `graph` |
| Action, outcome | A change asked of the live world, and what came of it | `live::action` |
| Action log | `plateau-log 1`: one JSON line an action | `live::log` |
| Changes | What the window must show after actions: SDF snapshots, layers, words | `live::Changes` |
