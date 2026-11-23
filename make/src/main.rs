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

use plateau_engine::sketch::Sketch;
use plateau_make::claude::Claude;
use plateau_make::{Builder, Made, Settings, guide, judge, log, make, pictures};

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
            one(&repo, &name, &words, sketch.as_deref()).map(|made| println!("{}", log(&words, &made)))
        }
        Some("test-set") => {
            let named: Vec<&str> = args[1..].iter().map(String::as_str).collect();
            let mut failed = None;
            for (name, words) in TEST_SET.iter().filter(|(n, _)| named.is_empty() || named.contains(n)) {
                match one(&repo, name, words, None) {
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
            check(&repo, &name, sketch.as_deref())
        }
        _ => usage(),
    };
    if let Err(e) = result {
        eprintln!("plateau: {e}");
        std::process::exit(1);
    }
}

fn usage() -> ! {
    eprintln!("plateau make \"<words>\" [--sketch MAP.png] [--name NAME]\nplateau test-set [NAME ...]\nplateau check NAME [--sketch MAP.png]");
    std::process::exit(2)
}

/// One area made, into `made/NAME/`, the folder emptied first.
fn one(repo: &Path, name: &str, words: &str, sketch: Option<&Path>) -> Result<Made, String> {
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
    let made = make(&mut claude, &builder, words, sketch, &dir, Settings::default(), &mut |line| eprintln!("{name}: {line}"))?;
    std::fs::write(dir.join("log.md"), log(words, &made)).map_err(|e| e.to_string())?;
    // The build's own copies are not kept; the rounds and the kept version are.
    std::fs::remove_dir_all(dir.join("build")).ok();
    Ok(made)
}

/// `made/NAME/area.rs` built and judged; kept (its module and pictures made again) if it keeps
/// every law.
fn check(repo: &Path, name: &str, sketch_path: Option<&Path>) -> Result<(), String> {
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
    let judged = judge(&module, sketch.as_ref(), Settings::default());
    if !judged.kept() {
        let broken: String = judged.broken.iter().map(|(law, why)| format!("\n- {law}: {why}")).collect();
        return Err(format!("{name} breaks:{broken}"));
    }
    std::fs::write(dir.join("area.wasm"), &module).map_err(|e| e.to_string())?;
    pictures(&module, &dir.join("pictures"), Settings::default())?;
    println!("{name} keeps every law ({:.0} s); area.wasm and pictures made again", judged.seconds);
    Ok(())
}
