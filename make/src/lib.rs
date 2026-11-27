//! Claude makes an area (POC_PLAN.md, step 5). Given a request in words, Claude writes the area's
//! program (`area.rs`, against the SDK) from the guide (`GUIDE.md`, with the SDK's source, the
//! world's interface and the walled garden as its example). The making loop:
//!
//! 1. The file is built to a module as `areas/build.sh` builds the areas (`Builder`). If it does
//!    not build, the compiler's words go back to Claude.
//! 2. It is checked and held to the laws (`judge`). What broke goes back to Claude, in the laws'
//!    own words, with where and at what size.
//! 3. When it keeps every law, Claude is shown its pictures and reach maps once (`pictures`), to
//!    make it better.
//!
//! At most `MOST_FIXES` answers go back for fixing, of either kind, and one look. The last version
//! that kept every law is the one kept. Claude is whatever `Writer` is given: `claude::Claude` in
//! use (the Claude Code command line, with the owner's own login), stand-ins in the tests, which
//! never call Claude (POC_PLAN.md, "Tests").

pub mod claude;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;

use plateau_engine::atlas::{BorderProfile, CompiledAtlas, Side, TileSpec};
use plateau_engine::compose::CompiledElement;
use plateau_engine::fit::{SiteRegion, cavity_fit, ground_fit, staged};
use plateau_engine::picture::{Camera, Plan, map, picture, plan, plans, seam_camera, seam_plan, tint, views};
use plateau_engine::reach::World;
use plateau_engine::scene::{Combined, SceneWorld};
use plateau_engine::site::SiteContext;
use plateau_engine::sketch::Sketch;
use plateau_engine::world::{OpenWorld, Solids};
use plateau_engine::{Compiled, Description, Limits, PlayerState, Region, Size, Wasm, check, laws, points};

/// The seed every area is made and judged with.
pub const SEED: u64 = 7;
/// The most answers sent back for fixing, compiler's and laws' together.
pub const MOST_FIXES: usize = 3;
/// The most of the compiler's words sent back.
const MOST_WORDS: usize = 12_000;

/// What Claude answered: its words, what it is reckoned to have cost (the API's price for it: with
/// a subscription, nothing is paid but the plan's usage) and how long it took.
#[derive(Clone, Debug, Default)]
pub struct Reply {
    pub text: String,
    pub cost: f64,
    pub seconds: f64,
}

/// Whoever writes the area: asked one thing after another, in one conversation.
pub trait Writer {
    /// The answer to `prompt`. With `pictures`, the writer may read the files in that folder.
    fn write(&mut self, prompt: &str, pictures: Option<&Path>) -> Result<Reply, String>;
}

/// How thoroughly an area is judged and pictured: thoroughly in use, quickly in the tests.
#[derive(Clone, Copy, Debug)]
pub struct Settings {
    /// Rays for each size of the laws that look along rays.
    pub rays: usize,
    /// How wide the views are, in pixels (16:9), and the plans (square).
    pub view_width: usize,
    pub plan_width: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { rays: 400, view_width: 640, plan_width: 512 }
    }
}

/// The guide Claude is given as its instructions: `GUIDE.md`, then the SDK's source, the world's
/// interface, and the walled garden, each whole.
pub fn guide(repo: &Path) -> Result<String, String> {
    let read = |p: &str| std::fs::read_to_string(repo.join(p)).map_err(|e| format!("{p}: {e}"));
    Ok(format!(
        "{}\n\n# The SDK: `sdk/src/lib.rs`\n\n```rust\n{}```\n\n# The procedural mesh & scene workshop: `sdk/src/mesh.rs`\n\n```rust\n{}```\n\n# The world's interface: `sdk/wit/region.wit`\n\n```wit\n{}```\n\n\
         # The walled garden: `areas/garden/area.rs`\n\n```rust\n{}```\n",
        read("make/GUIDE.md")?,
        read("sdk/src/lib.rs")?,
        read("sdk/src/mesh.rs")?,
        read("sdk/wit/region.wit")?,
        read("areas/garden/area.rs")?
    ))
}

/// The guide Claude is given when authoring a standalone composable `.wasm` element.
pub fn element_guide(repo: &Path) -> Result<String, String> {
    let read = |p: &str| std::fs::read_to_string(repo.join(p)).map_err(|e| format!("{p}: {e}"));
    Ok(format!(
        "{}\n\n# The SDK: `sdk/src/lib.rs`\n\n```rust\n{}```\n\n# The procedural mesh & scene workshop: `sdk/src/mesh.rs`\n\n```rust\n{}```\n\n# The world's interface: `sdk/wit/region.wit`\n\n```wit\n{}```\n\n\
         # Example interactive Scene element: `elements/lantern/element.rs`\n\n```rust\n{}```\n\n\
         # Example carvable SDF element: `elements/hill/element.rs`\n\n```rust\n{}```\n",
        read("make/ELEMENT_GUIDE.md")?,
        read("sdk/src/lib.rs")?,
        read("sdk/src/mesh.rs")?,
        read("sdk/wit/region.wit")?,
        read("elements/lantern/element.rs")?,
        read("elements/hill/element.rs")?
    ))
}

/// The same, for an element made for a measured site: with the round door, made for openings, as its
/// example of a frame and a leaf.
pub fn element_guide_at(repo: &Path) -> Result<String, String> {
    let door = std::fs::read_to_string(repo.join("elements/hobbit-door/element.rs")).map_err(|e| format!("elements/hobbit-door/element.rs: {e}"))?;
    Ok(format!("{}\n\n# Example fitted Scene element: `elements/hobbit-door/element.rs`\n\n```rust\n{door}```\n", element_guide(repo)?))
}

/// The area's source in an answer: its last ```rust block.
pub fn source_of(text: &str) -> Option<String> {
    let start = text.rfind("```rust")?;
    let body = &text[start + "```rust".len()..];
    let end = body.find("```")?;
    Some(body[..end].trim_start_matches('\n').to_string())
}

/// Builds areas to modules as `areas/build.sh` does: the repo's rustc, the SDK built once as one
/// rlib, the area as one file, the same fixed flags.
pub struct Builder {
    repo: PathBuf,
    sdk: PathBuf,
}

const FIXED: [&str; 11] =
    ["--edition", "2024", "--target", "wasm32-unknown-unknown", "-O", "-C", "codegen-units=1", "-C", "panic=abort", "-C", "strip=debuginfo"];

impl Builder {
    /// The SDK built, into `work`.
    pub fn new(repo: &Path, work: &Path) -> Result<Builder, String> {
        std::fs::create_dir_all(work).map_err(|e| e.to_string())?;
        let sdk = work.join("libplateau_sdk.rlib");
        let out = Command::new("rustc")
            .current_dir(repo)
            .args(FIXED)
            .args(["--crate-type", "rlib", "--crate-name", "plateau_sdk", "--remap-path-prefix"])
            .arg(format!("{}={}", repo.join("sdk").display(), "/sdk"))
            .arg(repo.join("sdk/src/lib.rs"))
            .arg("-o")
            .arg(&sdk)
            .output()
            .map_err(|e| format!("rustc: {e}"))?;
        if !out.status.success() {
            return Err(format!("the SDK does not build: {}", String::from_utf8_lossy(&out.stderr)));
        }
        Ok(Builder { repo: repo.to_path_buf(), sdk })
    }

