//! Integration tests for Phase C: site & cavity measurement (`site`), the fit laws (`fit`), saved
//! worlds opened without a window (`world`), and the carried round door (`hobbit-door`):
//! 1. A tiny's round tunnel into an oak's trunk is measured the same from wherever it is pointed at,
//!    kept bit for bit by its file; square, hill and giant-scale openings are measured for what they
//!    are; a lawn and a pond's edge have none.
//! 2. The round door goes into each to scale; the law refuses it too small, too large, aside, too far
//!    out, and where the opening is not round.
//! 3. Played: a tiny is kept out by the shut door, opens it with `F`, walks in, and shuts it behind.
//! 4. Elements for open ground stand on it; saved worlds open as they were played.

mod played;

use std::path::PathBuf;
use std::sync::Arc;

use plateau_engine::body::{self, Body, Intent};
use plateau_engine::compose::{CompiledElement, Composed, ComposedLog, ElementCatalog, ElementRuntime, PlacedElement, PlacementTransform, SdfElementEntry};
use plateau_engine::cut::world_edit_to_local;
use plateau_engine::edit::{Edit, EditLog, SavedTile, Shape, WorldSave, surface_normal};
use plateau_engine::fit::{SiteRegion, cavity_fit, fit_check, fit_transform, ground_fit, rim_fit};
use plateau_engine::scene::{Combined, SceneWorld};
use plateau_engine::site::{CONTOUR, SiteContext, SiteProbe};
use plateau_engine::world::OpenWorld;
use plateau_engine::{Limits, Material, PlayerState, Region, SIZES, hash, laws};

const TINY: f32 = SIZES[0].1;
const PERSON: f32 = SIZES[1].1;
const GIANT: f32 = SIZES[2].1;

type World = Composed<played::Native, Arc<ComposedLog>>;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn load_element(name: &str) -> Arc<CompiledElement> {
    let rel = format!("elements/modules/{name}.wasm");
    let path = repo_root().join(&rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run ./elements/build.sh)", path.display()));
    CompiledElement::load(rel, &bytes, Limits::default()).unwrap_or_else(|e| panic!("{name}: {e:#}"))
}

fn placed(id: u32, element: &CompiledElement, tf: PlacementTransform, size: f32) -> PlacedElement {
    PlacedElement {
        id,
        module: element.module.clone(),
        seed: 7,
        pos: tf.pos,
        yaw: 0.0,
        rot: Some(tf.rot),
        scale: tf.scale,
        stamp_size: size,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: None,
        params: Vec::new(),
    }
}

/// The oak at (32, 0, 48) in the garden, promoted, with `carves` sculpted onto it (as `B` does: in
/// its own space, so they travel with it).
fn oak(carves: &[Edit]) -> (PlacedElement, SdfElementEntry) {
    let tree = load_element("tree");
    let mut pl = placed(1, &tree, PlacementTransform::new([32.0, 0.0, 48.0], 0.0, 1.0), PERSON);
    pl.promoted = Some(0);
    pl.local_edits = carves.iter().map(|e| world_edit_to_local(e, pl.transform())).collect();
    let mesh = tree.build_mesh_sdf(pl.seed, &[], &[]).unwrap().unwrap();
    (pl.clone(), SdfElementEntry::from_promoted_mesh(pl, mesh))
}

fn garden_with(edits: &[Edit], sdf: Vec<SdfElementEntry>) -> World {
    let log = ComposedLog::new(Arc::new(EditLog::from_edits(edits.to_vec()).unwrap()), sdf, 1);
    Composed::new(played::area("garden", 7), Arc::new(log))
}

/// A tunnel of the sphere brush (`sculpt`: radius 0.75 of the player, blended by 0.35 of it) from
/// `from` along `way`, a stamp every 0.45 of its radius.
fn tunnel(from: [f32; 3], way: [f32; 3], stamps: usize, size: f32) -> Vec<Edit> {
    bore(from, way, stamps, size, 0.75 * size)
}

