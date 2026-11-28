//! A sculpted `Scene` element draws itself (`plateau_engine::carved`): its own triangles kept as
//! made away from the carve, the carve's surface remade on the SDF physics reads, with no crack
//! between them, coloured as the element was (bark) or as cut wood; the same every time. In the
//! world, a promoted tree's layer carries its carved solid, and the terrain's chunks leave it out.

use std::collections::HashSet;
use std::path::PathBuf;

use plateau_engine::body::{self, Body};
use plateau_engine::carved::{Carver, Carving};
use plateau_engine::compose::ElementCatalog;
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::live::{Action, Actor, LayerChange, Outcome, SculptTarget, World};
use plateau_engine::mesh_sdf::MeshSdf;
use plateau_engine::modules::ModuleCheck;
use plateau_engine::physics::Tool;
use plateau_engine::{Answer, Limits, Material, MeshData, Region, Scene};

const TREE: &str = "elements/modules/tree.wasm";
const PERSON: f32 = 1.7;
const LAWN: [f32; 3] = [38.0, 0.0, 50.0];
/// How bark and fresh-cut wood are tinted (`elements/tree`, `cut::cut_face_tint`).
const BARK: [u8; 3] = [215, 198, 178];
const CUT_WOOD: [u8; 3] = [248, 228, 192];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn tree_scene(catalog: &mut ElementCatalog) -> Scene {
    let tree = catalog.get_or_load(TREE).unwrap();
    tree.compiled.instantiate(7).unwrap().scene().unwrap()
}

fn solid_mesh(scene: &Scene) -> &MeshData {
    let solid: Vec<_> = scene.instances.iter().filter(|i| i.solid).collect();
    assert_eq!(solid.len(), 1, "a carved scene has one solid instance");
    &scene.meshes[solid[0].mesh as usize]
}

fn tris(mesh: &MeshData) -> Vec<[[f32; 3]; 3]> {
    mesh.indices.as_chunks::<3>().0.iter().map(|t| [0, 1, 2].map(|k| mesh.vertices[t[k] as usize].pos)).collect()
}

fn bits(t: &[[f32; 3]; 3]) -> [[u32; 3]; 3] {
    t.map(|p| p.map(f32::to_bits))
}

fn field(sdf: &MeshSdf, edits: &[Edit], p: [f32; 3]) -> f32 {
    let mut a = sdf.sample(p, f32::INFINITY).unwrap();
    for e in edits {
        a = e.apply(p, a);
    }
    a.distance
}

/// Where the trunk's bark is east of its axis at height `y` (on the SDF).
fn bark_east(sdf: &MeshSdf, y: f32) -> f32 {
    let (mut inside, mut outside) = (0.0f32, 1.5f32);
    for _ in 0..40 {
        let mid = 0.5 * (inside + outside);
        if sdf.sample([mid, y, 0.0], f32::INFINITY).unwrap().distance < 0.0 {
            inside = mid;
        } else {
            outside = mid;
        }
    }
    0.5 * (inside + outside)
}

