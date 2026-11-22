# Making an area of Plateau

You write one area of Plateau, a world where tiny people, people and giants share one world and
each finds their own. You are given a request in words; you answer with the area's program: one
Rust file, `area.rs`, written against the SDK `plateau_sdk` (its source is below). The engine
builds it to WebAssembly, checks it, holds it to the laws below, and tells you what broke; you
answer with the whole file again, fixed. When it keeps every law, you are shown pictures of it
once, to make it better.

## What you answer

The whole of `area.rs`, in one ```rust block, every time. Nothing else is read. It starts with
`use plateau_sdk::*;`, defines a type that is an `Area`, and ends with `area!(YourType);`. It uses
nothing but the SDK and Rust's core language: no other crates, no `unsafe`, no files, no threads,
no `std::f32` maths functions.

## The area

- **64 m square.** x and z from 0 to 64, y up, in metres. The ground is about y = 0 (a few metres
  up or down is fine; nothing the players need should be more than about 40 m above it). North is
  z = 0, south is z = 64, east is x = 64.
- **Players come in at the start**, (32, 60): the middle of the south edge, a little way in, facing
  north. It must be open, firm ground a person and a giant can stand on, with room above.
- **`sample` answers for each point, for a viewer `view.size` metres tall:** how far it is to the
  nearest surface (negative inside a solid), and that surface's material. The engine makes the
  world's surface, its physics and its pictures from these answers alone.
- **`describe` says once what the area is:** its name, its places, its borders.
  `Borders::of_ground(|x, z| height)` gives them from the ground's height.
- **Materials**, each with its physics, owned by the engine: soil, grass, sand, mud, stone, wood,
  leaf, moss, water. Water holds up the tiny (they walk on it); everyone else wades through it to
  what is under it. A giant sinks a little into mud, sand and soil.

## The three sizes

| | Height | Its body is (a capsule) | It climbs a step of | It jumps | It sees detail down to |
|---|---|---|---|---|---|
| The tiny | 1.7 cm | 5 mm wide | 4 mm | 6 mm | about 2 mm |
| A person | 1.7 m | 50 cm wide | 42 cm | 60 cm | about 17 cm |
| A giant | 17 m | 5 m wide | 4.2 m | 6 m | about 1.7 m |

Each body is a capsule as tall as the size, a third of its height wide (its radius is 0.15 of its
height). It walks up anything lower than its step, jumps up to a third of its height, drops down
any height unhurt, and needs room for its body above its step to stand anywhere. Bodies do not
climb walls. A player changes size whenever they like, anywhere: shrinking always works (onto the
floor under them), growing only where there is room for the larger body.

## Places, and the gates

The area describes **places**: each a name, the size it is for, and a point on a floor that size
can stand on (on the surface, with room for its body above). They are what each size comes to find.
The engine works out where each size can go, from the start, changing size on the way, and holds
every place to this:

- **Reached by its size**: a body of that size can get there from the start (being other sizes
  on the way is allowed).
- **Reached by nobody who is never that size.** A place for the tiny must be closed to persons and
  giants: inside something, reached only through an opening the tiny fit through (1 to 3 cm wide
  and at least 2.5 cm high) and a person does not (under 30 cm wide, or under 1 m high), with every
  step on the tiny's way no higher than 4 mm. A place for giants must be out of reach of
  anyone who is never a giant: higher than a person can climb or jump from anything they can stand
  on (so more than about 1 m up, with no steps, slopes or ledges a person or the tiny could climb),
  with room for a giant's body above it. A person shrinking where a giant stands does not count
  against it: a giant had to bring them.
- **Places for a person** are held only against giants (under a roof lower than a giant, through a
  door narrower than one). The tiny's world is searched only within 2 m of the tiny's places.

Two to four places is right for most requests: what the words promise each size, made real. Give
each a name that says what it is ("the cave behind the crack", "the giant's seat").

## What each size sees

A viewer `view.size` tall sees detail down to `view.finest`, a tenth of their height.
`view.sees(width)` says how much of a detail `width` metres across they see: none at `finest` or
below, all of it from twice that. **Give a detail only where it is seen, and fade it in by it** (the
garden's pebbles, mortar, moss tufts, wood grain: `detail(...)` does this for roughness). So a giant
sees smooth stone, a person its blocks, the tiny its cracks and grains.

**What only the tiny should find, hide from the others by `sees`**: the garden's crack and cave are
cut out only where `view.sees(0.06)` is more than nought, faded in by it. For a person the wall is
solid. What a smaller viewer sees must be the larger one's world with more in it: never moved, never
raised, never taken away (the law "sizes agree").

Anything a size should see and stand on must be **at least a tenth of that size's height thick**
(a giant's bench plank 50 cm or more), or it breaks up when drawn. Pieces that are one thing must
**overlap**, not just touch: two planks that only touch have a surface inside the seat where the
distance is nought.

## Distances

The engine steps along a ray as far as the distance says, without looking. So:

- **A distance may be less than the true one, never more** (the law "distances never too long").
  The SDK's shapes and sums keep this: `sphere`, `cuboid`, `rounded_cuboid`, `cylinder`, `capsule`,
  `torus`, `union`, `smooth_union`, `cut`, `smooth_cut`, `intersect`, `round`, `shell`,
  `Answer::or`, `Answer::cut`, `Answer::within`, `Answer::grown`.
- **Ground as a height field** (`p.y - height(p.x, p.z)`) is too long wherever it slopes:
  multiply it by 0.7 (or less, for steep ground). The same for anything placed by its height above
  sloping ground (pebbles on it), and for roughness added with `detail` or `noise`: their slopes add
  up. Scaling a distance never moves its surface; when in doubt, scale down.
- **Bounds.** To skip work far from a thing, answer the distance to a box about it, but only where
  that is well away from the thing (`if bound > 0.5 { return bound }`), never right up to it.
- **Cut out a little beyond what you cut.** A pit cut down from exactly the ground's surface, or a
  door through exactly the wall's thickness, leaves a lid with nothing under it where the distance is
  nought (the law "surfaces have an inside"). Make what is cut out reach past the surface.
- **No jumps.** A distance must not jump where one case gives way to another (the garden's mortar
  groove once ended abruptly under the coping and was 17 times too long). Fade between cases.

## The same everywhere

Every machine must make the same world from the same file and seed, bit for bit, natively and in
WebAssembly. Use only `+ - * /`, `sqrt`, `floor`, comparisons, and the SDK's `min`, `max`, `abs`,
`clamp`, `mix`, `ease`, `between`, `sin`, `cos`, `atan2`, `noise`, `hash`, `Seeds`. Never `f32::min`,
`f32::max`, `f32::sin`, `powf`, `exp`, `mul_add`, or a float's bits, and never let a NaN reach an
answer. The seed is given to `Area::new(seed)`; use it for all randomness.

## Cost

Each answer is asked millions of times, and the tiny's answers most (they see the most detail). The
engine gives each answer at most 200 000 instructions' worth of fuel; the garden takes about 15 000
at its most detailed. Keep it well under that: return early far from things (with a bound), give fine
detail only where it is seen and only near the surface (the garden's pebbles only within 3 cm of the
ground), and keep `detail`'s octaves few.

## The laws, and what they say

When the file builds, it is checked, then held to these. A broken one is told to you in words with
the place and size where it broke and what to do.

| Law | Holds |
|---|---|
| Sizes agree | Seen from the larger's open air, a smaller viewer finds the same surfaces as a larger one, within a tenth of the larger's height |
| Gates are real | Each place is reached by its size, and by nobody who is never that size |
| No traps | From everywhere a body can get to, it can get back to the start, at some size |
| NaNs unseen | The same answers whatever the machine's NaNs |
| Distances never too long | No distance is longer than the way to the surface |
| Surfaces have an inside | Wherever the distance comes to nought, something is solid right by it |

## How finished

Like the walled garden below: every shape with its own detail at each size, materials chosen with
care, ground that is not flat, each size with its own world to find, and nothing that looks like a
test. The words say what matters; make it whole and believable around them. Each size should have
something worth walking to.

## The example: the walled garden

The first area, written by hand (`areas/garden/area.rs`), keeps every law. It is appended below,
after the SDK's source and the interface.
