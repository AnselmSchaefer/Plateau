# Plateau: a concept

An open world building game where tiny people, people and giants share one world, and where
each area of it is a program Claude writes from a few words and a sketch, run in a sandbox. Some
places can only be reached when you are tiny, some only when you are a giant, and many areas,
made by many people, lie side by side and are built on together.

Written 2026-11-21, from what was learned making Caprice's theme programs (see "What Plateau takes
from Caprice" at the end).

## Why programs

A world whose detail runs from a giant's view down to a tiny person's cannot be stored: at a
thousand times the range, every square metre a giant crosses in a step holds a landscape for the
tiny. It can be *described*: by a program that makes, for whoever looks, the detail their size
sees. A few words become a program, and the program becomes a world at every scale. That is the
same idea as a Caprice theme (short request, program, endless output), in three dimensions and
across sizes, which is where it pays most.

Programs also give what images or stored models do not: they can be checked (they compile, pass
the engine's checks, keep its laws), played again exactly (the same module and seed give the same
world on every machine), and shared with strangers, because they run in a sandbox that can do
nothing but answer the engine.

## The world

### Three sizes

| Size | Height | What the world is to them |
|---|---|---|
| Giant | about 17 m | rivers to step over, forests as gardens, a village as a toy |
| Person | about 1.7 m | the world as we know it |
| Tiny | about 1.7 cm | a crack in a wall is a cave, moss is a forest, a puddle is a lake |

A range of a thousand. The ratios are to be tried in the PoC (`POC_PLAN.md`).

### Places only some sizes reach

A gate is not a rule but geometry: a crack 2 cm wide is passable for the tiny and for nobody else;
a ledge 12 m up is reachable for a giant; a bridge of planks holds a person and breaks under a
giant. The engine never trusts an area that says "this is tiny-only": it measures what each size
can reach (see "Laws").

### Physics that changes with size

Owned by the engine, so every area has it without its author writing it:

- The tiny walk on water (surface tension), ride a leaf, are blown by wind, fall without harm.
- Giants sink into mud, break bridges, step over rivers, and are heavy for their size: weight grows
  with the cube of size, strength with its square.
- People are in between, as we are.

This is where the game can be its own: what is a puzzle at one size is a step at another.

### Many areas, side by side

The world is a grid of areas, each made by someone (with Claude) and owned by its program. Areas
lie next to each other and are built on by everyone who plays there.

## The engine and the programs

**The engine decides everything; an area's program only answers what is where.** (Caprice's
R30: a theme only draws; Caprice decides everything else.)

The engine owns: physics, collision, the players and their size, who can reach where, what
players build, saving, networking, what crosses from one area to the next (water, creatures,
light, sound), and every limit.

An area's program answers one question, in batches: **what is here, at this point, for a viewer
of this size?** Its answer is a signed distance (how far to the nearest surface) and a material.
This is a signed distance field (SDF), as some engines already use (Media Molecule's Dreams, for
one):

- **Detail comes as you shrink.** Asked for a giant, a wall is smooth stone; asked for the tiny,
  the program adds the cracks, pebbles that are boulders to them, moss that is a forest.
- **Collision is the same question**, at the player's size.
- **Far areas are cheap**: they are asked only at a coarse size.
- **Materials are labels.** Water, stone, soil, wood and leaf come from the engine's own list, with
  their physical properties at every size. So the engine knows what is water and what is land
  from the first day, which Caprice's themes never told it (their shapes are only shapes).

An area also describes itself once: its name, its borders (heights, water level and climate along
each edge) and the places it means for particular sizes ("the cave behind the crack is for the
tiny").

**What players build is stored; what the program makes is derived.** Edits are kept on top of the
program's world and laid over it wherever it is asked. (Caprice: one stored flow, everything else
derived; files hold only stored state.)

**Programs never talk to each other.** Everything that crosses a border goes through the engine.
A border is a contract: before an area is made, its neighbours' borders are fixed, and the new
program must meet them.

**Creatures are programs of their own** (later): the engine owns where they are; the program says
what they want to do, within limits the engine keeps.

## Making an area

As Caprice makes a theme (`THEME_PROGRAMS_PLAN.md`, step 5):

1. The maker says what they want, and may sketch it (below).
2. Claude writes the area as one Rust file against Plateau's SDK.
3. It is compiled to WebAssembly. The compiler's words go back to Claude, never to the maker.
4. The engine checks it (only `host` imported, memory declared, no SIMD, no threads) and plays it:
   the laws, at every size. What breaks them goes back to Claude, with the smallest input that
   breaks it.
