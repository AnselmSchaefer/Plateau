//! The action log (`WORLD_PLAN.md`, phase E): one JSON line an action, so a session can be read
//! (by people, by the inspector of phase K) and replayed (`replay`) onto the save it began from.
//!
//! ```text
//! {"log":"plateau-log 1"}
//! {"seq":1,"player":{"id":"local","feet":[..],"size":1.7,"yaw":0.0},"action":{..},"outcome":{..},"revision":[3,4]}
//! ```
//!
//! Numbers are written shortest and read back to the same bits (`f32`, `f64`), so a replayed action
//! is the very action that was applied. Ids are the save's, so a log and its save read together.

use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};
use wasmtime::{bail, format_err};

use super::{Action, Actor, Outcome, World};
use crate::Result;
use crate::body::Body;

/// The log format's name and version, on its first line.
pub const LOG_VERSION: &str = "plateau-log 1";

#[derive(Serialize, Deserialize)]
struct Header {
    log: String,
}

/// Who acted, as they were when they did.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoggedPlayer {
    pub id: String,
    pub feet: [f64; 3],
    pub size: f32,
    pub yaw: f32,
}

/// One line of the log.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Logged {
    pub seq: u64,
    pub player: LoggedPlayer,
    pub action: Action,
    pub outcome: Outcome,
    /// The world's SDF revision before and after.
    pub revision: [u64; 2],
}

/// A log being written, flushed after every line.
pub struct ActionLog {
    out: BufWriter<File>,
    seq: u64,
}

impl ActionLog {
    /// Starts a log at `path` (replacing what is there).
    pub fn create(path: &Path) -> Result<ActionLog> {
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        let mut out = BufWriter::new(File::create(path).map_err(|e| format_err!("{}: {e}", path.display()))?);
        serde_json::to_writer(&mut out, &Header { log: LOG_VERSION.into() })?;
        out.write_all(b"\n")?;
        out.flush()?;
        Ok(ActionLog { out, seq: 0 })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write(&mut self, id: &str, feet: [f64; 3], size: f32, yaw: f32, action: &Action, outcome: &Outcome, before: u64, after: u64) -> Result<()> {
        self.seq += 1;
        let line = Logged {
            seq: self.seq,
            player: LoggedPlayer { id: id.to_string(), feet, size, yaw },
            action: action.clone(),
            outcome: outcome.clone(),
            revision: [before, after],
        };
        serde_json::to_writer(&mut self.out, &line)?;
        self.out.write_all(b"\n")?;
        self.out.flush()?;
        Ok(())
    }

    /// Reads the log at `path`: its lines, in order.
    pub fn read(path: &Path) -> Result<Vec<Logged>> {
        let file = File::open(path).map_err(|e| format_err!("{}: {e}", path.display()))?;
        let mut lines = BufReader::new(file).lines();
        let header: Header = match lines.next() {
            Some(line) => serde_json::from_str(&line?).map_err(|e| format_err!("{}: line 1: {e}", path.display()))?,
            None => bail!("{}: empty, not a {LOG_VERSION}", path.display()),
        };
        if header.log != LOG_VERSION {
            bail!("{}: `{}`, not {LOG_VERSION}", path.display(), header.log);
        }
        let mut out = Vec::new();
        for (n, line) in lines.enumerate() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            out.push(serde_json::from_str(&line).map_err(|e| format_err!("{}: line {}: {e}", path.display(), n + 2))?);
        }
        Ok(out)
    }
}

/// Applies the logged actions to `world` in order, each by its player as they then stood, and checks
/// each comes out as it did: an error names the first that does not.
pub fn replay(world: &mut World, log: &[Logged]) -> Result<()> {
    let mut body = Body::of_size(1.7);
    for line in log {
        body.feet = line.player.feet;
        body.size = line.player.size;
        body.yaw = line.player.yaw;
        let outcome = world.apply(Actor { id: &line.player.id, body: &mut body }, line.action.clone());
        if outcome != line.outcome {
            bail!("replaying action {} ({}): came out {:?}, logged {:?}", line.seq, line.action.name(), outcome, line.outcome);
        }
    }
    Ok(())
}
