# Plateau: What the PoC Found (Step 10)

Written 2026-11-25 at the close of Steps 0–9 of [`POC_PLAN.md`](POC_PLAN.md).

---

## 1. Measures vs. Targets

| Measure | Target (`POC_PLAN.md`) | PoC Result |
|---|---|---|
| Areas keeping every law | ≥ 3 of 5 | **9 of 9 (100%)** — 5/5 unsketched test set, 2/2 sketched, 1/1 interactive procedural-scene (`windmill`), 1/1 border-matched adjacent (`forest-beside-pond`) |
| Rounds of Claude per area | ≤ 3 fixes + 1 look | **0–2 fixes** across all 9 areas (mean **1.0 fix**, 17–28 min, `$2.31–$5.96` reckoned) |
| Frame rate (GTX 1650, `1600×900`) | ≥ 60 FPS at each size | **66–159 FPS** on `pond-sketched` (`96k` scene tris, 0 frames with holes): Tiny 66–80 FPS, Person 85–104 FPS, Giant 155–159 FPS; **112–256 FPS** on `garden` |
| Chunk build time near player | < 20 ms | **3 ms** mean (person), **13 ms** mean (tiny); worst-case complex crack chunks (~180 ms) hidden off-thread with zero-hole fallback + Bayer cross-fade |
| Bit-exact determinism | Always (2 runs, `-O` vs `-O0`, native vs WASM) | **100% held** — verified by `same_everywhere` hashes, `nans_unseen`, `WorldSave` round-trip chunk hashes, and `Atlas::single` pass-through |
| Sandboxed execution | Zero host capabilities beyond `seed()` | **100% held** — `wasmparser` load check, Wasmtime fuel/memory caps, no SIMD/threads, capped procedural `Scene` & `Command` verbs |
| Automated verification | Offline, never calls Claude | **106 passing tests** across `plateau-engine`, `plateau-make`, `plateau-sdk`, and `spike-host` |

### Complete Record of Claude-Generated Areas (`made/`)

| Area | Mode | Kept every law | Rounds | Fixes | Look | Time | Reckoned | What broke before fixing |
|---|---|---|---|---|---|---|---|---|
| `table` | words | yes | 2 | 0 | yes | 18 min | `$2.42` | nothing |
| `headland` | words | yes | 3 | 1 | yes | 22 min | `$3.41` | `no traps` (round 1) |
| `pond` | words | yes | 4 | 2 | yes | 18 min | `$3.22` | after look: build error, then `sizes agree` |
| `forest` | words | yes | 2 | 0 | yes | 19 min | `$2.31` | nothing |
| `castle` | words | yes | 3 | 1 | yes | 17 min | `$2.61` | `surfaces have an inside` (round 1) |
| `headland-sketched` | words + sketch | yes | 3 | 2 | no (rate limit) | 28 min | `$3.43` | `surfaces have an inside`, `no traps` |
| `pond-sketched` | words + sketch | yes | 3 | 1 | yes | 27 min | `$3.26` | `no traps` (round 1) |
| `windmill` | words + `Scene`/`tick`/`interact` | yes | 3 | 1 | yes | 19 min | `$3.80` | `surfaces have an inside` (round 1) |
| `forest-beside-pond` | words + `--beside west:pond-sketched` | yes | 4 | 2 | yes | 26 min | `$5.96` | `distances never too long`, `surfaces have an inside` (passed `borders match` on round 1) |

---

## 2. Answering the Two PoC Questions

### Question 1: *Can Claude make an area, from words and a sketch, that is good at every size and keeps the laws?*

**Yes — unequivocally.** Every single prompt across words-only, sketched, interactive procedural-scene, and border-matched multi-area runs converged to a valid, sandboxed, bit-exact `.wasm` area within 0–2 fixes.