    /// The module `source` builds to, built in `dir` (as `dir/area.rs`), or the compiler's words.
    pub fn build(&self, source: &str, dir: &Path) -> Result<Vec<u8>, String> {
        std::fs::create_dir_all(dir.join("build")).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("area.rs"), source).map_err(|e| e.to_string())?;
        let out = Command::new("rustc")
            .current_dir(&self.repo)
            .args(["--crate-name", "area"])
            .args(FIXED)
            .args(["--crate-type", "cdylib", "-C", "link-arg=--max-memory=67108864", "-A", "warnings", "--remap-path-prefix"])
            .arg(format!("{}=/area", dir.display()))
            .arg("--extern")
            .arg(format!("plateau_sdk={}", self.sdk.display()))
            .arg(dir.join("area.rs"))
            .arg("-o")
            .arg(dir.join("build/area.wasm"))
            .output()
            .map_err(|e| format!("rustc: {e}"))?;
        if !out.status.success() {
            let mut words = String::from_utf8_lossy(&out.stderr).into_owned();
            if words.len() > MOST_WORDS {
                let cut = (0..=MOST_WORDS).rev().find(|&i| words.is_char_boundary(i)).unwrap_or(0);
                words.truncate(cut);
                words += "\n[... the rest left out]";
            }
            return Err(words);
        }
        std::fs::read(dir.join("build/area.wasm")).map_err(|e| e.to_string())
    }

    /// Builds a standalone element (`dir/element.rs`) to `dir/build/element.wasm`, returning the bytes
    /// or the compiler's words.
    pub fn build_element(&self, source: &str, dir: &Path) -> Result<Vec<u8>, String> {
        std::fs::create_dir_all(dir.join("build")).map_err(|e| e.to_string())?;
        std::fs::write(dir.join("element.rs"), source).map_err(|e| e.to_string())?;
        let out = Command::new("rustc")
            .current_dir(&self.repo)
            .args(["--crate-name", "area"])
            .args(FIXED)
            .args(["--crate-type", "cdylib", "-C", "link-arg=--max-memory=67108864", "-A", "warnings", "--remap-path-prefix"])
            .arg(format!("{}=/element", dir.display()))
            .arg("--extern")
            .arg(format!("plateau_sdk={}", self.sdk.display()))
            .arg(dir.join("element.rs"))
            .arg("-o")
            .arg(dir.join("build/element.wasm"))
            .output()
            .map_err(|e| format!("rustc: {e}"))?;
        if !out.status.success() {
            let mut words = String::from_utf8_lossy(&out.stderr).into_owned();
            if words.len() > MOST_WORDS {
                let cut = (0..=MOST_WORDS).rev().find(|&i| words.is_char_boundary(i)).unwrap_or(0);
                words.truncate(cut);
                words += "\n[... the rest left out]";
            }
            return Err(words);
        }
        std::fs::read(dir.join("build/element.wasm")).map_err(|e| e.to_string())
    }
}

/// What an area was found to be: what it says it is, if it could be asked, and each law it broke,
/// with why.
#[derive(Clone, Debug, Default)]
pub struct Judged {
    pub description: Option<Description>,
    pub broken: Vec<(String, String)>,
    pub seconds: f64,
}

impl Judged {
    pub fn kept(&self) -> bool {
        self.broken.is_empty()
    }
}

/// An existing neighbouring area bordering one side of the area being made or checked (Step 9).
#[derive(Clone, Debug)]
pub struct Neighbor {
    /// Which side of the *new* area touches this neighbour (e.g. `Side::West` means this neighbour
    /// lies to the west of the new area, so its east edge meets the new area's west edge).
    pub side: Side,
    pub path: PathBuf,
    pub seed: u64,
    pub module: Vec<u8>,
    pub profile: BorderProfile,
}

impl Neighbor {
    /// Loads a neighbour `.wasm` module, instantiates it with `seed`, and extracts the border
    /// profile on the edge facing the new area (`side.opposite()`).
    pub fn load(side: Side, path: PathBuf, seed: u64) -> Result<Neighbor, String> {
        let module = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut wasm = Wasm::new(&module, seed, Limits::default()).map_err(|e| format!("{}: {e:#}", path.display()))?;
        let profile = BorderProfile::extract(&mut wasm, side.opposite()).map_err(|e| format!("{}: {e:#}", path.display()))?;
        Ok(Neighbor { side, path, seed, module, profile })
    }
}

/// Saves neighbour references to `dir/neighbors.txt` so `plateau check` can re-verify `borders_match`.
pub fn save_neighbors(dir: &Path, neighbors: &[Neighbor]) -> Result<(), String> {
    if neighbors.is_empty() {
        return Ok(());
    }
    let mut text = String::from("# Neighbouring areas for border continuity checks (side seed path)\n");
    for n in neighbors {
        text.push_str(&format!("{} {} {}\n", n.side.name(), n.seed, n.path.display()));
    }
    std::fs::write(dir.join("neighbors.txt"), text).map_err(|e| format!("neighbors.txt: {e}"))
}

/// Loads saved neighbour references from `dir/neighbors.txt` if present.
pub fn load_neighbors(dir: &Path) -> Result<Vec<Neighbor>, String> {
    let file = dir.join("neighbors.txt");
    if !file.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.splitn(3, char::is_whitespace);
        let side_s = parts.next().unwrap_or("");
        let seed_s = parts.next().unwrap_or("");
        let path_s = parts.next().unwrap_or("").trim();
        let side = Side::from_name(side_s).ok_or_else(|| format!("{}:{}: unknown side `{side_s}`", file.display(), idx + 1))?;
        let seed = seed_s.parse::<u64>().map_err(|e| format!("{}:{}: bad seed `{seed_s}`: {e}", file.display(), idx + 1))?;
        if path_s.is_empty() {
            return Err(format!("{}:{}: missing module path", file.display(), idx + 1));
        }
        out.push(Neighbor::load(side, PathBuf::from(path_s), seed)?);
    }
    Ok(out)
}

/// The module checked (`check::check`), made with `SEED`, and held to the laws: sizes agree,
/// distances never too long, surfaces have an inside, gates are real, no traps, NaNs unseen, and
/// (when `sketch` is given) the sketch's two laws (`sketch_materials`, `sketch_places`). ("The
/// same everywhere" asks for hashes kept, which a new area has none of; it is the areas of `areas/`
/// that keep them.) A law that cannot be asked of it (the module traps, or runs out of fuel) is
/// broken.
pub fn judge(module: &[u8], sketch: Option<&Sketch>, settings: Settings) -> Judged {
    judge_with_neighbors(module, sketch, &[], settings)
}

