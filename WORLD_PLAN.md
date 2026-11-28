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

Short of the done-when: nothing. Relations go stale when the world changes under them without the
element moving (terrain carved from under a log, water rising round it): G's support check is what
should mend them; until then the laws say so.

*Slice 2 done 2026-11-26 (headless tests only; not yet played by hand):*

- New nodes: the terrain's edits (`edit:4`, by place in the edit log) and water bodies
  (`water:2`). New relations: `attached_to` and `fills` (with the `Opening` it was measured at:
  target, normal, size, enough to measure it again in I), `carved_into`, `flooded_by`.
- `Action::Place` takes `opening` and `seed` (both optional, so older logs still read). A fitted
  place `fills` and is `attached_to` its host (`World::host_at`: the terrain or SDF element solid
  just behind the opening's wall, else the scene element whose mesh it is), asked before the
  element is there. Being attached holds an element up (`none_unsupported`). `play`'s fitted snap
  carries its opening; **`make --place` now places through `World::apply`** (owned by `local`, the
  seed it was judged with, its fill and rests; `--accept-changed-modules` passes through).
- A terrain carve is `carved_into` the terrain if there was any within it and every SDF element it
  reaches inside, with those it promotes; a carve that floods writes `flooded_by` for every basin
  newly filled (a dry carve it joins too). Undo reverts them with the water.
- Leaving or moving: an element takes with it what rested on, was attached to or filled it; set
  down elsewhere, it no longer fills or hangs from what it did. History stays (`derived_from`,
  `carved_into` to elements gone).
- **A site's neighbourhood** (`live/near.rs`, `World::neighbourhood`): what the site is in
  (with its owner, its lineage of cuts, how it was carved, what holds it up), whether an opening
  there is filled already and by what, the carves round about (how many, at what size, into what),
  water let in through them, and up to eight elements near by, in the site's local metres.
  `SiteContext::near`, saved as `near` lines in `.site` files and given to Claude in
  `describe_for_prompt` ("Round about, in the world"); filled by `play`'s `I`.
- Laws: `relations_are_whole` (every end there, or history; whatever fills is attached).
- Tests: `engine/tests/graph.rs` (8, two new): a hobbit door placed into a tiny's home bored in an
  oak fills it and hangs from the oak, the site naming the oak, its owner and carving, and once the
  door is in, the door and what it fills; the site file keeps its neighbourhood; taken off and
  undone; saved and reopened. A terrain carve aimed at a tree is carved into the tree, a cave into
  the terrain; the hand-played channel floods and is `flooded_by` its dig, undone and redone
  exactly; a site at its edge names the carves and the water; saved and reopened.

Not yet: element parts (`part_of`); re-measuring an opening when an edit changes it (I).

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

*Slice 1 done 2026-11-27 (headless tests only; not yet played by hand):*

- **`engine/src/support.rs`**, the arithmetic alone: a grid of cells over an element's own box
  (48 along its longest side, at most 48³), its parts joined face to face, numbered in order; a
  plane between a part and the rest (the world's up first, then the element's axes and the twelve
  between them, half way across the widest gap); and `overhang`, whether a centre of mass stands
  over the points that hold it, asked along sixteen directions, with a grip (a share of the
  section's width it may overhang by: `SECTION_GRIP` 0.25 for a cross-section, none for a body on
  its contacts).
- **`engine/src/live/falls.rs`**, after every action but undo, redo and paddling
  (`World::settle_support`), in at most `MAX_ROUNDS` rounds:
  - **mended**: each loose element the action may have shaken (the terrain carved or raised round
    it, what it rested on moved, cut or taken away) rests on what it now touches;
  - **come apart**: an element carved (a terrain stroke at least half a cell, or a stroke into the
    element) or cut is made into cells from the SDF the world has of it (its meshes clipped, its
    own carving, the terrain's carves since it was promoted, sampled in blocks of four so clear
    space is not asked); a part not joined to what holds it up (its roots in the ground, what it
    rests on or hangs from) comes away along a plane between them, as a cut makes pieces: the
    original retired, two new pieces `derived_from` it, owned by its owner, the rest keeping what it
    hung from, what rested on, hung from or filled it found again;
  - **broken**: where it still holds together, at up to twelve heights each carve reached, the
    cross-section there is sampled 24 by 24 and asked whether it holds the cells above it; a notch
    half through holds, about three-fifths and deeper breaks there along a level plane, the part
    above tipping first over the hinge the section left;
  - **fallen**: every loose element nothing reaching the ground holds up (`World::grounded`: resting
    on or attached to the terrain or to something grounded; two pieces resting only on each other
    are not), its rests found again first, and every piece unsteady or pushed, lowest first: it drops
    straight down until a point meets something, then tips over the edge its centre of mass
    overhangs (or the way a chop pushed it), turning about that edge with steps no larger than the
    nearest thing allows, until something going down meets it, up to a quarter turn at a time, up
    to eight times; then water settles it. It meets the terrain without water and every other
    element (scene solids by their meshes, SDF ones by their fields). A falling body is its surface
    cells, each a ball as deep as it lies inside; a carved field, which says less than the distance
    near the carve's lip, is only met where a point moved on a little is in it. A promoted piece
    that falls takes the terrain's carves into it along as its own. What rested on it is asked
    next round.
  - At most `World::fall_limit` falls an action (`FALL_LIMIT` = 24); what is left falls with the
    next action. Each split and fall is in the action's undo (`WorldAction::Fell`, and
    `WorldAction::Many` for the several changes one action makes) and its outcome
    (`Summary::fell`, "…; Oak Tree fell").