fn bore(from: [f32; 3], way: [f32; 3], stamps: usize, size: f32, r: f32) -> Vec<Edit> {
    (0..stamps)
        .map(|k| {
            let at = [0, 1, 2].map(|a| from[a] + way[a] * 0.45 * r * k as f32);
            Edit::carve(Shape::Sphere { radius: r }, at, 0.35 * r, None, size)
        })
        .collect()
}

/// What someone `size` tall points at from `from`, looking along `dir`.
fn aim(area: &mut dyn Region, from: [f32; 3], dir: [f32; 3], size: f32) -> SiteProbe {
    let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    let t = body::look(area, from.map(|c| c as f64), dir.map(|c| c / l), size, size * 45.0).unwrap().expect("something looked at");
    let normal = surface_normal(area, t, size).unwrap();
    SiteProbe { target: t.map(|c| c as f32), normal, from: Some(from), size }
}

fn measured(area: &mut dyn Region, from: [f32; 3], dir: [f32; 3], size: f32) -> SiteContext {
    let probe = aim(area, from, dir, size);
    SiteContext::measure(area, &probe).unwrap()
}

/// Where the trunk's surface is, half a metre up, on its south side.
const SOUTH: f32 = 48.294_56;

/// A round home's tunnel, 7 cm into the trunk, half a metre up.
fn trunk_home() -> World {
    let carves = tunnel([32.0, 0.5, SOUTH], [0.0, 0.0, -1.0], 12, TINY);
    garden_with(&[], vec![oak(&carves).1])
}

#[test]
fn a_tinys_tunnel_into_a_trunk_is_measured_the_same_from_wherever_it_is_pointed_at() {
    let mut area = trunk_home();
    // Straight in at its end; slantwise at its side wall; down at its lip; at the bark beside it;
    // and obliquely on the flared inner mouth lip where the surface normal points sideways (-X).
    let looks = [
        ([32.0, 0.505, SOUTH + 0.06], [0.0, 0.0, -1.0]),
        ([32.02, 0.505, SOUTH + 0.06], [-0.3, -0.03, -0.95]),
        ([32.0, 0.53, SOUTH + 0.08], [0.0, -0.1, -1.0]),
        ([32.03, 0.50, SOUTH + 0.08], [-0.05, 0.0, -1.0]),
        ([31.995, 0.50, SOUTH + 0.04], [0.38, 0.02, -0.92]),
    ];
    for (from, dir) in looks {
        let site = measured(&mut area, from, dir, TINY);
        let c = site.cavity.as_ref().unwrap_or_else(|| panic!("no opening found from {from:?} along {dir:?}"));
        assert!((c.center[0] - 32.0).abs() < 4e-4 && (c.center[1] - 0.5).abs() < 4e-4, "its middle is on the tunnel's: {:?}", c.center);
        assert!((c.center[2] - SOUTH).abs() < 0.006 && c.center[2] < SOUTH, "its rim is just within the bark: {}", c.center[2]);
        assert!(c.axis[2] > 0.998 && c.axis[1] == 0.0, "it faces south, level: {:?}", c.axis);
        // 1.275 cm of brush and what the blend takes off besides.
        assert!((0.0130..0.0145).contains(&c.mean_radius()), "its radius: {}", c.mean_radius());
        assert!(c.roundness() > 0.97 && c.shape() == "round", "round: {} {}", c.roundness(), c.shape());
        assert!((0.065..0.080).contains(&c.depth) && !c.through, "its depth: {}", c.depth);
        assert_eq!(c.material, Material::Wood);
        assert!(c.walkable, "the tiny who carved it fit in");
        assert!(c.wall.iter().all(|&w| w > 0.005), "there is wood all round it: {:?}", c.wall);
        // The frame stands on the sill, upright, facing out, the tiny 1.7 m tall in it.
        assert_eq!(site.frame.scale, TINY / PERSON);
        assert!((site.frame.pos[1] - c.floor()).abs() < 1e-6);
        let out = site.frame.local_to_world_dir([0.0, 0.0, 1.0]);
        assert!((0..3).all(|a| (out[a] - c.axis[a]).abs() < 1e-5), "{out:?}");
        // In its local metres the opening is as tall as a doorway is to a person.
        let outline = site.outline();
        let top = outline.iter().map(|p| p[1]).fold(0.0, f32::max);
        assert!(outline.len() == CONTOUR && (2.6..2.9).contains(&top), "{top}");
        assert!(site.materials[0].0 == Material::Wood, "{:?}", site.materials);
    }
    // Somewhere a person cannot see an opening at all: too small for them.
    assert!(measured(&mut area, [32.0, 0.5, 50.0], [0.0, 0.0, -1.0], PERSON).cavity.is_none());
}