#[test]
fn a_carved_tree_keeps_its_own_triangles_and_remakes_only_the_carve() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let scene = tree_scene(&mut catalog);
    let sdf = catalog.get_or_build_mesh_sdf(TREE, 7, &[], &[]).unwrap().unwrap();
    let source = Carving::source_of(TREE, 7, &[]);

    let uncarved = Carver::new().carve(&scene, &Carving { source, sdf: sdf.clone(), edits: Vec::new() });
    let original = tris(solid_mesh(&uncarved));
    let non_solid = scene.instances.iter().filter(|i| !i.solid).count();
    assert_eq!(uncarved.instances.len(), non_solid + 1, "the canopy is kept, the solid merged into one instance");

    // A ball carved into the bark a metre up, from the east.
    let east = bark_east(&sdf, 1.0);
    let r = 0.15;
    let edit = Edit::carve(Shape::Sphere { radius: r }, [east + 0.05, 1.0, 0.0], 0.02, None, PERSON);
    let edits = vec![edit];
    let carving = Carving { source, sdf: sdf.clone(), edits: edits.clone() };
    let carved = Carver::new().carve(&scene, &carving);
    let mesh = solid_mesh(&carved);
    let out = tris(mesh);

    // Away from the carve, every original triangle is drawn exactly as it was.
    let made: HashSet<_> = out.iter().map(bits).collect();
    // Far from the carve all over (not only at its corners: a long trunk triangle may pass by it).
    let far = |t: &[[f32; 3]; 3]| {
        (0..=12).all(|i| {
            (0..=12 - i).all(|j| {
                let (u, v) = (i as f32 / 12.0, j as f32 / 12.0);
                let p = [0, 1, 2].map(|k| t[0][k] + (t[1][k] - t[0][k]) * u + (t[2][k] - t[0][k]) * v);
                edit.distance(p) > 0.12
            })
        })
    };
    let kept = original.iter().filter(|t| far(t)).filter(|t| made.contains(&bits(t))).count();
    let away = original.iter().filter(|t| far(t)).count();
    assert!(away > 100, "most of the tree is away from the carve ({away})");
    assert_eq!(kept, away, "every original triangle away from the carve is kept exactly");

    // Nothing is drawn inside what was carved, and the remade surface is on the SDF, within a
    // quarter of its grid's cell (a person's carve: 3.1 cm, a fortieth of their height).
    for v in &mesh.vertices {
        assert!(edit.distance(v.pos) > -0.01, "a vertex inside the carve: {:?}", v.pos);
        if edit.distance(v.pos) < 0.08 {
            let f = field(&sdf, &edits, v.pos);
            assert!(f.abs() < 0.25 * 0.03125, "a remade vertex off the surface by {f}: {:?}", v.pos);
        }
    }

    // The carved surface is cut wood; the bark about it is still bark.
    let cut = mesh.vertices.iter().filter(|v| v.tint == CUT_WOOD && edit.distance(v.pos) < 0.03).count();
    let bark = mesh.vertices.iter().filter(|v| v.tint == BARK && (0.0..0.06).contains(&edit.distance(v.pos))).count();
    assert!(cut > 20, "the carve is drawn as fresh-cut wood ({cut} vertices)");
    assert!(bark > 20, "the bark about the carve keeps its tint ({bark} vertices)");
    assert!(mesh.vertices.iter().all(|v| v.material == Material::Wood), "a tree's solid is wood");

    // No crack: every point of the carved surface about the carve is on a drawn triangle.
    let drawn = MeshSdf::from_solid_scene(&carved).unwrap();
    let mut asked = 0;
    for i in 0..24 {
        for j in 0..24 {
            // Where a ray from +z toward the trunk first meets the carved surface.
            let (x, y) = (east - 0.25 + 0.5 * i as f32 / 23.0, 0.75 + 0.5 * j as f32 / 23.0);
            let mut z = 0.45f32;
            let mut hit = None;
            while z > -0.45 {
                if field(&sdf, &edits, [x, y, z - 0.004]) < 0.0 {
                    let (mut out_z, mut in_z) = (z, z - 0.004);
                    for _ in 0..30 {
                        let mid = 0.5 * (out_z + in_z);
                        if field(&sdf, &edits, [x, y, mid]) < 0.0 { in_z = mid } else { out_z = mid }
                    }
                    hit = Some([x, y, 0.5 * (out_z + in_z)]);
                    break;
                }
                z -= 0.004;
            }
            let Some(p) = hit else { continue };
            if edit.distance(p) > 0.1 {
                continue;
            }
            asked += 1;
            let d = drawn.surface_at(p, 1.0).unwrap().distance;
            assert!(d < 0.004, "the drawn surface misses the carved one by {d} at {p:?}");
        }
    }
    assert!(asked > 40, "enough of the carved surface was asked ({asked})");

    // The same every time, made afresh or kept.
    let mut carver = Carver::new();
    let again = carver.carve(&scene, &carving);
    let kept_again = carver.carve(&scene, &carving);
    assert_eq!(solid_mesh(&again), mesh);
    assert_eq!(solid_mesh(&kept_again), mesh);
}

#[test]
fn a_rim_cut_by_a_box_stays_sharp() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let scene = tree_scene(&mut catalog);
    let sdf = catalog.get_or_build_mesh_sdf(TREE, 7, &[], &[]).unwrap().unwrap();
    let east = bark_east(&sdf, 1.0);
    // A flat-floored notch: its floor at y = 0.94, cut in from the east.
    let edit = Edit::carve(Shape::Box { half: [0.3, 0.06, 0.6], round: 0.0 }, [east + 0.15, 1.0, 0.0], 0.0, None, PERSON);
    let carving = Carving { source: Carving::source_of(TREE, 7, &[]), sdf, edits: vec![edit] };
    let carved = Carver::new().carve(&scene, &carving);
    let mesh = solid_mesh(&carved);
    // The notch's floor is flat and level: its vertices are on it, facing up, not rounded off.
    let floor: Vec<_> = mesh.vertices.iter().filter(|v| (v.pos[1] - 0.94).abs() < 0.002 && v.pos[0] > east - 0.12 && v.normal[1] > 0.9).collect();
    assert!(floor.len() > 10, "the notch has a level floor ({} vertices)", floor.len());
    // Its rim (where floor meets bark) is a corner, not a bevel: nothing between floor and bark
    // facing half up and half out.
    let bevel = mesh
        .vertices
        .iter()
        .filter(|v| v.pos[1] > 0.941 && v.pos[1] < 0.95 && v.pos[0] > east - 0.12)
        .filter(|v| v.normal[1] > 0.35 && v.normal[1] < 0.85)
        .count();
    assert!(bevel <= 2, "the notch's rim is rounded ({bevel} bevel vertices)");
}

fn garden() -> World {
    let mut save = WorldSave::new("The walled garden", 7, Vec::new());
    save.tiles = vec![SavedTile { at: [0, 0], module: "areas/modules/garden.wasm".into(), seed: 7 }];
    World::open(save, None, &repo_root(), Limits::default(), ModuleCheck::Strict, PERSON).unwrap().0
}