5. Claude is shown pictures at every size and where each size can reach, and makes it better.
6. The maker walks it, keeps it, or asks for a change. A version that worked is never lost.

### The sketch

A map from above with a few pens that each mean one thing, as planned for Caprice
(`LAWS_PLAN.md`, "Realism"):

| Pen | Means |
|---|---|
| Brown, an area | land, ground |
| Grey, an area | rock, cliff |
| Blue, an area | water |
| Black, with a word | a thing: "lighthouse", "table" |
| Red, an arrow | something moves, along this path |
| Orange | a place for the tiny only |
| Purple | a place for giants only |

It fixes where things are and how they move, never how they look. Because the engine knows
materials, it can check the area against the sketch: water where the blue is, the orange places
reachable only by the tiny.

## Laws

What must hold for every input, searched for rather than sampled (`LAWS_PLAN.md`). Most of them
are Caprice's laws again; the zoom bugs Caprice's themes had are this game's central problem.

| Law | Holds | In Caprice |
|---|---|---|
| Sizes agree | Detail added for a smaller size never changes the shape a larger one sees beyond a tolerance: the crack a giant sees is where the tiny find the cave | T1, nothing slides with the zoom |
| Gates are real | For each size, the engine works out what it can reach (flood fill, path search); a place meant for one size is reachable by it and by no other | new |
| No traps | From everywhere a size can reach, a size can get back out | new |
| Borders match | Where two areas meet, ground, water and climate are continuous | T2, the edges |
| The same everywhere | The same module and seed give the same world (its hashes) on every machine | the twins, the fixtures |
| NaNs unseen | An area gives the same answers whether NaNs are made one or left as each machine makes them, so it can run without the cost of making them one | new |
| Steps do not matter | What moves gives the same whether time is stepped finely or coarsely | T3 |
| Made again equals played | An area made again from its seed and its stored edits is the one that was played | T4, `fresh` |
| Within its budget | Samples, instructions and memory stay within the engine's limits, at every size, over hours | the points budget |
| Moves where drawn | What moves follows the sketch's red paths and keeps to its material: a boat on water | S1 |
| Completable | A puzzle or a way through can be finished: bots search for it at each size | new |

Problems of the world that no law sees (a cliff as a tall narrow island) go to a critic: a second
asking of Claude, never the one that wrote the area, shown the pictures and the sketch and asked
questions that grow with each surprise.

## Sharing

As Caprice's gallery: an area is published only if it was built again byte for byte from its
source and seen to play, and it is signed. Every player's machine then runs the same module and
gets the same world, so only modules, seeds and edits travel, never worlds.

With many players, children among them, more can go wrong than beside a story:

- **Griefing**: an area built to trap players. "No traps" catches it.
- **Offence**: a huge structure seen from far away. Published areas are looked at by a person.
- **Deception**: fake buttons or messages drawn in the world. Words and the game's controls are
  never drawn by areas.

## What this does not solve

- **Shared state between untrusted authors.** Who owns a creature that walks into my area? Two
  players building on one spot? The engine decides: one authority, clear ownership, edits merged
  by the engine.
- **Networking.** Determinism helps a lot, but a server still has the last word.
- **Time to make.** A Caprice theme takes Claude 7 to 25 minutes: fine to make an area, nothing
  during play. During play only the programs run.
- **Precision across a range of a thousand.** Positions are local to each area, in 64-bit floats in
  the engine.
- **Quality.** Whether Claude's areas are good at every size, and whether moving between sizes is
  fun, are the PoC's two questions.

## What Plateau takes from Caprice

From `~/Projects/caprice-web`:

- `THEME_PROGRAMS_PLAN.md`: themes as programs written by Claude, built and run in a sandbox; the
  making loop; the gallery and its signatures.
- `ARCHITECTURE.md`, R36: the interface never edited once published; the checks on load; the
  limits kept by the host; no WebAssembly in the tests, each module standing for its source
  played natively.
- `LAWS_PLAN.md`: laws instead of symptoms; search, shrinking and a corpus; the sketch, the long
  exposure, the critic, the world the SDK knows.
- `toolchain/`, `web/compiler/`: rustc and wasm-ld as WebAssembly in a browser tab, for a later
  browser version of Plateau.

## Open questions

- The sizes' ratios, and whether there are three sizes or a range.
- Changing size: anywhere, or at places in the world.
- First person or from behind.
- The look: a style that holds up at every size (likely stylized rather than realistic).
- How large an area is, and how many a world has.
- Desktop first, the browser later (Caprice's toolchain in the tab), or both from the start.
