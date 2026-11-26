//! Which modules a saved world was made of (`WORLD_PLAN.md`, phase D).
//!
//! A save's edits, water and placed elements are only meaningful against the very modules they were
//! made on: an area rebuilt with a changed `sample` gives the same carves a different hill to cut.
//! So a save (`plateau-world 2`) names each module it uses by a path relative to the world's root
//! (the folder its modules are looked for under, as `OpenWorld::open` and `ElementCatalog` have it)
//! and by the SHA-256 of the module's bytes:
//!
//! ```text
//! module areas/modules/garden.wasm sha256 6f1c…
//! ```
//!
//! On loading, a module whose bytes no longer match is refused, naming it, unless the player asks
//! to go on anyway (`ModuleCheck::AcceptChanged`), in which case it is reported as changed. Saves of
//! version 1 carry no `module` records: their modules are taken as they are.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use sha2::{Digest, Sha256};
use wasmtime::{bail, format_err};

use crate::Result;
use crate::edit::WorldSave;

/// One module a save was made of: where it is, relative to the world's root, and its bytes' hash.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct SavedModule {
    pub path: String,
    /// SHA-256 of the module's bytes, 64 lowercase hex digits.
    pub sha256: String,
}

/// What to do with a module whose bytes are not those the save was made with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModuleCheck {
    /// Refuse it, naming the module (the default).
    #[default]
    Strict,
    /// Load it anyway, and say it changed.
    AcceptChanged,
}

/// A module's bytes as read for a save, and whether they are the ones it was saved with.
#[derive(Clone, Debug)]
pub struct ModuleRead {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
    /// True when the save recorded a different hash and `ModuleCheck::AcceptChanged` let it through.
    pub changed: bool,
}

/// SHA-256 of `bytes` as 64 lowercase hex digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push(char::from_digit((b >> 4) as u32, 16).unwrap());
        s.push(char::from_digit((b & 0xf) as u32, 16).unwrap());
    }
    s
}

/// Where a module a save names is: as it is named, or under `root`.
pub fn locate(module: &str, root: &Path) -> Result<PathBuf> {
    let as_named = PathBuf::from(module);
    let under_root = root.join(module);
    [as_named, under_root]
        .into_iter()
        .find(|p| p.is_file())
        .ok_or_else(|| format_err!("{module}: not found (nor under {})", root.display()))
}