What made it work:
- **Laws over prompt engineering**: prose warnings in `GUIDE.md` about flush-cut lids, traps, or steep slopes were not enough on their own, but the automated laws (`no traps`, `surfaces have an inside`, `distances never too long`, `sizes agree`, `gates are real`, `sketch_*`, `borders match`) caught every defect with exact coordinates and sizes, and Claude fixed each in a single round.
- **Sketches transform spatial quality**: `headland-sketched` and `pond-sketched` passed both sketch laws on round 1 and replaced generic geometric primitives with organic shorelines, curving cliffs, stepping-stone trails, and accurate landmark placement.
- **Hybrid SDF + procedural `Scene` broke the Nyquist ceiling**: pure CPU Surface Nets hit a hard limit on 2–4 cm stalks/reeds/flowers at medium distance. Keeping carvable landforms in the SDF while moving thin, repeating, architectural, and animated elements into `MeshBuilder`/`SceneBuilder` + `tick`/`interact` jumped both visual fidelity and frame rate (200+ FPS potential, 96k crisp triangles in 1 draw call).
- **Border tables make multi-area authoring reliable**: giving Claude a copy-pasteable `const <SIDE>_GROUND: [f32; 65]` array plus `edge_height` and `laws::borders_match` yielded a seamless join between `pond-sketched` and `forest-beside-pond` on the very first round.

What still trips Claude up (lessons for the guide and tooling):
- **Mid-distance `view.sees` fade on person-scale landmarks**: in `forest-beside-pond`, gating the 72 cm fallen log across the pool on `view.sees(0.7)` caused it to shrink to a thin twig in level `-2` chunks (asked at `size = 5.0 m`, ~12 m ahead of a person) and pop in half-by-half across a chunk boundary. Walkable landmarks and bridges belong either in the procedural `Scene` layer (`solid: true`) or unfaded in the SDF (fading only fine surface `detail()`).
- **Hidden tiny entrances**: an opening 2 cm wide vanishes to a person unless marked visually (dark cleft, moss arch, miniature lantern, or `M` flags).
- **Interior lighting in automated pictures**: tiny rooms inside rocks/trunks need the camera headlamp and door-framed viewpoints so Claude can actually judge them on the `look` round.
- **Authoring latency (~18–28 min/area)**: fine for asynchronous creation and sharing, too slow for synchronous wait-in-game generation (though standalone elements build and verify much faster).

### Question 2: *Is moving between sizes worth it — does being tiny or a giant show things worth finding?*

**Yes — especially once exploration is paired with multi-scale physical modification (Steps 8a–8c).**

What worked best in play:
- **Genuine scale contrast**: walking across lily pads as a 1.7 cm tiny person on water surface tension, slipping into a 2 cm root door or mossy stone crack (`garden`, `pond-sketched`, `forest-beside-pond`, `windmill`), then growing to 17 m to wade the pond, sink into mud, and sit on tree crowns or a giant's bench creates an immediate sense of discovery that fixed-scale games do not have.
- **Multi-scale crafting & modification (Steps 8a–8c)** turned scale-switching from a sightseeing gimmick into a core game mechanic:
  - Raising a person-scale hill (`B`), shrinking to tiny, and carving a 2 cm tunnel/home inside it.
  - Placing a standalone `.wasm` oak tree (`P`), carving a tiny home right into its trunk (lazy `Scene → SDF` promotion), growing to person/giant scale to chop it down (`K`), carrying the carved trunk (`G` + `H` flat flip) across the world (with its carved tiny tunnel traveling intact inside it!), and laying it down as a solid walkable bridge.
- **Seamless multi-area crossing (Step 9)**: walking from the sunlit `pond-sketched` meadow directly into the shaded `forest-beside-pond` stools with no loading screen or visible seam makes the world feel like a continuous shared landscape rather than isolated dioramas.

What was weaker or still rough:
- **Empty scale transitions without cues**: finding a 2 cm entrance in a 64 m area without visual landmarks (lanterns, paths, dark arches, or `M` flags) can feel like pixel-hunting.
- **Coarse giant LOD vs. small sculpted structures**: handled cleanly for promoted `Scene` elements via the macro-LOD proxy (`PROMOTED_SDF_MAX_SIZE_RATIO`), whereas tiny SDF-only specks naturally dissolve when viewed from 17 m (true to real optics, though landmark silhouettes still need giant-scale markers).
- **Scale ratio of 1000× (`1.7 cm` ↔ `1.7 m` ↔ `17 m`)**: works well numerically (64-bit world coordinates, player-scaled Bevy rendering, 1 s smooth growth/shrink), and keeping intermediate continuous sizes (`Q`/`E` or arbitrary `size`) open for future play remains easy.