#[test]
fn a_site_file_keeps_every_bit_and_refuses_what_is_not_one() {
    let (mut a, mut b) = (trunk_home(), trunk_home());
    let look = ([32.02, 0.505, SOUTH + 0.06], [-0.3, -0.03, -0.95]);
    let mut site = measured(&mut a, look.0, look.1, TINY);
    assert_eq!(site.to_text(), measured(&mut b, look.0, look.1, TINY).to_text(), "measured again in the world made again: the same bits");
    site.area = "The walled garden".into();
    let text = site.to_text();
    let back = SiteContext::from_text(&text).unwrap();
    assert_eq!(back, site);
    assert_eq!(back.to_text(), text);
    let pond = measured(&mut a, [22.0, 1.6, 43.0], [0.0, -0.6, -0.8], PERSON);
    assert_eq!(SiteContext::from_text(&pond.to_text()).unwrap(), pond);

    for (from, to) in [
        ("plateau-site 1", "plateau-site 2"),
        ("contour ", "contour -1 "),
        ("wall wood", "wall jelly"),
        ("target ", "target NaN "),
        ("walkable 1", "walkable 1 7"),
        ("ground ", "grounds "),
        ("scale ", "scale -"),
    ] {
        assert!(SiteContext::from_text(&text.replacen(from, to, 1)).is_err(), "{from:?} made {to:?} must be refused");
    }
    let cut: String = text.lines().filter(|l| !l.starts_with("margin")).map(|l| format!("{l}\n")).collect();
    assert!(SiteContext::from_text(&cut).is_err(), "a cavity without its margin must be refused");

    // What Claude is told of it: local metres, the outline as Rust, the ground's grid.
    let words = site.describe_for_prompt();
    for want in ["const OPENING: [(f32, f32); 48]", "round", "walks in through it", "**Depth**", "**Ground**", "wood"] {
        assert!(words.contains(want), "{want:?} missing from:\n{words}");
    }
}

/// A block of soil on the lawn for a person to dig into, its south face at z = 49.
fn block() -> Edit {
    Edit::add(Shape::Box { half: [3.0, 2.5, 3.0], round: 0.1 }, [32.0, 1.5, 46.0], 0.0, Material::Soil, PERSON)
}

