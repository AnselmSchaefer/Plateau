# Step 0: what the spike found

The spike of `POC_PLAN.md`, step 0, done 2026-11-21 on this machine: an Intel i7 (TigerLake-H,
12 threads), 16 GB, a GeForce GTX 1650 (Vulkan, NVIDIA 615.78), Hyprland on Wayland; rustc 1.99.1,
wasmtime 51, Bevy 0.20.1, wgpu 30 with winit 0.30.

## What is here

| Path | What |
|---|---|
| `garden/area.rs` | The hand-written area: 64 m of rolling ground, a pond, a garden wall, with detail down to a twentieth of the viewer's height. One file, built as a module and into the host |
| `build.sh` | Builds it to `modules/garden-O.wasm` and `garden-O0.wasm` with rustc alone |
| `host/` | The module in wasmtime (fuel, memory limit, no SIMD, no threads), the law `nans_unseen`, the same source played natively, surface nets, hashes; `tests/same.rs`; `bin/bench.rs` |
| `view-bevy/`, `view-wgpu/` | The whole garden, meshed from the module, to fly through |

```
spike/build.sh                                         # after changing area.rs
cargo test -p spike-host                               # native = -O = -O0, NaNs unseen, fuel stops
cargo run --release -p spike-host --bin bench          # the numbers below
cargo run -p view-bevy                                 # or view-wgpu; WASD, Space, Shift, drag to look
```

## Numbers

**A sample** (ns, batches of 4096; the batch size from 1 to 65536 changes little, at most 45 ns a
call, because a sample costs far more than a call). "Module" is how areas run in play: fuel
counted, the machine's own NaNs:

| Size | Native | Module | Module, no fuel | Module, one NaN | Module at `-O0` | Fuel a sample |
|---|---|---|---|---|---|---|
| Tiny (1.7 cm) | 619 | 504 | 478 | 1643 | 2896 | 9588 |
| Person (1.7 m) | 237 | 203 | 190 | 631 | 1212 | 3834 |
| Giant (17 m) | 80 | 71 | 68 | 226 | 461 | 1372 |

- **Fuel costs about 5%.** Keep it on, always.
- **NaN canonicalization ("one NaN") costs over three times.** It is off in play, and a law stands
  in for it (below). Without it the module is a little faster than the native build.
- A tiny person's sample costs seven times a giant's, because more octaves of detail are seen. The
  cost of detail is the area's to keep down, and the budget's to enforce (fuel per sample).
- Compiling and instantiating the 23 KB module: 25 ms.

**A chunk** of 32 × 32 × 32 cells at the size's voxel, on the wall's face, one thread (ms):

| Size | Voxel | Sampling, native | Sampling, module | Meshing | Triangles |
|---|---|---|---|---|---|
| Tiny | 0.85 mm | 21.8 | 17.8 | 0.35 | 2156 |
| Person | 8.5 cm | 8.6 | 7.2 | 0.39 | 4146 |
| Giant | 85 cm | 2.9 | 2.6 | 0.34 | 2510 |

Every size is under the target of 20 ms a chunk near the player, the tiny only just (with one NaN
they were 56, 22 and 8 ms). Meshing is nothing; sampling is everything. Two ways down for step 2,
where many chunks are wanted at once: chunks on all cores (each thread its own instance), and
sampling only near the surface (a distance larger than a cell's diagonal says the whole cell is
empty or full, so a coarse pass skips most of a chunk: the surface is two-dimensional and the
chunk three).

**The whole garden**, one chunk of 264 × 32 × 264 cells at 25 cm: 2.2 million samples in 1.5 s
from the module with one NaN, meshed in 0.1 s, 170 000 triangles.

**Determinism.** Native, `-O` and `-O0` give the same answers, bit for bit, at every size
(`tests/same.rs`, 32 768 points each). That holds because `area.rs` uses only operations IEEE 754
makes exact, and its own `min` and `max`. **The SDK must do the same**: no `sin`, `exp` or `powf`
from `std` (they come from different libms natively and in WebAssembly), and its own `min`/`max`
(the standard ones may differ on the sign of zero). A law can check it: the same hashes natively
and as a module.

## The engine: Bevy

| | Bevy 0.20 | wgpu 30 and winit 0.30 by hand |
|---|---|---|
| The viewer | 80 lines | 354 lines, with its own shader |
| Shadows, PBR, tonemapping, input, frame times | given | none but what was written |
| Frames a second, the garden, 170 000 triangles | about 400 uncapped | 60 (the cap: no-vsync fell back to vsync) |
| First build | 8 min 55 s | 2 min 22 s |
| A rebuild after a change | 5.0 s | 1.3 s |
| Crates | 582 | 234 |
| Debug binary | 1.8 GB | 440 MB |

Both run on the GTX 1650 over Vulkan, under Wayland, first time. **Chosen: Bevy.** What it gives
(shadows that already show the wall's shape, a camera, input, diagnostics, and later text, audio
and gizmos) would take weeks by hand; its cost is build times, a few seconds a change once built,
and a large tree of crates. Its version is pinned (`=0.20.1` from step 1) and not chased.

Screenshots (`screenshots/bevy.png`, `screenshots/wgpu.png`): the same view in both, the wall from
above the garden's corner, Bevy's with shadows.

## NaNs: a law instead of canonicalization

Decided 2026-11-21. Canonicalization costs over three times. What it protects against: an area
that looks at a NaN's bits (reinterpreting a float as an integer, or storing it and loading an
integer) answers differently on x86 and ARM, whose NaNs differ, and so makes a different world on
another machine. Areas run with the machine's own NaNs, and the law **NaNs unseen**
(`host::nans_unseen`) asks the area the same points with canonicalization on and off and requires
the same answers, bit for bit, saying the first point and size where they differ. On x86 the
machine's NaN is not the canonical one, so an area that depends on NaN bits shows it here.

Like every law it is searched, not proved: an area could look at NaN bits only somewhere the search
never asks. Step 3 puts it with the other laws over every area, and the check on publishing can
search further. `tests/same.rs` shows the garden keeps it and a module written to read a NaN's sign
breaks it.

## For the next steps

- **Seams between chunks**: this spike meshes one chunk at a time; quads along a chunk's edge are
  left out. Step 2 samples one cell beyond each chunk.
- **Materials** are per vertex, so their edges are jagged (the soil patches, the wall's foot).
  Blending them in the shader, or a material weight from the area, is for the look, later.
- The tiny chunk first missed the surface: the wall's roughness moves its face 8 cm, three times
  the chunk's width. At a tiny person's size the world near them must be found by searching, not
  assumed: chunks are placed where the player is, not where the coarse surface was.