/// Same as [`judge`], additionally holding the area to `laws::borders_match` against every
/// neighbouring area in `neighbors`.
pub fn judge_with_neighbors(module: &[u8], sketch: Option<&Sketch>, neighbors: &[Neighbor], settings: Settings) -> Judged {
    let started = Instant::now();
    let mut judged = Judged::default();
    if let Err(why) = check::check(module) {
        judged.broken.push(("the engine's check".into(), why.join("; ")));
        return judged;
    }
    let mut area = match Wasm::new(module, SEED, Limits::default()).and_then(|mut w| w.describe().map(|d| (w, d))) {
        Ok((area, d)) => {
            judged.description = Some(d);
            area
        }
        Err(e) => {
            judged.broken.push(("describing itself".into(), format!("{e:#}")));
            return judged;
        }
    };
    let d = judged.description.clone().unwrap();
    let ran = |law: &str, verdict: plateau_engine::Result<laws::Verdict>, broken: &mut Vec<(String, String)>| match verdict {
        Ok(Ok(())) => {}
        Ok(Err(why)) => broken.push((law.into(), why)),
        Err(e) => broken.push((law.into(), format!("asking the area for this law failed: {e:#}"))),
    };
    // Each law asks a module that has not failed before (a failed one is not to be asked again).
    let fresh = || Wasm::new(module, SEED, Limits::default());
    macro_rules! with {
        ($law:expr, $f:expr) => {
            match fresh() {
                Ok(mut a) => ran($law, $f(&mut a), &mut judged.broken),
                Err(e) => judged.broken.push(($law.into(), format!("{e:#}"))),
            }
        };
    }
    with!("sizes agree", |a: &mut Wasm| laws::sizes_agree(a, &d, 1, settings.rays));
    with!("distances never too long", |a: &mut Wasm| laws::never_too_long(a, &d, 1, settings.rays));
    with!("surfaces have an inside", |a: &mut Wasm| laws::surfaces_have_an_inside(a, &d, 1, settings.rays));
    with!("materials go down", |a: &mut Wasm| laws::materials_go_down(a, &d, 1, settings.rays));
    if let Some(s) = sketch {
        with!("sketch materials", |a: &mut Wasm| laws::sketch_materials(a, s));
    }
    for n in neighbors {
        match Wasm::new(&n.module, n.seed, Limits::default()) {
            Ok(mut nb) => {
                let law_name = format!("borders match ({})", n.side.name());
                with!(&law_name, |a: &mut Wasm| laws::borders_match(a, n.side, &mut nb));
            }
            Err(e) => judged.broken.push(("borders match".into(), format!("loading neighbour {}: {e:#}", n.path.display()))),
        }
    }
    let mut world = World::new(&mut area, &d.places);
    match world.reach(&Size::ALL) {
        Ok(all) => {
            ran("gates are real", laws::gates_are_real(&mut world, &all, &d), &mut judged.broken);
            if let Some(s) = sketch {
                ran("sketch places", laws::sketch_places(&mut world, &all, &d, s), &mut judged.broken);
            }
            ran("no traps", laws::no_traps(&world, &all), &mut judged.broken);
        }
        Err(e) => judged.broken.push(("gates are real".into(), format!("working out where each size can go failed: {e:#}"))),
    }
    ran("NaNs unseen", laws::nans_unseen(module, SEED, &points(20_000, 30.0, 1)), &mut judged.broken);
    judged.seconds = started.elapsed().as_secs_f64();
    judged
}