#[test]
fn openings_are_measured_for_what_they_are_and_level_ground_has_none() {
    // A square doorway, dug with the block brush.
    let mut edits = vec![block()];
    edits.extend((0..6).map(|k| Edit::carve(Shape::Box { half: [0.6, 1.1, 0.6], round: 0.05 }, [32.0, 1.1, 49.0 - 0.5 * k as f32], 0.0, None, PERSON)));
    let mut area = garden_with(&edits, Vec::new());
    let site = measured(&mut area, [32.3, 1.6, 52.0], [0.0, 0.0, -1.0], PERSON);
    let c = site.cavity.as_ref().expect("the doorway");
    assert_eq!(c.shape(), "square-cornered");
    assert!((c.width() - 1.2).abs() < 0.02 && (c.height() - 2.2).abs() < 0.03, "{} by {}", c.width(), c.height());
    assert!(c.floor().abs() < 0.03 && c.walkable && (2.8..3.2).contains(&c.depth), "{c:?}");
    assert!(c.axis[2] > 0.999);

    // A round cave a person dug into the mossy hill, seen from the south: on its sloping flank, level
    // and upright all the same.
    let hill = load_element("hill");
    let mut pl = placed(1, &hill, PlacementTransform::new([32.0, 0.0, 48.0], 0.0, 1.0), PERSON);
    pl.rot = None;
    let entry = SdfElementEntry::new(pl, hill.compiled.clone(), hill.sdf_bounds.unwrap());
    let cave: Vec<Edit> = (0..10).map(|k| Edit::carve(Shape::Sphere { radius: 0.45 }, [32.0, 0.55, 50.6 - 0.25 * k as f32], 0.05, None, 0.5)).collect();
    let mut area = garden_with(&cave, vec![entry]);
    let site = measured(&mut area, [32.0, 0.8, 52.5], [0.0, -0.05, -1.0], 0.5);
    let c = site.cavity.as_ref().expect("the hill's cave");
    assert!(c.upright() && c.axis[2] > 0.95, "{:?}", c.axis);
    assert!((0.38..0.52).contains(&c.mean_radius()) && c.roundness() > 0.85, "{} {}", c.mean_radius(), c.roundness());

    // Level lawn, and the pond's edge: ground and water, no opening.
    let mut garden = played::area("garden", 7);
    let lawn = measured(&mut garden, [46.0, 1.6, 48.0], [0.0, -0.6, -0.8], PERSON);
    assert!(lawn.cavity.is_none() && lawn.ground.rise < 0.1 && lawn.materials[0].0 == Material::Grass, "{lawn:?}");
    assert!(lawn.overhead.is_none() && (lawn.ground.elevation - lawn.target[1]).abs() < 0.05);
    let shore = measured(&mut garden, [22.0, 1.6, 46.5], [0.0, -0.6, -0.8], PERSON);
    let water = shore.water.as_ref().expect("the pond");
    assert!(shore.cavity.is_none() && (water.level + 0.3).abs() < 0.03 && water.depth > 0.3 && water.toward[1] < -0.5, "{water:?}");
    assert!(shore.summary().contains("water"), "{}", shore.summary());
    for size in [TINY, GIANT] {
        assert!(measured(&mut garden, [46.0, 0.5 * size, 48.0], [0.0, -0.6, -0.8], size).cavity.is_none(), "{size}");
    }
}

