//! Plateau's engine (POC_PLAN.md). An area as the engine sees it: a `Region`, answering what is
//! where for a viewer of a size and what it is, from its module (`Wasm`, checked on load by
//! `check`, run in wasmtime with the limits the engine keeps) or, in tests, from its source played
//! natively. The interface is the WIT world `plateau:region@0.1.0` (`sdk/wit/region.wit`); the
//! engine writes and reads its canonical ABI itself (`wasm`).
//!
//! What the engine makes of an area, without a window: the surface in chunks at the detail each
//! size needs (`chunk`, `rings`), a player's body walking it at their size (`body`), where each
//! size can go (`reach`), pictures (`picture`), sketched maps (`sketch`), and the laws an area
//! must keep (`laws`). What players make of it: sculpting (`edit`), placed elements (`compose`,
//! `scene`, `cut`, `physics`), water (`water`), and measured sites (`site`) for elements made to fit
//! them (`fit`). The window is `play/`.

pub mod atlas;
pub mod body;
pub mod carved;
pub mod check;
pub mod chunk;
pub mod compose;
pub mod cut;
pub mod edit;
pub mod fit;
pub mod graph;
pub mod laws;
pub mod live;
pub mod math;
pub mod mesh_sdf;
pub mod modules;
pub mod physics;
pub mod picture;
pub mod reach;
pub mod rings;
pub mod rules;
pub mod scene;
pub mod site;
pub mod sketch;
pub mod support;
pub mod wasm;
pub mod water;
pub mod world;

pub use atlas::{Atlas, AtlasTile, BorderProfile, BorderSample, CompiledAtlas, SEAM_HALF, Side, TileSpec};
pub use compose::{CompiledElement, Composed, ComposedLog, ElementCatalog, ElementRuntime, PlacedElement, PlacementTransform, SdfElementEntry, SdfSource};
pub use cut::{CarryDecision, ClipPlane, ObjectPhysics};
pub use edit::{Edit, EditLog, Edited, Op, SavedTile, Shape, WorldSave};
pub use fit::{CavityFit, RimFit, SiteRegion, cavity_fit, fit_check, fit_transform, ground_fit};
pub use mesh_sdf::MeshSdf;
pub use modules::{ModuleCheck, ModuleRead, SavedModule};
pub use physics::{BuoyancyState, MaterialProps, Tool, WATER_DENSITY, compute_buoyancy, effective_displacement_m3, material_props, player_mass_kg, step_floating_move};
pub use rules::{Action, AreaPolicy, CutCarryPolicy, Denial, LOCAL_PLAYER, Plot, Role, RolePolicy, RuleBook};
pub use site::{Cavity, SiteContext, SiteProbe};
pub use wasm::{Compiled, Limits, Wasm};
pub use water::{WaterBasin, WaterBody, WaterLog};
pub use world::{OpenWorld, Solids};
pub use wasmtime::{Error, Result};

/// An area, to the engine: an answer for each point, at a size, what it says it is, any procedural
/// meshes/instances/lights it emits, and optional sandboxed runtime behavior (`tick`, `interact`).
pub trait Region {
    /// Fills `out` with an answer for each of `points`, in order, for a viewer `size` metres tall.
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()>;
    /// What the area says it is.
    fn describe(&mut self) -> Result<Description>;
    /// Raw procedural meshes, static/dynamic instances, and initial lights emitted once on load.
    fn scene(&mut self) -> Result<Scene> {
        Ok(Scene::default())
    }
    /// Sandboxed runtime update called each frame/step, returning clamped commands.
    fn tick(&mut self, _dt: f32, _player: &PlayerState) -> Result<Vec<Command>> {
        Ok(Vec::new())
    }
    /// Sandboxed runtime interaction called when the player presses interact (`F` / right-click).
    fn interact(&mut self, _player: &PlayerState) -> Result<Vec<Command>> {
        Ok(Vec::new())
    }
    /// Local SDF bounding box & smooth blend width when this module is a standalone carvable
    /// landform element (`None` for full areas and pure procedural `Scene` elements).
    fn element_bounds(&mut self) -> Result<Option<ElementBounds>> {
        Ok(None)
    }
}

impl<R: Region + ?Sized> Region for &mut R {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        (**self).sample(points, size, out)
    }

    fn describe(&mut self) -> Result<Description> {
        (**self).describe()
    }

    fn scene(&mut self) -> Result<Scene> {
        (**self).scene()
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Result<Vec<Command>> {
        (**self).tick(dt, player)
    }

    fn interact(&mut self, player: &PlayerState) -> Result<Vec<Command>> {
        (**self).interact(player)
    }

    fn element_bounds(&mut self) -> Result<Option<ElementBounds>> {
        (**self).element_bounds()
    }
}

impl<R: Region + ?Sized> Region for Box<R> {
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
        (**self).sample(points, size, out)
    }

    fn describe(&mut self) -> Result<Description> {
        (**self).describe()
    }

    fn scene(&mut self) -> Result<Scene> {
        (**self).scene()
    }

    fn tick(&mut self, dt: f32, player: &PlayerState) -> Result<Vec<Command>> {
        (**self).tick(dt, player)
    }

    fn interact(&mut self, player: &PlayerState) -> Result<Vec<Command>> {
        (**self).interact(player)
    }

    fn element_bounds(&mut self) -> Result<Option<ElementBounds>> {
        (**self).element_bounds()
    }
}

/// How wide an area is, along x and along z, in metres (the world's `region.wit`).
pub const WIDTH: f32 = 64.0;