---

## 3. What the Next Stage Needs

The PoC deliberately left five major systems out of scope (`POC_PLAN.md`, "Out"). Based on what we built and measured, here is what the next stage requires:

### 1. Networking & Shared World Authority
- **Why the PoC architecture helps**: areas and standalone elements are small deterministic `.wasm` binaries (`28–115 KB`), so clients only download `(module_hash, seed)` once and never stream world geometry.
- **What to build**:
  - An authoritative server (or peer-hosted room) that sequences **world deltas**: `Edit` records, `PlacedElement` spawns/cuts/relocations/local-edits, and player state (`feet`, `size`, `yaw`, carried object ID).
  - Conflict resolution & carry locks (preventing two players from grabbing or chopping the same element simultaneously).
  - Bandwidth-light sync for deterministic `tick`/`interact` state (server-authoritative `time` + event log for `interact` triggers, or periodic snapshot checksums for stateful area plugins).
  - Multi-area `WorldSave` manifest listing the tile grid `(gx, gz) -> (module_hash, seed)` alongside the shared edit/element log.

### 2. Sandboxed Creatures
- Separate `plateau:creature@0.1.0` (or extension of `Element`) where **the engine owns physics, collision, spatial partitioning, and cross-border movement** (just as `Body` does for players), while the creature's `.wasm` program receives local perception (nearby players/sizes, ground material, obstacles) and emits clamped steering/animation/sound intents.
- Because creatures cross area borders through the engine (`Atlas`), areas still never talk to each other directly.

### 3. The Gallery, Reproducible Builds & Signing
- A publishing pipeline (mirroring Caprice's gallery) that:
  1. Accepts `area.rs` / `element.rs` + optional `sketch.png` + target neighbour slot `(gx, gz)`.
  2. Compiles reproducibly with the pinned toolchain and verifies byte-for-byte reproducibility.
  3. Runs the full law suite (`check`, `sizes_agree`, `never_too_long`, `surfaces_have_an_inside`, `gates_are_real`, `no_traps`, `nans_unseen`, `borders_match`, plus a future `within_budget` worst-case fuel law and critic pass).
  4. Signs the verified `.wasm` + metadata (chunk hashes, border profiles, preview renders) for safe distribution to strangers.

### 4. The Browser Client
- `plateau:region@0.1.0` already uses a core WASM module with ahead-of-time canonical ABI bindings and zero host imports beyond `seed()`, so **existing `.wasm` areas and elements need zero changes** to run in a browser.
- **Engine portability**: `plateau-engine` has zero Bevy dependency and compiles cleanly on desktop; targeting `wasm32` requires abstracting the WASM host runner (`Wasm` backend trait: Wasmtime on native, browser `WebAssembly.Instance` + Web Worker pool with fuel/timeout budgeting on web) and porting `play/` to WebGPU (Bevy WebGPU or a thin `wgpu` frontend).
- **In-browser authoring**: can reuse Caprice's in-tab `rustc`/`wasm-ld` toolchain when ready, or delegate compilation to a build service.

### 5. Desktop Playability & Polish
- **Streaming beyond 2×2 grids**: `COARSEST = 0` (`192 m` span) already handles a `2×2` atlas seamlessly; larger world grids (`N×M`) should lazily load/evict distant `Atlas` tiles and add a coarser far-horizon LOD (`level = 1` / impostor heightmap) so distant hills stay visible.
- **Sound, menus & world browser**: scale-aware audio (footsteps/water/chopping pitched or filtered by player size), in-game world/area picker, and saved world slots.
- **Authoring polish**: add a `GUIDE.md` rule against small `view.sees` cutoffs on walkable bridges/landmarks, add a worst-case per-sample fuel law (`within its budget`), and add a second-model critic pass (`CONCEPT.md`, "Laws") to catch visual/aesthetic issues laws cannot formalize.