#[test]
fn the_round_door_goes_into_round_openings_of_every_scale_and_nothing_else() {
    let door = load_element("hobbit-door");
    let world = SceneWorld::default();
    // The trunk's; a person's into the soil block; a giant's into a hill of their own.
    let person = {
        let mut edits = vec![block()];
        edits.extend(tunnel([32.0, 1.15, 49.0], [0.0, 0.0, -1.0], 8, PERSON));
        garden_with(&edits, Vec::new())
    };
    let giant = {
        let mut edits = vec![Edit::add(Shape::Sphere { radius: 30.0 }, [32.0, 0.0, 12.0], 0.0, Material::Stone, GIANT)];
        edits.extend(bore([32.0, 10.0, 40.0], [0.0, 0.0, -1.0], 5, GIANT, 10.4));
        garden_with(&edits, Vec::new())
    };
    for (what, mut area, from, size) in [
        ("tiny", trunk_home(), [32.0, 0.505, SOUTH + 0.06], TINY),
        ("person", person, [32.0, 1.6, 53.0], PERSON),
        ("giant", giant, [32.0, 12.0, 66.0], GIANT),
    ] {
        let site = measured(&mut area, from, [0.0, 0.0, -1.0], size);
        let c = site.cavity.clone().unwrap_or_else(|| panic!("{what}: no opening"));
        assert!(c.walkable, "{what}: {}", site.summary());
        let (tf, rim) = fit_transform(&mut area, &world, &site, &door.preview_scene).unwrap().unwrap_or_else(|why| panic!("{what}: {why}"));
        let mean = c.mean_radius();
        assert!((tf.scale - mean * 1.07 / 1.13).abs() < 0.05 * mean, "{what}: scale {} for a radius of {mean}", tf.scale);
        assert!(rim.least > 0.04 * mean && rim.most < 0.15 * mean, "{what}: goes in {} to {} round a radius of {mean}", rim.least, rim.most);
        let fit = cavity_fit(&mut area, &world, &site, &door, 7, &[], tf).unwrap().unwrap_or_else(|why| panic!("{what}: {why}"));
        assert!(fit.leaf && fit.walkable, "{what}: {fit:?}");
        assert_eq!(cavity_fit(&mut area, &world, &site, &door, 7, &[], tf).unwrap(), Ok(fit), "{what}: judged again, the same");

        // From the site's file alone, its world not there: the same verdict.
        let mut alone = SiteRegion::new(SiteContext::from_text(&site.to_text()).unwrap());
        let (again, _) = fit_transform(&mut alone, &world, &site, &door.preview_scene).unwrap().unwrap();
        assert!((again.scale - tf.scale).abs() < 0.01 * tf.scale, "{what}");
        assert_eq!(cavity_fit(&mut alone, &world, &site, &door, 7, &[], tf).unwrap().map(|f| f.leaf), Ok(true), "{what}");

        // Wrong, and said how.
        let refused = |area: &mut World, tf: PlacementTransform| cavity_fit(area, &world, &site, &door, 7, &[], tf).unwrap().unwrap_err();
        let small = refused(&mut area, PlacementTransform { scale: 0.9 * tf.scale, ..tf });
        assert!(small.contains("a gap of") && small.contains("o'clock"), "{what}: {small}");
        let large = refused(&mut area, PlacementTransform { scale: 1.6 * tf.scale, ..tf });
        assert!(large.contains("inside the wall") || large.contains("into the wall") || large.contains("no frame"), "{what}: {large}");
        let mut aside = tf;
        aside.pos[0] += 0.25 * mean;
        let aside = refused(&mut area, aside);
        assert!(aside.contains("9 o'clock") || aside.contains("3 o'clock"), "{what}: {aside}");
        let mut out = tf;
        out.pos[2] += 0.6 * mean;
        assert!(refused(&mut area, out).contains("no frame"), "{what}");
    }

    // A square doorway is not round enough, and is told so; a bare lawn has nothing to fit.
    let mut edits = vec![block()];
    edits.extend((0..6).map(|k| Edit::carve(Shape::Box { half: [0.6, 1.1, 0.6], round: 0.05 }, [32.0, 1.1, 49.0 - 0.5 * k as f32], 0.0, None, PERSON)));
    let mut square = garden_with(&edits, Vec::new());
    let site = measured(&mut square, [32.3, 1.6, 52.0], [0.0, 0.0, -1.0], PERSON);
    let why = fit_transform(&mut square, &world, &site, &door.preview_scene).unwrap().unwrap_err();
    assert!(why.contains("o'clock"), "{why}");
    let mut garden = played::area("garden", 7);
    let lawn = measured(&mut garden, [46.0, 1.6, 48.0], [0.0, -0.6, -0.8], PERSON);
    assert!(fit_transform(&mut garden, &world, &lawn, &door.preview_scene).unwrap().is_err());

    // A pillar has no frame, a hill is no door, and the bridge fills nothing.
    let mut trunk = trunk_home();
    let site = measured(&mut trunk, [32.0, 0.505, SOUTH + 0.06], [0.0, 0.0, -1.0], TINY);
    for name in ["pillar", "lantern", "bridge", "tree"] {
        let element = load_element(name);
        assert!(fit_transform(&mut trunk, &world, &site, &element.preview_scene).unwrap().is_err(), "{name}");
        assert!(rim_fit(&mut trunk, &site, &element.preview_scene, site.frame).unwrap().is_err(), "{name}");
    }
    let hill = load_element("hill");
    assert!(cavity_fit(&mut trunk, &world, &site, &hill, 7, &[], site.frame).unwrap().unwrap_err().contains("landform"));
}