- **A chop is a cut plus the support check**: both pieces stay where the tree stood; the top is
  pushed over the way the player faces and falls as anything falls (onto the lawn, the stump's edge,
  a bridge), not through other things; the scripted 90° topple and its ground probe are gone (the
  free `cut::toppled_transform` is kept for its own test). A lying log's pieces are still nudged
  apart.
- **A fall is shown, not kept**: the element is where it comes to rest at once (as saves and later
  actions see it); `FallingElement` (with the edge it tipped over, if it did) eases it from where
  it was, and `tick_falling` and `finish_all_falls` move only what is shown.
- **Fixed on the way** (phase F): an element asked alone (contacts, hosts, carves) is asked with the
  terrain's carves made since it entered the SDF, so a stump carved through is carved; a field is
  touched from above where a point let down by the tolerance is in it; a carved element's contacts
  come from its cells, not its meshes (which do not know its carving); placed elements are kept in
  order of id, so undoing a removal saves as before.
- **Uprooted** (played by hand the same day: bowls dug round an oak's foot carved its base and roots
  away, as a terrain carve cuts what it reaches, yet it stood on, rooted by its meshes, and nothing
  carved into it after could fell it): an element rooted by its shape whose cells no longer touch
  the terrain (checked when it is carved or cut, or the terrain near it is carved) is
  `uprooted` (a relation to the terrain, saved and undone like any other), so it is loose and
  falls; pieces cut or broken from it are uprooted too. A part held up stays with the element
  however many parts are held (a root the cells see apart from its trunk no longer comes away as a
  second stump), and a part comes away along a level plane whenever one parts it, so leaves go
  with the top. `a_tree_whose_foot_is_dug_away_falls_and_stays_uprooted`, from the session's bowls.
- Laws: `none_unsupported` now asks `grounded` (and says when something rests only on what does not
  reach the ground); `rests_are_contacts` ends a fall still shown first.
