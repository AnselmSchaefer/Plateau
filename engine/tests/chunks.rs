//! The garden's surface in chunks (POC_PLAN.md, step 2): the coarse pass changes nothing but how
//! much is asked, neighbours meet without a seam, the triangles face out of the solid, and the
//! chunks a viewer wants fill the space about them once, with no gaps, at every size.

mod played;

use std::collections::{HashMap, HashSet};

use plateau_engine::chunk::{self, CELLS, Key, Making, Mesh};
use plateau_engine::rings::{self, COARSEST, Rings, finest, shown, wanted};
use plateau_engine::{Region, SIZES, WIDTH};

const SEED: u64 = 7;

/// The chunk at level `level` holding the point `at`.
fn holding(at: [f64; 3], level: i32) -> Key {
    let side = 2f64.powi(level) * CELLS as f64;
    Key::new(level, at.map(|c| (c / side).floor() as i32))
}

/// Chunks where the garden has something to mesh, at each size's finest level and coarser: the
/// crack, the bench's seat, a lily pad's pond, the gate, the lawn.
fn chunks() -> Vec<Key> {
    let mut keys = Vec::new();
    for at in [[53.7, 0.05, 30.0], [40.0, 4.5, 18.0], [22.0, -0.3, 36.0], [32.0, 1.2, 54.0], [30.0, 0.0, 30.0]] {
        for level in [-11, -10, -6, -4, -1, 0] {
            keys.push(holding(at, level));
        }
    }
    keys
}

fn made(key: Key, making: Making) -> (Option<Mesh>, usize) {
    let made = chunk::make_with(&mut played::area("garden", SEED), key, making).unwrap();
    (made.mesh, made.samples)
}

#[test]
fn the_coarse_pass_makes_the_same_surface_asking_far_less() {
    let plain = Making { coarse: false, skirts: false, shading: false };
    let (mut all, mut fewer, mut meshes) = (0, 0, 0);
    for key in chunks() {
        let (a, n) = made(key, plain);
        let (b, m) = made(key, Making { coarse: true, ..plain });
        assert_eq!(a, b, "{key:?}: the coarse pass changed the surface");
        all += n;
        fewer += m;
        meshes += a.is_some() as usize;
    }
    assert!(meshes >= 20, "only {meshes} of the chunks have a surface");
    assert!(fewer * 2 < all, "the coarse pass asked {fewer} points of {all}");
}

/// A mesh's edges that only one triangle has, as their ends' places in the area, to a micrometre.
fn open_edges(key: Key, mesh: &Mesh) -> Vec<([i64; 3], [i64; 3])> {
    let o = key.origin();
    let place = |v: u32| -> [i64; 3] { [0, 1, 2].map(|a| ((o[a] + mesh.positions[v as usize][a] as f64) * 1e6).round() as i64) };
    let mut count: HashMap<(u32, u32), usize> = HashMap::new();
    for t in mesh.indices.as_chunks::<3>().0 {
        for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let mut edges: Vec<_> = count.into_iter().filter(|(_, n)| *n == 1).map(|((a, b), _)| (place(a).min(place(b)), place(a).max(place(b)))).collect();
    edges.sort();
    edges
}

#[test]
fn neighbouring_chunks_of_a_level_meet_without_a_seam() {
    let plain = Making { skirts: false, shading: false, ..Making::default() };
    let mut checked = 0;
    for key in chunks() {
        for axis in 0..3 {
            let mut next = key;
            next.at[axis] += 1;
            let (Some(a), _) = made(key, plain) else { continue };
            let (Some(b), _) = made(next, plain) else { continue };
            // Where the two meet: within a cell of their shared face, away from its edges (where
            // the mesh ends because the chunks do, along the other axes).
            let (o, v, side) = (next.origin(), key.voxel(), key.side());
            let near = |e: &([i64; 3], [i64; 3])| {
                [e.0, e.1].iter().all(|p| {
                    (0..3).all(|a| {
                        let c = p[a] as f64 * 1e-6;
                        if a == axis { (c - o[a]).abs() < v * 1.5 } else { c > o[a] + 1.5 * v && c < o[a] + side - 1.5 * v }
                    })
                })
            };
            let (ea, eb): (Vec<_>, Vec<_>) = (open_edges(key, &a).into_iter().filter(near).collect(), open_edges(next, &b).into_iter().filter(near).collect());
            // The same edges, but for a micrometre or so: each chunk keeps its vertices in 32-bit
            // floats from its own corner.
            let close = |p: [i64; 3], q: [i64; 3]| (0..3).all(|a| (p[a] - q[a]).abs() <= 4);
            assert_eq!(ea.len(), eb.len(), "{key:?} and the next along axis {axis} end at different edges");
            for e in &ea {
                assert!(
                    eb.iter().any(|f| (close(e.0, f.0) && close(e.1, f.1)) || (close(e.0, f.1) && close(e.1, f.0))),
                    "{key:?} ends at {e:?}, and the next along axis {axis} does not"
                );
            }
            checked += ea.len();
        }
    }
    assert!(checked > 500, "only {checked} edges where chunks meet");
}

#[test]
fn the_triangles_face_out_of_the_solid() {
    // Not at the coarsest levels, where the wall is thinner than a voxel and its two faces meet.
    for key in chunks().into_iter().filter(|k| k.level <= -2) {
        let (Some(mesh), _) = made(key, Making { skirts: false, ..Making::default() }) else { continue };
        let mut agree = 0;
        for t in mesh.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| mesh.positions[i as usize]);
            let (u, v) = ([0, 1, 2].map(|i| b[i] - a[i]), [0, 1, 2].map(|i| c[i] - a[i]));
            let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let g = mesh.normals[t[0] as usize];
            agree += (n[0] * g[0] + n[1] * g[1] + n[2] * g[2] > 0.0) as usize;
        }
        let triangles = mesh.indices.len() / 3;
        assert!(agree * 100 >= triangles * 97, "{key:?}: {agree} of {triangles} triangles face along the gradient");
    }
}