#[test]
fn a_tiny_is_kept_out_by_the_shut_door_opens_it_walks_in_and_shuts_it_behind() {
    // A home at the foot of the oak, between its roots (they grow along the axes): its floor level
    // with a path dug through the grass before it, from outside the trunk 12 cm in.
    let way = [std::f32::consts::FRAC_1_SQRT_2, 0.0, std::f32::consts::FRAC_1_SQRT_2];
    let at = |d: f32, y: f32| [32.0 + way[0] * d, y, 48.0 + way[2] * d];
    let bark = {
        let mut uncarved = garden_with(&[], vec![oak(&[]).1]);
        let t = body::look(&mut uncarved, at(0.8, 0.16).map(|c| c as f64), [-way[0], 0.0, -way[2]], TINY, 2.0).unwrap().unwrap();
        (((t[0] - 32.0).powi(2) + (t[2] - 48.0).powi(2)) as f32).sqrt()
    };
    let y = 0.16;
    let mut carves = tunnel(at(bark + 0.005, y), [-way[0], 0.0, -way[2]], 22, TINY);
    // A ledge to come along, level with the tunnel's floor.
    let ledge = Edit::add(Shape::Box { half: [0.05, 0.06, 0.05], round: 0.0 }, at(bark + 0.055, y - 0.0735), 0.0, Material::Stone, TINY);
    let mut edits = vec![ledge];
    edits.append(&mut carves);
    let mut pl = oak(&[]).0;
    pl.promoted = Some(0);
    let mesh = load_element("tree").build_mesh_sdf(pl.seed, &[], &[]).unwrap().unwrap();
    let mut area = garden_with(&edits, vec![SdfElementEntry::from_promoted_mesh(pl.clone(), mesh)]);

    let eye = at(bark + 0.06, y);
    let site = measured(&mut area, eye, [-way[0], 0.0, -way[2]], TINY);
    let cavity = site.cavity.clone().expect("the home's opening");
    assert!(cavity.walkable, "{}", site.summary());

    // The door, fitted, placed, kept in the world's save to the last bit.
    let door = load_element("hobbit-door");
    let mut world = SceneWorld::default();
    let (tf, _) = fit_transform(&mut area, &world, &site, &door.preview_scene).unwrap().unwrap_or_else(|why| panic!("{why}"));
    cavity_fit(&mut area, &world, &site, &door, 7, &[], tf).unwrap().unwrap_or_else(|why| panic!("{why}"));
    let pl_door = placed(2, &door, tf, TINY);
    let save = WorldSave::with_elements("The walled garden", 7, edits.clone(), vec![pl, pl_door.clone()]);
    let kept = WorldSave::from_text(&save.to_text()).unwrap();
    assert_eq!(kept, save);
    assert_eq!(kept.elements[1].transform(), pl_door.transform());

    let mut runtime = ElementRuntime::new();
    runtime.spawn_for_size(kept.elements[1].clone(), &door, &mut world, TINY).unwrap();

    let depth_in = |b: &Body| -> f32 {
        let d = [0, 1, 2].map(|a| b.feet[a] as f32 - cavity.center[a]);
        -(d[0] * cavity.axis[0] + d[2] * cavity.axis[2])
    };
    let yaw_in = cavity.axis[0].atan2(cavity.axis[2]);
    let walk = |area: &mut World, world: &SceneWorld, b: &mut Body, forward: f32, seconds: f32| {
        for _ in 0..(seconds * 120.0) as usize {
            let mut combined = Combined { region: area, world };
            b.step(&mut combined, Intent { forward, ..Intent::default() }, 1.0 / 120.0).unwrap();
        }
    };
    let mut time = 0.0;
    let mut press = |b: &Body, look: [f32; 3], runtime: &mut ElementRuntime, world: &mut SceneWorld| -> Option<String> {
        let eye = b.eye().map(|c| c as f32);
        let (owner, id) = world.raycast_dynamic_owned(eye, look, TINY * 12.0).map_or((0, 0), |(o, id, _)| (o, id));
        let mut ps = PlayerState { feet: b.feet.map(|c| c as f32), eye, look, size: TINY, grounded: true, target: id, aim: cavity.center, time, ..PlayerState::default() };
        let said = runtime.interact(&ps, owner, id, world).into_iter().find_map(|a| a.said);
        for _ in 0..60 {
            time += 1.0 / 30.0;
            ps.time = time;
            runtime.tick_all(1.0 / 30.0, &ps, owner, id, world);
        }
        said
    };

    let start = { [0, 1, 2].map(|a| (cavity.center[a] + cavity.axis[a] * 0.035) as f64) };
    let mut tiny = {
        let mut combined = Combined { region: &mut area, world: &world };
        Body::standing(&mut combined, start, TINY).unwrap()
    };
    tiny.yaw = yaw_in;
    assert!(tiny.on.is_some() && depth_in(&tiny) < -0.03, "the tiny stands before the door: {tiny:?}");

    // Shut, it keeps them out.
    walk(&mut area, &world, &mut tiny, 1.0, 4.0);
    assert!(depth_in(&tiny) < 0.0, "walked through the shut door, {} in", depth_in(&tiny));
    // Opened with F (the crosshair on it), they walk in.
    let inward = cavity.axis.map(|c| -c);
    assert_eq!(press(&tiny, inward, &mut runtime, &mut world).as_deref(), Some("The round door swings open."));
    assert!(world.lights.contains_key(&(2, 1)), "its lamp is lit while it is open");
    walk(&mut area, &world, &mut tiny, 1.0, 4.0);
    let within = depth_in(&tiny);
    assert!(within > 1.5 * TINY, "walked {} in through the open door", within);
    assert_eq!(tiny.on, Some(Material::Wood), "standing on the trunk's own wood inside");
    // Turned round, it is shut behind them, and they do not walk out.
    tiny.yaw += std::f32::consts::PI;
    assert_eq!(press(&tiny, cavity.axis, &mut runtime, &mut world).as_deref(), Some("The round door swings shut."));
    assert!(!world.lights.contains_key(&(2, 1)), "its lamp is out");
    walk(&mut area, &world, &mut tiny, 1.0, 4.0);
    assert!(depth_in(&tiny) > 0.0, "walked out through the shut door, {} in", depth_in(&tiny));
    // Nor is there any way to see out past it: no ray from the room's middle gets out of the trunk.
    let inside = [0, 1, 2].map(|a| cavity.center[a] as f64 - (cavity.axis[a] * 0.03) as f64);
    for i in -12i32..=12 {
        for j in -12i32..=12 {
            let (right, up) = cavity.across();
            let k = 0.95 * cavity.mean_radius() / 12.0;
            let to = [0, 1, 2].map(|a| cavity.center[a] + right[a] * i as f32 * k + up[a] * j as f32 * k);
            let d = [0, 1, 2].map(|a| to[a] - inside[a] as f32);
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
            let mut combined = Combined { region: &mut area, world: &world };
            let hit = body::look(&mut combined, inside, d.map(|c| c / l), TINY, 0.2).unwrap().expect("the door or the wall");
            let out: f32 = (0..3).map(|a| (hit[a] as f32 - cavity.center[a]) * cavity.axis[a]).sum();
            assert!(out < 0.25 * cavity.mean_radius(), "seen out past the shut door, at {hit:?}, {out} m out");
        }
    }
    // From inside it opens outwards, and lets them out.
    press(&tiny, cavity.axis, &mut runtime, &mut world);
    walk(&mut area, &world, &mut tiny, 1.0, 4.0);
    assert!(depth_in(&tiny) < -TINY, "walked out to {}", depth_in(&tiny));
}

