# Plateau: a plan for a world that holds together

`POC_PLAN.md` answered whether Claude can make areas worth visiting at every size, and phases A–C
(2026-11-25) added rules, water, tools, buoyancy, and site-fitted elements. Each was built as its
own showcase. This plan joins them into one model, so that changing the world, physics consequences,
permissions, and fitting new elements all go through the same place and agree with each other.

Written 2026-11-26, from the review of the work of 2026-11-23 to 2026-11-25. About six to seven
weeks. Phases continue the lettering of A–C.

## What the review found

| # | Finding | Where |
|---|---|---|
| 1 | Every world change (sculpt, place, chop, carry, promote, buoyancy, rule and tool checks) lives in the Bevy crate; `look_and_ask` alone is ~1170 lines | `play/src/main.rs:921–2094`, `play/src/place.rs:33` (`WorldAction`) |
| 2 | `RuleBook` says it is enforced by the engine, but is only called from about eight sites in `play` | `engine/src/rules.rs`, `play/src/main.rs` |
| 3 | Saves name modules by absolute path, with no content hash; a rebuilt module silently replays old edits onto a different base | `engine/src/edit.rs` (`WorldSave`), e.g. `made/elements/oak-hollow-door/element.plateau` |
| 4 | Clicks skip stroke spacing, so identical stamps pile up in the log | `play/src/main.rs` (`spacing_ok`) |
| 5 | libm math where results are stored or will be synced | `physics.rs:356,416`, `compose.rs:252`, `body.rs:504` |
| 6 | Objects have no relations: a chopped stump and trunk don't know they were one tree, nothing knows what rests on what | `engine/src/compose.rs` (`PlacedElement`) |
| 7 | Chopping is a scripted 90° topple onto terrain only (through other objects); carving through a trunk with `B` leaves the canopy floating | `engine/src/cut.rs:400,432`, `play/src/main.rs:1560–1700` |
| 8 | Materials are surface labels; carving exposes whatever an area returns inside, and a carve may repaint walls with any material | `edit.rs:227`, `sdk` (`Answer`) |
| 9 | Tools check only the aimed point, not the brush volume; `Add` is not gated; the default tool is `All`; edits don't record their tool | `physics.rs:65`, `play/src/main.rs:1205–1215` |
| 10 | Fitting is door-shaped (`cavity_fit`, `ground_fit`); the site is baked into the prompt, so each opening needs a new Claude run (605 s), and widening it breaks the fit | `engine/src/fit.rs`, `engine/src/site.rs` (`describe_for_prompt`) |
| 11 | Sampling cost is CPU and WASM bound; placed SDF elements are looped linearly per batch | `engine/src/compose.rs:621` |
| 12 | Phases A–C are not recorded in `POC_PLAN.md`; no glossary or module map | — |

## Decisions to make first

| Decision | Recommendation | Why |
|---|---|---|
| Does terrain fall when undercut? | **No.** Terrain (SDF landforms) stays rigid; discrete objects (Scene elements, promoted or not) fall | Cheap and predictable; floating overhangs are acceptable; sand slump can come later |
| Does adding material cost material? | **A mode**: `workshop` keeps free building; `world` mode makes carving yield material (volume × density) and adding consume it | Keeps the sandbox for authors and closes the dig-to-build loop for play |
| Default tool | `Hands` in `world` mode, `All` only in `workshop` mode | Restrictions should be on unless switched off |
| Elements taking parameters | **Yes**: a new bounded, pure-data host import `params() -> list<f32>` in `plateau:region@0.2.0` | One door fits every opening and fitting runs again when the opening changes; no capability beyond data |
| Old saves | Keep reading `plateau-world 1`; write version 2 | Nothing made so far is lost |
| Graph database | **Not in the engine.** A small typed graph inside `plateau-engine`; a database only later for the gallery and server | Bit-exact, saved with the world, browser-portable |

## Order

```
D groundwork ─► E World::apply + log ─► F world graph ─► G support & falling
                    │                       ├─────────► I parametric elements & fit
                    │                       └─────────► K inspector (needs E's log and F's graph)
                    ├─► H materials & tools (masked carve finished in J)
                    └─► J brick cache & performance (uses E's tracing spans)
```

