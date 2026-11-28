//! Phase I (WORLD_PLAN.md): one element, made once, fits any number of openings by the params each
//! site gives it (`SiteContext::params`), placed where the site is, scaled by nothing but the
//! site's own frame: the carried door into a round tunnel, a square doorway and a wide low arch,
//! and into each again once it is widened.

mod played;

use std::path::PathBuf;
use std::sync::Arc;

use plateau_engine::body;
use plateau_engine::compose::{CompiledElement, Composed, ComposedLog};
use plateau_engine::edit::{Edit, EditLog, Shape, surface_normal};
use plateau_engine::fit::{cavity_fit, fit_to_site, fit_transform};
use plateau_engine::scene::SceneWorld;
use plateau_engine::site::{self, SiteContext, SiteProbe};
use plateau_engine::{Limits, Material, Region, SIZES};

const TINY: f32 = SIZES[0].1;
const PERSON: f32 = SIZES[1].1;

type World = Composed<played::Native, Arc<ComposedLog>>;

fn door() -> Arc<CompiledElement> {
    let rel = "elements/modules/hobbit-door.wasm";
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join(rel);
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e} (run ./elements/build.sh)", path.display()));
    CompiledElement::load(rel, &bytes, Limits::default()).unwrap()
}

fn garden_with(edits: &[Edit]) -> World {
    let log = ComposedLog::new(Arc::new(EditLog::from_edits(edits.to_vec()).unwrap()), Vec::new(), 1);
    Composed::new(played::area("garden", 7), Arc::new(log))
}

fn measured(area: &mut dyn Region, from: [f32; 3], size: f32) -> SiteContext {
    let t = body::look(area, from.map(|c| c as f64), [0.0, 0.0, -1.0], size, size * 45.0).unwrap().expect("something looked at");
    let normal = surface_normal(area, t, size).unwrap();
    SiteContext::measure(area, &SiteProbe { target: t.map(|c| c as f32), normal, from: Some(from), size }).unwrap()
}

/// A block of soil on the lawn, a person's height and more (`scale` times as large), its face at
/// z = 49.
fn block(scale: f32, size: f32) -> Edit {
    Edit::add(Shape::Box { half: [3.0 * scale, 2.5 * scale, 3.0 * scale], round: 0.1 * scale }, [32.0, 1.5 * scale, 49.0 - 3.0 * scale], 0.0, Material::Soil, size)
}

/// A tunnel of the round brush, radius `r`, straight in from z = 49 at height `y`.
fn round(r: f32, y: f32, size: f32) -> Vec<Edit> {
    (0..6).map(|k| Edit::carve(Shape::Sphere { radius: r }, [32.0, y, 49.0 - 0.45 * r * k as f32], 0.35 * r, None, size)).collect()
}

/// A square doorway `w` wide and `h` high, dug in from z = 49 with the block brush.
fn square(w: f32, h: f32, size: f32) -> Vec<Edit> {
    (0..6).map(|k| Edit::carve(Shape::Box { half: [0.5 * w, 0.5 * h, 0.27 * h], round: 0.02 * h }, [32.0, 0.5 * h, 49.0 - 0.23 * h * k as f32], 0.0, None, size)).collect()
}

/// A low doorway twice as wide as it is high, its corners well rounded.
fn arch(w: f32, size: f32) -> Vec<Edit> {
    (0..6).map(|k| Edit::carve(Shape::Box { half: [0.5 * w, 0.25 * w, 0.3 * w], round: 0.2 * w }, [32.0, 0.25 * w, 49.0 - 0.25 * w * k as f32], 0.0, None, size)).collect()
}

/// The door fitted to the opening seen from `from` in `edits` by the site's params, and held to it.
fn fits_by_params(what: &str, edits: &[Edit], from: [f32; 3], size: f32) -> (SiteContext, Vec<f32>) {
    let door = door();
    let world = SceneWorld::default();
    let mut area = garden_with(edits);
    let site = measured(&mut area, from, size);
    let c = site.cavity.clone().unwrap_or_else(|| panic!("{what}: no opening"));
    assert!(c.walkable, "{what}: {}", site.summary());
    let fitted = fit_to_site(&mut area, &world, &site, &door, 7).unwrap().unwrap_or_else(|why| panic!("{what}: {why}"));
    assert_eq!(fitted.tf, site.frame, "{what}: placed where the site is, as it is");
    assert_eq!(fitted.params, site.params(), "{what}: given the site's params");
    let fit = cavity_fit(&mut area, &world, &site, &door, 7, &fitted.params, fitted.tf).unwrap().unwrap_or_else(|why| panic!("{what}: {why}"));
    assert!(fit.leaf && fit.walkable, "{what}: {fit:?}");
    (site, fitted.params)
}

