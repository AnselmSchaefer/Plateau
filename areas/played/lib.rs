//! The areas of `areas/`, each its source built for the machine: `areas_played::garden::played(seed)`.

#[path = "../garden/area.rs"]
pub mod garden;

/// An area made with its seed, to play.
pub type Play = fn(u64) -> Box<dyn plateau_sdk::native::Played>;

/// Every area here, by the name of its folder (and of its module, `areas/modules/<name>.wasm`).
pub const AREAS: &[(&str, Play)] = &[("garden", garden::played)];