#[test]
fn elements_for_open_ground_stand_on_it() {
    let mut garden = played::area("garden", 7);
    let world = SceneWorld::default();
    let site = measured(&mut garden, [46.0, 1.6, 48.0], [0.0, -0.6, -0.8], PERSON);
    for name in ["pillar", "lantern", "bridge", "tree"] {
        let element = load_element(name);
        let fits = fit_check(&mut garden, &world, &site, &element, 7, &[]).unwrap();
        assert_eq!(fits, Ok(()), "{name} on the lawn");
        let mut lifted = site.frame;
        lifted.pos[1] += 0.6;
        let why = ground_fit(&mut garden, PERSON, &element.preview_scene, lifted).unwrap().unwrap_err();
        assert!(why.contains("floats"), "{name}: {why}");
        let mut sunk = site.frame;
        sunk.pos[1] -= 12.0;
        let why = ground_fit(&mut garden, PERSON, &element.preview_scene, sunk).unwrap().unwrap_err();
        assert!(why.contains("buried"), "{name}: {why}");
    }
    // Over water it may reach out: the bridge from the shore.
    let mut shore = measured(&mut garden, [22.0, 1.6, 45.0], [0.0, -0.6, -0.8], PERSON).frame;
    shore.pos[2] -= 1.0;
    assert_eq!(ground_fit(&mut garden, PERSON, &load_element("bridge").preview_scene, shore).unwrap(), Ok(()));
}

