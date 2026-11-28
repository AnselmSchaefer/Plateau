//! Phase I (WORLD_PLAN.md): `wall_fit` holds a sign to the wall it hangs on, over its back; and
//! `style_fit` says how much of an element is of what is round about its site.

mod played;

use std::path::PathBuf;

use plateau_engine::body;
use plateau_engine::compose::{CompiledElement, PlacementTransform};
use plateau_engine::edit::surface_normal;
use plateau_engine::fit::{style_fit, wall_fit};
use plateau_engine::scene::SceneWorld;
use plateau_engine::site::{SiteContext, SiteProbe};
use plateau_engine::{Limits, Material, SIZES, Scene};

const PERSON: f32 = SIZES[1].1;

fn element(name: &str) -> Scene {
    let rel = format!("elements/modules/{name}.wasm");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(&rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run ./elements/build.sh)", path.display()));
    CompiledElement::load(rel, &bytes, Limits::default()).unwrap().preview_scene.clone()
}

/// The east wall of the garden, pointed at from the lawn at `y`, as a site.
fn east_wall(y: f32) -> SiteContext {
    let mut garden = played::area("garden", 7);
    let from = [50.0f32, y, 40.0];
    let t = body::look(&mut garden, from.map(|c| c as f64), [1.0, 0.0, 0.0], PERSON, PERSON * 45.0).unwrap().expect("the wall");
    let normal = surface_normal(&mut garden, t, PERSON).unwrap();
    SiteContext::measure(&mut garden, &SiteProbe { target: t.map(|c| c as f32), normal, from: Some(from), size: PERSON }).unwrap()
}

fn moved(tf: PlacementTransform, out: f32) -> PlacementTransform {
    let way = tf.local_to_world_dir([0.0, 0.0, 1.0]);
    PlacementTransform { pos: [0, 1, 2].map(|a| tf.pos[a] + way[a] * out), ..tf }
}

#[test]
fn a_sign_hung_where_the_wall_was_pointed_at_meets_it_over_its_back() {
    let sign = element("sign");
    let site = east_wall(1.5);
    assert!(site.cavity.is_none(), "{}", site.summary());
    let out = site.frame.local_to_world_dir([0.0, 0.0, 1.0]);
    assert!(out[0] < -0.95, "the frame faces out of the wall, into the garden: {out:?}");
    let world = SceneWorld::default();
    let mut garden = played::area("garden", 7);
    assert_eq!(wall_fit(&mut garden, &world, &sign, site.frame, PERSON).unwrap(), Ok(()));

    // Stood off it, sunk into it, or hung where the wall ends: refused, and told where.
    let off = wall_fit(&mut garden, &world, &sign, moved(site.frame, 0.12), PERSON).unwrap().unwrap_err();
    assert!(off.contains("a gap of") && off.contains("between its back and the wall"), "{off}");
    let sunk = wall_fit(&mut garden, &world, &sign, moved(site.frame, -0.2), PERSON).unwrap().unwrap_err();
    assert!(sunk.contains("sunk into the wall"), "{sunk}");
    let top = east_wall(2.38);
    let over = wall_fit(&mut garden, &world, &sign, top.frame, PERSON).unwrap().unwrap_err();
    assert!(over.contains("no wall behind its top"), "{over}");

    // A door has no flat back; a bridge's back is no wall's.
    let lawn_bridge = wall_fit(&mut garden, &world, &element("bridge"), site.frame, PERSON).unwrap();
    assert!(lawn_bridge.is_err(), "{lawn_bridge:?}");
}

#[test]
fn style_fit_notes_a_wooden_sign_on_a_stone_wall_and_not_a_stone_pillar() {
    let site = east_wall(1.5);
    assert_eq!(site.materials[0].0, Material::Stone, "{:?}", site.materials);
    let sign = style_fit(&site, &element("sign"));
    assert_eq!(sign.materials[0].0, Material::Wood);
    assert!(sign.share < 0.5, "{sign:?}");
    let note = sign.note.expect("noted for the look round");
    assert!(note.contains("wood") && note.contains("stone"), "{note}");
    let pillar = style_fit(&site, &element("pillar"));
    assert!(pillar.share > 0.5 && pillar.note.is_none(), "{pillar:?}");
}
