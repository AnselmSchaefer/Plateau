//! `plateau`, Plateau's command line (POC_PLAN.md, steps 5 and 6).
//!
//! - `plateau make "<words>" [--sketch MAP.png] [--name NAME]`: Claude makes an area of the words
//!   (and optional sketch map), into `made/NAME/` (`area.rs` and `area.wasm` if it kept every law,
//!   its pictures, every round in `rounds/`, `sketch.png` if given, and `log.md`). Walk it with
//!   `cargo run --release -p plateau-play -- made/NAME/area.wasm`.
//! - `plateau test-set [NAME ...]`: the five requests of the test set (or those named), one after
//!   another, each into `made/NAME/`, and what came of each into `made/results.md`.
//! - `plateau check NAME [--sketch MAP.png]`: `made/NAME/area.rs`, changed by hand, built, checked
//!   and held to the laws (including `made/NAME/sketch.png` or `--sketch` if present); if it keeps
//!   every law, its module and pictures are made again.
//!
//! Claude is asked through the Claude Code command line with the owner's own login (`claude -p`).

use std::path::{Path, PathBuf};

use plateau_engine::atlas::Side;
use plateau_engine::sketch::Sketch;
use plateau_engine::{Limits, Wasm, laws};
use plateau_make::claude::Claude;
use plateau_make::{
    Builder, Made, Neighbor, SEED, Settings, element_guide, guide, judge_element, judge_with_neighbors, load_neighbors, log, make_element,
    make_with_neighbors, pictures, save_neighbors, seam_pictures,
};

/// The test set (POC_PLAN.md, step 5): five requests, kept, made again whenever the guide or the
/// SDK changes.
const TEST_SET: [(&str, &str); 5] = [
    ("table", "A giant's kitchen table with a village of tiny people living under it."),
    ("headland", "A headland ending in a cliff, a lighthouse on it, and smugglers' tunnels only the tiny fit."),
    ("pond", "A pond with lily pads the tiny use as stepping stones, and a heron's nest only giants can reach."),
    ("forest", "A forest where the trees are houses for the tiny and stools for giants."),
    ("castle", "A ruined castle whose walls are hills to the tiny and whose keep a giant can look into."),
];

fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).map(|i| args.get(i + 1).cloned().unwrap_or_else(|| usage()))
}

fn flags(args: &[String], name: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            out.push(args.get(i + 1).cloned().unwrap_or_else(|| usage()));
            i += 2;
        } else {
            i += 1;
        }
    }
    out
}

fn resolve_area(repo: &Path, spec: &str) -> Result<PathBuf, String> {
    let direct = PathBuf::from(spec);
    if direct.exists() {
        return Ok(direct);
    }
    let rel = repo.join(spec);
    if rel.exists() {
        return Ok(rel);
    }
    let made = repo.join("made").join(spec).join("area.wasm");
    if made.exists() {
        return Ok(made);
    }
    let carried = repo.join("areas/modules").join(format!("{spec}.wasm"));
    if carried.exists() {
        return Ok(carried);
    }
    Err(format!("area not found: `{spec}` (tried path, made/{spec}/area.wasm, areas/modules/{spec}.wasm)"))
}

fn parse_beside(repo: &Path, specs: &[String]) -> Result<Vec<Neighbor>, String> {
    let mut out = Vec::new();
    for spec in specs {
        let Some((side_s, area_s)) = spec.split_once(':') else {
            return Err(format!("--beside takes SIDE:AREA (e.g. west:pond-sketched), got `{spec}`"));
        };
        let side = Side::from_name(side_s).ok_or_else(|| format!("unknown side `{side_s}` (expected north, east, south, or west)"))?;
        let path = resolve_area(repo, area_s)?;
        out.push(Neighbor::load(side, path, SEED)?);
    }
    Ok(out)
}