#[test]
fn skirts_hang_from_every_open_edge_and_hollows_are_darker() {
    let key = holding([53.7, 0.05, 30.0], -6);
    let (Some(bare), _) = made(key, Making { skirts: false, ..Making::default() }) else { panic!("no surface at the crack") };
    let (Some(skirted), _) = made(key, Making::default()) else { panic!() };
    let open = open_edges(key, &bare).len();
    assert!(skirted.indices.len() >= bare.indices.len() + open * 6, "skirts and collar missing");
    // The wall's foot, where it meets the ground, is darker than the open lawn.
    let (Some(lawn), _) = made(holding([30.0, 0.0, 30.0], -4), Making::default()) else { panic!("no lawn") };
    let mean = |m: &Mesh| m.open.iter().sum::<f32>() / m.open.len() as f32;
    assert!(mean(&lawn) > 0.9, "the lawn is {} open", mean(&lawn));
    assert!(bare.open.iter().any(|&o| o < 0.6), "nothing at the wall's foot is in shade");
}

#[test]
fn the_empty_sky_is_asked_once() {
    let key = holding([30.0, 40.0, 30.0], -4);
    let (mesh, samples) = made(key, Making::default());
    assert!(mesh.is_none());
    assert_eq!(samples, 1);
}

/// Numbers from 0 to 1, the same every time (xorshift).
fn numbers(seed: u32) -> impl FnMut() -> f64 {
    let mut s = seed | 1;
    move || {
        s ^= s << 13;
        s ^= s >> 17;
        s ^= s << 5;
        s as f64 / u32::MAX as f64
    }
}

/// That `keys`, wanted by a viewer at `eye`, `size` tall, hold every point between `lo` and `hi`
/// over the area once (`n` points, half of them near the eye), the finest about the eye.
fn fill_once(keys: &[Key], eye: [f64; 3], size: f32, (lo, hi): ([f64; 3], [f64; 3]), n: usize) {
    assert_eq!(keys[0].level, finest(size));
    assert!(keys.iter().all(|k| k.level >= keys[0].level && k.level <= COARSEST));
    let holds = |k: &Key, p: &[f64; 3]| (0..3).all(|a| (k.origin()[a]..k.origin()[a] + k.side()).contains(&p[a]));
    assert!(keys.iter().any(|k| k.level == finest(size) && holds(k, &eye)), "the eye at {eye:?} is not in a finest chunk");
    let mut next = numbers(0x2545_f491);
    for k in 0..n {
        let p: [f64; 3] = if k % 2 == 0 {
            [0, 1, 2].map(|a| (eye[a] + (next() * 2.0 - 1.0) * size as f64 * 4.0).clamp(lo[a], hi[a] - 1e-9))
        } else {
            [0, 1, 2].map(|a| lo[a] + next() * (hi[a] - lo[a]))
        };
        if p[0] < 0.0 || p[2] < 0.0 || p[0] >= WIDTH as f64 || p[2] >= WIDTH as f64 {
            continue;
        }
        let holders = keys.iter().filter(|k| holds(k, &p)).count();
        assert_eq!(holders, 1, "{p:?} is in {holders} chunks for a viewer {size} m tall at {eye:?}");
    }
}