/// Checks and judges a standalone composable `.wasm` element module:
/// - Wasmtime sandbox & export check (`check::check`), `describe()`, `element_bounds()`, `scene()`
/// - Runtime `tick()` and `interact()` smoke test
/// - Pure-Scene elements must emit at least one mesh+instance or light
/// - SDF landform elements (`element_bounds() == Some(bounds)`) are verified for bounds containment
///   (surface stays strictly inside `lo..hi`), non-empty interior, local Lipschitz safety
///   (`|d(p) - d(q)| <= 1.05 * |p - q|`), and `nans_unseen`.
pub fn judge_element(module: &[u8], _settings: Settings) -> Judged {
    let started = Instant::now();
    let mut judged = Judged::default();
    if let Err(why) = check::check(module) {
        judged.broken.push(("the engine's check".into(), why.join("; ")));
        return judged;
    }
    let mut elem = match Wasm::new(module, SEED, Limits::default()) {
        Ok(w) => w,
        Err(e) => {
            judged.broken.push(("loading element".into(), format!("{e:#}")));
            return judged;
        }
    };
    match elem.describe() {
        Ok(d) => judged.description = Some(d),
        Err(e) => {
            judged.broken.push(("describing itself".into(), format!("{e:#}")));
            return judged;
        }
    }
    let bounds = match elem.element_bounds() {
        Ok(b) => b,
        Err(e) => {
            judged.broken.push(("element bounds".into(), format!("{e:#}")));
            return judged;
        }
    };
    let scene = match elem.scene() {
        Ok(s) => s,
        Err(e) => {
            judged.broken.push(("procedural scene".into(), format!("{e:#}")));
            return judged;
        }
    };
    // Smoke-test runtime hooks in local coordinates.
    let ps = PlayerState {
        feet: [0.0, 0.0, 2.0],
        eye: [0.0, 1.5, 2.0],
        look: [0.0, 0.0, -1.0],
        velocity: [0.0; 3],
        size: 1.7,
        grounded: true,
        holding_use: false,
        target: scene.instances.iter().find(|i| i.id > 0).map_or(0, |i| i.id),
        aim: [0.0, 1.0, 0.0],
        time: 1.0,
    };
    if let Err(e) = elem.tick(0.016, &ps) {
        judged.broken.push(("runtime tick".into(), format!("{e:#}")));
    }
    if let Err(e) = elem.interact(&ps) {
        judged.broken.push(("runtime interact".into(), format!("{e:#}")));
    }

    if let Some(b) = bounds {
        // 1. Bounds containment: all 6 faces of lo..hi must be outside solid (d >= -1e-4).
        let mut face_pts = Vec::new();
        let steps = 8usize;
        for a in 0..3 {
            let b1 = (a + 1) % 3;
            let b2 = (a + 2) % 3;
            for side in [b.lo[a], b.hi[a]] {
                for i in 0..=steps {
                    for j in 0..=steps {
                        let u = i as f32 / steps as f32;
                        let v = j as f32 / steps as f32;
                        let mut p = [0.0f32; 3];
                        p[a] = side;
                        p[b1] = b.lo[b1] + (b.hi[b1] - b.lo[b1]) * u;
                        p[b2] = b.lo[b2] + (b.hi[b2] - b.lo[b2]) * v;
                        face_pts.push(p);
                    }
                }
            }
        }
        // 2. Interior sampling for non-empty solid + local Lipschitz check.
        let mut grid_pts = Vec::new();
        let gn = 8usize;
        for iz in 0..=gn {
            for iy in 0..=gn {
                for ix in 0..=gn {
                    grid_pts.push([
                        b.lo[0] + (b.hi[0] - b.lo[0]) * (ix as f32 / gn as f32),
                        b.lo[1] + (b.hi[1] - b.lo[1]) * (iy as f32 / gn as f32),
                        b.lo[2] + (b.hi[2] - b.lo[2]) * (iz as f32 / gn as f32),
                    ]);
                }
            }
        }
        if let Ok(mut sdf_wasm) = Wasm::new(module, SEED, Limits::default()) {
            let mut out = Vec::new();
            let mut clip_err = None;
            let mut has_solid = false;
            let mut lip_err = None;
            for size in [Size::Tiny.height(), Size::Person.height(), Size::Giant.height()] {
                if let Err(e) = sdf_wasm.sample(&face_pts, size, &mut out) {
                    judged.broken.push(("sampling SDF bounds".into(), format!("{e:#}")));
                    break;
                }
                for (&p, ans) in face_pts.iter().zip(&out) {
                    if !ans.distance.is_finite() {
                        clip_err = Some(format!("non-finite distance {} at {p:?}", ans.distance));
                        break;
                    }
                    if ans.distance < -1e-3 && clip_err.is_none() {
                        clip_err = Some(format!(
                            "solid extends past declared bounds lo={:?} hi={:?} at {p:?} (d={:.4} for size={size})",
                            b.lo, b.hi, ans.distance
                        ));
                    }
                }
                if let Err(e) = sdf_wasm.sample(&grid_pts, size, &mut out) {
                    judged.broken.push(("sampling SDF interior".into(), format!("{e:#}")));
                    break;
                }
                let mut pairs_a = Vec::new();
                let mut pairs_b = Vec::new();
                let eps = (size * 0.05).clamp(0.005, 0.15);
                for (&p, ans) in grid_pts.iter().zip(&out) {
                    if ans.distance < 0.0 {
                        has_solid = true;
                    }
                    if ans.distance.abs() < 0.5 {
                        pairs_a.push((p, ans.distance));
                        pairs_b.push([p[0] + eps, p[1], p[2]]);
                        pairs_b.push([p[0], p[1] + eps, p[2]]);
                        pairs_b.push([p[0], p[1], p[2] + eps]);
                    }
                }
                if !pairs_b.is_empty() && lip_err.is_none() {
                    let mut out_b = Vec::new();
                    if sdf_wasm.sample(&pairs_b, size, &mut out_b).is_ok() {
                        for (i, &(p, da)) in pairs_a.iter().enumerate() {
                            for k in 0..3 {
                                let db = out_b[i * 3 + k].distance;
                                let slope = (db - da).abs() / eps;
                                if slope > 1.08 {
                                    lip_err = Some(format!(
                                        "distances grow {slope:.2}x too fast near {p:?} (size={size}): scale distances down so |grad d| <= 1"
                                    ));
                                    break;
                                }
                            }
                            if lip_err.is_some() {
                                break;
                            }
                        }
                    }
                }
            }
            if let Some(why) = clip_err {
                judged.broken.push(("element bounds containment".into(), why));
            }
            if !has_solid {
                judged.broken.push(("element has solid".into(), "SDF element has no solid volume inside its declared bounds".into()));
            }
            if let Some(why) = lip_err {
                judged.broken.push(("distances never too long".into(), why));
            }
        }
        match laws::nans_unseen(module, SEED, &grid_pts) {
            Ok(Ok(())) => {}
            Ok(Err(why)) => judged.broken.push(("NaNs unseen".into(), why)),
            Err(e) => judged.broken.push(("NaNs unseen".into(), format!("{e:#}"))),
        }
    } else if scene.instances.is_empty() && scene.lights.is_empty() {
        judged.broken.push((
            "non-empty element".into(),
            "element has sdf_bounds() == None and an empty scene() (no instances or lights)".into(),
        ));
    }

    judged.seconds = started.elapsed().as_secs_f64();
    judged
}

/// Which law of `fit` an element made for a site is held to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FitLaw {
    /// The opening's if the site has one, else the ground's.
    Auto,
    Cavity,
    Ground,
    /// None: the element is only told of the site (a lamp beside a door, a rug within).
    Free,
}

impl FitLaw {
    pub fn from_name(s: &str) -> Option<FitLaw> {
        match s {
            "auto" => Some(FitLaw::Auto),
            "cavity" => Some(FitLaw::Cavity),
            "ground" => Some(FitLaw::Ground),
            "none" => Some(FitLaw::Free),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            FitLaw::Auto => "auto",
            FitLaw::Cavity => "cavity",
            FitLaw::Ground => "ground",
            FitLaw::Free => "none",
        }
    }
}

/// A site an element is made for: its measure, the world it was measured in if that is there (else
/// it is judged and pictured in what its measure tells of it, `fit::SiteRegion`), and which law of
/// fitting it is held to.
pub struct Site {
    pub context: SiteContext,
    pub world: Option<OpenWorld>,
    pub law: FitLaw,
}

impl Site {
    /// The site's surroundings, for one thread, with `scene`'s solid geometry in them.
    fn region(&self, scene: Arc<SceneWorld>) -> plateau_engine::Result<Solids<Box<dyn Region>>> {
        let region: Box<dyn Region> = match &self.world {
            Some(w) => Box::new(w.region()?),
            None => Box::new(SiteRegion::new(self.context.clone())),
        };
        Ok(Solids { region, world: scene })
    }

    fn scene(&self) -> Arc<SceneWorld> {
        self.world.as_ref().map_or_else(|| Arc::new(SceneWorld::default()), |w| w.scene.clone())
    }

    /// The law it comes to.
    fn held_to(&self) -> FitLaw {
        match self.law {
            FitLaw::Auto if self.context.cavity.is_some() => FitLaw::Cavity,
            FitLaw::Auto => FitLaw::Ground,
            law => law,
        }
    }
}

/// `judge_element`, and the element held to its site as well, where it is placed for it.
pub fn judge_element_at(module: &[u8], site: &Site, settings: Settings) -> Judged {
    let mut judged = judge_element(module, settings);
    if !judged.kept() {
        return judged;
    }
    let started = Instant::now();
    let (name, verdict) = match site.held_to() {
        FitLaw::Cavity => ("cavity fit", held_to_site(module, site, true)),
        FitLaw::Ground => ("ground fit", held_to_site(module, site, false)),
        _ => return judged,
    };
    match verdict {
        Ok(Ok(())) => {}
        Ok(Err(why)) => judged.broken.push((name.into(), why)),
        Err(e) => judged.broken.push((name.into(), format!("asking the element for this law failed: {e:#}"))),
    }
    judged.seconds += started.elapsed().as_secs_f64();
    judged
}

fn held_to_site(module: &[u8], site: &Site, cavity: bool) -> plateau_engine::Result<laws::Verdict> {
    let element = CompiledElement::load("element", module, Limits::default())?;
    let scene = site.scene();
    let mut region = site.region(scene.clone())?;
    if cavity {
        return Ok(cavity_fit(&mut region.region, &scene, &site.context, &element, SEED, site.context.frame)?.map(|_| ()));
    }
    if element.is_sdf() {
        return Ok(Ok(())); // A landform goes into the ground as it likes.
    }
    let made = element.compiled.instantiate(SEED)?.scene()?;
    let mut combined = Combined { region: &mut region.region, world: &scene };
    ground_fit(&mut combined, site.context.size, &made, site.context.frame)
}