fn main() {
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("make") => {
            let words = args.get(1).cloned().unwrap_or_else(|| usage());
            let name = flag(&args, "--name").unwrap_or_else(|| {
                words.split_whitespace().take(4).collect::<Vec<_>>().join("-").to_lowercase().replace(|c: char| !c.is_alphanumeric() && c != '-', "")
            });
            let sketch = flag(&args, "--sketch").map(PathBuf::from);
            let beside = flags(&args, "--beside");
            parse_beside(&repo, &beside).and_then(|neighbors| one(&repo, &name, &words, sketch.as_deref(), &neighbors).map(|made| println!("{}", log(&words, &made))))
        }
        Some("test-set") => {
            let named: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            let mut failed = None;
            for (name, words) in TEST_SET.iter().filter(|(n, _)| named.is_empty() || named.contains(n)) {
                match one(&repo, name, words, None, &[]) {
                    Ok(made) => {
                        let row = format!(
                            "| {name} | {words} | {} | {} | {} | {} | {:.0} min | ${:.2} |\n",
                            if made.kept.is_some() { "yes" } else { "no" },
                            made.rounds.len(),
                            made.fixes,
                            if made.looked { "yes" } else { "no" },
                            made.seconds / 60.0,
                            made.cost()
                        );
                        std::fs::write(repo.join("made").join(name).join("row.md"), row).ok();
                    }
                    Err(e) => {
                        eprintln!("{name}: {e}");
                        failed = Some(e);
                    }
                }
            }
            // Every area of the test set made so far, by this run or another.
            let mut results = String::from(
                "# The test set\n\n| Area | Request | Kept every law | Rounds | Fixes | Look | Took | Reckoned |\n|---|---|---|---|---|---|---|---|\n",
            );
            for (name, _) in TEST_SET {
                results += &std::fs::read_to_string(repo.join("made").join(name).join("row.md")).unwrap_or_default();
            }
            std::fs::write(repo.join("made/results.md"), &results).ok();
            println!("{results}");
            failed.map_or(Ok(()), Err)
        }
        Some("check") => {
            let name = args.get(1).cloned().unwrap_or_else(|| usage());
            let sketch = flag(&args, "--sketch").map(PathBuf::from);
            let beside = flags(&args, "--beside");
            parse_beside(&repo, &beside).and_then(|neighbors| check(&repo, &name, sketch.as_deref(), &neighbors))
        }
        Some("check-pair") => {
            let a = args.get(1).cloned().unwrap_or_else(|| usage());
            let side_s = args.get(2).cloned().unwrap_or_else(|| usage());
            let b = args.get(3).cloned().unwrap_or_else(|| usage());
            check_pair(&repo, &a, &side_s, &b)
        }
        Some("make-element") => {
            let words = args.get(1).cloned().unwrap_or_else(|| usage());
            let name = flag(&args, "--name").unwrap_or_else(|| {
                words.split_whitespace().take(4).collect::<Vec<_>>().join("-").to_lowercase().replace(|c: char| !c.is_alphanumeric() && c != '-', "")
            });
            one_element(&repo, &name, &words).map(|made| println!("{}", log(&words, &made)))
        }
        Some("check-element") => {
            let name = args.get(1).cloned().unwrap_or_else(|| usage());
            check_element(&repo, &name)
        }
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("plateau: {e}");
        std::process::exit(1);
    }
}

fn usage() -> ! {
    eprintln!(
        "plateau make \"<words>\" [--sketch MAP.png] [--name NAME] [--beside SIDE:AREA ...]\n\
         plateau make-element \"<words>\" [--name NAME]\n\
         plateau test-set [NAME ...]\n\
         plateau check NAME [--sketch MAP.png] [--beside SIDE:AREA ...]\n\
         plateau check-pair AREA_A SIDE AREA_B\n\
         plateau check-element NAME"
    );
    std::process::exit(2)
}

/// One area made, into `made/NAME/`, the folder emptied first.
fn one(repo: &Path, name: &str, words: &str, sketch: Option<&Path>, neighbors: &[Neighbor]) -> Result<Made, String> {
    let dir = repo.join("made").join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Its own work folder, so areas can be made side by side.
    let work = repo.join("target/plateau-make").join(name);
    let builder = Builder::new(repo, &work)?;
    let guide_file = work.join("guide.md");
    std::fs::write(&guide_file, guide(repo)?).map_err(|e| e.to_string())?;
    let mut claude = Claude::new(&guide_file, &dir);
    let made = make_with_neighbors(&mut claude, &builder, words, sketch, neighbors, &dir, Settings::default(), &mut |line| eprintln!("{name}: {line}"))?;
    std::fs::write(dir.join("log.md"), log(words, &made)).map_err(|e| e.to_string())?;
    // The build's own copies are not kept; the rounds and the kept version are.
    std::fs::remove_dir_all(dir.join("build")).ok();
    Ok(made)
}