D and E come first and block everything else. After E, H and J can run alongside F and G. K starts
once F's graph is saved; it is a separate project that only reads what the engine writes.

---

### D. Groundwork (two or three days)

- **Saves that mean the same thing everywhere** (`engine/src/edit.rs`):
  - `plateau-world 2`: each `tile` and `element` record carries the module's content hash
    (BLAKE3 or SHA-256 of the `.wasm` bytes) next to a path relative to the save's root.
  - Loading checks the hash; a mismatch is refused with a clear message, or loaded only with
    `--accept-changed-modules`, which is reported in the HUD.
  - Version 1 saves still load; they are written back as version 2.
- **No duplicate stamps**: a stamp is skipped if it equals the previous one (same op, shape, centre,
  material) or lies within the spacing of the last stamp, click or hold.
- **Determinism**: replace `sin`, `cos`, `atan2`, `powf` in `physics.rs`, `compose.rs` and `body.rs`
  with the SDK's polynomial versions (as `site.rs` already does). Add a test that greps the engine's
  sources for libm calls outside `picture.rs`.
- **Record phases A–C** in `POC_PLAN.md` (what was built, what was found), as steps 0–9 are.

**Done when** a save made, the module rebuilt with a change, and loaded again is refused with the
module's name; a save carried to another directory loads; `engine/tests/edits.rs` round-trips v1 and
v2; the libm test passes.

*Done 2026-11-26 (not yet played by hand):*

- `engine/src/modules.rs`: `module <path> sha256 <hex>` records; `WorldSave::seal` (paths made
  relative to the root, hashes recorded, a hash kept for a module that can no longer be read) and
  `save_sealed`; `read_module` and `ElementCatalog::expect_hashes` refuse a changed module by name
  under `ModuleCheck::Strict`, or let it through and list it under `AcceptChanged`.
  `OpenWorld::open_checked` and `changed_modules`.
- `play`: a save opens on the areas it names unless others are given; every module is checked
  before anything loads (a reload is refused before the world shown is touched);
  `--accept-changed-modules` loads anyway and says which changed in the HUD; saves are sealed
  against the repository. `make` seals the saves it writes.