#[test]
fn the_chunks_a_viewer_wants_fill_the_space_about_them_once() {
    let eyes = [[32.0, 1.6, 60.0], [53.65, 0.06, 30.0], [40.0, 13.0, 23.0], [0.3, 0.0, 63.9], [12.0, 30.0, 5.0]];
    for (_, size) in SIZES.iter().chain(&[(SIZES[0].0, 0.2), (SIZES[0].0, 5.0)]) {
        for eye in eyes {
            let mut rings = Rings::default();
            let keys = rings.wanted(eye, *size);
            fill_once(&keys, eye, *size, rings.reach().unwrap(), 20_000);
        }
    }
    // A tiny person sees twelve levels; a giant two.
    assert_eq!(rings::finest(0.017), -11);
    assert_eq!(rings::finest(1.7), -4);
    assert_eq!(rings::finest(17.0), -1);
}

#[test]
fn blocks_lag_behind_a_walker_and_still_fill_the_space_once() {
    for (_, size) in SIZES {
        let mut rings = Rings::default();
        let mut next = numbers(7);
        let mut eye = [53.6, 0.5 * size as f64, 30.0];
        let (mut changes, mut changes_without_lag, mut before, mut before_without_lag) = (0, 0, Vec::new(), Vec::new());
        // To and fro about where it started, and drifting, a tenth of a height a step.
        for step in 0..400 {
            let way = if step % 40 < 20 { 1.0 } else { -1.0 };
            for a in [0, 2] {
                eye[a] = (eye[a] + (way * 0.7 + next() - 0.5) * 0.1 * size as f64).clamp(1.0, WIDTH as f64 - 1.0);
            }
            let mut keys = rings.wanted(eye, size);
            fill_once(&keys, eye, size, rings.reach().unwrap(), 400);
            keys.sort();
            changes += (keys != before) as usize;
            before = keys;
            let mut plain = wanted(eye, size);
            plain.sort();
            changes_without_lag += (plain != before_without_lag) as usize;
            before_without_lag = plain;
        }
        if changes_without_lag > 1 {
            assert!(changes * 3 < changes_without_lag * 2, "a {size} m viewer's chunks changed {changes} times lagging, {changes_without_lag} not");
        }
    }
    for (_, size) in SIZES {
        let mut rings = Rings::default();
        let mut eye = [32.0, 0.9 * size as f64, 40.0];
        for step in 0..24 {
            let angle = step as f64 * 0.6;
            let look = [angle.cos(), angle.sin()];
            eye[0] = (eye[0] + look[0] * 0.35 * size as f64).clamp(1.0, WIDTH as f64 - 1.0);
            eye[2] = (eye[2] + look[1] * 0.35 * size as f64).clamp(1.0, WIDTH as f64 - 1.0);
            let keys = rings.wanted_looking(eye, look, size, COARSEST);
            fill_once(&keys, eye, size, rings.reach().unwrap(), 800);
        }
    }
}

/// That no chunk shown overlaps another: none is shown twice, or within another shown.
fn no_overlaps(shown: &[Key]) {
    let set: HashSet<Key> = shown.iter().copied().collect();
    assert_eq!(set.len(), shown.len(), "a chunk is shown twice");
    for k in shown {
        let mut p = *k;
        while p.level < COARSEST {
            p = p.parent();
            assert!(!set.contains(&p), "{k:?} is shown within {p:?}, also shown");
        }
    }
}

