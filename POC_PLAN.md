# Plateau: a plan for a proof of concept

The concept is `CONCEPT.md`. This plan builds the smallest thing that answers its two questions:

1. **Can Claude make an area, from words and a sketch, that is good at every size and keeps the
   laws?**
2. **Is moving between sizes worth it**: does being tiny or a giant show things worth finding?

Written 2026-11-21. About five to six weeks.

## In and out

In:

- One player, on this machine, the desktop.
- One area at a time, then two side by side (the last step, if there is time).
- Three sizes, changing at will.
- Claude making an area from a command line, with the making loop and the laws.
- Building a little: placing and taking away material, kept as edits.

Out, until the PoC has answered:

- Other players, a server, networking.
- Creatures.
- The gallery, signatures, publishing.
- The browser.
- Sound, menus, saving more than one world.

## Decisions for the PoC

| Decision | Choice | Why |
|---|---|---|
| Language | Rust | As Caprice; Claude writes Rust for themes already, and its compiler's strictness is free feedback |
| Engine | Bevy, or `wgpu` and `winit` by hand: chosen in step 0 | Bevy gives a camera, input and rendering in days; by hand is smaller and steadier. Step 0 tries both on the same mesh |
| Running areas | `wasmtime`, with fuel for instructions, a memory limit, no SIMD, no threads; NaNs left as the machine makes them, the law "NaNs unseen" in place of canonicalization | Limits kept by the engine, and the same results on every machine; canonicalization made areas over three times slower (step 0) |
| The interface | A WIT world, `plateau:region@0.1.0`, a core module, bindings made ahead of time and carried in the SDK | As Caprice (R36), so a browser host can come later without changing areas |
| What an area answers | `sample(points, size) -> (distance, material)` in batches; `describe() -> borders, places` once | The SDF idea (`CONCEPT.md`); batches because a call costs more than a sample |
| Meshing | Surface nets on the CPU, per chunk, per level of detail | Simple, no seams within a level; the levels' seams handled in step 2 |
| Detail per size | Near the viewer, a voxel about a twentieth of their height (tiny ~1 mm, person ~8 cm, giant ~85 cm), coarser further off | To be tuned in step 2 |
| Positions | 64-bit floats in the engine, 32-bit relative to the area within it | Precision over a range of a thousand |
| Compiling | The machine's rustc (the version pinned in the repo), `--target wasm32-unknown-unknown -O` | The tab's toolchain is for the browser, later |
| Claude | `claude-opus-5-5` at effort medium, as Caprice's theme writer (`web/api/theme.js`); the key from the environment | Known to write good theme programs |
| Tests | Never call Claude (stand-ins, as Caprice's R12); never run WebAssembly where the area's source can be played natively | As Caprice; fast and offline |
| Pictures | Ray-marched on the CPU from the area's own `sample`, headless | Needs no GPU, the same in tests, on any machine and for Claude |

## Steps

Each step ends with `cargo test` green and something to look at.

### 0. A spike (two or three days)

- A hand-written area (a slab of ground, a wall, a pond) built to a module, run in `wasmtime`.
- How long a million samples take, by batch size; with fuel on and off.
- Surface nets over one chunk; shown in Bevy and in `wgpu` by hand. Choose.
- The same samples' hash twice, and at `-O` and `-O0`: must agree.

**Done when** a chunk of the hand-written area is shown, the numbers are written down, and the
engine is chosen.

Done 2026-11-21: `spike/RESULTS.md`. Bevy chosen. Fuel costs 5%; NaN canonicalization over three
times, so it is off and a law stands in for it. A chunk near the player takes 18 ms on one thread for
the tiny, 7 for a person, 3 for a giant.

### 1. The interface and the SDK (three days)

- `plateau:region@0.1.0`: `sample`, `describe`, and `host` (the area's seed only, for now).
- `plateau-sdk`: distances and their sums (union, smooth union, cut away, round), noise, the
  engine's materials, `Seeds`; the bindings made ahead, no dependencies.
- The engine's check on load: only `host` imported, memory declared and at most the limit,
  WebAssembly 2.0 without SIMD or threads.
- The first area, written by hand: **a walled garden**. A garden wall with a crack, behind it
  a cave for the tiny; a pond; a giant's bench whose seat only a giant reaches.
- Areas played natively in the tests (as Caprice's `themes/played`).

**Done when** the walled garden is checked, run, and its samples are the same native and as a
module.

Done 2026-11-21. The world is `sdk/wit/region.wit`: an area is 64 m square, materials are an enum
of nine (soil, grass, sand, mud, stone, wood, leaf, moss, water), a place is a name, a size and a
point, an edge is 65 heights and a water level. The SDK (`sdk/`) carries the bindings made ahead and
checked against the WIT, `area!`, its own `sin`, `cos`, `atan2`, `min` and `max`, distances and
their sums on `f32` and on answers, `noise`, `detail` faded by the viewer, `scatter` and `Seeds`.
The engine (`engine/`) checks a module on load, runs it in wasmtime (compiled once, one instance a
thread), reads the canonical ABI itself with every pointer, length and number checked, and keeps
`nans_unseen`. The garden (`areas/garden/area.rs`, 35 KB as a module) answers bit for bit as its
source played natively at the three sizes and three in between (100 000 points, many of them at
the crack, the seat, the pads and the wall), and is built again byte for byte by the tests.
Pictures are ray-marched from the module (`engine/src/picture.rs`;
`cargo run --release -p plateau-engine --example look`, `areas/garden/pictures/`).

Found:

- **The tiny at the crack cost 808 ns a sample** (15 000 fuel), against 382 ns elsewhere for the
  tiny, 228 for a person and 125 for a giant. A chunk there would take about 29 ms on one thread,
  over the 20 ms target: step 2 needs the cores and the coarse pass that skips cells far from the
  surface (`spike/RESULTS.md`).
- **One answer a point hides the floor under water.** Inside the pond the answer is the water's
  surface, not the mud below it, so the engine finds the floor only as the place where the material
  turns from water to mud. Enough for step 2's "the tiny stand on water", maybe not for swimming.
  If it is not, `0.2.0` answers a second distance, to the nearest solid.
- **A bound must be answered only well away from it.** The lily pads' bounding slab, answered right
  up to its edge, made rays crawl along it and hit a sheet that was not there. The guide (step 4)
  should say so, and the laws (step 3) can catch it: a distance near zero where nothing is.
- **Hollows need shading to be seen.** Before ambient occlusion the crack and the cave were lit as
  brightly as the lawn. Claude's pictures (step 5) need it too.

### 2. Walking at three sizes (four days)

- The player: a capsule sized to their size, colliding with the area's distances asked at that
  size; walking, jumping, looking.
- Changing size at will, growing or shrinking over a second, the camera keeping the same point
  in view.
- Chunks around the player at the detail their size needs, coarser further away, made off the
  frame; the seams between levels hidden.
- Physics by size, first two: the tiny stand on water; a giant sinks into soft ground.

**Done when** one can walk the garden at each size at 60 frames a second on this machine, find
the cave as a tiny person, and sit on the bench as a giant.

Built 2026-11-21; checked by the tests and a scripted tour, not yet played by hand. The engine
(`engine/`, no window) gains `chunk` (surface nets over 32 cells a side, voxels a power of two
metres, each chunk asked for the viewer twenty voxels tall, one cell asked beyond its edges so
neighbours meet exactly, skirts two voxels deep against the seams between levels, a coarse pass
that never asks a block its middle's distance proves empty or full, and shading for hollows),
`rings` (the chunks a viewer sees: four a side on each level, from the power of two at or below a
twentieth of their height up to a metre, every point about them in one chunk) and `body` (a
capsule as tall as the size, proportions, pace and gravity scaled with it; feet held up by rays
down, spheres above the step pushed out of what rays find; the tiny stand on water, a giant sinks
up to 6% of its height into mud; growing stops where there is no room; sitting on a surface at knee
height). The window is `play/` (Bevy): the chunks made by ten threads, each with its own instance,
finest and nearest first; the world drawn about the eye and scaled so the player is always 1.7 m
tall to the renderer. `cargo run --release -p plateau-play`; `-- --tour <folder>` visits nine
places, measures and takes screenshots (`play/tour/`, with its numbers in `tour.txt`).

Shown by `engine/tests/walking.rs` (the garden played natively): a person comes in at the gate and
is stopped by the wall; a giant steps over it; the tiny walk in at the crack to the cave, where a
person cannot go; the tiny cannot grow in the cave beyond its roof, nor a person under the bench
into a giant; a giant sits on the bench and a person finds no seat; the tiny stand on the pond and a
person wades to its floor and out; a giant stands a metre deeper in the mud than a person; every
size jumps its proportion and lands. And by `engine/tests/chunks.rs`: the coarse pass makes the
same surface asking under half as much, neighbours meet edge for edge, the levels fill the space
about any viewer once.

The tour (1600 × 900, the GTX 1650), uncapped (under Hyprland, vsync is sometimes kept anyway,
and the same run gives 60):

| Where | Frames a second | Worst frame | Triangles | Chunks made, and how long until all are |
|---|---|---|---|---|
| A person at the start | 190 | 6.6 ms | 103k | 156 in 0.24 s |
| A person under the bench | 175 | 6.7 ms | 210k | 174 in 0.20 s |
| A person shrinking to tiny at the crack | 114 | 20.8 ms | 473k | 552 in 1.0 s (during the second of shrinking) |
| The tiny at the crack | 113 | 10.0 ms | 515k | 208 in 0.30 s |
| The tiny in the cave | 112 | 9.6 ms | 535k | 312 in 0.43 s |
| The tiny on the pond | 153 | 15.9 ms | 351k | 538 in 0.30 s |
| A giant on the bench | 256 | 4.6 ms | 58k | |

A chunk takes 3 ms for a person and 13 for the tiny on average, but the slowest at the crack take
up to 180 ms: under the 20 ms target on average, not at worst, and hidden by making them off the
frame and keeping the old chunks shown until the new are all made. A step of the body costs 0.06
ms at most.

Found:

- **A distance is not a contact.** Wherever an area cuts one shape out of another, its distances
  are only lower bounds, even in open air: in the gate, the wall said its nearest surface was the
  gate's own outline, 15 cm away, and a person could not pass. The body never takes a short
  distance for a touch; rays stepped by the distances find the surface itself (26 from each
  sphere, out to its radius).
- **The garden's distances were too long**, against the SDK's first rule: by up to 1.9 times at
  the moss's tufts, 1.24 in the bench's grain, 1.13 on the wall's rough stone. The coarse pass
  showed it (with a margin of a cell's diagonal it changed the surface by the crack), and the
  physics would have walked into it. Scaled down; scaling a distance never moves a surface. The
  coarse pass keeps a margin of two cells for areas that slip. **Step 3 needs the law "distances
  never too long"**: near every surface, the distance grows no faster than the way to it.
- **Thin things break up, and touching pieces are hollow.** A giant's view is drawn in voxels of
  half a metre near it and a metre further off, so the bench's 35 cm planks fell apart; and planks
  that only touch have distance nought inside, where they meet: a grid on whole metres asked only
  there and saw no seat at all. The planks are now 90 cm thick, their 4 cm gaps closed (overlapping)
  for whoever is too large to see them. The guide (step 5) must say: a thing a size should see is
  at least a tenth of its height thick, detail it should not see is closed up, and pieces that are
  one thing overlap.
- **Shadows and the near plane are made for a person.** Bevy's cascades round their width up to
  whole metres and its shadow bias is in metres, which would make the tiny's shadows coarse and
  loose. Drawing the world scaled to the player keeps them right at every size. Haze is the
  air's, in metres, so it is set against the scale (the tiny saw haze 70 cm away before).
- **Played by hand, it flickered while moving** (2026-11-21, after the first commit): old chunks
  stayed shown until every new one anywhere was made, so while moving, old and new overlapped and
  then the old vanished all at once. Now what is shown is chosen each frame from what is made
  (`rings::shown`): a chunk not yet made is stood in for by the coarser chunk about it or the
  finer ones within it, never overlapping; blocks lag a little behind the viewer (`rings::Rings`),
  so walking to and fro does not swap chunks; and the chunks where the player is going, and at
  the size they are changing to, are made ahead. The tour's walks (in at the gate, the tiny along
  the wall, a giant across the garden) have no frame with a hole. Left: chunks still change from
  coarse to fine at once, without blending; and the worst frame while the tiny run is 20 to 45 ms
  (the choosing and placing of chunks takes up to 7 ms of it, the rest is Bevy's), a dropped frame
  now and then under vsync.
- **One answer a point hides the floor under water** (step 1) cost little: rays through water a
  body does not float on step a twentieth of its height at a time and stop where the answers turn
  solid. A person wades the pond, a giant sinks in its mud.

### 3. The laws, first set (three days)

In `cargo test`, over the walled garden and every area in `areas/`:

- **Sizes agree**: at many random points, the distance asked for a size, against the one asked for
  the next size up: within a tolerance of the larger size's detail.
- **Gates are real**: for each size, what it can reach from the area's start (flood fill over the
  voxels at that size, with how high it can climb and how far it can step); each place the area
  describes is reached by the sizes it names and by no other.
- **No traps**: from every voxel a size can reach, it can get back to the start.
- **The same everywhere**: the hashes of a fixed set of chunks, kept in the repo; CI fails if they
  change unasked.
- **NaNs unseen**: the same answers with NaN canonicalization on and off (step 0's
  `nans_unseen`), since areas run without it.
- **Distances never too long** (found in step 2): near every surface, at every size, an area's
  distance grows no faster than the way to that surface, so the engine can step as far as it says.

Seeded, with the smallest failing case printed (as `LAWS_PLAN.md`).

**Done when** the walled garden keeps all four, and an area made wrong on purpose (a crack wide
enough for a person) breaks "gates are real" with a clear message.

### 4. Pictures (two days)

- Ray-marched pictures at fixed views for each size: from above, from the start, at each place it
  describes, shaded by material, with light from one side.
- **Where each size can reach**: the map from above, each size's reach in its colour; the "long
  exposure" of Plateau.

**Done when** the pictures of the walled garden show the crack, the cave and the bench, and its
reach map shows the cave as the tiny's alone.

### 5. Claude makes an area (four or five days)

- `plateau make "<words>" [--sketch map.png]`: Claude writes the area against the SDK, given the
  guide (written in this step: the SDK, the sizes, the laws, how finished an area should be, the
  walled garden as the example).
- The loop: the compiler's words back to Claude (three rounds at most); the check and the laws
  back to Claude with the smallest failing case; the pictures and the reach map shown to Claude
  once, to make it better; the last version that worked kept.
- **The test set**: five requests, kept, run again whenever the guide or the SDK changes:
  1. "A giant's kitchen table with a village of tiny people living under it."
  2. "A headland ending in a cliff, a lighthouse on it, and smugglers' tunnels only the tiny fit."
  3. "A pond with lily pads the tiny use as stepping stones, and a heron's nest only giants can
     reach."
  4. "A forest where the trees are houses for the tiny and stools for giants."
  5. "A ruined castle whose walls are hills to the tiny and whose keep a giant can look into."
- For each: whether it keeps every law in the rounds allowed, how many rounds, what it cost, how
  long it took. Written in this file.

**Done when** at least three of the five keep every law and are worth walking at each size.

### 6. The sketch (two or three days)

- A map from above, drawn in any paint program with the pens of `CONCEPT.md` (a palette file in
  the repo to draw with), read by the engine: its areas, arrows and words.
- Claude is given the sketch as a picture and as shapes (outlines in the area's metres).
- Two laws from it: the materials lie where the sketch has them (water where it is blue, within a
  tolerance); the orange and purple places are reached only by the tiny and only by giants.

**Done when** requests 2 and 3, sketched, keep their sketch's laws, and come out closer to what was
meant than without one.

### 7. Building (three days)

- Placing and taking away material at the player's size (a block as big as the size's detail),
  kept as edits laid over the area's world, saved to a file.
- **Made again equals played**: the area made again from its seed and its saved edits gives the
  same chunks.
- A tiny person's building is tiny; a giant's is huge; both are in the same world.

**Done when** a tiny person can build a room inside the cave and a giant can see it as a speck.

### 8. Two areas side by side (three or four days, if there is time)

- Two areas made one after the other, the second told the first's border.
- **Borders match**: ground, water and material continuous along the shared edge, within a tolerance.
- Walking from one to the other at every size.

**Done when** two of the test set's areas lie side by side with no seam a giant or a tiny person
notices.

### 9. What the PoC found (two days)

- An hour's play at each size in the best areas; what was worth finding, what was not.
- The two questions answered, with the numbers of step 5.
- What the next stage needs: networking, creatures, the gallery, the browser.

## Measures

| Measure | Target |
|---|---|
| Areas of the test set that keep every law | 3 of 5 or more |
| Rounds of Claude per area | at most 3 fixes and 1 look |
| Frames a second, at each size, on this machine | 60 |
| A chunk made, near the player | under 20 ms |
| Hashes the same on two runs and two optimization levels | always |
| Moving between sizes worth it | judged in step 9 |

## Risks

- **Claude's sense of space in three dimensions** may be weaker than in Caprice's flat desk.
  The pictures, the reach map and the sketch are there for it; step 5 measures it.
- **Detail for the tiny may look like noise.** The SDK needs good pieces for small things (moss,
  pebbles, grain of wood), and the guide needs examples.
- **Sampling in WebAssembly may be slow.** Batches, fewer samples far off, chunks made off the
  frame; step 0 measures it first.
- **Seams between levels of detail** are a known hard problem for surface nets: skirts or
  stitching, decided in step 2.
- **Bevy changes often.** The PoC pins its version and does not chase new ones.
- **Scope.** Everything under "Out" stays out until step 9.

## Open questions

- The sizes' ratios (a thousand from tiny to giant, or less).
- Changing size anywhere, or at places in the world. The PoC allows it anywhere, to try.
- First person or from behind. The PoC starts first person.
- The look that holds at every size.