- Duplicate stamps dropped (the same edit as the last, on the terrain or in an element's frame).
- `engine/src/math.rs`: `sin`, `cos`, `atan2` (the SDK's), `log2`, `log10`, `exp2`, `pow`, from IEEE
  operations and bits only; used by `body` (walking, growing, sinking), `physics` (steering, bobbing),
  `compose` (yaw from a quaternion) and `rings`. Growing now ends at exactly the size asked for.
- Tests: `engine/tests/phase_d.rs` (8, the v1/v2 round trips among them, kept here rather than in
  `edits.rs`), and `math`'s own two.

Left: `powi` with whole exponents (squares, powers of two) is still the library's, exact in practice;
`sketch.rs`'s colour distances are made once, by `make`. The duplicates in the oak-hollow-door save
came from clicks whose aim did not move after carving into a promoted trunk, which suggests the aim
ray did not see that carve; looked at again in E, when the verbs move.

### E. Changes go through the engine: `World::apply` (one to one and a half weeks)

The engine owns the world and every change to it; `play/` turns input into actions and shows what
changed.

- **`engine/src/world.rs`**: grow `OpenWorld` into `World`, the one live world used by `play`, the
  tests, `site`, `fit` and `picture`. It holds the atlas, `ComposedLog`, `SceneWorld`,
  `ElementRuntime`, `ElementCatalog`, `RuleBook`, and the undo history.
- **`Action`** (engine): `Sculpt { op, shape, at, normal, material, tool }`, `Place { module, seed,
  tf }`, `Remove { id }`, `Cut { id, hit, dir, tool }`, `Grab { id }`, `Drop { at, rot }`,
  `Relocate { id, tf }`, `Pilot { forward, turn }`, `Interact { hit }`, `Undo`, `Redo`. Each carries
  the player (`id`, `size`, `feet`, `yaw`).
- **`World::apply(&mut self, player, Action) -> Result<Delta, Denial>`**: checks rules and tools,
  does the change, and runs its consequences (promotion, water expansion, buoyancy; falling in G).
- **`Delta`**: edits added or removed, elements spawned, removed or moved (with their Scene layers),
  water bodies changed, bounds to invalidate, messages for the player. Undo keeps the inverse
  `Delta`; `WorldAction` moves out of `play`.
- **Verbs in their own modules**: `engine/src/verbs/{sculpt,place,cut,carry,boat}.rs`, moved out of
  `look_and_ask` and tested headless.
- **`play/`**: input → `Action`; `Delta` → `Chunks::set_edits` invalidation, scene spawning, HUD.
  Ghosts and aiming stay in `play`.
- **`ARCHITECTURE.md`**: a glossary (site, cavity, opening, promoted, stamp size, after-edits,
  macro proxy, basin) and a module map (who owns what, which layer each concept is in). Linked from
  `AGENTS.md`. A script (`cargo-modules` or rustdoc JSON) writes the module list, so it cannot drift.
- **The world's log** (the inspector's data, phase K; written while the verbs move, not after):
  - **`tracing` spans** round each stage an action passes through: `apply`, the rule check, the
    tool check, promotion, the edit log, water expansion, buoyancy, chunk invalidation, and in
    `play` the workers' sampling and meshing. Exported as a Chrome trace (`--trace <file>`) and
    read in the Perfetto UI. They time, they never change a result: the same hashes with tracing
    on and off.
  - **An action log**: `World::apply` writes one JSON line per action (`--log <file>.jsonl`): a
    sequence number, the player (id, size, feet), the `Action`, then the `Delta` it gave or the
    `Denial` it met, and the world's revision before and after. Plain data in a versioned format
    (`plateau-log 1`), with ids matching the save's, so a log and its save read together.
  - **Replay**: a log applied to the save it started from gives the save it ended with, the same
    bits (`plateau replay <save> <log>`). This makes the log a test, not only a record.

**Done when** the PoC's showcase (place a tree, carve a tiny home into the trunk, chop it, carry it,
lay it down as a bridge, walk across it at each size, save, reload, same chunk hashes) runs in
`engine/tests/world.rs` through `World::apply` alone; `play` has no `rules.check` and no `Tool`
check; `look_and_ask` is under 300 lines; the game plays as before; the showcase's log replays to
its save bit for bit, and its chunk hashes are the same with tracing on and off.

*Slice 1 done 2026-11-26 (played by the tour, not yet by hand):*

- `engine/src/live/`: `World` owns the areas, the sculpting, water, rules, placed elements, their
  runtimes and scenes, the history (`WorldAction`, moved from `play`), and what is falling, floating,
  carried or piloted (by id, ordered, so every machine walks them alike). `World::open` checks every
  module the save names; `save` seals; `reload` checks before it replaces anything.
- `World::apply` for `Sculpt` (terrain or a placed element, the tool checked at the aimed point,
  the rule book, water flooding a carve, promotion, the same stamp dropped), `Place`, `Remove`,
  `Undo`, `Redo`: `Outcome::Done(Summary)`, `Denied(reason)` or `Unchanged`.
- The window learns what changed from `World::take_changes`: SDF snapshots with the chunks they make
  stale (`Invalidation`, owned, replacing `play`'s), elements' layers to make, move or take away,
  and words for the player. `play` keeps only input, aiming, ghosts and the carried item's look:
  `main.rs` 2615 → 1696 lines, `look_and_ask` ~1170 → 781.
- The action log: `--log <file>.jsonl`, `plateau-log 1`, one line an action with its player, outcome
  and revisions; `live::replay` applies a log and checks every outcome; `tracing` spans round
  `apply`, the rule and tool checks, promotion, water and commits.
- Tests: `engine/tests/world.rs` (4): the showcase's opening (place, hill, a tiny home in the trunk,
  a cave, undo, redo) through `apply` alone; refusals by tool and rule book; a log replayed onto
  its starting save gives the same save text and the same samples; save and reload.

*Slice 2 done 2026-11-26 (played by the tour, not yet by hand), and with it phase E:*

- Actions for objects (`engine/src/live/objects.rs`): `Cut` (rule book, tool and size checked, the
  cut plane, the topple decided and its rest found on terrain and water), `Grab`, `Drop`, `Return`,
  `Pilot`, `Steer`, `Interact`. `play` has no rule or tool check left.
