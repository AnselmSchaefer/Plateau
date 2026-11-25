//! A saved world opened without a window (Phase C): its areas, sculpting, water and placed elements,
//! made again from the save as the player has them, for measuring sites in it (`site`), judging what
//! is made for them (`fit`), and picturing both (`picture`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use wasmtime::{bail, format_err};

use crate::atlas::{Atlas, CompiledAtlas, TileSpec};
use crate::compose::{Composed, ComposedLog, ElementCatalog, ElementRuntime, SdfElementEntry};
use crate::edit::{EditLog, SavedTile, WorldSave};
use crate::scene::{Combined, SceneWorld};
use crate::water::WaterLog;
use crate::{Answer, Compiled, Description, Limits, Region, Result, Wasm};

/// A world's areas, what was done to them, and their solid procedural geometry.
pub struct OpenWorld {
    pub save: WorldSave,
    pub atlas: CompiledAtlas,
    pub log: Arc<ComposedLog>,
    /// The areas' and placed elements' procedural scenes, as someone of `size` meets them.
    pub scene: Arc<SceneWorld>,
    pub size: f32,
}

/// Where `path`, as a save has it, is: as it is, or under `root`.
fn found(path: &str, root: &Path) -> Result<PathBuf> {
    [PathBuf::from(path), root.join(path)].into_iter().find(|p| p.exists()).ok_or_else(|| format_err!("{path}: not found (nor under {})", root.display()))
}

impl OpenWorld {
    /// Opens `save` as someone `size` tall meets it. Its modules are looked for as they are named and
    /// under `root`; `area` is its area's module if the save names none.
    pub fn open(save: WorldSave, root: &Path, area: Option<&Path>, size: f32, limits: Limits) -> Result<OpenWorld> {
        let mut tiles = save.tiles.clone();
        if let Some(area) = area {
            tiles.retain(|t| t.at != [0, 0]);
            tiles.insert(0, SavedTile { at: [0, 0], module: area.to_string_lossy().into_owned(), seed: save.seed });
        }
        if tiles.is_empty() {
            bail!("the save does not say which module its area is: name it");
        }
        let specs = tiles
            .iter()
            .map(|t| {
                let path = found(&t.module, root)?;
                let bytes = std::fs::read(&path).map_err(|e| format_err!("{}: {e}", path.display()))?;
                Ok(TileSpec { at: t.at, seed: t.seed, compiled: Arc::new(Compiled::new(&bytes, limits)?) })
            })
            .collect::<Result<Vec<_>>>()?;
        let atlas = CompiledAtlas::new(specs)?;
        let mut scene = SceneWorld::default();
        atlas.instantiate()?.load_scenes_into(&mut scene)?;
        let mut catalog = ElementCatalog::new(root, limits);
        let mut runtime = ElementRuntime::new();
        let mut sdf = Vec::new();
        for pl in &save.elements {
            let element = catalog.get_or_load(&pl.module)?;
            runtime.spawn_for_size(pl.clone(), &element, &mut scene, size)?;
            if let Some(bounds) = element.sdf_bounds {
                sdf.push(SdfElementEntry::new(pl.clone(), element.compiled.clone(), bounds));
            } else if pl.is_promoted()
                && let Some(mesh) = catalog.get_or_build_mesh_sdf(&pl.module, pl.seed, &pl.clips)?
            {
                sdf.push(SdfElementEntry::from_promoted_mesh(pl.clone(), mesh));
            }
        }
        let edits = Arc::new(EditLog::from_edits(save.edits.clone())?);
        let water = Arc::new(WaterLog::from_bodies(save.water.clone())?);
        let log = Arc::new(ComposedLog::new_with_water(edits, sdf, water, 1));
        Ok(OpenWorld { save, atlas, log, scene: Arc::new(scene), size })
    }

    /// An area's module as it is, untouched, opened as a world.
    pub fn bare(area: &Path, seed: u64, size: f32, limits: Limits) -> Result<OpenWorld> {
        OpenWorld::open(WorldSave::new("", seed, Vec::new()), Path::new("."), Some(area), size, limits)
    }

    /// The world's SDF, made anew (one for each thread): areas, sculpting, water, promoted elements.
    pub fn region(&self) -> Result<Composed<Atlas<Wasm>, Arc<ComposedLog>>> {
        Ok(Composed::new(self.atlas.instantiate()?, self.log.clone()))
    }

    /// The same with its solid procedural geometry, owned: what a picture or a measure is taken of.
    pub fn solid(&self) -> Result<Solids<Composed<Atlas<Wasm>, Arc<ComposedLog>>>> {
        Ok(Solids { region: self.region()?, world: self.scene.clone() })
    }
}

/// A `Region` with a `SceneWorld`'s solid geometry in it, owning both (`scene::Combined` borrows).
pub struct Solids<R> {
    pub region: R,
    pub world: Arc<SceneWorld>,
}

impl<R: Region> Region for Solids<R> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        Combined { region: &mut self.region, world: &self.world }.sample(points, size, out)
    }

    fn describe(&mut self) -> Result<Description> {
        self.region.describe()
    }
}
