//! A saved world opened without a window (Phase C): its areas, sculpting, water and placed elements,
//! made again from the save as the player has them, for measuring sites in it (`site`), judging what
//! is made for them (`fit`), and picturing both (`picture`).

use std::path::Path;
use std::sync::Arc;

use wasmtime::bail;

use crate::atlas::{Atlas, CompiledAtlas, TileSpec};
use crate::compose::{Composed, ComposedLog, ElementCatalog, ElementRuntime, SdfElementEntry};
use crate::edit::{EditLog, SavedTile, WorldSave};
use crate::modules::ModuleCheck;
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
    /// Modules loaded although their bytes are not those the save recorded
    /// (`ModuleCheck::AcceptChanged`).
    pub changed_modules: Vec<String>,
}

impl OpenWorld {
    /// Opens `save` as someone `size` tall meets it. Its modules are looked for as they are named and
    /// under `root`; `area` is its area's module if the save names none. A module whose bytes are
    /// not those the save recorded is refused (`open_checked` to let it through).
    pub fn open(save: WorldSave, root: &Path, area: Option<&Path>, size: f32, limits: Limits) -> Result<OpenWorld> {
        OpenWorld::open_checked(save, root, area, size, limits, ModuleCheck::Strict)
    }

    /// As `open`, with `check` saying what to do with a module that changed since the save.
    pub fn open_checked(save: WorldSave, root: &Path, area: Option<&Path>, size: f32, limits: Limits, check: ModuleCheck) -> Result<OpenWorld> {
        let mut tiles = save.tiles.clone();
        if let Some(area) = area {
            tiles.retain(|t| t.at != [0, 0]);
            tiles.insert(0, SavedTile { at: [0, 0], module: area.to_string_lossy().into_owned(), seed: save.seed });
        }
        if tiles.is_empty() {
            bail!("the save does not say which module its area is: name it");
        }
        let mut changed_modules = Vec::new();
        let mut specs = Vec::with_capacity(tiles.len());
        for t in &tiles {
            let read = save.read_module(&t.module, root, check)?;
            if read.changed && !changed_modules.contains(&t.module) {
                changed_modules.push(t.module.clone());
            }
            specs.push(TileSpec { at: t.at, seed: t.seed, compiled: Arc::new(Compiled::new(&read.bytes, limits)?) });
        }
        let atlas = CompiledAtlas::new(specs)?;
        let mut scene = SceneWorld::default();
        atlas.instantiate()?.load_scenes_into(&mut scene)?;
        let mut catalog = ElementCatalog::new(root, limits);
        catalog.expect_hashes(&save.modules, check);
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
        for m in catalog.changed_modules() {
            if !changed_modules.contains(m) {
                changed_modules.push(m.clone());
            }
        }
        Ok(OpenWorld { save, atlas, log, scene: Arc::new(scene), size, changed_modules })
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