- **Logical and shown, apart**: a topple is decided at the cut and ends before the next action
  (`finish_all_falls` at the start of `apply`), seen or not; elements settle unloaded; bobbing and
  a rider's weight are `World::animate`d and move no saved transform. So a session's log replays
  with no frames between its actions.
- `play --trace trace.json`: the engine's spans (`apply` and its stages, chunks made, `tick`) as
  Chrome trace events, through a layer of Bevy's own subscriber (`play/src/trace.rs`); the tour
  runs as fast with it.
- `look_and_ask` 195 lines: each mode its own handler (`measure_site`, `sculpt_input`,
  `place_input`, `chop_input`, `carry_input`, `history_input`); `main.rs` 2615 → ~1580 lines.
- `ARCHITECTURE.md` (crates, the two layers, how a change is made, words) and `docs/MODULES.md`,
  made from the code by `engine/tests/docs.rs` (each module's first sentence and what it uses,
  across crates), failing while stale.
- Tests: `engine/tests/world.rs` (6): the whole showcase (a tree placed, a tiny's home carved in
  it, chopped, the trunk carried and laid flat as a bridge, stood on, the move undone and redone,
  the session's log replayed to the same save and samples), a boat paddled across the pond and
  replayed; `engine/tests/tracing.rs`: the same session traced and not gives the same world, and
  the spans are there (alone in its binary: tracing's interest is process-wide).

Short of the done-when: walking across the laid bridge is checked by standing on it at a person's
size, not walked at each size; nothing yet played by hand.

### F. A world graph (one week)

Objects know what they come from, rest on, belong to, and fill.

- **`engine/src/graph.rs`**: nodes are placed elements and the base areas (later: an element's
  declared parts). Edges are typed, kept in ordered maps with stable ids:
  - `derived_from(piece, original, cut)`: written by `Cut`;
  - `rests_on(a, b | terrain)`: written by `Drop`, settling and falling, from real contacts;
  - `owned_by(node, player)`: chopped pieces inherit the original's owner, not the cutter;
  - then `attached_to`, `fills(element, opening)`, `carved_into(edit, node)`, `flooded_by(basin,
    edit)`.
- **Changes as graph edits**: `Delta` reports edges added and removed; undo restores them.
- **Saved**: `rel <kind> <from> <to> [data]` lines in `plateau-world 2`.
- **Rules through the graph**: `CutCarryPolicy::OwnOnly` uses `owned_by`, so a piece cut from
  someone else's tree stays theirs.
- **Laws** (`engine/src/laws.rs`):
  - every `rests_on` edge is a real contact, within a tolerance of the smaller object's size;
  - no object is unsupported (no `rests_on`, not rooted, not floating);
  - every piece made by a cut has `derived_from`;
  - the graph round-trips through the save exactly.
- **Context for Claude**: `SiteContext::describe_for_prompt` adds the site's neighbourhood ("inside
  oak #12, owned by …, a carved tunnel, 3 m from the pond's edge").

**Done when** `engine/tests/graph.rs`: chopping gives two pieces linked to the original; a log laid
on a bridge rests on it; ownership survives a chop by another player; every law passes on the
showcase world; saves round-trip.

*Slice 1 done 2026-11-26 (headless tests only; not yet played by hand):*

- `engine/src/graph.rs`: nodes are the terrain, placed elements by id (an original that was cut
  keeps its id, never given out again) and players; relations `derived_from` (with the piece's
  side of the cut), `rests_on`, `owned_by`, at most one of a kind between two nodes, in a
  `BTreeMap`. `GraphDelta` (added, removed) undoes and redoes.
- Saved as `rel <kind> <from> <to> [data]` after the elements (`rel derived_from 3 1 clip …`,
  `rel rests_on 3 terrain`, `rel owned_by 3 @local`); refused if they name an element not in the
  save (but for a piece's original), name one twice, or come in a `plateau-world 1` save. A save
  with no `rel` records (made before the graph) is given one when opened: owners from who placed
  each element, `rests_on` from contacts.
- Written by `World::apply` only, through a journal (`live/relations.rs`): every action's delta is
  in its `Summary` (so the action log carries it and `replay` checks it), kept with its undo, and
  reverted by it. `Place` writes `owned_by` and `rests_on`; `Remove` takes the element's relations
  and what rested on it; **a cut now makes two new pieces** (the stump is no longer the original's
  id), each `derived_from` the original and owned by its owner, not the cutter, the top piece
  resting where its fall ends; `Drop` rests the element on what it touches there and frees what
  rested on it. A carried element keeps its relations until set down.
- `rests_on` from real contacts (`World::contacts_at`): up to 512 points of the element's solid
  meshes, at a transform, asked of the terrain (areas and sculpting, no water: a floating thing
  rests on nothing) and of each other element alone (scene solids, or the SDF entry of a landform
  or promoted one); a contact is a point within 5 % of the smaller one's size with the surface
  below it. `Footing` says what needs holding up: ground, rooted, afloat, carried, or loose.
- The rule book asks the graph: `rules::Action::{CutOrCarry, RemoveElement, SculptElement}` take
  `owner` (from `owned_by`) instead of the placer.
- Laws (`laws::world_graph`): `rests_are_contacts`, `none_unsupported`, `cuts_are_derived`,
  `graph_round_trips`.
- Tests: `engine/tests/graph.rs` (5): a chop gives a stump and a trunk derived from the tree, the
  cut's delta undone and redone exactly; a trunk laid on a bridge rests on the bridge (not the lawn),
  undone onto the lawn, the law failing when it is moved behind `apply`'s back, and loose (law
  failing, as it should until G) when the bridge is removed; Bob chops Alice's tree and the pieces
  stay hers, so under `own` he cannot carry them and she can; the showcase (bridge, tree, a tiny's
  home, a hill, a chop, the trunk laid on the bridge) keeps every law at each step, saves and opens
  with the same graph and text, its log replays to the same graph, and its save stripped of `rel`
  records derives the same owners and rests; bad `rel` records are refused.

Played by hand the same day: a short log set down on a bridge over a flooded channel sank through
the deck to the water, because buoyancy asks only the terrain and water (a phase E fault, not the
graph's). Now, when buoyancy would move something down while it rests on another element, the
support wins, and a rider's weight doesn't press it down either (`settle_buoyancy_for`, `animate`);
`a_log_laid_on_a_bridge_over_water_stays_on_the_bridge`, from the session's own channel and bridge.
Logs played before this fix replay only up to such a drop.

Short of the done-when: nothing. Not yet in the graph: `attached_to`, `fills`, `carved_into`,
`flooded_by`, element parts, and `SiteContext::describe_for_prompt`'s neighbourhood. Relations go
stale when the world changes under them without the element moving (terrain carved from under a
log, water rising round it): G's support check is what should mend them; until then the laws say so.

### G. Support and falling (one to one and a half weeks)

When a tree's trunk is removed, the tree falls.

- **What is still connected** (`engine/src/support.rs`): after any `Delta` that touches an object
  (a carve into it, a cut, a removed supporter), voxelise the object in its own frame, 32–64 cells
  along its longest axis, from its promoted SDF or solid Scene meshes, and flood fill from its
  anchor (roots below ground, or its `rests_on` contacts).
- **Split**: each part no longer connected becomes a new placed element with `derived_from`. Planes
  (`clips`) cannot describe an arbitrary part, so add a component mask: the part's own baked
  `MeshSdf`, or carve edits of the rest, kept with it.
- **Fall**: if a part's centre of mass is outside its support footprint, it tips about the support
  edge; otherwise it drops straight down. Where it lands is found against terrain **and** other
  objects (`Combined` without itself), then buoyancy settles it. Animated as `tick_falling` already
  is, but driven by the engine.
- **Chain reactions**: whatever `rests_on` a part that moved is checked again, through the graph,
  with a fixed depth and count per `Delta` so one action cannot cascade without end.
- **Chopping becomes a cut plus a support check**: `K`'s planar cut stays, and the scripted 90°
  topple is replaced by the general fall. A notch that leaves enough cross-section (by mass and
  lever arm) keeps the tree standing.
- **Determinism**: voxelising, flood fill, centre of mass, and the fall use IEEE arithmetic and
  SDK math only; `Delta`s hash the same on every machine.

**Done when** `engine/tests/support.rs`:
- a trunk carved through with `B` drops its canopy;
- a notch half through leaves the tree standing; a deeper one fells it;
- a felled tree lands on a neighbouring bridge rather than through it;
- removing a bridge drops the log lying on it;
- a cascade stops at its limit;
- the same results and hashes natively and as modules, and at `-O` and `-O0`.

### H. Materials and tools (one week; the masked carve is finished in J)

A shovel stops at the rock.

- **Materials below the surface**: an SDK helper for layers by depth below the local surface
  (`strata(depth, &[(Material, thickness)])`); a `GUIDE.md` rule; a new law, **materials go
  down**: below every surface reached, at a few depths, the material is plausible (no grass 30 cm
  down, an author-declared core material). Carried areas that break it are fixed and their hashes
  written again.
- **A carve may not repaint walls** in `world` mode; in `workshop` mode it may.
- **Tools check the whole brush**: before a carve, sample a 3³–5³ grid inside the brush at the
  stamp's size. Any disallowed material refuses the stroke, or shrinks the brush until it fits,
  with a message naming the material.
- **`Add` is gated**: by tool, and in `world` mode by carried material.
- **Edits record their tool**: `Edit::tool`, saved, so a server or replay can check every edit
  again.
- **Material economy** (`world` mode): carving yields material by volume × density into the
  player's inventory; adding uses it; carrying weight counts it.
- **Masked carve** (`Op::Carve { only: MaterialMask }`): removes only the allowed materials.
  Analytic CSG gives distances that are too long at material boundaries (breaking `never_too_long`),
  so this is done on the brick cache (J); until then the brush check above stands in.

**Done when** `engine/tests/tools.rs`: a shovel aimed at moss over a stone core is refused or
shrunk; a saw cannot raise stone in `world` mode; replaying a save checks every edit's tool; every
carried area keeps "materials go down".

### I. Parametric elements and general fit (one to one and a half weeks)

One element fits every site it is made for, and keeps fitting.

- **`plateau:region@0.2.0`**: `host.params() -> list<f32>`, at most 64 values, validated as
  finite on load, given per placement and saved with it. `0.1.0` modules keep working (no params).
  `engine/src/check.rs` allows the new import; the WIT bindings are made again and carried in the
  SDK.
- **Sites become params**: `SiteContext` gives a fixed, documented vector (opening width, height,
  roundness, depth, outline radii at 8 spokes, floor drop, wall thickness). `ELEMENT_GUIDE.md`
  teaches Claude to read it.
- **Fitting again**: a `fills(element, opening)` edge; when an edit changes the opening, the site is
  measured again and the element re-spawned with new params; if it no longer passes `cavity_fit`,
  the player is told.
- **More fit laws** (`engine/src/fit.rs`):
  - `span_fit`: a bridge's ends rest on both banks, and its deck carries each size it is meant for;
  - `wall_fit`: a shelf or sign meets a wall over its back face;
  - `style_fit`: the element's materials and colours, against those sampled round the site, within
    a tolerance or noted for the `look` round.
- **`check-element`** judges an element over a range of params (the site's and others nearby), not
  one.

**Done when** the hobbit door, made once, fits three different openings by params alone, and fits
again after the cave is widened; a bridge passes `span_fit` across two gaps; `make/tests/make.rs`
covers param ranges.

### J. Brick cache and performance (one to one and a half weeks)

- **Measure first**: per frame, chunk build time split into WASM sampling, edits, elements and
  meshing; triangles and draw calls by size. Written to `tour.txt` as in step 2.
- **Sparse bricks**: per chunk level, 8³ blocks of baked distance and material, sampled once from
  the base area. Edits re-bake only the bricks they touch; the edit list stays the save format, and
  bricks are a cache made again from it (same bits).
- **Masked carve** (from H) on bricks: exact at material boundaries, so `never_too_long` holds.
- **Placed SDF elements indexed**: a BVH over `sdf_elements` like the edit log's, keeping their
  chronological order.
- **Triangles**: simplify flat regions per chunk (the tiny's 350–535k), keeping seams matched.
- **Look, cheaply**: per-material textures projected from three sides, screen-space ambient
  occlusion, sky and fog that scale with size. Stylised and coherent, not photoreal.

**Done when** the tiny holds ≥ 90 FPS on the GTX 1650 on `pond-sketched`; a person-size stroke
re-meshes its chunks in under 20 ms; 200 promoted trees cost no more per sample than 20 did; every
law and hash still holds.

### K. The inspector (one to one and a half weeks)

A separate project, `inspect/`, in which a person (and, by its command line, an agent) can follow
how the SDK, engine, `play` and `make` work together, and why the world is as it is.

- **Read only, outside the engine's dependencies**: it reads files the engine and tools write and
  links nothing of theirs at run time, so the engine keeps its invariants (no Bevy, no host
  capabilities, determinism) whatever the inspector does.
- **Inputs, all made, none written by hand**:
  - the code map: crates, modules, public types and their dependencies, from `cargo metadata` and
    rustdoc JSON, with each concept of `ARCHITECTURE.md`'s glossary placed on its module
    (`inspect/gen.sh`, run in CI so it never drifts);
  - E's action log (`plateau-log 1`) and Chrome traces;
  - F's world graph, read from `plateau-world 2` saves.
- **Storage**: SQLite (recursive queries for walking the graph), filled by `plateau inspect load`.
  A graph database only if queries across many worlds are wanted later, at the gallery and server
  stage.
- **Views** (a local web page, a graph library such as Cytoscape.js; nothing served beyond
  `localhost`):
  1. **Architecture**: crates and modules, which depends on which, where each concept lives, and
     which layer (SDF or Scene) it belongs to.
  2. **An action's path**: one log line, with the stages its trace passed through in order (input,
     rules, tool, promotion, edit log, water, invalidation, sampling, meshing), each linked to its
     code, and what its `Delta` changed.
  3. **An object's history**: one object's timeline: placed, carved, cut (`derived_from`),
     carried, fell, landed on (`rests_on`), owned by, each step linked to its log line and save
     records.
  4. **A world's graph**: objects and their relations at any revision, stepping through the log.
- **For agents, a command line over the same data**: `plateau inspect why <id>` (an object's
  history as text), `plateau inspect trace <seq>` (an action's path), `plateau inspect owns
  <module>` (what a module owns and who calls it).

**Done when** the showcase's save and log load; its trunk's history shows planted, carved, cut, carried
and laid as a bridge, each linked to its log line; the carve's path shows every stage it passed
through in order; the code map is made by `inspect/gen.sh` with nothing written by hand; and
`plateau inspect why` gives an agent the same history as text.

---

## Measures

| Measure | Target |
|---|---|
| World changes outside `World::apply` | None (no `rules.check`, `Tool`, or `WorldAction` in `play`) |
| Showcase world played headless | Place, carve, chop, carry, bridge, walk, save, reload, same hashes |
| Unsupported objects after any action | None (law) |
| Tool refusals | Every disallowed material in the brush, not only the aimed point |
| Sites fitted per Claude run | Any number, by params |
| Frame rate, tiny, GTX 1650 | ≥ 90 FPS |
| Determinism | Every `Delta`, fall and save the same on every machine, `-O` and `-O0` |
| The log | Every action logged; a log replays to its save bit for bit; tracing changes no hash |
| Following a change | Any object's history and any action's path shown by the inspector, from made data only |

## Risks

- **E is a large move** of working code. Do it verb by verb, with the showcase test written first
  and kept green after each verb moves.
- **Splitting arbitrary parts** (G) needs a representation beyond clip planes; baked `MeshSdf`s per
  part may grow saves. Keep a part's mask as carve edits where possible.
- **"Materials go down"** may make carried areas fail and need re-authoring by Claude.
- **`0.2.0`** is the first interface change since step 1; the browser host must follow it.
- **Bricks** add memory per chunk; bound the cache and evict by distance.

## Open questions

- Should promoted trees also tip when pushed by a giant, beyond falling when unsupported?
- Does water drain when a dam below its level is removed (today water only expands)?
- Do element parts (`part_of`) come from elements declaring them, or from connected components?