- Tests: `engine/tests/support.rs` (9, the ninth from the hand-played session above): a ball carved through a trunk drops the canopy 0.76 m onto
  its carved stump, undone and redone to the same save; a notch half through stands, four-fifths
  breaks and the top falls east, towards the notch, onto the lawn; a chopped tree lands on a bridge
  beside it, held up by the deck; a bridge removed drops the log lying on it, undone and redone to
  the same save; a pit dug under a log lets it fall to the pit's floor; a lying log cut in two lies on
  the lawn, and written as two pieces resting on each other alone, the law says so and the next
  action mends it; three pillars on a bridge with a limit of two fall two, then one; and a session
  of falls replays bit for bit. `graph.rs`: a chopped trunk rests on the lawn and its stump's edge;
  a log on a removed bridge falls (no longer "loose until G").

*Slice 2 done 2026-11-27 (headless tests only; not yet played by hand), and with it phase G:*

- **Component mask for non-planar splits** (`support::mask_boxes`, `live::falls`, `graph`, `cut`):
  where no single `separating_plane` parts two disconnected components (a ring cut around an island
  in a bridge deck, an interlocking mortise-and-tenon cut), `mask_boxes` greedily covers the other
  part's cells (merging empty cells not 26-adjacent to the kept part into large slabs via a 3D
  prefix sum, capped at `MAX_MASK_BOXES = 128`) as element-local rounded `Op::Carve` boxes (`pad =
  0.30 * cell`, `round = pad`, `blend = 0`) appended to each piece's `local_edits` (`el-sdf` in
  `plateau-world 2`). Both pieces are promoted and linked by `rel derived_from <piece> <orig> split`
  (`graph::Data::Split`); `cuts_are_derived` and `live::near` (`Graph::parent_of`) walk both planar
  and masked splits; `cut::cull_non_solid_by_mask` (clamped to the solid AABB) keeps handrails,
  leaves and lights only on the piece they belong to; `World::grab` clears `phys.rooted` on
  `uprooted` pieces; and `show_falls` skips flashing the uncarved parent mesh for masked splits.
  Persists cleanly in `plateau-world 2`, round-trips through save/reload, and undoes/redoes
  bit-exactly via `WorldAction::CutElement`.
- **Cross-build and native-vs-module determinism** (`engine/tests/support.rs`):
  `falls_and_hashes_match_natively_as_modules_and_across_o_and_o0` compiles `garden`, `tree`,
  `bridge` and `pillar` at `-O0` alongside the carried `-O` `.wasm` modules and their native Rust
  sources (`areas/played` and `elements/*/element.rs`), runs the full Phase G fall suite (carved
  trunk drop, deep notch break & tip, felled tree on bridge + bridge removal, ring-cut bridge
  component-mask split), and verifies bit-identical saves, action logs, SDF sample hashes, and
  chunk hashes across `-O` modules, `-O0` modules, `canonical_nans: true`, native execution, and
  kept cross-build golden digests (verified with `plateau-engine` itself built in `dev`,
  `--release` (`-O`), and `-C opt-level=0` (`-O0`)).
- **Tests**: `engine/tests/support.rs` (12, three new) and `support` unit test (3, one new):
  `a_ring_cut_island_in_a_bridge_deck_splits_by_component_mask_and_drops_through`,
  `an_interlocking_mortise_and_tenon_cut_splits_by_mask_and_survives_a_further_cut`, and
  `falls_and_hashes_match_natively_as_modules_and_across_o_and_o0`.