/// The three sizes, by name, and their heights in metres (CONCEPT.md, "Three sizes").
pub const SIZES: [(Size, f32); 3] = [(Size::Tiny, 0.017), (Size::Person, 1.7), (Size::Giant, 17.0)];

/// Where a player comes into an area, across the ground (x, z): the middle of its south edge, a
/// little way in, facing north. Areas say nothing of it yet; borders will (step 9).
pub const START: [f64; 2] = [32.0, 60.0];

/// The smallest and largest a viewer may be, so an area is asked only at sizes it was made for:
/// the tiny's height halved, a giant's doubled.
pub const SMALLEST: f32 = 0.0085;
pub const LARGEST: f32 = 34.0;

/// The engine's materials, in the world's order: a material's number is its place here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[repr(u8)]
pub enum Material {
    Soil,
    Grass,
    Sand,
    Mud,
    Stone,
    Wood,
    Leaf,
    Moss,
    Water,
}

impl Material {
    pub const ALL: [Material; 9] = [
        Material::Soil,
        Material::Grass,
        Material::Sand,
        Material::Mud,
        Material::Stone,
        Material::Wood,
        Material::Leaf,
        Material::Moss,
        Material::Water,
    ];

    pub fn from_number(n: u8) -> Option<Material> {
        Material::ALL.get(n as usize).copied()
    }

    pub fn name(self) -> &'static str {
        ["soil", "grass", "sand", "mud", "stone", "wood", "leaf", "moss", "water"][self as usize]
    }
}

/// What is at a point: how far to the nearest surface (negative inside) and its material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Answer {
    pub distance: f32,
    pub material: Material,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Size {
    Tiny,
    Person,
    Giant,
}

impl Size {
    pub const ALL: [Size; 3] = [Size::Tiny, Size::Person, Size::Giant];

    pub fn height(self) -> f32 {
        SIZES[self as usize].1
    }

    pub fn name(self) -> &'static str {
        ["tiny", "person", "giant"][self as usize]
    }
}

/// A place an area means for one size.
#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub name: String,
    pub size: Size,
    pub at: [f32; 3],
}

/// One edge of an area: the ground's height every metre along it, and any water's level.
#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub ground: Vec<f32>,
    pub water: Option<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Borders {
    pub north: Edge,
    pub east: Edge,
    pub south: Edge,
    pub west: Edge,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Description {
    pub name: String,
    pub places: Vec<Place>,
    pub borders: Borders,
}

/// One vertex of a raw procedural mesh emitted by an area.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub material: Material,
    pub tint: [u8; 3],
}

/// A raw procedural triangle mesh emitted by an area.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MeshData {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// One placed instance of a mesh (`id == 0` static, `id > 0` dynamic/controllable).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Instance {
    pub id: u32,
    pub mesh: u32,
    pub pos: [f32; 3],
    /// Unit quaternion `[x, y, z, w]`.
    pub rot: [f32; 4],
    pub scale: [f32; 3],
    pub solid: bool,
}

/// A point light placed in the area (`intensity <= 0.0` disables/removes it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    pub id: u32,
    pub pos: [f32; 3],
    pub color: [f32; 3],
    pub intensity: f32,
    pub radius: f32,
}

/// The initial procedural scene emitted once on load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scene {
    pub meshes: Vec<MeshData>,
    pub instances: Vec<Instance>,
    pub lights: Vec<Light>,
}

/// What the runtime WASM plugin senses about the player on each tick or interaction.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub feet: [f32; 3],
    pub eye: [f32; 3],
    pub look: [f32; 3],
    pub velocity: [f32; 3],
    pub size: f32,
    pub grounded: bool,
    pub holding_use: bool,
    pub target: u32,
    pub aim: [f32; 3],
    pub time: f32,
}

/// Local SDF bounding box and optional smooth blend width for a standalone composable element.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ElementBounds {
    pub lo: [f32; 3],
    pub hi: [f32; 3],
    pub blend: f32,
}

/// Orthogonal physical & world verbs a WASM plugin may request (all validated & clamped by engine).
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    SetTransform(Instance),
    SetVisible(u32, bool),
    SetSolid(u32, bool),
    ApplyImpulse([f32; 3]),
    SetLight(Light),
    Say(String),
}

/// FNV-1a over answers' bits: the same answers give the same hash on every machine.
pub fn hash(answers: &[Answer]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for a in answers {
        for b in a.distance.to_bits().to_le_bytes().into_iter().chain([a.material as u8]) {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    h
}

/// Numbers from 0 to 1, the same every time for the same seed (xorshift).
pub(crate) fn numbers(seed: u32) -> impl FnMut() -> f32 {
    let mut state = seed | 1;
    move || {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        state as f32 / u32::MAX as f32
    }
}

/// Points scattered over an area, from 3 m below its ground to `above` metres over it, the same
/// every time for the same `n` and `seed`: for hashing, comparing and timing.
pub fn points(n: usize, above: f32, seed: u32) -> Vec<[f32; 3]> {
    let mut next = numbers(seed);
    (0..n).map(|_| [next() * WIDTH, next() * (above + 3.0) - 3.0, next() * WIDTH]).collect()
}

/// Points scattered within `reach` metres of `at` along each axis, the same every time: for
/// looking closely at a place, as the tiny see it.
pub fn points_near(at: [f32; 3], reach: f32, n: usize, seed: u32) -> Vec<[f32; 3]> {
    let mut next = numbers(seed);
    (0..n).map(|_| [0, 1, 2].map(|i| at[i] + (next() * 2.0 - 1.0) * reach)).collect()
}