fn done(outcome: Outcome) -> plateau_engine::live::Summary {
    match outcome {
        Outcome::Done(s) => s,
        other => panic!("not done: {other:?}"),
    }
}

#[test]
fn a_sculpted_tree_draws_itself_carved_and_the_chunks_leave_it_out() {
    let mut world = garden();
    let mut body = Body::standing(&mut world.combined(), [LAWN[0] as f64 - 3.0, 40.0, LAWN[2] as f64], PERSON).unwrap();
    let tree = done(world.apply(
        Actor { id: "local", body: &mut body },
        Action::Place { module: TREE.into(), pos: LAWN, yaw: 0.0, rot: None, scale: 1.0, fitted: false, opening: None, seed: Some(7), params: Vec::new() },
    ))
    .added[0];
    let solid_before = world.runtime.local_scene_of(tree).unwrap().instances.iter().filter(|i| i.solid).count();
    world.take_changes();

    // A shallow ball carved into the bark from the east.
    let east = body::look(&mut world.combined(), [LAWN[0] as f64 + 2.0, 1.0, LAWN[2] as f64], [-1.0, 0.0, 0.0], PERSON, 4.0).unwrap().unwrap()[0] as f32;
    let edit = Edit::carve(Shape::Sphere { radius: 0.12 }, [east + 0.04, 1.0, LAWN[2]], 0.02, None, PERSON);
    let hit = [east, 1.0, LAWN[2]];
    done(world.apply(Actor { id: "local", body: &mut body }, Action::Sculpt { edit, hit, tool: Tool::Saw, target: SculptTarget::Terrain { aimed: Some(tree) } }));
    assert!(world.find(tree).unwrap().is_promoted(), "the tree is promoted by the carve");

    // Its layer is made again with its solid in it, carved (one instance, cut wood in it).
    let changes = world.take_changes();
    let spawned = changes
        .layers
        .iter()
        .rev()
        .find_map(|c| match c {
            LayerChange::Spawn { owner, scene } if *owner == tree => Some(scene),
            _ => None,
        })
        .expect("the tree's layer is made again");
    let mesh = solid_mesh(spawned);
    assert!(mesh.vertices.iter().any(|v| v.tint == CUT_WOOD), "the carve is drawn as cut wood");
    assert!(mesh.vertices.iter().any(|v| v.tint == BARK), "the rest is drawn as bark");
    assert!(solid_before >= 1);
    assert!(world.runtime.is_carved(tree));

    // Physics still has the tree in the SDF; the chunks draw the SDF without it.
    let composed = world.composed();
    assert_eq!(composed.sdf_elements.iter().filter(|e| e.placed.id == tree).count(), 1);
    let drawn = composed.without_promoted();
    assert!(drawn.sdf_elements.iter().all(|e| e.placed.id != tree));
    let mut c = world.combined();
    let mut out: Vec<Answer> = Vec::new();
    c.sample(&[[LAWN[0], 2.0, LAWN[2]]], PERSON, &mut out).unwrap();
    assert!(out[0].distance < 0.0, "the trunk is still solid to physics: {:?}", out[0]);
}

/// Played by hand (2026-11-28): a timber bridge dug round with a pickaxe (fifteen box strokes, 10
/// to 29 cm) was drawn with 538 thousand triangles from its 216, and its cut pieces with up to 1.2
/// million, until the graphics card ran out of memory: every plank triangle near a stroke was
/// divided down to a cell. Only those its kept part's edge crosses are divided now.
#[test]
fn a_bridge_carved_many_times_stays_light() {
    let mut catalog = ElementCatalog::new(repo_root(), Limits::default());
    let bridge = "elements/modules/bridge.wasm";
    let scene = catalog.get_or_load(bridge).unwrap().compiled.instantiate(7).unwrap().scene().unwrap();
    let sdf = catalog.get_or_build_mesh_sdf(bridge, 7, &[], &[]).unwrap().unwrap();
    let source = Carving::source_of(bridge, 7, &[]);
    let (lo, hi) = (sdf.lo(), sdf.hi());
    let mut edits = Vec::new();
    for k in 0..15 {
        let u = (k as f32 + 0.5) / 15.0;
        let half = [0.286, 0.214, 0.143, 0.1][k % 4];
        let at = [lo[0] + (hi[0] - lo[0]) * u, lo[1] + (hi[1] - lo[1]) * 0.4, lo[2] + (hi[2] - lo[2]) * (0.2 + 0.6 * ((k * 7 % 5) as f32 / 4.0))];
        edits.push(Edit::carve(Shape::Box { half: [half; 3], round: 0.07 * half }, at, 0.0, None, PERSON));
    }
    let original = tris(solid_mesh(&Carver::new().carve(&scene, &Carving { source, sdf: sdf.clone(), edits: Vec::new() }))).len();
    let carved = Carver::new().carve(&scene, &Carving { source, sdf: sdf.clone(), edits: edits.clone() });
    let n = tris(solid_mesh(&carved)).len();
    println!("bridge: {original} triangles, carved fifteen times: {n}");
    assert!(n < 150_000, "a carved bridge is drawn with {n} triangles (from {original})");
}