Deliberately scoped (not blocking G's done-when):
- Strokes smaller than half a cell (`worth_asking`: `radius < 0.5 * world_span / CELLS`, e.g. a
  tiny tunnelling into a person-size oak) do not re-voxelise the element on each stroke because a
  single sub-half-cell gap cannot clear a cell centre in the 48-cell grid; any larger stroke or
  cross-section cut checks it.
- Deliberate `Drop` / `Place` where an object touches a support keeps the player's aimed placement
  without auto-rolling (so laid log bridges stay flat as aimed); loose objects placed or dropped in
  mid-air (touching nothing) fall and balance automatically via `settle_support`, and anything
  carried is left until set down (as in F).
- Falling bodies use their 48-cell surface balls (~13 cm for a person-size oak) while cross-section
  breaks sample the exact SDF on a 24×24 slice — fast, deterministic, and within G's 32–64 cell
  spec.

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

*Done 2026-11-27 (headless tests only; not yet played by hand; masked carve remains for J):*

- **Materials below the surface**: `plateau_sdk::strata(depth, &[(Material, thickness)])`
  (deterministic SDK math only, documented in `make/GUIDE.md`); new law `laws::materials_go_down`
  (16×16 downward grid + seeded rays probing 30 cm / 60 cm / 1.2 m along the inward normal, refusing
  thin surface coats `Grass`, `Moss`, `Leaf` deep inside solid, wired into `engine/tests/laws.rs`
  and `make::judge_with_neighbors`). `areas/garden/area.rs` (lawn `Grass → Soil → Stone`, pond shore
  `Mud → Soil → Stone`, mossy wall `Moss 3 cm → Stone`) and `elements/hill/element.rs` updated,
  rebuilt (`./areas/build.sh`, `./elements/build.sh`), and `areas/garden/hashes.txt` updated.
- **Whole-brush tool check** (`physics::check_carve_brush`, `physics::Tool::can_add`): samples a
  `5×5×5` cell-centered grid inside the brush at `edit.stamp_size` against `Region` + exact
  `MeshSdf`s of overlapping unpromoted scene elements. Any disallowed solid material refuses the
  stroke or shrinks the brush toward `hit` (`[0.75, 0.50, 0.35, 0.25]`) until it fits, naming the
  blocking material; `Op::Add` is gated by `Tool::can_add`.
- **Edits record their tool** (`edit::Edit::tool`, `Tool::save_name` / `from_name`): persisted as
  `tool <name>` in `plateau-world 2` (backward-compatible with v1/older saves omitting it, which
  default to `Tool::All`). `live::verify_save_edits` replays every `sdf` and non-mask `el-sdf` edit
  chronologically against its base area(s) and placed elements to check every edit's recorded tool
  and mode rules.
- **World vs workshop mode & material economy** (`physics::WorldMode`, `physics::Inventory`,
  `Action::SetMode`): persisted as `mode world` and `inv <player> <mat> <kg>` in `plateau-world 2`
  (legacy saves default to `Workshop`). In `world` mode: default tool is `Hands`; `Tool::All` is
  refused; carves may not repaint walls (`material: Some(_)` disallowed); carving yields material
  (volume × density) into the player's `Inventory`; `Add` consumes carried material; carrying weight
  (`cut::check_carry_with_load`, `World::carry_speed_factor`, `World::can_still_hold`, boat rider
  load) counts the pack; undo/redo reverts and reapplies inventory deltas bit-exactly (`Done::inventory`).
- **`play/`**: defaults fresh sessions to `world` mode (`--mode world|workshop`, `--workshop`, `U`
  toggles via `Action::SetMode`, `N` cycles `world.next_tool(...)`), shows mode, tool, pack weight &
  selected material stock in HUD, and delegates all carry-weight/tool checks to `World`.
- **Tests**: `engine/tests/tools.rs` (7): `strata` unit/determinism; every carried area keeps
  `materials_go_down`; shovel aimed at moss over stone core refused at person scale and shrunk at
  tiny scale (plus lawn over bedrock and `MossyHill`); saw refused raising stone in `world` mode,
  wall repainting refused in `world` mode & allowed in `workshop` mode, dig-to-build consumption and
  undo/redo; carrying weight counting carried material pack (slowing pace and blocking heavy log
  grab); save & action-log replay verifying every edit's tool (catching 5 tampered cases) and
  canonical-NaN determinism; legacy v1 save backward compatibility. `engine/tests/laws.rs` adds
  `grass_thirty_centimetres_underground_breaks_materials_go_down`.

*Review fixes 2026-11-27:*

- **A stroke is checked against only what it changes** (`physics::BrushTarget`): a terrain stroke
  against the SDF layer plus the exact solids of unpromoted elements it reaches; a stroke on an
  element against that element alone, in its own frame (`World::element_brush`), so the ground
  under a trunk neither stops a saw nor goes into the pack. Only cells inside a solid (signed
  distance `<= 0`) count; the base areas' own scene meshes are never carved and no longer count.
- **`Add` pays for what it turns to its material**, not only for open cells (`estimate_add_mass_kg`
  follows `Edit::apply`), so repainting ground with an add is no longer free; adds are tool-gated
  only in `world` mode.
- **Inventory journal records what was applied** (a debit clamps at empty), so undo gives back no
  more than was taken.
- **`SetMode` needs the world's owner** (`RuleBook::check_world`: owner of every claimed area).
- **Each edit records its mode** (`Edit::mode`, ` mode world` in `plateau-world 2`, absent =
  workshop): `verify_save_edits` checks each edit under the mode it was made in; terrain edits
  against the base areas, SDF landforms and earlier edits; element edits in the element's own
  frame against it and its earlier edits (wherever it was carried since); mask boxes recognised on
  every piece descended from a split.
- **`World::carry_physics`** (uprooted means not rooted) is shared by `grab` and `can_still_hold`.
- **Made areas held to "materials go down"**: the areas the engine tests walk (`pond-sketched`,
  `forest-beside-pond`) layer their coats by depth (`deepen`, by hand, shapes unchanged); the
  others, made before the law, were removed (`plateau test-set` makes the five again).

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

*Done 2026-11-28:*

- **`plateau:region@0.2.0`** (`sdk/wit/region.wit`): `host.params() -> list<f32>`. The engine writes
  the list into memory the module allocates (`wasm::give_params`). `check` accepts imports from
  either host version, refuses a module that asks both, and `check::reads_params` tells whether a
  module asks for its params at all.
  - Params are checked: at most 64 (`wasm::MOST_PARAMS`), each finite.
  - They are given per placement (`PlacedElement::params`, `Action::Place { params }`) and saved
    as ` params N v…`.
  - Every cache of an element's instance, mesh SDF and bounds is keyed by its params
    (`CompiledElement::sdf_bounds_for`).
  - SDK: `params()`, `param`, `native::with_params`. The carried areas and elements are rebuilt;
    the made ones stay `0.1.0` and still load.
- **Sites become params** (`SiteContext::params`, `site::P_*`): 58 numbers.
  - Width, height, middle, depth, through, wall, roundness, headroom and walkable, then all 48
    outline radii.
  - This departs from the planned 8 spokes: a square doorway's corners fall between 8 (or 16)
    spokes, and its frame then leaves a gap there.
  - `ELEMENT_GUIDE.md` ("Params") gives the table, and a test holds it to the constants. The
    site's prompt says the numbers are the element's `params()`.
- **The hobbit door makes itself for any opening** from its params: frame, leaf, trim and stones
  follow the 48 radii. Given none, it is the round door as before.
  - `fit::fit_to_site` first places an element where the site is, given the site's params. If that
    fits, it is placed so; otherwise it falls back to `fit_transform`, given no params.
  - Play's `O` and `make --place` give the params.
- **Fitting again** (`live/refit.rs`): after a terrain sculpt, each element that `fills` an opening
  the stroke reached is re-checked. The opening is measured again without the element, in both the
  scene and the SDF.
  - An element placed with params is given the new site's params where the new site is, and is
    made again whole. Any carve the stroke promoted it for goes with the old opening.
  - The refit is undone and redone with the stroke (`WorldAction::Refit`).
  - Anything that then fails `cavity_fit` is told of in the outcome, for example a door fitted by
    scale alone in an opening that outgrew it.
- **More fit laws** (`engine/src/fit.rs`):
  - `span_fit`: both ends rest on the banks, there is a gap under the middle (lower than the banks,
    or water), nothing rises through the deck, and each size walks across. The deck's width is
    checked first.
  - `wall_fit`: the back meets the wall over at least ¾ of it, neither standing off nor sunk in.
    It comes with a carried `sign` element.
  - `style_fit`: measures materials only (a site measures no colours), and is a note for the
    `look` round, never a failure.
  - `plateau --fit` gains `span` and `wall`.
- **`check-element` and `make-element` judge over a range**: an element that reads params is
  held, given their params, to four openings beside its site's (`OTHER_OPENINGS`: 15% wider,
  narrower, higher, lower; `SiteContext::varied`). One that reads none is noted to fit its own
  opening alone. Notes (`Judged::notes`) go into the `look` round.
