# Plateau

An open world building game where tiny people (1.7 cm), people (1.7 m), and giants (17 m) share one
world. Each area and standalone composable element is a Rust program Claude writes from words and
an optional sketch, compiled to a single WebAssembly module and run in a strict Wasmtime sandbox.
Players can sculpt landforms at any scale, place standalone `.wasm` elements, chop and carry
physical objects across scales, and walk seamlessly from one area to the next.

- `CONCEPT.md`: the idea, the engine and the programs, the laws, and what it does not solve.
- `POC_PLAN.md`: the proof of concept, Steps 0–10.
- `POC_FINDINGS.md`: Step 10 summary — answers to the two PoC questions, full numbers, and next-stage architecture.
- `AGENTS.md`: core architectural invariants (Hybrid Layer Ownership, strict WASM sandboxing, bit-exact determinism).
- `docs/architecture.html` / `docs/architecture.png`: system architecture overview.
- `sdk/`: what areas and elements are written against — WIT world `plateau:region@0.1.0`
  (`sdk/wit/region.wit`), pre-generated bindings, deterministic SDF math/noise, `MeshBuilder` /
  `SceneBuilder`, `Area` / `Element` traits, and `area!` / `element!` macros.
- `engine/`: renderer-agnostic engine — Wasmtime loader & validator (`check`, `wasm`), multi-area
  `Atlas` & C¹ border stitching (`atlas`), Surface Nets chunks & view-directed LOD rings (`chunk`,
  `rings`), multi-scale physics (`body`), reachability (`reach`), CPU ray-marched pictures & reach
  maps (`picture`), sketch parser (`sketch`), laws including `borders_match` (`laws`), scale-stamped
  SDF edit overlay & `.plateau` saves (`edit`), composable `.wasm` elements & lazy `Scene → SDF`
  promotion (`compose`, `mesh_sdf`), and watertight cutting / object physics (`cut`, `scene`).
- `make/`: `plateau` CLI — asks Claude to write areas (`make`, `--sketch`, `--beside SIDE:AREA`) or
  standalone elements (`make-element`), compiles, verifies all laws, renders pictures & seam views,
  and checks pairs (`check`, `check-pair`, `check-element`).
- `play/`: Bevy desktop client — multi-area walking at three sizes (`1`/`2`/`3`, `Q`/`E`), live
  multi-scale SDF sculpting (`B`), element placement (`P`), chopping (`K`), carrying & rotating
  (`G`, `H`, `R`/`T`, wheel), undo/redo (`Z`/`Y`), and save/reload (`Ctrl+S`/`Ctrl+L`).
- `areas/`: carried hand-written areas (`areas/garden/area.rs`, built via `areas/build.sh`).
- `elements/`: carried standalone composable `.wasm` elements (`tree`, `bridge`, `lantern`, `hill`,
  built via `elements/build.sh`).
- `made/`: Claude-generated areas (`table`, `headland`, `pond`, `forest`, `castle`,
  `headland-sketched`, `pond-sketched`, `windmill`, and `forest-beside-pond`).
- `sketches/`: paint palette (`plateau.gpl`) and example sketches (`headland.png`, `pond.png`).
- `spike/`: Step 0 spike and benchmarks (`spike/RESULTS.md`).

```bash
./areas/build.sh && ./elements/build.sh                                    # rebuild carried areas & elements
cargo test --workspace --exclude view-bevy --exclude view-wgpu --exclude plateau-play

# Walk a single area (or two stitched side by side!)
cargo run --release -p plateau-play -- made/pond-sketched/area.wasm
cargo run --release -p plateau-play -- made/pond-sketched/area.wasm --east made/forest-beside-pond/area.wasm

# Generate or verify areas and standalone elements with Claude
cargo run --release -p plateau-make --bin plateau -- make "<words>" [--sketch sketches/pond.png] [--beside west:pond-sketched]
cargo run --release -p plateau-make --bin plateau -- check-pair pond-sketched east forest-beside-pond
cargo run --release -p plateau-make --bin plateau -- make-element "<words>"
```
