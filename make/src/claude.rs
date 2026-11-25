//! Claude, asked through the Claude Code command line (`claude -p`), with the owner's own login
//! (POC_PLAN.md, "Decisions": a subscription, not an API key). One conversation an area: the first
//! question starts a session of a known id, every later one resumes it, so Claude has the whole
//! making in mind. The guide is the session's instructions in place of Claude Code's own. Claude
//! has no tools but when shown pictures, and then only Read.
//!
//! An `ANTHROPIC_API_KEY` in the environment would be used in place of the login without a word,
//! so it is taken out of Claude Code's.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::{Reply, Writer};

/// The model and effort of POC_PLAN.md, "Decisions".
pub const MODEL: &str = "claude-opus-5-5";
pub const EFFORT: &str = "medium";
/// Default Antigravity CLI (`agy`) model when `--writer agy` is selected.
pub const AGY_MODEL: &str = "gemini-4-neon-high";

pub struct Claude {
    /// The file the guide is in (`crate::guide`), given as the instructions every time.
    guide: PathBuf,
    /// Where Claude Code runs: its sessions are kept by the folder they ran in, so it is the same
    /// for every question of one area.
    folder: PathBuf,
    session: String,
    started: bool,
    /// What the session is reckoned to have cost so far: Claude Code's `total_cost_usd` is the
    /// whole session's, every question before included, not the last question's alone.
    spent: f64,
}

impl Claude {
    /// Claude for one area, made in `folder`, instructed by the guide in the file `guide`.
    pub fn new(guide: &Path, folder: &Path) -> Claude {
        Claude { guide: guide.to_path_buf(), folder: folder.to_path_buf(), session: uuid(), started: false, spent: 0.0 }
    }
}

impl Writer for Claude {
    fn write(&mut self, prompt: &str, pictures: Option<&Path>) -> Result<Reply, String> {
        let started = Instant::now();
        let mut c = Command::new("claude");
        c.current_dir(&self.folder)
            .env_remove("ANTHROPIC_API_KEY")
            .stdin(Stdio::null())
            .arg("-p")
            .arg(prompt)
            .args(["--model", MODEL, "--effort", EFFORT, "--output-format", "json", "--system-prompt-file"])
            .arg(&self.guide);
        if self.started {
            c.args(["--resume", &self.session]);
        } else {
            c.args(["--session-id", &self.session]);
        }
        match pictures {
            Some(folder) => {
                c.args(["--tools", "Read", "--allowedTools", "Read", "--max-turns", "40", "--add-dir"]).arg(folder);
            }
            None => {
                c.args(["--tools", "", "--max-turns", "2"]);
            }
        }
        let out = c.output().map_err(|e| format!("claude: {e}"))?;
        let json: serde_json::Value = serde_json::from_slice(&out.stdout).map_err(|e| {
            format!("claude answered what is not JSON ({e}): {}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
        })?;
        self.started = true;
        if json["is_error"].as_bool().unwrap_or(false) || !out.status.success() {
            return Err(format!("claude failed: {}", json));
        }
        // This question's share of the session's total (found 2026-11-22: summing the totals
        // counted the first question once a round).
        let total = json["total_cost_usd"].as_f64().unwrap_or(self.spent);
        let cost = (total - self.spent).max(0.0);
        self.spent = self.spent.max(total);
        Ok(Reply {
            text: json["result"].as_str().unwrap_or_default().to_string(),
            cost,
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}

/// A new random id, in the form Claude Code's sessions take (a version-4 UUID).
fn uuid() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut b);
    }
    // Never two the same, even without /dev/urandom: the time mixed in.
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
    for (i, byte) in t.to_le_bytes().iter().enumerate() {
        b[i] ^= byte;
    }
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-{}-{}-{}", &h[0..8], &h[8..12], &h[12..16], &h[16..20], &h[20..32])
}

/// Antigravity CLI (`agy -p`) writer, e.g. with `gemini-4-neon-high`.
/// Uses one persistent conversation per area/element via `--conversation <id>` and injects the
/// guide at the top of the first round's prompt.
pub struct Agy {
    guide: PathBuf,
    folder: PathBuf,
    model: String,
    conversation: Option<String>,
}

impl Agy {
    pub fn new(guide: &Path, folder: &Path, model: Option<&str>) -> Agy {
        Agy {
            guide: guide.to_path_buf(),
            folder: folder.to_path_buf(),
            model: model.unwrap_or(AGY_MODEL).to_string(),
            conversation: None,
        }
    }
}

impl Writer for Agy {
    fn write(&mut self, prompt: &str, pictures: Option<&Path>) -> Result<Reply, String> {
        let started = Instant::now();
        let full_prompt = if self.conversation.is_none() {
            let guide = std::fs::read_to_string(&self.guide).map_err(|e| format!("{}: {e}", self.guide.display()))?;
            format!(
                "{guide}\n\n---\n\nImportant: Do NOT write/edit files or run shell commands. You may ONLY read the reference image files listed below to inspect the site/renders. Answer with the complete Rust source in a single ```rust block.\n\n{prompt}"
            )
        } else {
            format!(
                "Important: Do NOT write/edit files or run shell commands (you may read any listed PNG pictures). Answer with the complete Rust source in a single ```rust block.\n\n{prompt}"
            )
        };
        let mut c = Command::new("agy");
        c.current_dir(&self.folder)
            .stdin(Stdio::null())
            .arg("-p")
            .arg(&full_prompt)
            .args([
                "--model",
                &self.model,
                "--output-format",
                "json",
                "--disable-slash-commands",
                "--dangerously-skip-permissions",
            ]);
        if let Some(id) = &self.conversation {
            c.args(["--conversation", id]);
        }
        if let Some(folder) = pictures {
            c.arg("--add-dir").arg(folder);
        }
        let out = c.output().map_err(|e| format!("agy: {e}"))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        // `agy` may print a status line before the final JSON line; parse the whole output or its last JSON line.
        let json: serde_json::Value = serde_json::from_str(stdout.trim())
            .or_else(|_| {
                stdout
                    .lines()
                    .rev()
                    .find(|l| l.trim_start().starts_with('{'))
                    .ok_or_else(|| serde_json::Error::io(std::io::Error::other("no JSON line")))
                    .and_then(serde_json::from_str)
            })
            .map_err(|e| format!("agy answered what is not JSON ({e}): {stdout}{}", String::from_utf8_lossy(&out.stderr)))?;
        if let Some(cid) = json["conversation_id"].as_str() {
            self.conversation = Some(cid.to_string());
        }
        let status = json["status"].as_str().unwrap_or("");
        if !out.status.success() || (!status.is_empty() && status != "SUCCESS") {
            return Err(format!("agy failed: {json}"));
        }
        Ok(Reply {
            text: json["response"].as_str().unwrap_or_default().to_string(),
            cost: 0.0,
            seconds: started.elapsed().as_secs_f64(),
        })
    }
}
