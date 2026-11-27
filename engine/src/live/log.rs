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

use super::{Action, Actor, Invalidation, Outcome, World};
use crate::body::Body;
use crate::compose::PlacementTransform;
use crate::edit::{Edit, Op, SavedTile, Shape, WorldSave};
use crate::graph::{Data, Kind};
use crate::modules::ModuleCheck;
use crate::physics::{Tool, verify_edit_tool};
use crate::{Limits, Result, SIZES};

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

/// Converts an element-local `Edit` back to world space under `tf` (inverse of `world_edit_to_local`).
fn local_edit_to_world(edit: &Edit, tf: PlacementTransform) -> Edit {
    let s = tf.scale.max(1e-5);
    let center = tf.local_to_world_point(edit.center);
    let shape = edit.shape.scaled(s);
    Edit {
        op: edit.op,
        shape,
        center,
        blend: edit.blend * s,
        material: edit.material,
        stamp_size: edit.stamp_size * s,
        tool: edit.tool,
    }
}

/// Replays every SDF edit in `save` chronologically against its base area(s) and placed elements,
/// checking every edit's recorded `Edit::tool` and `save.mode` rules (`WORLD_PLAN.md`, phase H).
///
/// Returns `Ok(Ok(()))` if every edit is valid for its tool, `Ok(Err(reason))` naming the first
/// edit whose tool cannot perform it (e.g. a tampered save), or `Err` on module load error.
pub fn verify_save_edits(
    save: &WorldSave,
    tiles: Option<Vec<SavedTile>>,
    root: &Path,
    limits: Limits,
    check: ModuleCheck,
) -> Result<std::result::Result<(), String>> {
    use std::collections::BTreeSet;
    use std::sync::Arc;

    // Start from the save's base areas, modules, and mode, with no edits or elements yet.
    let mut base = WorldSave::new(save.area.clone(), save.seed, Vec::new());
    base.mode = save.mode;
    base.tiles = save.tiles.clone();
    base.modules = save.modules.clone();
    base.rules = save.rules.clone();
    let (mut world, _) = World::open(base, tiles, root, limits, check, SIZES[1].1)?;

    // Elements produced by a non-planar component-mask split carry engine-generated mask boxes
    // (`Tool::All`, sharp `Shape::Box` carves) in `local_edits`; skip those when checking player tools.
    let split_ids: BTreeSet<u32> = save
        .relations
        .iter()
        .filter(|r| r.kind == Kind::DerivedFrom && r.data == Data::Split)
        .filter_map(|r| r.from.clone().element())
        .collect();

    let total_edits = save.edits.len();
    for edit_idx in 0..=total_edits {
        // Spawn elements placed at this point in the edit timeline and verify their local edits.
        for pl in save.elements.iter().filter(|e| e.after_edits.min(total_edits) == edit_idx) {
            let mut initial = pl.clone();
            initial.local_edits.clear();
            initial.promoted = None;
            let tf = initial.transform();
            world.spawn_placed(initial, None)?;
            let is_split = split_ids.contains(&pl.id);
            for (li, le) in pl.local_edits.iter().enumerate() {
                let is_mask_box = is_split
                    && le.op == Op::Carve
                    && le.tool == Tool::All
                    && le.blend == 0.0
                    && matches!(le.shape, Shape::Box { .. });
                let world_le = local_edit_to_world(le, tf);
                if !is_mask_box {
                    let unpromoted = world.unpromoted_sdfs_near(&world_le, Some(pl.id));
                    if let Err(why) = verify_edit_tool(save.mode, &world_le, &mut world.region, &world.scene, &unpromoted)? {
                        return Ok(Err(format!(
                            "element #{} local edit #{li} ({} with {}): {why}",
                            pl.id,
                            le.op.name(),
                            le.tool.save_name()
                        )));
                    }
                }
                if let Some(pos) = world.placed.iter().position(|e| e.id == pl.id) {
                    let newly_promoted = !world.placed[pos].is_promoted();
                    world.placed[pos].local_edits.push(*le);
                    if newly_promoted {
                        world.set_promoted(pl.id, Some(edit_idx));
                    }
                    world.commit(Invalidation::edit(world_le));
                }
            }
        }
        if edit_idx == total_edits {
            break;
        }
        let edit = save.edits[edit_idx];
        let unpromoted = world.unpromoted_sdfs_near(&edit, None);
        if let Err(why) = verify_edit_tool(save.mode, &edit, &mut world.region, &world.scene, &unpromoted)? {
            return Ok(Err(format!(
                "edit #{edit_idx} ({} with {}): {why}",
                edit.op.name(),
                edit.tool.save_name()
            )));
        }
        // Apply this edit to the running world so subsequent edits see its result.
        let water_bounds = if edit.op == Op::Carve {
            let mut next = (*world.water).clone();
            match next.expand_from_carve(&mut world.region, &world.edits, &edit, None) {
                Ok(Some(b)) => {
                    world.water = Arc::new(next);
                    Some(b)
                }
                _ => None,
            }
        } else {
            None
        };
        let (_, promo_bounds, _) = if edit.op == Op::Carve {
            world.promote_overlapping_scene_elements(&edit, None, edit_idx)
        } else {
            (Vec::new(), Vec::new(), Vec::new())
        };
        let mut next = (*world.edits).clone();
        next.push(edit)?;
        world.edits = Arc::new(next);
        world.commit(Invalidation::Some { edits: vec![edit], bounds: promo_bounds, water: water_bounds });
    }
    Ok(Ok(()))
}
