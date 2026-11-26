//! Phase E (`WORLD_PLAN.md`): the module map in `docs/MODULES.md` is made from the code, never
//! written by hand, so it cannot drift: each crate's modules, what each says it is (its first
//! comment's first sentence), and which modules it uses, in its own crate and in the engine. Kept as
//! the chunk hashes are: this test fails if the file is not what the code makes;
//! `PLATEAU_KEEP=1 cargo test -p plateau-engine --test docs` writes it again.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The crates mapped: their package name, folder, and the name other crates use them by.
const CRATES: [(&str, &str, &str); 4] =
    [("plateau-sdk", "sdk", "plateau_sdk"), ("plateau-engine", "engine", "plateau_engine"), ("plateau-play", "play", "plateau_play"), ("plateau-make", "make", "plateau_make")];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Every `.rs` file under `src`, sorted, as (module path, file).
fn modules(src: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![src.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path.strip_prefix(src).unwrap().with_extension("");
                let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                let name = match parts.last().map(String::as_str) {
                    Some("mod") => parts[..parts.len() - 1].join("::"),
                    _ => parts.join("::"),
                };
                out.push((name, path));
            }
        }
    }
    out.sort();
    out
}

/// The first sentence of the file's first comment.
fn what_it_is(text: &str) -> String {
    let mut words = Vec::new();
    for line in text.lines() {
        let t = line.trim_start();
        let body = ["//!", "///", "//"].iter().find_map(|p| t.strip_prefix(p));
        match body {
            Some(b) if b.trim().is_empty() && !words.is_empty() => break,
            Some(b) => words.push(b.trim().to_string()),
            None if words.is_empty() && t.is_empty() => continue,
            None => break,
        }
    }
    let joined = words.join(" ");
    let end = joined.find(". ").map_or(joined.len(), |i| i + 1);
    joined[..end].replace('|', "\\|").trim().to_string()
}

/// Which modules `text` uses: of its own crate (`crate::x`, `super::x`, a sibling `x::` it
/// declares), and of the engine and SDK (`plateau_engine::x`).
fn uses(text: &str, own: &BTreeSet<String>, me: &str, crates: &[(&str, &str, &str)]) -> Vec<String> {
    let mut found = BTreeSet::new();
    let code: String = text.lines().filter(|l| !l.trim_start().starts_with("//")).collect::<Vec<_>>().join("\n");
    let word = |s: &str| s.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect::<String>();
    for (i, _) in code.match_indices("crate::") {
        let before = code[..i].chars().last();
        if before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let m = word(&code[i + 7..]);
        if own.contains(&m) && m != me {
            found.insert(format!("`{m}`"));
        }
    }
    // A sibling named by its own path (`scene::spawn`, `use carry::X` at the crate's root).
    for m in own {
        if m == me {
            continue;
        }
        let pat = format!("{m}::");
        let named = code.match_indices(&pat).any(|(i, _)| !code[..i].chars().last().is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':'));
        if named {
            found.insert(format!("`{m}`"));
        }
    }
    // `use crate::{a, b::c}` lists.
    for (i, _) in code.match_indices("crate::{") {
        let rest = &code[i + 8..];
        let list = &rest[..rest.find('}').unwrap_or(0)];
        for item in list.split(',') {
            let m = word(item.trim());
            if own.contains(&m) && m != me {
                found.insert(format!("`{m}`"));
            }
        }
    }
    for &(package, _, ident) in crates {
        for (i, _) in code.match_indices(&format!("{ident}::")) {
            let rest = &code[i + ident.len() + 2..];
            let names: Vec<String> = if let Some(list) = rest.strip_prefix('{') {
                list[..list.find('}').unwrap_or(0)].split(',').map(|s| word(s.trim())).collect()
            } else {
                vec![word(rest)]
            };
            for m in names.into_iter().filter(|m| !m.is_empty() && m.chars().next().is_some_and(|c| c.is_ascii_lowercase())) {
                found.insert(format!("{}::`{m}`", package.trim_start_matches("plateau-")));
            }
        }
    }
    found.into_iter().collect()
}

fn module_map() -> String {
    let root = repo_root();
    let mut out = String::from(
        "# Plateau's modules\n\n\
         Made from the code by `engine/tests/docs.rs`, never by hand: each crate's modules, what each \
         says it is (its first comment's first sentence), and which modules it uses (in its own crate, \
         and `engine::`/`sdk::` across crates). `PLATEAU_KEEP=1 cargo test -p plateau-engine --test docs` \
         writes it again; the test fails while it is stale. The concepts are in `ARCHITECTURE.md`.\n",
    );
    for (package, dir, _) in CRATES {
        let src = root.join(dir).join("src");
        let mods = modules(&src);
        let own: BTreeSet<String> = mods.iter().map(|(m, _)| m.split("::").next().unwrap().to_string()).collect();
        out += &format!("\n## {package} (`{dir}/`)\n\n| Module | What it is | Uses |\n|---|---|---|\n");
        for (name, file) in &mods {
            let text = std::fs::read_to_string(file).unwrap();
            let top = name.split("::").next().unwrap();
            let others: Vec<_> = CRATES.iter().copied().filter(|c| c.0 != package).collect();
            let used = uses(&text, &own, top, &others);
            out += &format!("| `{name}` | {} | {} |\n", what_it_is(&text), used.join(", "));
        }
    }
    out
}

#[test]
fn the_module_map_is_made_from_the_code() {
    let path = repo_root().join("docs/MODULES.md");
    let made = module_map();
    if std::env::var_os("PLATEAU_KEEP").is_some() {
        std::fs::write(&path, &made).unwrap();
        return;
    }
    let kept = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(kept == made, "docs/MODULES.md is not what the code makes: PLATEAU_KEEP=1 cargo test -p plateau-engine --test docs");
}
