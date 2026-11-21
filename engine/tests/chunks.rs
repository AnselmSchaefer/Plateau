//! The garden's surface in chunks (POC_PLAN.md, step 2): the coarse pass changes nothing but how
//! much is asked, neighbours meet without a seam, the triangles face out of the solid, and the
//! chunks a viewer wants fill the space about them once, with no gaps, at every size.

mod played;

use std::collections::{HashMap, HashSet};

use plateau_engine::chunk::{self, CELLS, Key, Making, Mesh};
use plateau_engine::rings::{self, COARSEST, Rings, finest, shown, wanted};
use plateau_engine::{SIZES, WIDTH};

const SEED: u64 = 7;

/// The chunk at level `level` holding the point `at`.
fn holding(at: [f64; 3], level: i32) -> Key {
    let side = 2f64.powi(level) * CELLS as f64;
    Key { level, at: at.map(|c| (c / side).floor() as i32) }
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
    assert_eq!(skirted.indices.len(), bare.indices.len() + open * 6);
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
        assert!(changes * 3 < changes_without_lag * 2, "a {size} m viewer's chunks changed {changes} times lagging, {changes_without_lag} not");
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