- Tests: `engine/tests/params.rs`, `fit_params.rs`, `span.rs`, `wall.rs`, and a param-range test
  in `make/tests/make.rs`.
  - One door fits a round tunnel, a square doorway and a tiny's low arch by params alone. It fits
    each again once widened, and the old params no longer fit the widened one.
  - The bridge spans a person's ditch and a tiny's.

*Still open in I:*

- Refitting follows terrain sculpts only, not element sculpts or falls.
- A cut element (with clips) is not made again.
- Each stroke near a filled opening costs a measure and a `cavity_fit`.
- The bridge takes no params: a ground site measures no gap to span.
- Play's placement ghost still shows the element as it is given no params.
- No browser host is in this repo; when one exists it must give `params` as `wasm.rs` does.

*Played by hand 2026-11-28 (phases D–I; the session's log replayed headless to find causes):*

- **Fixed: far lawns showed soil, walls blotches of stone** (since H's strata). A chunk vertex took
  the material of the solid corner nearest it, up to a voxel deep, under coats thinner than a
  coarse voxel (grass 8 cm, moss 3 cm). `chunk::surface_materials` asks the material just beneath
  the surface where the vertex lies; flat lawn now shows no soil at any level to 50 cm voxels
  (23–59% before).
- **Fixed: a shovel cut a timber bridge**: the whole-brush grid (5³, 22 cm apart for a person)
  missed its deck, and the stroke promoted and cut it. `physics::thin_blocked` asks each unpromoted
  element's exact distance on a 9³ grid with half a cell's slack. Test:
  `a_shovel_never_cuts_a_thin_wooden_deck_it_reaches`, with the played bridge and stroke.
- **Fixed: `F` at a fitted door worked only sometimes**: what `F` targets was cast from the body's
  eyes, not the crosshair's ray from behind (`Player::pivot` now serves both); a scene object
  behind the SDF layer (an oak's uncarved triangles) is not targeted; and if what is targeted does
  nothing with `F`, what is near is asked (`ElementRuntime::interact`).
- **Fixed: what was set down was never asked how it stands**, and what had roots by its shape stayed
  rooted wherever it was carried (a half stump stood tilted on a root). `Drop` now uproots what it
  carried and puts it in `pending.unsteady`; a trunk laid with a bough down now tips onto its end.
  Test: `what_is_set_down_tilted_with_roots_is_uprooted_and_tips_over`.
- **Fixed: the graphics card ran out of memory** after a carved bridge was cut a dozen times
  (4.3 M triangles, 3.8 M of them elements). `carved::keep_outside` divided every original
  triangle near a stroke down to a cell (1 mm on a small piece), so 216 plank triangles became
  538 k, and cut pieces up to 1.2 M. It now divides only triangles the kept part's edge may cross
  (a distance bound at the middle, and the reach's ball about the triangle). The played session
  replays to 358 k in all (from 4.5 M). Test: `a_bridge_carved_many_times_stays_light` (500 k
  before, 117 k now; the rest is the remade surface's 1.6 cm grid, for J).
- **Fixed: a pickaxe on a bridge's stone footing sliced the planks beside it**: the same thin-solid
  check now also asks the element being sculpted (its solid as carved so far).
- **Explained, open**: a felled top rose 1.36 m when the bridge it rested on was removed: buoyancy
  (it lay half under water, held only by resting on the bridge). It looked wrong because the
  canopy is not solid: it weighs nothing and touches nothing, so crowns hang in the air and pass
  through stumps. Whether the canopy counts for mass and resting (staying walk-through) is to decide.
- **Decided: the economy is off unless a world switches it on** (`economy on` in the save's rule
  book, `RuleBook::economy`, `World::counts_material`). Without it, `world` mode keeps the tools'
  rules, but digging removes material and building makes it; nothing is carried or weighs. Test:
  `without_the_economy_material_is_not_counted_but_tools_still_are`. What follows is why.
- **The pack (with the economy on).** In `world` mode everything dug is carried (4.6 t after a while: speed at its
  35% floor, every grab refused). A cap does not work: one person's stroke digs about as much as a
  person carries (≈1 t; a tiny's 2.3 g against 1.4 g), so brush size, yield and capacity must be
  balanced together, or the pack made weightless, or the excess left as a heap.

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

#### Bounded cost under heavy play (from the session played by hand 2026-11-28)

Players will carve, cut and pile things up all the time. **The rule: what an action costs follows
what it touches, never how much happened before it** (how many edits a thing has had, how many
pieces lie near it, how fine the smallest stroke ever was). The played session (a bridge dug round
with 378 strokes, then cut a dozen times) broke it in four places; measured by replaying its log
under tracing spans (`carve*`, `fall`, `tip`, `obstacle_distances`, `entry_distances`, …):

| Where | Grew with | Before | Done 2026-11-28 | After |
|---|---|---|---|---|
| Drawing a carved element | triangles near any edit, divided to a cell | 4.5 M triangles, GPU out of memory | divide only where the kept edge crosses (`keep_outside`) | 358 k |
| A stroke beside a carved element | its edit count; the finest stroke ever made | 0.2–2.9 s (309 bricks) | bricks asked of the edits that reach them; grid bound to the maker's size (`CELLS_PER_STAMP`) | 0.1–0.3 s (18–37 bricks) |
| A cut | the piece's whole carve history, made again | up to 3.2 s | grid no finer than a piece's length ÷ 24 | up to 1.2 s |
| Settling after a cut | every nearby piece, at every tipping step | up to 1.8 s | pieces whose bounds are no nearer than what was met are not asked (exact: the session replays identically) | up to 1.0 s |

Still to do, in order:

1. **Drawing never holds up an action.** A carved element's mesh is made on a worker, as chunks are,
   and swapped in when ready (the old one shown meanwhile). It is not part of the world's state, so
   determinism is untouched; strokes and cuts then cost their physics alone.
2. **A cut reuses its parent's bricks.** A piece is carved from its parent's source with its clip
   planes as part of the field, so only bricks a plane crosses are made again: a cut costs its cut
   face, not its history. Pieces keep only the edits that reach them.
3. **Edits are compacted.** Past a number of edits, an element's (and a region's) edits are baked into
   sparse bricks of distance and material (the bricks above), so a sample costs the same after
   1,000 strokes as after 10. Saves keep the edits; bricks are a cache made again from them.
4. **Settling has a budget.** Tipping advances by the clearance a broad phase finds (fewer, larger
   steps), and obstacle queries go through a BVH over elements, so cost follows contacts, not pile
   size; `FALL_LIMIT` stays as the hard stop.
5. **Elements have levels of detail.** A carved element is drawn for the size class that sees it
   (tiny, person, giant), chosen by distance like chunks, within a triangle budget per element.
6. **Budgets are laws.** A heavy-play corpus (sessions like this one, kept as logs and replayed) holds
   every action under a time budget and every element under a triangle budget, so growth is caught
   by tests, not by playing.

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