#[test]
fn the_site_s_params_are_its_opening_in_local_metres() {
    let mut edits = vec![block(1.0, PERSON)];
    edits.extend(square(1.2, 2.2, PERSON));
    let mut area = garden_with(&edits);
    let s = measured(&mut area, [32.3, 1.6, 52.0], PERSON);
    let p = s.params();
    assert_eq!(p.len(), site::PARAMS);
    assert_eq!(p[site::P_OPENING], 1.0);
    assert!((p[site::P_WIDTH] - 1.2).abs() < 0.02 && (p[site::P_HEIGHT] - 2.2).abs() < 0.03, "{p:?}");
    assert!((p[site::P_MIDDLE] - 1.1).abs() < 0.05, "{p:?}");
    assert_eq!(p[site::P_WALKABLE], 1.0);
    // Straight right, up, left and down from the middle: half the width, half the height.
    let r = |k: usize| p[site::P_RADII + k];
    let q = site::SPOKES / 4;
    assert!((r(0) - 0.6).abs() < 0.02 && (r(2 * q) - 0.6).abs() < 0.02, "{p:?}");
    assert!((r(q) - 1.1).abs() < 0.03 && (r(3 * q) - p[site::P_MIDDLE]).abs() < 1e-6, "{p:?}");

    // The same doorway dug by a tiny at a hundredth the size: the same params, to within what
    // a hundredth of the measuring is.
    let mut edits = vec![block(0.01, TINY)];
    edits.extend(square(0.012, 0.022, TINY));
    let mut area = garden_with(&edits);
    let tiny = measured(&mut area, [32.003, 0.016, 49.03], TINY).params();
    for k in 0..site::PARAMS {
        assert!((tiny[k] - p[k]).abs() < 0.05 + 0.03 * p[k].abs(), "param {k}: {} against {}", tiny[k], p[k]);
    }

    // Lawn: no opening, all nought but the first.
    let mut lawn = played::area("garden", 7);
    let probe = SiteProbe { target: [46.0, 0.0, 47.0], normal: [0.0, 1.0, 0.0], from: Some([46.0, 1.6, 48.0]), size: PERSON };
    let none = SiteContext::measure(&mut lawn, &probe).unwrap().params();
    assert_eq!(none, vec![0.0; site::PARAMS]);
}

