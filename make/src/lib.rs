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
use std::time::Instant;

use plateau_engine::picture::{Camera, map, picture, plan, plans, tint, views};
use plateau_engine::reach::World;
use plateau_engine::sketch::Sketch;
use plateau_engine::{Compiled, Description, Limits, Region, Size, Wasm, check, laws, points};

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
        "{}\n\n# The SDK: `sdk/src/lib.rs`\n\n```rust\n{}```\n\n# The world's interface: `sdk/wit/region.wit`\n\n```wit\n{}```\n\n\
         # The walled garden: `areas/garden/area.rs`\n\n```rust\n{}```\n",
        read("make/GUIDE.md")?,
        read("sdk/src/lib.rs")?,
        read("sdk/wit/region.wit")?,
        read("areas/garden/area.rs")?
    ))
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

/// The module checked (`check::check`), made with `SEED`, and held to the laws: sizes agree,
/// distances never too long, surfaces have an inside, gates are real, no traps, NaNs unseen, and
/// (when `sketch` is given) the sketch's two laws (`sketch_materials`, `sketch_places`). ("The
/// same everywhere" asks for hashes kept, which a new area has none of; it is the areas of `areas/`
/// that keep them.) A law that cannot be asked of it (the module traps, or runs out of fuel) is
/// broken.
pub fn judge(module: &[u8], sketch: Option<&Sketch>, settings: Settings) -> Judged {
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
    if let Some(s) = sketch {
        with!("sketch materials", |a: &mut Wasm| laws::sketch_materials(a, s));
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
fn look_said(dir: &Path, shown: &[(String, String)], has_sketch: bool) -> String {
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
    format!(
        "It builds and keeps every law. These are pictures of it, as the engine draws it (read them with your Read tool; start with \
         `giant-over-the-area.png`, `plan.png`, `reach.png`, and each place's view and reach map):\n\n\
         {list}\nLook at them as the one who asked for it would.{sketch_note} Does it look like what was asked? Is each size's world \
         worth walking: the tiny's, a person's, a giant's? Is anything missing, wrong, too plain, or broken up? Then answer with the \
         whole of `area.rs` again, made better, in one ```rust block, keeping every law. If it is already as good as you can make \
         it, answer with it unchanged."
    )
}

/// The first prompt, with or without a sketch map.
fn first_prompt(words: &str, sketch: Option<(&Path, &Sketch)>) -> String {
    match sketch {
        None => format!("Make this area: {words}\n\nAnswer with the whole of `area.rs` in one ```rust block."),
        Some((path, s)) => format!(
            "Make this area: {words}\n\n\
             A sketch map from above is at `{}` — read it with your Read tool before writing. North is at the top (z = 0), south at \
             the bottom (z = 64), west on the left (x = 0), east on the right (x = 64). Below are the shapes the engine read from \
             the sketch in area metres:\n\n{}\
             Follow the sketch's layout and keep its two laws (materials within 3 m; orange places for the tiny only and purple \
             places for giants only), while making the world natural, rich and finished at every size.\n\n\
             Answer with the whole of `area.rs` in one ```rust block.",
            path.display(),
            s.describe()
        ),
    }
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
    let started = Instant::now();
    let rounds_dir = dir.join("rounds");
    std::fs::create_dir_all(&rounds_dir).map_err(|e| e.to_string())?;
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
    let mut prompt = first_prompt(words, sketch_info.as_ref().map(|(p, s)| (p.as_path(), s)));
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
                        let judged = judge(&module, sketch, settings);
                        if judged.kept() {
                            std::fs::write(dir.join("area.rs"), &source).map_err(|e| e.to_string())?;
                            std::fs::write(dir.join("area.wasm"), &module).map_err(|e| e.to_string())?;
                            made.kept = Some(source);
                            if made.looked {
                                ("keeps every law".into(), None)
                            } else {
                                let pics = dir.join("pictures");
                                let shown = pictures(&module, &pics, settings)?;
                                if let Some((copied, _)) = &sketch_info {
                                    std::fs::copy(copied, pics.join("sketch.png")).ok();
                                }
                                pictures_for = Some(pics.clone());
                                ("keeps every law".into(), Some(("look", look_said(&pics, &shown, sketch.is_some()))))
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
