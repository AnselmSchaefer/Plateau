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
  SDF (`mesh_sdf::MeshSdf`) so the carve cuts it, and it can still be chopped and carried. It is
  still drawn from its own triangles, not by the terrain's chunks (which leave it out,
  `ComposedLog::without_promoted`): `carved` keeps every triangle away from the carves as made and
  dual-contours only the carved surface, sharp-rimmed, in bark and fresh-cut tints.

## How a change is made

```
play (input, aim)          engine (live::World)                                play (shown)
──────────────────         ─────────────────────────────────────────────       ─────────────────
key, ray, brush  ──Action──▶ apply ─▶ fall under way finished                   take_changes:
                            ├─ rule book (rules::RuleBook) ─▶ Denied           ├─ SDF snapshot +
                            ├─ tool (physics::Tool)        ─▶ Denied           │  Invalidation ─▶ chunks
                            ├─ the change (edit, place, cut, carry, steer)     ├─ LayerChange ─▶ scene meshes
                            ├─ what follows (promotion, water, buoyancy)       └─ words ─▶ HUD
                            ├─ support (live::falls): mended, come apart, fallen
                            ├─ the graph (derived_from, rests_on, owned_by, attached_to, fills, carved_into, flooded_by, uprooted)
                            ├─ commit: a new ComposedLog snapshot
                            ├─ history (undo/redo, with the graph's delta)
                            └─ action log line (--log) ─▶ Outcome
```

- Everything a save keeps changes only through `World::apply`; `play` asks and shows. What is only
  seen (a fall under way, bobbing, a rider pressing a boat down) is shown between actions
  (`World::tick_falling`, `World::animate`) and moves nothing saved, so a log replays
  (`live::replay`) onto the save it began from. One action's changes (a cut, then what came apart
  and fell) are undone and redone together (`WorldAction::Many`).
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
- **Support and falling** (`live::falls`, with the arithmetic in `support`): an action notes what
  it shook (carved, cut, moved or took away from under something), and `settle_support` runs after
  it, in rounds. Loose elements the world changed under rest on what they now touch. An element
  carved or cut is made into cells in its own frame (about 48 along its longest side, from the SDF
  the world has of it); a part no longer joined to what holds it up comes away along a plane
  between them — or, where no single plane parts them (a ring cut round an island, an interlocking
  joint), by complementary box-carve masks in each piece's `local_edits` (`support::mask_boxes`,
  recorded as `derived_from ... split`, with non-solid trim and lights culled to match) — as two
  new pieces `derived_from` it; where a carve left a cross-section that cannot hold the mass above
  it (overhanging it by more than a quarter of its width), it breaks there.
  Then every loose element that nothing reaching the ground holds up (`World::grounded`: two pieces
  resting on each other alone are not), and every piece that may not stand, falls: straight down to
  the first thing beneath, then over the edge its centre of mass overhangs, turning until it meets
  something, against the terrain and every other element, then onto water. What rested on it is
  asked next. At most `World::fall_limit` falls an action (`FALL_LIMIT`, 24); what is left falls
  with the next action. A chop (`K`) is a cut and a push the way the player faces: no scripted
  topple.
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
| Carved mesh | How a promoted element is drawn: its own triangles, remade only about its carves | `carved::Carver`, `ElementRuntime::drawn_scene_of` |
| Macro proxy | A promoted element's mesh kept in collisions for viewers far larger than it (whose coarse SDF skips it) | `compose::PROMOTED_SDF_MAX_SIZE_RATIO` |
| Clip | A plane an element was chopped along: a piece is its scene clipped and capped | `cut::ClipPlane` |
| Water body, basin | Engine-owned water flooding a carve that opens onto it, at its level | `engine::water` |
| Rule book | Who may do what where, up to which size | `engine::rules` |
| Tool | What the player holds; each carves and cuts some materials | `physics::Tool` |
| Site, cavity, opening | A place measured at the player's size; an opening's rim, outline, depth, floor | `engine::site` |
| Fit | Whether an element meets its site (rim fit, ground fit), and where it goes to | `engine::fit` |
| Law | A property every area must keep, searched for, never assumed (sizes agree, gates are real, …) | `engine::laws` |
| World save | `plateau-world 2`: tiles, module hashes, rules, water, edits, elements, relations (`rel`) | `edit::WorldSave`, `modules` |
| World graph, relation | What each element was cut from (`derived_from`), rests on (`rests_on`), is owned by (`owned_by`), is attached to (`attached_to`) and fills (`fills`); what each carve cut (`carved_into`) and let water into (`flooded_by`); which elements lost their roots (`uprooted`) | `engine::graph` |
| Host | What an opening is in: the terrain or an element, solid just behind its wall | `World::host_at` |
| Neighbourhood | A site's surroundings in the world, in words, for Claude | `live::near`, `SiteContext::near` |
| Footing | How an element is held up: ground (an SDF landform), rooted, afloat, carried, or loose (resting on something) | `live::relations::Footing` |
| Contact | An element's solid touching something from above, within 5 % of the smaller one's size (a field only where the point, let down that far, is in it): what `rests_on` is made of. A carved element touches with its cells, not its meshes | `World::contacts_at` |
| Uprooted | Rooted by its shape, but no cell of it touches the terrain any more (dug round, carved away): loose, and its pieces too | `graph::Kind::Uprooted` |
| Grounded | Held up by what reaches the ground: ground, rooted, afloat, carried, or resting on or attached to the terrain or something grounded | `World::grounded` |
| Support check | What runs after an action: rests mended, parts come away, cross-sections break, what is not grounded falls | `live::falls` |
| Cells | An element's solid sampled on a grid in its own frame, to find its parts, its centre of mass and what it falls with | `support::Grid` |
| Part, come apart | Cells joined face to face; a part not joined to what holds the element up is split off along a plane between them (`support::separating_plane`), or by complementary box-carve masks (`support::mask_boxes`, `derived_from ... split`) when no plane parts them | `support::separating_plane`, `support::mask_boxes` |
| Section, grip | A level cross-section a carve reached; it holds the mass above it while that overhangs it by at most a quarter of its width (`SECTION_GRIP`) | `support::overhang` |
| Fall, tip | Dropping straight down to what is beneath, and turning over the edge a centre of mass overhangs, against everything but itself | `live::falls` |
| Fall limit | The most falls one action sets off; the rest fall with the next | `World::fall_limit` |
| Piece, original | The two elements a cut makes, and the element they were cut from (gone, its id kept) | `live::objects`, `graph` |
| Action, outcome | A change asked of the live world, and what came of it | `live::action` |
| Action log | `plateau-log 1`: one JSON line an action | `live::log` |
| Changes | What the window must show after actions: SDF snapshots, layers, words | `live::Changes` |
