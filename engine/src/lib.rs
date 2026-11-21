//! Plateau's engine (POC_PLAN.md). For now, an area as the engine sees it: a `Region`, answering
//! what is where for a viewer of a size and what it is, from its module (`Wasm`, checked on load by
//! `check`, run in wasmtime with the limits the engine keeps) or, in tests, from its source played
//! natively. The interface is the WIT world `plateau:region@0.1.0` (`sdk/wit/region.wit`); the
//! engine writes and reads its canonical ABI itself (`wasm`).

pub mod check;
pub mod laws;
pub mod picture;
pub mod wasm;

pub use wasm::{Compiled, Limits, Wasm};
pub use wasmtime::{Error, Result};

/// An area, to the engine: an answer for each point, at a size, and what it says it is.
pub trait Region {
    /// Fills `out` with an answer for each of `points`, in order, for a viewer `size` metres tall.
    fn sample(&mut self, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()>;
    /// What the area says it is.
    fn describe(&mut self) -> Result<Description>;
}

/// How wide an area is, along x and along z, in metres (the world's `region.wit`).
pub const WIDTH: f32 = 64.0;

/// The three sizes, by name, and their heights in metres (CONCEPT.md, "Three sizes").
pub const SIZES: [(Size, f32); 3] = [(Size::Tiny, 0.017), (Size::Person, 1.7), (Size::Giant, 17.0)];

/// The smallest and largest a viewer may be, so an area is asked only at sizes it was made for:
/// the tiny's height halved, a giant's doubled.
pub const SMALLEST: f32 = 0.0085;
pub const LARGEST: f32 = 34.0;

/// The engine's materials, in the world's order: a material's number is its place here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
fn numbers(seed: u32) -> impl FnMut() -> f32 {
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
