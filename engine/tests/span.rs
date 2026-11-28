//! Phase I (WORLD_PLAN.md): `span_fit`, the law of a bridge: its ends rest on both banks, it spans
//! a gap, and who it is for walks across it. The carried bridge spans a person's ditch and a tiny's,
//! and is refused, and told why, on the level, over a ditch too wide, and too narrow for a giant.

mod played;

use std::path::PathBuf;
use std::sync::Arc;

use plateau_engine::compose::{CompiledElement, Composed, ComposedLog, PlacementTransform};
use plateau_engine::edit::{Edit, EditLog, Shape};
use plateau_engine::fit::span_fit;
use plateau_engine::scene::SceneWorld;
use plateau_engine::{Limits, Material, SIZES, Scene};

const TINY: f32 = SIZES[0].1;
const PERSON: f32 = SIZES[1].1;
const GIANT: f32 = SIZES[2].1;
/// The garden's open lawn, where the ditches are dug.
const LAWN: [f32; 3] = [40.0, 0.0, 50.0];

type World = Composed<played::Native, Arc<ComposedLog>>;

fn bridge() -> Scene {
    let rel = "elements/modules/bridge.wasm";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run ./elements/build.sh)", path.display()));
    CompiledElement::load(rel, &bytes, Limits::default()).unwrap().preview_scene.clone()
}

/// How high the lawn is at `LAWN`.
fn ground() -> f32 {
    plateau_engine::body::Body::standing(&mut lawn(), [LAWN[0] as f64, 1.0, LAWN[2] as f64], TINY).unwrap().feet[1] as f32
}

/// A smooth slab of soil on the lawn (no grass to stand up through a tiny's bridge), `scale` times
/// a person's, its top half that above the lawn: what the ditches are dug into and bridges stand on.
fn slab(scale: f32, size: f32) -> (Edit, f32) {
    let g = ground();
    (Edit::add(Shape::Box { half: [3.0 * scale, 0.5 * scale, 4.0 * scale], round: 0.0 }, [LAWN[0], g, LAWN[2]], 0.0, Material::Soil, size), g + 0.5 * scale)
}

/// The slab with a ditch across it, `wide` along z and `deep`.
fn ditch(wide: f32, deep: f32, scale: f32, size: f32) -> World {
    let (slab, top) = slab(scale, size);
    let cut = Edit::carve(Shape::Box { half: [2.0 * scale, 0.5 * (deep + scale), 0.5 * wide], round: 0.0 }, [LAWN[0], top + 0.5 * scale - 0.5 * (deep + scale), LAWN[2]], 0.0, None, size);
    let log = ComposedLog::new(Arc::new(EditLog::from_edits(vec![slab, cut]).unwrap()), Vec::new(), 1);
    Composed::new(played::area("garden", 7), Arc::new(log))
}

/// The slab, whole.
fn level(scale: f32, size: f32) -> World {
    let log = ComposedLog::new(Arc::new(EditLog::from_edits(vec![slab(scale, size).0]).unwrap()), Vec::new(), 1);
    Composed::new(played::area("garden", 7), Arc::new(log))
}

fn lawn() -> World {
    Composed::new(played::area("garden", 7), Arc::new(ComposedLog::empty()))
}

/// The bridge standing on the slab, across the ditch.
fn across(scale: f32) -> PlacementTransform {
    PlacementTransform::new([LAWN[0], ground() + 0.5 * scale, LAWN[2]], 0.0, scale)
}

#[test]
fn the_bridge_spans_a_person_s_ditch_and_a_tiny_s() {
    let bridge = bridge();
    let world = SceneWorld::default();
    let mut person = ditch(2.6, 1.5, 1.0, PERSON);
    assert_eq!(span_fit(&mut person, &world, &bridge, across(1.0), &[PERSON]).unwrap(), Ok(()));
    let mut tiny = ditch(0.026, 0.015, 0.01, TINY);
    assert_eq!(span_fit(&mut tiny, &world, &bridge, across(0.01), &[TINY]).unwrap(), Ok(()));
    // The person's bridge is not the tiny's: its first plank is a step up of 7 cm from where its
    // end rests, many times what a tiny climbs.
    let tiny_on_person = span_fit(&mut person, &world, &bridge, across(1.0), &[TINY, PERSON]).unwrap().unwrap_err();
    assert!(tiny_on_person.contains("1.7 cm tall gets only"), "{tiny_on_person}");
}

#[test]
fn a_bridge_that_spans_nothing_or_is_too_short_or_too_narrow_is_refused_and_told_why() {
    let bridge = bridge();
    let world = SceneWorld::default();
    let flat = span_fit(&mut level(1.0, PERSON), &world, &bridge, across(1.0), &[PERSON]).unwrap().unwrap_err();
    assert!(flat.contains("spans no gap"), "{flat}");
    let wide = span_fit(&mut ditch(6.0, 1.5, 1.0, PERSON), &world, &bridge, across(1.0), &[PERSON]).unwrap().unwrap_err();
    assert!(wide.contains("end does not rest on the bank"), "{wide}");
    let giant = span_fit(&mut ditch(2.6, 1.5, 1.0, PERSON), &world, &bridge, across(1.0), &[GIANT]).unwrap().unwrap_err();
    assert!(giant.contains("too narrow"), "{giant}");
    // Turned across the ditch, it lies along it: both ends in the ditch, off any bank.
    let along = PlacementTransform { rot: plateau_engine::compose::euler_quat(std::f32::consts::FRAC_PI_2, 0.0, 0.0), ..across(1.0) };
    assert!(span_fit(&mut ditch(2.6, 1.5, 1.0, PERSON), &world, &bridge, along, &[PERSON]).unwrap().is_err());
}