#[test]
fn what_is_shown_never_overlaps_and_stands_in_for_what_is_not_made() {
    for (_, size) in SIZES {
        let eye = [53.6, 0.5 * size as f64, 30.0];
        let mut rings = Rings::default();
        let old = rings.wanted(eye, size);
        // All made: all shown, as wanted.
        let (all, holes) = shown(&old, &|_| true);
        assert_eq!((all.len(), holes), (old.len(), 0));
        // Nothing made: nothing shown, every wanted chunk a hole.
        let (none, holes) = shown(&old, &|_| false);
        assert_eq!((none.len(), holes), (0, old.len()));
        // Some made at random: no overlaps, and only made chunks.
        let mut next = numbers(3);
        let made: HashSet<Key> = old.iter().flat_map(|k| [*k, k.parent()]).filter(|_| next() < 0.6).collect();
        let (some, _) = shown(&old, &|k| made.contains(&k));
        no_overlaps(&some);
        assert!(some.iter().all(|k| made.contains(k)));
        // A few steps on, with only the old view made: the new view is all stood in for.
        let mut eye = eye;
        let made: HashSet<Key> = old.iter().copied().collect();
        for _ in 0..6 {
            eye[0] += 0.3 * size as f64;
            let new = rings.wanted(eye, size);
            let (stand_ins, holes) = shown(&new, &|k| made.contains(&k));
            no_overlaps(&stand_ins);
            assert_eq!(holes, 0, "a {size} m viewer walking to {eye:?} with only the old view made sees holes");
        }
        // Grown, with only the smaller view made: the finer chunks stand in for the coarser.
        let grown = Rings::default().wanted(eye, size * 4.0);
        let (finer, _) = shown(&grown, &|k| made.contains(&k));
        no_overlaps(&finer);
        assert!(finer.iter().any(|k| k.level < finest(size * 4.0)), "nothing finer stands in for a viewer grown");
    }
}

/// The tiny in the cave: a person's world, and every larger, has wall where they stand, so the
/// coarser levels would be drawn as solid all about the eye, and the cave's far walls and roof
/// facing away. So the levels coarser than the last that sees the eye in the open are asked for
/// that one's viewer; in the open nothing changes. Found walking the table of step 5, in the
/// village under its bowl.
#[test]
fn in_a_hollow_only_the_small_see_the_coarser_levels_are_asked_for_a_viewer_who_sees_it() {
    let mut area = played::area("garden", SEED);
    let floor = area.describe().unwrap().places.iter().find(|p| p.name.contains("cave")).unwrap().at[1] as f64;
    let tiny = SIZES[0].1;
    let cave = [54.0, floor + 0.015, 30.15];
    let open = rings::open(&mut area, cave, tiny).unwrap();
    assert!(open < finest(SIZES[1].1), "in the cave the coarser levels are asked for a person ({open})");
    assert!(open > finest(tiny), "the tiny's own level and some above it see the cave ({open})");
    // In the open, nothing is capped: on the lawn, at every size.
    for (_, size) in SIZES {
        assert_eq!(rings::open(&mut area, [32.0, 0.9 * size as f64, 50.0], size).unwrap(), COARSEST, "a {size} m viewer on the lawn");
    }
    // The chunks wanted still fill the space once, and stand in for one another as before.
    let mut rings = Rings::default();
    let keys = rings.wanted_open(cave, tiny, open);
    fill_once(&keys, cave, tiny, rings.reach().unwrap(), 20_000);
    assert!(keys.iter().all(|k| k.asked == k.level.min(open)));
    let (all, holes) = shown(&keys, &|_| true);
    assert_eq!((all.len(), holes), (keys.len(), 0));
    let mut next = numbers(5);
    let made: HashSet<Key> = keys.iter().flat_map(|k| [*k, k.parent().capped(open)]).filter(|_| next() < 0.6).collect();
    let (some, _) = shown(&keys, &|k| made.contains(&k));
    no_overlaps(&some);
    assert!(some.iter().all(|k| made.contains(k)));
    // And the cave's roof is there in a coarse chunk over it, asked so, and not asked as before.
    let over = [54.0, floor + 0.3, 30.15];
    let level = finest(SIZES[1].1);
    let surface_near = |key: Key| {
        let mesh = chunk::make(&mut played::area("garden", SEED), key).unwrap().mesh;
        let o = key.origin();
        mesh.is_some_and(|m| m.positions.iter().any(|p| ((o[0] + p[0] as f64 - 54.0).powi(2) + (o[2] + p[2] as f64 - 30.15).powi(2)).sqrt() < 0.05 && o[1] + (p[1] as f64) < floor + 0.3))
    };
    let key = holding(over, level);
    assert!(!surface_near(key), "a person's chunk has no cave in it");
    assert!(surface_near(key.capped(open)), "asked for a viewer who sees the cave, it has");
}

