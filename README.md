# Plateau

An open world building game where tiny people, people and giants share one world. Each area of
it is a program Claude writes from a few words and a sketch, run in a WebAssembly sandbox; some
places can only be reached when you are tiny, some only as a giant.

- `CONCEPT.md`: the idea, the engine and the programs, the laws, what it does not solve.
- `POC_PLAN.md`: the proof of concept, step by step.
- `assets/icon.svg`: the icon (a winged lion with a boy's face standing on a plateau at dusk, and a
  crack in the cliff below it only the tiny fit through), with PNGs from 32 to 512 px made from it by `rsvg-convert`.
- `sdk/`: what an area's program is written against: the WIT world `plateau:region@0.1.0`
  (`sdk/wit/region.wit`), its bindings made ahead, distances, noise and `area!`.
- `engine/`: the engine, without a window: an area's module checked on load and run in wasmtime,
  the law "NaNs unseen", pictures ray-marched on the CPU, the surface in chunks at each size's
  detail (`chunk`, `rings`), and a player's body walking it at its size (`body`).
- `play/`: the window (Bevy): one area walked first person at three sizes, its chunks made off
  the frame; `--tour` visits fixed places, measures, and takes screenshots (`play/tour/`).
- `areas/`: the areas, each one file (`areas/garden/area.rs`, the walled garden); `areas/build.sh`
  builds them into `areas/modules/`; `areas/played/` builds them for the machine, so the tests can
  play them natively.
- `spike/`: step 0, the spike, and what it found (`spike/RESULTS.md`).

```
areas/build.sh                                              # after changing an area or the SDK
cargo test --workspace --exclude view-bevy --exclude view-wgpu --exclude plateau-play
cargo run --release -p plateau-engine --example look        # pictures of the garden, into target/pictures
cargo run --release -p plateau-play                         # walk the garden; 1 2 3 for the sizes
cargo run --release -p plateau-play -- --tour target/tour   # the tour: numbers and screenshots
```