/// `made/NAME/area.rs` built and judged; kept (its module and pictures made again) if it keeps
/// every law (including any saved or explicit neighbour border constraints).
fn check(repo: &Path, name: &str, sketch_path: Option<&Path>, explicit_neighbors: &[Neighbor]) -> Result<(), String> {
    let dir = repo.join("made").join(name);
    let source = std::fs::read_to_string(dir.join("area.rs")).map_err(|e| format!("{}: {e}", dir.join("area.rs").display()))?;
    let work = repo.join("target/plateau-make").join(name);
    let module = Builder::new(repo, &work)?.build(&source, &work.join("check"))?;
    let auto_sketch = dir.join("sketch.png");
    let sketch = match sketch_path {
        Some(p) => Some(Sketch::open(p)?),
        None if auto_sketch.exists() => Some(Sketch::open(&auto_sketch)?),
        None => None,
    };
    let neighbors = if explicit_neighbors.is_empty() { load_neighbors(&dir)? } else { explicit_neighbors.to_vec() };
    let judged = judge_with_neighbors(&module, sketch.as_ref(), &neighbors, Settings::default());
    if !judged.kept() {
        let broken: String = judged.broken.iter().map(|(law, why)| format!("\n- {law}: {why}")).collect();
        return Err(format!("{name} breaks:{broken}"));
    }
    if !explicit_neighbors.is_empty() {
        save_neighbors(&dir, explicit_neighbors)?;
    }
    std::fs::write(dir.join("area.wasm"), &module).map_err(|e| e.to_string())?;
    pictures(&module, &dir.join("pictures"), Settings::default())?;
    seam_pictures(&module, &neighbors, &dir.join("pictures"), Settings::default())?;
    println!("{name} keeps every law ({:.0} s); area.wasm and pictures made again", judged.seconds);
    Ok(())
}

/// Checks `laws::borders_match` between `area_a`'s `side` edge and `area_b`'s opposite edge.
fn check_pair(repo: &Path, area_a: &str, side_s: &str, area_b: &str) -> Result<(), String> {
    let side = Side::from_name(side_s).ok_or_else(|| format!("unknown side `{side_s}` (expected north, east, south, or west)"))?;
    let path_a = resolve_area(repo, area_a)?;
    let path_b = resolve_area(repo, area_b)?;
    let bytes_a = std::fs::read(&path_a).map_err(|e| format!("{}: {e}", path_a.display()))?;
    let bytes_b = std::fs::read(&path_b).map_err(|e| format!("{}: {e}", path_b.display()))?;
    let mut wasm_a = Wasm::new(&bytes_a, SEED, Limits::default()).map_err(|e| format!("{}: {e:#}", path_a.display()))?;
    let mut wasm_b = Wasm::new(&bytes_b, SEED, Limits::default()).map_err(|e| format!("{}: {e:#}", path_b.display()))?;
    match laws::borders_match(&mut wasm_a, side, &mut wasm_b).map_err(|e| format!("{e:#}"))? {
        Ok(()) => {
            println!("borders match: {} ({}) <-> {} ({})", path_a.display(), side.name(), path_b.display(), side.opposite().name());
            Ok(())
        }
        Err(why) => Err(why),
    }
}

/// One standalone composable element made into `made/elements/NAME/`.
fn one_element(repo: &Path, name: &str, words: &str) -> Result<Made, String> {
    let dir = repo.join("made/elements").join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let work = repo.join("target/plateau-make/elements").join(name);
    let builder = Builder::new(repo, &work)?;
    let guide_file = work.join("element-guide.md");
    std::fs::write(&guide_file, element_guide(repo)?).map_err(|e| e.to_string())?;
    let mut claude = Claude::new(&guide_file, &dir);
    let made = make_element(&mut claude, &builder, words, &dir, Settings::default(), &mut |line| eprintln!("{name}: {line}"))?;
    std::fs::write(dir.join("log.md"), log(words, &made)).map_err(|e| e.to_string())?;
    std::fs::remove_dir_all(dir.join("build")).ok();
    Ok(made)
}

/// Verifies `made/elements/NAME/element.rs` (or `elements/NAME/element.rs`) and writes its `.wasm`.
fn check_element(repo: &Path, name: &str) -> Result<(), String> {
    let made_dir = repo.join("made/elements").join(name);
    let carried_dir = repo.join("elements").join(name);
    let (src_path, out_wasm) = if made_dir.join("element.rs").exists() {
        (made_dir.join("element.rs"), made_dir.join("element.wasm"))
    } else if carried_dir.join("element.rs").exists() {
        (carried_dir.join("element.rs"), repo.join("elements/modules").join(format!("{name}.wasm")))
    } else {
        return Err(format!("no element.rs found in {} or {}", made_dir.display(), carried_dir.display()));
    };
    let source = std::fs::read_to_string(&src_path).map_err(|e| format!("{}: {e}", src_path.display()))?;
    let work = repo.join("target/plateau-make/elements").join(name);
    let module = Builder::new(repo, &work)?.build_element(&source, &work.join("check"))?;
    let judged = judge_element(&module, Settings::default());
    if !judged.kept() {
        let broken: String = judged.broken.iter().map(|(law, why)| format!("\n- {law}: {why}")).collect();
        return Err(format!("{name} breaks:{broken}"));
    }
    if let Some(parent) = out_wasm.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&out_wasm, &module).map_err(|e| e.to_string())?;
    println!("{name} keeps every element law ({:.2} s); wrote {}", judged.seconds, out_wasm.display());
    Ok(())
}