/// `module` as a path relative to `root`, with `/` between its parts, if it lies under `root`;
/// otherwise as it is. Paths are compared as written (made absolute and with `.` and `..` folded
/// away), and, failing that, as the file system resolves them.
pub fn relative_to_root(module: &str, root: &Path) -> String {
    let path = Path::new(module);
    if path.is_relative() && !module.split(['/', '\\']).any(|part| part == "..") {
        return module.replace('\\', "/");
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let absolute = |p: &Path| if p.is_absolute() { p.to_path_buf() } else { cwd.join(p) };
    let candidates = [
        (lexical(&absolute(path)), lexical(&absolute(root))),
        (
            std::fs::canonicalize(path).unwrap_or_else(|_| absolute(path)),
            std::fs::canonicalize(root).unwrap_or_else(|_| absolute(root)),
        ),
    ];
    for (p, r) in candidates {
        if let Ok(rel) = p.strip_prefix(&r) {
            let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
            if !parts.is_empty() {
                return parts.join("/");
            }
        }
    }
    module.to_string()
}

/// `p` with `.` dropped and each `..` taking away the part before it, without asking the disk.
fn lexical(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

impl WorldSave {
    /// The modules the save's tiles and placed elements name, each once, in order.
    pub fn named_modules(&self) -> Vec<String> {
        let mut names: Vec<String> = self.tiles.iter().map(|t| t.module.clone()).chain(self.elements.iter().map(|e| e.module.clone())).collect();
        names.sort();
        names.dedup();
        names
    }

    /// The hash the save recorded for `module`, if any.
    pub fn recorded_hash(&self, module: &str) -> Option<&str> {
        self.modules.iter().find(|m| m.path == module).map(|m| m.sha256.as_str())
    }

    /// Makes the save stand on its own before it is written: every module its tiles and elements
    /// name is renamed relative to `root` where it lies under it, and its bytes' hash recorded.
    ///
    /// A module that cannot be read keeps the hash the save already had for it (loaded from an
    /// earlier save); one with neither is an error, naming it.
    pub fn seal(&mut self, root: &Path) -> Result<()> {
        for t in &mut self.tiles {
            t.module = relative_to_root(&t.module, root);
        }
        for e in &mut self.elements {
            e.module = relative_to_root(&e.module, root);
        }
        let before: BTreeMap<String, String> =
            self.modules.iter().map(|m| (relative_to_root(&m.path, root), m.sha256.clone())).collect();
        let mut modules = Vec::new();
        for name in self.named_modules() {
            let sha256 = match locate(&name, root).and_then(|p| std::fs::read(&p).map_err(|e| format_err!("{}: {e}", p.display()))) {
                Ok(bytes) => sha256_hex(&bytes),
                Err(e) => match before.get(&name) {
                    Some(h) => h.clone(),
                    None => bail!("cannot record module {name} in the save: {e:#}"),
                },
            };
            modules.push(SavedModule { path: name, sha256 });
        }
        self.modules = modules;
        Ok(())
    }

    /// Seals the save against `root` (`seal`) and writes it to `path`.
    pub fn save_sealed(&mut self, path: &Path, root: &Path) -> Result<()> {
        self.seal(root)?;
        self.save(path)
    }

    /// Reads `module` as the save names it (as named, or under `root`), and checks its bytes against
    /// the hash the save recorded for it, if it recorded one.
    pub fn read_module(&self, module: &str, root: &Path, check: ModuleCheck) -> Result<ModuleRead> {
        let path = locate(module, root)?;
        let bytes = std::fs::read(&path).map_err(|e| format_err!("{}: {e}", path.display()))?;
        let changed = check_bytes(self.recorded_hash(module), module, &bytes, check)?;
        Ok(ModuleRead { path, bytes, changed })
    }

    /// The modules the save names whose bytes, under `root`, are no longer those it recorded.
    /// Modules that cannot be found are left to whoever loads them to report.
    pub fn changed_modules(&self, root: &Path) -> Vec<String> {
        self.named_modules()
            .into_iter()
            .filter(|name| {
                let Some(recorded) = self.recorded_hash(name) else { return false };
                locate(name, root).ok().and_then(|p| std::fs::read(p).ok()).is_some_and(|bytes| sha256_hex(&bytes) != recorded)
            })
            .collect()
    }
}

/// Checks `bytes` of `module` against `recorded` (if any): `Ok(false)` if they match or nothing
/// was recorded, `Ok(true)` if they differ and `check` lets it through, an error naming the module
/// otherwise.
pub fn check_bytes(recorded: Option<&str>, module: &str, bytes: &[u8], check: ModuleCheck) -> Result<bool> {
    let Some(recorded) = recorded else { return Ok(false) };
    let now = sha256_hex(bytes);
    if now == recorded {
        return Ok(false);
    }
    match check {
        ModuleCheck::AcceptChanged => Ok(true),
        ModuleCheck::Strict => bail!(
            "module {module} has changed since this world was saved (saved sha256 {}…, now {}…): \
             its edits and elements were made on the old one. Rebuild it from the same source, or \
             open the world with --accept-changed-modules to load it anyway",
            &recorded[..recorded.len().min(12)],
            &now[..12]
        ),
    }
}

/// Parses a `module <path> sha256 <hex>` record.
pub(crate) fn parse_module_line(line: &str, line_no: usize) -> Result<SavedModule> {
    let tok: Vec<&str> = line.split_whitespace().collect();
    if tok.len() != 4 || tok[0] != "module" || tok[2] != "sha256" {
        bail!("line {line_no}: expected `module <path> sha256 <hex>`, got `{line}`");
    }
    let sha256 = tok[3].to_string();
    if sha256.len() != 64 || !sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
        bail!("line {line_no}: module sha256 must be 64 lowercase hex digits, got `{sha256}`");
    }
    Ok(SavedModule { path: tok[1].to_string(), sha256 })
}

/// Writes the save's `module` records, one a line.
pub(crate) fn write_module_records(modules: &[SavedModule], out: &mut String) {
    for m in modules {
        out.push_str(&format!("module {} sha256 {}\n", m.path, m.sha256));
    }
}