#[test]
fn a_saved_world_opens_as_it_was_played() {
    let carves = tunnel([32.0, 0.5, SOUTH], [0.0, 0.0, -1.0], 12, TINY);
    let (tree, entry) = oak(&carves);
    let lawn = [Edit::add(Shape::Sphere { radius: 1.2 }, [38.0, 0.0, 50.0], 0.3, Material::Soil, PERSON)];
    let mut native = garden_with(&lawn, vec![entry]);
    let site = measured(&mut native, [32.0, 0.505, SOUTH + 0.06], [0.0, 0.0, -1.0], TINY);
    let door = load_element("hobbit-door");
    let (tf, _) = fit_transform(&mut native, &SceneWorld::default(), &site, &door.preview_scene).unwrap().unwrap();
    let mut save = WorldSave::with_elements("The walled garden", 7, lawn.to_vec(), vec![tree, placed(2, &door, tf, TINY)]);
    save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];

    // The same SDF, bit for bit, from the save's modules as from the sources played natively; the
    // same site measured in it, bit for bit.
    let open = OpenWorld::open(save.clone(), &repo_root(), None, TINY, Limits::default()).unwrap();
    let points = plateau_engine::points_near([32.0, 0.5, 48.2], 0.4, 4000, 5);
    let (mut a, mut b) = (Vec::new(), Vec::new());
    native.sample(&points, TINY, &mut a).unwrap();
    open.region().unwrap().sample(&points, TINY, &mut b).unwrap();
    assert_eq!(hash(&a), hash(&b));
    let again = measured(&mut open.region().unwrap(), [32.0, 0.505, SOUTH + 0.06], [0.0, 0.0, -1.0], TINY);
    assert_eq!(again.to_text(), site.to_text());

    // With its solid geometry, the shut door fills it.
    let probe = aim(&mut open.solid().unwrap(), [32.0, 0.505, SOUTH + 0.06], [0.0, 0.0, -1.0], TINY);
    assert!(probe.target[2] > SOUTH - 0.01, "looked at the door, not past it: {:?}", probe.target);

    // Saves that name no module, or one that is not there, say so.
    save.tiles.clear();
    assert!(OpenWorld::open(save.clone(), &repo_root(), None, TINY, Limits::default()).is_err());
    let garden = repo_root().join("areas/modules/garden.wasm");
    assert!(OpenWorld::open(save.clone(), &repo_root(), Some(&garden), TINY, Limits::default()).is_ok());
    save.elements[0].module = "elements/modules/no-such.wasm".into();
    assert!(OpenWorld::open(save, &repo_root(), Some(&garden), TINY, Limits::default()).is_err());
}

#[test]
fn the_hobbit_door_is_a_sandboxed_scene_element_that_is_found() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    assert!(catalog.discover().iter().any(|e| e.name == "Hobbit Door"), "hobbit-door.wasm must be discovered");
    let door = load_element("hobbit-door");
    assert!(!door.is_sdf());
    let solid: Vec<u32> = door.preview_scene.instances.iter().filter(|i| i.solid).map(|i| i.id).collect();
    assert_eq!(solid, [0, 1], "a still frame and one leaf are solid");
    let bytes = std::fs::read(repo_root().join("elements/modules/hobbit-door.wasm")).unwrap();
    assert_eq!(laws::nans_unseen(&bytes, 7, &[[0.0; 3]]).unwrap(), Ok(()));
    // Sunk into the floor, so it neither floats off nor is taken up by accident.
    let phys = plateau_engine::cut::ObjectPhysics::compute(&door.preview_scene, 1.0);
    assert!(phys.rooted);
}