#[test]
fn the_door_made_once_fits_three_openings_by_params_alone_and_again_once_each_is_widened() {
    let person = |cut: Vec<Edit>| {
        let mut e = vec![block(1.0, PERSON)];
        e.extend(cut);
        e
    };
    // What, dug as it is and then widened, seen from where, by someone how tall.
    type Case = (&'static str, Vec<Edit>, Vec<Edit>, [f32; 3], f32);
    let cases: [Case; 3] = [
        ("round tunnel", person(round(1.15, 1.1, PERSON)), person(round(1.4, 1.3, PERSON)), [32.0, 1.3, 53.0], PERSON),
        ("square doorway", person(square(1.2, 2.2, PERSON)), person(square(1.6, 2.4, PERSON)), [32.3, 1.6, 53.0], PERSON),
        ("tiny's low arch", {
            let mut e = vec![block(0.015, TINY)];
            e.extend(arch(0.04, TINY));
            e
        }, {
            let mut e = vec![block(0.015, TINY)];
            e.extend(arch(0.05, TINY));
            e
        }, [32.0, 0.01, 49.05], TINY),
    ];
    let door = door();
    for (what, before, after, from, size) in cases {
        let (site, params) = fits_by_params(what, &before, from, size);
        // Made for no opening in particular, the round door does not fit the square or the arch
        // however it is turned and scaled; by params, it does.
        if what != "round tunnel" {
            let mut area = garden_with(&before);
            assert!(fit_transform(&mut area, &SceneWorld::default(), &site, &door.preview_scene).unwrap().is_err(), "{what}");
        }
        // Widened: the old params no longer fit where the opening now is; the new ones do.
        let (wider, again) = fits_by_params(&format!("{what}, widened"), &after, from, size);
        assert!(again[site::P_WIDTH] > params[site::P_WIDTH] * 1.1, "{what}: {} then {}", params[site::P_WIDTH], again[site::P_WIDTH]);
        let mut area = garden_with(&after);
        let old = cavity_fit(&mut area, &SceneWorld::default(), &wider, &door, 7, &params, wider.frame).unwrap();
        assert!(old.is_err(), "{what}: the door made for the opening before it was widened should not fit it now");
    }
}


mod in_the_world {
    //! The same, played: placed through `World::apply` with its site's params, the door is fitted
    //! again when the doorway it fills is widened (undone and redone with the widening); a door
    //! placed without params that no longer fits is told of.

    use super::*;
    use plateau_engine::body::Body;
    use plateau_engine::edit::{SavedTile, WorldSave};
    use plateau_engine::graph::Opening;
    use plateau_engine::live::{Action, Actor, Outcome, SculptTarget, Summary, World};
    use plateau_engine::modules::ModuleCheck;
    use plateau_engine::physics::Tool;

    const DOOR: &str = "elements/modules/hobbit-door.wasm";

    fn open() -> World {
        let mut save = WorldSave::new("The walled garden", 7, Vec::new());
        save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        World::open(save, None, &root, Limits::default(), ModuleCheck::Strict, PERSON).unwrap().0
    }

    fn done(outcome: Outcome) -> Summary {
        match outcome {
            Outcome::Done(s) => s,
            other => panic!("not done: {other:?}"),
        }
    }

    fn sculpt(world: &mut World, body: &mut Body, edits: &[Edit]) -> Vec<Summary> {
        edits
            .iter()
            .map(|&edit| done(world.apply(Actor { id: "local", body }, Action::Sculpt { edit, hit: edit.center, tool: Tool::All, target: SculptTarget::Terrain { aimed: None } })))
            .collect()
    }

    fn site(world: &mut World, from: [f32; 3]) -> SiteContext {
        measured(&mut world.combined(), from, PERSON)
    }

    /// Placed as play places what it fitted: where `fit_to_site` says, given what it says.
    fn place_fitted(world: &mut World, body: &mut Body, site: &SiteContext) -> u32 {
        let door = door();
        let fitted = fit_to_site(&mut world.combined(), &SceneWorld::default(), site, &door, 7).unwrap().unwrap();
        let tf = fitted.tf;
        let place = Action::Place {
            module: DOOR.into(),
            pos: tf.pos,
            yaw: 0.0,
            rot: Some(tf.rot),
            scale: tf.scale,
            fitted: true,
            opening: Some(Opening { target: site.target, normal: site.normal, size: site.size }),
            seed: Some(7),
            params: fitted.params,
        };
        done(world.apply(Actor { id: "local", body }, place)).added[0]
    }

    #[test]
    fn a_door_given_its_doorway_s_params_is_fitted_again_when_the_doorway_is_widened() {
        let mut world = open();
        let mut body = Body::standing(&mut world.combined(), [36.0, 40.0, 54.0], PERSON).unwrap();
        let mut doorway = vec![block(1.0, PERSON)];
        doorway.extend(square(1.2, 2.2, PERSON));
        sculpt(&mut world, &mut body, &doorway);
        let from = [32.3, 1.6, 53.0];
        let before = site(&mut world, from);
        let id = place_fitted(&mut world, &mut body, &before);
        let given = world.find(id).unwrap().params.clone();
        assert_eq!(given, before.params());

        // Widened by one stroke: fitted again, at once, and nothing to tell.
        let wider = Edit::carve(Shape::Box { half: [0.8, 1.2, 0.5], round: 0.05 }, [32.0, 1.2, 48.9], 0.0, None, PERSON);
        let s = sculpt(&mut world, &mut body, &[wider]).remove(0);
        assert!(s.said.as_deref().is_none_or(|w| !w.contains("no longer fits")), "{:?}", s.said);
        let now = world.find(id).unwrap().clone();
        assert!(!now.is_promoted() && now.local_edits.is_empty(), "made again whole, not carved by the stroke that widened its doorway");
        assert!(now.params[site::P_WIDTH] > 1.5 && now.params[site::P_WIDTH] < 1.7, "given {:?}", &now.params[..10]);
        assert!(now.params[site::P_HEIGHT] > 2.3, "given {:?}", &now.params[..10]);
        // Measured with the door taken away, the doorway is what it was given.
        let mut without = world.scene.clone();
        without.remove_layer(id);
        let probe = SiteProbe { target: before.target, normal: before.normal, from: None, size: PERSON };
        let again = SiteContext::measure(&mut plateau_engine::scene::Combined { region: &mut world.region, world: &without }, &probe).unwrap();
        assert_eq!(now.params, again.params());
        // Where the new site is (its turn normalised as a placement keeps it).
        let (a, b) = (now.transform(), again.frame);
        assert!(a.pos == b.pos && a.scale == b.scale && (0..4).all(|i| (a.rot[i] - b.rot[i]).abs() < 1e-5), "{a:?} {b:?}");
        let fit = cavity_fit(&mut world.region, &without, &again, &door(), now.seed, &now.params, now.transform()).unwrap();
        assert!(fit.is_ok_and(|f| f.leaf), "fits the doorway as it is now");

        // Undone, the door is as it was made for the narrower doorway; redone, for the wider.
        done(world.apply(Actor { id: "local", body: &mut body }, Action::Undo));
        assert_eq!(world.find(id).unwrap().params, given);
        done(world.apply(Actor { id: "local", body: &mut body }, Action::Redo));
        assert_eq!(world.find(id).unwrap().params, now.params);

        // Saved and opened again: the params are the door's.
        let path = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fit-params-door.plateau");
        world.save(&path).unwrap();
        let reopened = World::open(WorldSave::load(&path).unwrap(), None, &world.root.clone(), Limits::default(), ModuleCheck::Strict, PERSON).unwrap().0;
        assert_eq!(reopened.find(id).unwrap().params, now.params);
    }

    #[test]
    fn a_door_fitted_by_scale_alone_is_told_of_when_its_opening_outgrows_it() {
        let mut world = open();
        let mut body = Body::standing(&mut world.combined(), [36.0, 40.0, 54.0], PERSON).unwrap();
        let mut tunnel = vec![block(1.0, PERSON)];
        tunnel.extend(round(1.15, 1.1, PERSON));
        sculpt(&mut world, &mut body, &tunnel);
        let from = [32.0, 1.3, 53.0];
        let s = site(&mut world, from);
        let door = door();
        let (tf, _) = fit_transform(&mut world.combined(), &SceneWorld::default(), &s, &door.preview_scene).unwrap().unwrap();
        let place = Action::Place {
            module: DOOR.into(),
            pos: tf.pos,
            yaw: 0.0,
            rot: Some(tf.rot),
            scale: tf.scale,
            fitted: true,
            opening: Some(Opening { target: s.target, normal: s.normal, size: s.size }),
            seed: Some(7),
            params: Vec::new(),
        };
        let id = done(world.apply(Actor { id: "local", body: &mut body }, place)).added[0];
        let wider = Edit::carve(Shape::Sphere { radius: 1.5 }, [32.0, 1.4, 48.6], 0.3, None, PERSON);
        let said = sculpt(&mut world, &mut body, &[wider]).remove(0).said.unwrap_or_default();
        assert!(said.contains("Hobbit Door") && said.contains("no longer fits its opening"), "{said}");
        assert!(world.find(id).unwrap().params.is_empty() && world.find(id).unwrap().transform() == tf, "it was given nothing, and not moved");
    }
}

#[test]
fn the_guide_gives_the_params_where_the_engine_puts_them() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
    let guide = std::fs::read_to_string(root.join("make/ELEMENT_GUIDE.md")).unwrap();
    for (index, words) in [
        (site::P_OPENING.to_string(), "1 if there is an opening"),
        (format!("{}, {}", site::P_WIDTH, site::P_HEIGHT), "width and height"),
        (site::P_MIDDLE.to_string(), "middle is above the sill"),
        (site::P_DEPTH.to_string(), "how deep"),
        (site::P_THROUGH.to_string(), "goes on through"),
        (site::P_WALL.to_string(), "least wall"),
        (site::P_ROUND.to_string(), "roundness"),
        (site::P_HEADROOM.to_string(), "headroom"),
        (site::P_WALKABLE.to_string(), "walks in"),
        (format!("{} to {}", site::P_RADII, site::PARAMS - 1), &format!("{} distances", site::SPOKES)),
    ] {
        assert!(guide.lines().any(|l| l.starts_with(&format!("| {index} |")) && l.contains(words)), "the guide's row {index} ({words})");
    }
}