/// A rough vertical wall right on `x = 37.0` — a plane where a fine level (`-8`) meets a coarser
/// one (`-6`) when a tiny viewer approaches it — must have no holes through the wall, while a
/// 2.2 cm cleft in it stays open when approached closely. Found walking `headland-sketched`.
#[test]
fn a_rough_wall_grazing_a_level_boundary_has_no_holes() {
    use plateau_sdk::{Answer, Area, Borders, Description, Material, Point, View, cuboid, detail, point};

    struct GrazingWall;
    impl Area for GrazingWall {
        fn new(_: u64) -> Self {
            GrazingWall
        }
        fn describe(&self) -> Description {
            Description { name: "Grazing".into(), places: Vec::new(), borders: Borders::of_ground(|_, _| 0.5) }
        }
        fn sample(&self, p: Point, view: &View) -> Answer {
            let rough = detail(p, view, 0.04, 0.04, 0x53);
            let wall = (p.x - 37.0 + rough) * 0.5;
            let cleft = cuboid(p - point(36.9, 0.53, 26.8), point(0.2, 0.03, 0.011));
            let d = if view.sees(0.06) > 0.0 { wall.max(-cleft) } else { wall };
            Answer::new(d.min((p.y - 0.5) * 0.7), Material::Stone)
        }
    }

    let mut area = played::Native(Box::new(GrazingWall));
    let cast = |area: &mut dyn Region, eye: [f64; 3], ys: &[f64], zs: &[f64]| -> usize {
        let mut rings = Rings::default();
        let keys = rings.wanted(eye, SIZES[0].1);
        let mut tris = Vec::new();
        for k in keys {
            let (o, s) = (k.origin(), k.side());
            if o[0] >= 37.1 || o[0] + s <= 36.9 || o[1] >= 0.9 || o[1] + s <= 0.5 || o[2] >= 27.0 || o[2] + s <= 26.6 {
                continue;
            }
            if let Some(m) = chunk::make(area, k).unwrap().mesh {
                let pt = |idx: u32| {
                    let p = m.positions[idx as usize];
                    [o[0] + p[0] as f64, o[1] + p[1] as f64, o[2] + p[2] as f64]
                };
                for t in m.indices.as_chunks::<3>().0 {
                    tris.push((pt(t[0]), pt(t[1]), pt(t[2])));
                }
            }
        }
        let mut missed = 0;
        for &y in ys {
            for &z in zs {
                let hit = tris.iter().any(|(a, b, c)| {
                    let (v0y, v0z) = (b[1] - a[1], b[2] - a[2]);
                    let (v1y, v1z) = (c[1] - a[1], c[2] - a[2]);
                    let (py, pz) = (y - a[1], z - a[2]);
                    let det = v0y * v1z - v0z * v1y;
                    if det.abs() < 1e-12 {
                        return false;
                    }
                    let u = (py * v1z - pz * v1y) / det;
                    let v = (v0y * pz - v0z * py) / det;
                    if u >= -1e-6 && v >= -1e-6 && u + v <= 1.0 + 1e-6 {
                        let x = a[0] + u * (b[0] - a[0]) + v * (c[0] - a[0]);
                        (36.95..=37.10).contains(&x)
                    } else {
                        false
                    }
                });
                if !hit {
                    missed += 1;
                }
            }
        }
        missed
    };

    // Standing 26.6 cm from the wall (where level -8 meets level -6 on x = 37.0): zero holes above the cleft.
    let ys: Vec<f64> = (0..30).map(|i| 0.58 + i as f64 * 0.01).collect();
    let zs: Vec<f64> = (0..30).map(|i| 26.65 + i as f64 * 0.01).collect();
    let holes = cast(&mut area, [37.266, 0.515, 26.798], &ys, &zs);
    assert_eq!(holes, 0, "{holes} rays went through holes in the wall at x = 37.0");

    // Standing 3 cm in front of the cleft: the cleft itself stays open (rays pass straight through).
    let cleft_ys = [0.515, 0.525, 0.535, 0.545];
    let cleft_zs = [26.795, 26.800, 26.805];
    let open_rays = cast(&mut area, [37.03, 0.515, 26.80], &cleft_ys, &cleft_zs);
    assert_eq!(open_rays, cleft_ys.len() * cleft_zs.len(), "the cleft was plugged by the collar");
}