/// Pictures of a site as whoever measured it sees it, bare or (`module`) with an element placed there
/// and, if `F` changes it, once more after `F`: each its file name and what it shows, for Claude.
pub fn site_pictures(site: &Site, module: Option<&[u8]>, into: &Path, settings: Settings) -> Result<Vec<(String, String)>, String> {
    let e = |e: plateau_engine::Error| format!("{e:#}");
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let c = &site.context;
    let h = c.size;
    let tf = c.frame;
    let (w, high) = (settings.view_width, settings.view_width * 9 / 16);
    // Local metres: whoever measured it is 1.7 m tall in them.
    let at = |p: [f32; 3]| tf.local_to_world_point(p);
    let (middle, wide) = match &c.cavity {
        Some(cavity) => (c.local(cavity.drop()), c.local(cavity.width().max(cavity.height()))),
        None => (0.6, 3.0),
    };
    let off = (1.1 * wide).max(2.0);
    let camera = |eye: [f32; 3], look: [f32; 3]| Camera { eye: at(eye), at: at(look), fov: 70.0, size: h };
    let mut views = vec![
        ("ahead", camera([0.0, middle.max(0.9) + 0.2, off], [0.0, middle, 0.0]), "seen from straight before it".to_string()),
        ("aside", camera([0.75 * off, middle.max(0.9) + 0.6, 0.8 * off], [0.0, middle, 0.0]), "seen from the right before it".to_string()),
    ];
    if let Some(cavity) = &c.cavity
        && c.local(cavity.depth) > 1.6
    {
        let deep = -(0.8 * c.local(cavity.depth)).min(2.5);
        views.push(("within", camera([0.0, middle, deep], [0.0, middle, 1.0]), "seen from within, looking out".to_string()));
    }
    let mut stages: Vec<(&str, &str, Arc<SceneWorld>)> = Vec::new();
    match module {
        None => stages.push(("site", "the site as it is", site.scene())),
        Some(module) => {
            let element = CompiledElement::load("element", module, Limits::default()).map_err(e)?;
            let moves = element.preview_scene.instances.iter().any(|i| i.id > 0);
            let stage = |pressed| staged(&site.scene(), c, &element, SEED, tf, pressed).map(Arc::new).map_err(e);
            stages.push(("fitted", "your element in its place", stage(false)?));
            if moves {
                stages.push(("pressed", "your element after F was pressed before it", stage(true)?));
            }
        }
    }
    let mut shown = Vec::new();
    for (stage, what, scene) in stages {
        let make = || site.region(scene.clone());
        for (view, camera, seen) in &views {
            let pixels = picture(&make, camera, w, high).map_err(e)?;
            let file = format!("{stage}-{view}.png");
            write_png(&into.join(&file), &pixels, w, high)?;
            shown.push((file, format!("{what}, {seen}")));
        }
        if c.cavity.is_none() {
            let p = Plan { middle: [tf.pos[0], tf.pos[2]], half: 3.5 * h, top: tf.pos[1] + c.overhead.map_or(4.0 * h, |o| o * 0.9), size: h };
            let side = settings.plan_width;
            let pixels = plan(&make, &p, side).map_err(e)?;
            let file = format!("{stage}-plan.png");
            write_png(&into.join(&file), &pixels, side, side)?;
            shown.push((file, format!("{what}, straight down, north (-z of the world) at the top")));
        }
    }
    Ok(shown)
}

fn write_png(path: &Path, pixels: &[[u8; 3]], width: usize, height: usize) -> Result<(), String> {
    let w = std::io::BufWriter::new(std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
    let mut encoder = png::Encoder::new(w, width as u32, height as u32);
    encoder.set_color(png::ColorType::Rgb);
    encoder.write_header().and_then(|mut h| h.write_image_data(pixels.as_flattened())).map_err(|e| e.to_string())
}

/// The area's pictures and reach maps, written into `into` as PNGs: each its file name and what it
/// shows, in words for Claude.
pub fn pictures(module: &[u8], into: &Path, settings: Settings) -> Result<Vec<(String, String)>, String> {
    let e = |e: plateau_engine::Error| format!("{e:#}");
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let compiled = Compiled::new(module, Limits::default()).map_err(e)?;
    let make = || compiled.instantiate(SEED);
    let mut area = make().map_err(e)?;
    let d = area.describe().map_err(e)?;
    let write = |name: &str, pixels: &[[u8; 3]], width: usize, height: usize| -> Result<String, String> {
        let file = format!("{name}.png");
        let w = std::io::BufWriter::new(std::fs::File::create(into.join(&file)).map_err(|e| e.to_string())?);
        let mut encoder = png::Encoder::new(w, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header().and_then(|mut h| h.write_image_data(pixels.as_flattened())).map_err(|e| e.to_string())?;
        Ok(file)
    };
    let mut shown = Vec::new();
    let (w, h) = (settings.view_width, settings.view_width * 9 / 16);
    for (name, camera) in views(&mut area, &d).map_err(e)? {
        let Camera { size, .. } = camera;
        let what = if name == "giant-over-the-area" {
            "the whole area from above its south edge, as a giant sees it".to_string()
        } else if name.ends_with("-at-the-start") {
            format!("standing at the start, looking north into the area, as {} sees it", a_size(size))
        } else {
            format!("a place you describe, looked at from near it, as {} sees it", a_size(size))
        };
        shown.push((write(&name, &picture(&make, &camera, w, h).map_err(e)?, w, h)?, what));
    }
    let mut world = World::new(&mut area, &d.places);
    let reaches = world.reaches().map_err(e)?;
    let places: Vec<_> = d.places.iter().map(|p| (p.size, p.at)).collect();
    let side = settings.plan_width;
    for (name, p, band) in plans(&d) {
        let mut pixels = plan(&make, &p, side).map_err(e)?;
        let about = if name == "plan" { "the whole area".to_string() } else { format!("the place {}", name.trim_start_matches("plan-")) };
        shown.push((write(&name, &pixels, side, side)?, format!("{about}, straight down, north at the top, cut away a little above it")));
        let m = map(&world, &reaches, &p, band, side);
        tint(&mut pixels, &m, &p, side, &places);
        let reach = name.replacen("plan", "reach", 1);
        shown.push((
            write(&reach, &pixels, side, side)?,
            format!(
                "{about}'s reach map: tinted orange where only those who have been tiny get to, yellow persons, purple giants; \
                 left as it is where anyone gets; greyed where nobody stands at these heights, darkened where nobody stands at \
                 all; each place ringed in its size's colour"
            ),
        ));
    }
    Ok(shown)
}

/// Renders stitched overhead (`seam-<side>.png`) and elevated perspective (`seam-view-<side>.png`)
/// pictures across each shared border with `neighbors`, so Claude and humans can inspect how the
/// two areas meet side by side.
pub fn seam_pictures(module: &[u8], neighbors: &[Neighbor], into: &Path, settings: Settings) -> Result<Vec<(String, String)>, String> {
    if neighbors.is_empty() {
        return Ok(Vec::new());
    }
    let e = |e: plateau_engine::Error| format!("{e:#}");
    std::fs::create_dir_all(into).map_err(|e| e.to_string())?;
    let mine = Arc::new(Compiled::new(module, Limits::default()).map_err(e)?);
    let write = |name: &str, pixels: &[[u8; 3]], width: usize, height: usize| -> Result<String, String> {
        let file = format!("{name}.png");
        let w = std::io::BufWriter::new(std::fs::File::create(into.join(&file)).map_err(|e| e.to_string())?);
        let mut encoder = png::Encoder::new(w, width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header().and_then(|mut h| h.write_image_data(pixels.as_flattened())).map_err(|e| e.to_string())?;
        Ok(file)
    };
    let mut shown = Vec::new();
    let side_px = settings.plan_width;
    let (w, h) = (settings.view_width, settings.view_width * 9 / 16);
    for n in neighbors {
        let nb = Arc::new(Compiled::new(&n.module, Limits::default()).map_err(e)?);
        let atlas = CompiledAtlas::new(vec![
            TileSpec { at: [0, 0], seed: SEED, compiled: mine.clone() },
            TileSpec { at: n.side.step(), seed: n.seed, compiled: nb },
        ])
        .map_err(e)?;
        let make = || atlas.instantiate();
        let p = seam_plan(n.side);
        let pixels = plan(&make, &p, side_px).map_err(e)?;
        let pname = format!("seam-{}", n.side.name());
        shown.push((
            write(&pname, &pixels, side_px, side_px)?,
            format!(
                "overhead view straddling the {} border ({}) where your area meets \"{}\"",
                n.side.name(),
                n.side.equation(),
                n.profile.area_name
            ),
        ));
        let cam = seam_camera(n.side);
        let vpixels = picture(&make, &cam, w, h).map_err(e)?;
        let vname = format!("seam-view-{}", n.side.name());
        shown.push((
            write(&vname, &vpixels, w, h)?,
            format!(
                "elevated perspective looking along the {} border ({}) where your area meets \"{}\"",
                n.side.name(),
                n.side.equation(),
                n.profile.area_name
            ),
        ));
    }
    Ok(shown)
}

fn a_size(size: f32) -> &'static str {
    if size < 0.1 {
        "a tiny person"
    } else if size < 5.0 {
        "a person"
    } else {
        "a giant"
    }
}

/// One round of the making: what was asked, and what came of the answer.
#[derive(Clone, Debug)]
pub struct Round {
    /// "write", "compile", "laws" or "look": what the round asked for.
    pub asked: &'static str,
    pub reply: Reply,
    /// What came of it: "no source", "does not build", "breaks: ...", "keeps every law".
    pub came: String,
}

/// An area made: the source of the last version that kept every law (written to `dir/area.rs`,
/// its module to `dir/area.wasm`), and every round.
#[derive(Clone, Debug)]
pub struct Made {
    pub kept: Option<String>,
    pub rounds: Vec<Round>,
    pub fixes: usize,
    pub looked: bool,
    pub seconds: f64,
}

impl Made {
    pub fn cost(&self) -> f64 {
        self.rounds.iter().map(|r| r.reply.cost).sum()
    }
}

/// What to say when the file did not build.
fn compiler_said(words: &str) -> String {
    format!(
        "It does not build. The compiler says:\n\n```\n{words}\n```\n\nAnswer with the whole of `area.rs` again, fixed, in one \
         ```rust block."
    )
}

/// What to say when it broke laws.
fn laws_said(broken: &[(String, String)]) -> String {
    let list: String = broken.iter().map(|(law, why)| format!("- **{law}**: {why}\n")).collect();
    format!(
        "It builds, but it breaks {}:\n\n{list}\nAnswer with the whole of `area.rs` again, fixed, in one ```rust block. Keep what \
         is right; change what the laws found.",
        if broken.len() == 1 { "a law" } else { "these laws" }
    )
}

/// What to say when it keeps every law: look at the pictures, then make it better.
fn look_said(dir: &Path, shown: &[(String, String)], has_sketch: bool, has_neighbors: bool) -> String {
    let list: String = shown.iter().map(|(file, what)| format!("- `{}`: {what}\n", dir.join(file).display())).collect();
    let sketch_note = if has_sketch {
        format!(
            " Compare `plan.png` and `reach.png` against the sketch at `{}` to make sure the layout, features and places match what \
             was drawn.",
            dir.join("sketch.png").display()
        )
    } else {
        String::new()
    };
    let neighbor_note = if has_neighbors {
        " Check the `seam-*.png` and `seam-view-*.png` pictures to verify the join with the neighbouring area looks completely natural and seamless."
    } else {
        ""
    };
    format!(
        "It builds and keeps every law. These are pictures of it, as the engine draws it (read them with your Read tool; start with \
         `giant-over-the-area.png`, `plan.png`, `reach.png`, and each place's view and reach map):\n\n\
         {list}\nLook at them as the one who asked for it would.{sketch_note}{neighbor_note} Does it look like what was asked? Is each size's world \
         worth walking: the tiny's, a person's, a giant's? Is anything missing, wrong, too plain, or broken up? Then answer with the \
         whole of `area.rs` again, made better, in one ```rust block, keeping every law. If it is already as good as you can make \
         it, answer with it unchanged."
    )
}

/// The first prompt, with optional sketch map and/or neighbouring area borders.
fn first_prompt(words: &str, sketch: Option<(&Path, &Sketch)>, neighbors: &[Neighbor]) -> String {
    let mut prompt = format!("Make this area: {words}\n\n");
    if let Some((path, s)) = sketch {
        prompt.push_str(&format!(
            "A sketch map from above is at `{}` — read it with your Read tool before writing. North is at the top (z = 0), south at \
             the bottom (z = 64), west on the left (x = 0), east on the right (x = 64). Below are the shapes the engine read from \
             the sketch in area metres:\n\n{}\
             Follow the sketch's layout and keep its two laws (materials within 3 m; orange places for the tiny only and purple \
             places for giants only), while making the world natural, rich and finished at every size.\n\n",
            path.display(),
            s.describe()
        ));
    }
    if !neighbors.is_empty() {
        prompt.push_str(
            "This area lies side by side with existing Plateau areas. Match every shared border below (ground height within 15 cm, \
             water level within 5 cm, and surface material continuous) so players of every size (`tiny`, `person`, `giant`) can walk \
             straight across with no visible or physical seam:\n\n",
        );
        for n in neighbors {
            prompt.push_str(&n.profile.describe_for_prompt(n.side));
        }
    }
    prompt.push_str("Answer with the whole of `area.rs` in one ```rust block.");
    prompt
}

/// Makes an area of `words` (and an optional `sketch` PNG) with `writer`, in `dir`: each round's
/// answer and source kept in `dir/rounds/`, the version kept as `dir/area.rs` and `dir/area.wasm`,
/// its pictures in `dir/pictures/`, and any sketch copied to `dir/sketch.png`. `say` is told how
/// it goes, a line at a time.
pub fn make(
    writer: &mut dyn Writer,
    builder: &Builder,
    words: &str,
    sketch_path: Option<&Path>,
    dir: &Path,
    settings: Settings,
    say: &mut dyn FnMut(&str),
) -> Result<Made, String> {
    make_with_neighbors(writer, builder, words, sketch_path, &[], dir, settings, say)
}

/// Same as [`make`], additionally briefing Claude on `neighbors` and enforcing `laws::borders_match`.
#[allow(clippy::too_many_arguments)]
pub fn make_with_neighbors(
    writer: &mut dyn Writer,
    builder: &Builder,
    words: &str,
    sketch_path: Option<&Path>,
    neighbors: &[Neighbor],
    dir: &Path,
    settings: Settings,
    say: &mut dyn FnMut(&str),
) -> Result<Made, String> {
    let started = Instant::now();
    let rounds_dir = dir.join("rounds");
    std::fs::create_dir_all(&rounds_dir).map_err(|e| e.to_string())?;
    save_neighbors(dir, neighbors)?;
    let sketch_info: Option<(PathBuf, Sketch)> = match sketch_path {
        Some(path) => {
            let parsed = Sketch::open(path)?;
            let copied = dir.join("sketch.png");
            std::fs::copy(path, &copied).map_err(|e| format!("{}: {e}", copied.display()))?;
            Some((copied, parsed))
        }
        None => None,
    };
    let sketch = sketch_info.as_ref().map(|(_, s)| s);
    let mut made = Made { kept: None, rounds: Vec::new(), fixes: 0, looked: false, seconds: 0.0 };
    let mut prompt = first_prompt(words, sketch_info.as_ref().map(|(p, s)| (p.as_path(), s)), neighbors);
    let mut asked = "write";
    let mut pictures_for: Option<PathBuf> = sketch_info.as_ref().map(|_| dir.to_path_buf());
    loop {
        say(&format!("round {}: asking Claude ({asked})", made.rounds.len() + 1));
        let reply = match writer.write(&prompt, pictures_for.as_deref()) {
            Ok(reply) => reply,
            Err(e) if made.kept.is_some() => {
                // A version that kept every law is never lost (CONCEPT.md): keep it if a later
                // round (such as the look) cannot reach Claude.
                say(&format!("round {}: stopped ({e}); keeping the last version that kept every law", made.rounds.len() + 1));
                break;
            }
            Err(e) => return Err(e),
        };
        let n = made.rounds.len() + 1;
        std::fs::write(rounds_dir.join(format!("{n}-{asked}-asked.md")), &prompt).ok();
        std::fs::write(rounds_dir.join(format!("{n}-{asked}-answer.md")), &reply.text).ok();
        say(&format!("round {n}: answered in {:.0} s, reckoned at ${:.2}", reply.seconds, reply.cost));
        if asked == "write" {
            pictures_for = None;
        }
        if asked == "look" {
            made.looked = true;
            pictures_for = None;
        }
        // What came of the answer, and what to ask next, if anything.
        let (came, next): (String, Option<(&'static str, String)>) = match source_of(&reply.text) {
            None => ("no source".into(), Some(("compile", "There is no ```rust block in your answer. Answer with the whole of `area.rs` in one ```rust block.".into()))),
            Some(source) => {
                std::fs::write(rounds_dir.join(format!("{n}.rs")), &source).ok();
                match builder.build(&source, &dir.join("build")) {
                    Err(words) => ("does not build".into(), Some(("compile", compiler_said(&words)))),
                    Ok(module) => {
                        let judged = judge_with_neighbors(&module, sketch, neighbors, settings);
                        if judged.kept() {
                            std::fs::write(dir.join("area.rs"), &source).map_err(|e| e.to_string())?;
                            std::fs::write(dir.join("area.wasm"), &module).map_err(|e| e.to_string())?;
                            made.kept = Some(source);
                            if made.looked {
                                ("keeps every law".into(), None)
                            } else {
                                let pics = dir.join("pictures");
                                let mut shown = pictures(&module, &pics, settings)?;
                                shown.extend(seam_pictures(&module, neighbors, &pics, settings)?);
                                if let Some((copied, _)) = &sketch_info {
                                    std::fs::copy(copied, pics.join("sketch.png")).ok();
                                }
                                pictures_for = Some(pics.clone());
                                ("keeps every law".into(), Some(("look", look_said(&pics, &shown, sketch.is_some(), !neighbors.is_empty()))))
                            }
                        } else {
                            let names: Vec<_> = judged.broken.iter().map(|b| b.0.clone()).collect();
                            (format!("breaks: {}", names.join(", ")), Some(("laws", laws_said(&judged.broken))))
                        }
                    }
                }
            }
        };
        say(&format!("round {n}: {came}"));
        made.rounds.push(Round { asked, reply, came });
        let Some((kind, words)) = next else { break };
        if kind != "look" {
            if made.fixes >= MOST_FIXES {
                break;
            }
            made.fixes += 1;
        }
        asked = kind;
        prompt = words;
    }
    // The pictures of what is kept, whether or not it was looked at since.
    if let Some(source) = &made.kept {
        let module = builder.build(source, &dir.join("build"))?;
        pictures(&module, &dir.join("pictures"), settings)?;
        seam_pictures(&module, neighbors, &dir.join("pictures"), settings)?;
    }
    made.seconds = started.elapsed().as_secs_f64();
    Ok(made)
}

/// Makes a standalone composable `.wasm` element from `words` into `dir` (`dir/element.rs`,
/// `dir/element.wasm`, `dir/rounds/...`), verifying it with `judge_element` and retrying up to
/// `MOST_FIXES` rounds.
pub fn make_element(
    writer: &mut dyn Writer,
    builder: &Builder,
    words: &str,
    dir: &Path,
    settings: Settings,
    say: &mut dyn FnMut(&str),
) -> Result<Made, String> {
    make_element_with(writer, builder, words, None, dir, settings, say)
}

/// Makes an element for `site`: Claude is given its measures and pictures of it (`dir/site/`), the
/// element is held to the site's law of fitting too (`judge_element_at`), and, kept, Claude is shown
/// it in its place (`dir/pictures/`) once, to make it better. The site's file goes beside the element
/// (`dir/element.site`), so it can be placed and judged there again.
pub fn make_element_at(
    writer: &mut dyn Writer,
    builder: &Builder,
    words: &str,
    site: &Site,
    dir: &Path,
    settings: Settings,
    say: &mut dyn FnMut(&str),
) -> Result<Made, String> {
    make_element_with(writer, builder, words, Some(site), dir, settings, say)
}

/// The first prompt for an element made for a site.
fn site_prompt(words: &str, site: &Site, dir: &Path, shown: &[(String, String)]) -> String {
    let list: String = shown.iter().map(|(file, what)| format!("- `{}`: {what}\n", dir.join(file).display())).collect();
    let law = match site.held_to() {
        FitLaw::Cavity => {
            "It is made **for this opening** and is held to it (\"Fitting an opening\" in your guide): its frame meets the wall all the way round, going a \
             little into it; a leaf, if it has one, shuts it all, and `F` opens and shuts it; and whoever could walk in through the bare opening still can.\n\n"
        }
        FitLaw::Ground => "It is made **for this ground** and is held to it: it stands on the ground in every quarter of its footprint, where that is not water.\n\n",
        _ => "It is made to go here, at the frame's origin; nothing holds it to the site but sense.\n\n",
    };
    format!(
        "Make this standalone composable Plateau element: {words}\n\n## Its site\n\n{}\n{law}Pictures of the site (read them with your Read tool before \
         writing):\n\n{list}\nAnswer with the whole of `element.rs` in one ```rust block.",
        site.context.describe_for_prompt()
    )
}

/// What to say when a site's element keeps every law: look at it in its place.
fn fitted_said(dir: &Path, shown: &[(String, String)]) -> String {
    let list: String = shown.iter().map(|(file, what)| format!("- `{}`: {what}\n", dir.join(file).display())).collect();
    format!(
        "It builds, keeps every law, and fits. These are pictures of it in its place, as the engine draws its solid parts (read them with your Read tool; \
         tints and what is not solid are not drawn):\n\n{list}\nLook at them as the one who asked for it would. Does it look like what was asked, and as if it \
         had always been there? Is anything missing, wrong, or too plain? Then answer with the whole of `element.rs` again, made better, in one ```rust block, \
         keeping every law. If it is already as good as you can make it, answer with it unchanged."
    )
}

fn make_element_with(
    writer: &mut dyn Writer,
    builder: &Builder,
    words: &str,
    site: Option<&Site>,
    dir: &Path,
    settings: Settings,
    say: &mut dyn FnMut(&str),
) -> Result<Made, String> {
    let started = Instant::now();
    let rounds_dir = dir.join("rounds");
    std::fs::create_dir_all(&rounds_dir).map_err(|e| e.to_string())?;
    let mut made = Made { kept: None, rounds: Vec::new(), fixes: 0, looked: false, seconds: 0.0 };
    let mut pictures_for: Option<PathBuf> = None;
    let mut prompt = match site {
        None => format!(
            "Make this standalone composable Plateau element: {words}\n\n\
             Answer with the whole of `element.rs` in one ```rust block."
        ),
        Some(site) => {
            site.context.save(&dir.join("element.site")).map_err(|e| format!("{e:#}"))?;
            let folder = dir.join("site");
            let shown = site_pictures(site, None, &folder, settings)?;
            let prompt = site_prompt(words, site, &folder, &shown);
            pictures_for = Some(folder);
            prompt
        }
    };
    let mut asked = "write";
    loop {
        say(&format!("round {}: asking Claude ({asked})", made.rounds.len() + 1));
        let reply = match writer.write(&prompt, pictures_for.as_deref()) {
            Ok(reply) => reply,
            Err(e) if made.kept.is_some() => {
                say(&format!("round {}: stopped ({e}); keeping the last valid version", made.rounds.len() + 1));
                break;
            }
            Err(e) => return Err(e),
        };
        let n = made.rounds.len() + 1;
        std::fs::write(rounds_dir.join(format!("{n}-{asked}-asked.md")), &prompt).ok();
        std::fs::write(rounds_dir.join(format!("{n}-{asked}-answer.md")), &reply.text).ok();
        say(&format!("round {n}: answered in {:.0} s, reckoned at ${:.2}", reply.seconds, reply.cost));
        pictures_for = None;
        if asked == "look" {
            made.looked = true;
        }

        let (came, next): (String, Option<(&'static str, String)>) = match source_of(&reply.text) {
            None => (
                "no source".into(),
                Some(("compile", "There is no ```rust block in your answer. Answer with the whole of `element.rs` in one ```rust block.".into())),
            ),
            Some(source) => {
                std::fs::write(rounds_dir.join(format!("{n}.rs")), &source).ok();
                match builder.build_element(&source, &dir.join("build")) {
                    Err(words) => (
                        "does not build".into(),
                        Some((
                            "compile",
                            format!("It does not build. The compiler says:\n\n```\n{words}\n```\n\nAnswer with the whole of `element.rs` again, fixed, in one ```rust block."),
                        )),
                    ),
                    Ok(module) => {
                        let judged = match site {
                            Some(site) => judge_element_at(&module, site, settings),
                            None => judge_element(&module, settings),
                        };
                        if judged.kept() {
                            std::fs::write(dir.join("element.rs"), &source).map_err(|e| e.to_string())?;
                            std::fs::write(dir.join("element.wasm"), &module).map_err(|e| e.to_string())?;
                            made.kept = Some(source);
                            match site {
                                Some(site) => {
                                    let folder = dir.join("pictures");
                                    let shown = site_pictures(site, Some(&module), &folder, settings)?;
                                    if made.looked {
                                        ("keeps every law".into(), None)
                                    } else {
                                        pictures_for = Some(folder.clone());
                                        ("keeps every law".into(), Some(("look", fitted_said(&folder, &shown))))
                                    }
                                }
                                None => ("keeps every law".into(), None),
                            }
                        } else {
                            let names: Vec<_> = judged.broken.iter().map(|b| b.0.clone()).collect();
                            let list: String = judged.broken.iter().map(|(law, why)| format!("- **{law}**: {why}\n")).collect();
                            (
                                format!("breaks: {}", names.join(", ")),
                                Some((
                                    "laws",
                                    format!("It builds, but it breaks:\n\n{list}\nAnswer with the whole of `element.rs` again, fixed, in one ```rust block."),
                                )),
                            )
                        }
                    }
                }
            }
        };
        say(&format!("round {n}: {came}"));
        made.rounds.push(Round { asked, reply, came });
        let Some((kind, words)) = next else { break };
        if kind != "look" {
            if made.fixes >= MOST_FIXES {
                break;
            }
            made.fixes += 1;
        }
        asked = kind;
        prompt = words;
    }
    made.seconds = started.elapsed().as_secs_f64();
    Ok(made)
}

/// The making in words, for `dir/log.md`.
pub fn log(words: &str, made: &Made) -> String {
    let mut s = format!(
        "# {words}\n\n{} after {} rounds ({} fixes, {}), {:.0} s, reckoned at ${:.2}.\n\n| Round | Asked | Took | Reckoned | Came |\n|---|---|---|---|---|\n",
        if made.kept.is_some() { "Kept every law" } else { "Never kept every law" },
        made.rounds.len(),
        made.fixes,
        if made.looked { "one look" } else { "no look" },
        made.seconds,
        made.cost()
    );
    for (i, r) in made.rounds.iter().enumerate() {
        s += &format!("| {} | {} | {:.0} s | ${:.2} | {} |\n", i + 1, r.asked, r.reply.seconds, r.reply.cost, r.came);
    }
    s
}
