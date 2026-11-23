//! The laws an area must keep (CONCEPT.md, "Laws"; POC_PLAN.md, step 3), searched for rather than
//! proved. Each search is seeded, so a failure plays again exactly, and says what broke in words,
//! for whoever made the area (Claude, from step 5): where, at what size, by how much, and what the
//! law asks. Each needs no right answer to compare with: it relates the area to itself, at two
//! sizes, along a ray, or from the start.
//!
//! | Law | Holds |
//! |---|---|
//! | `sizes_agree` | What a smaller viewer sees is the larger's world with more in it: seen from the larger's air, the two find the same surfaces within the larger's detail |
//! | `gates_are_real` | Each place the area describes is reached by the size it names, and by nobody who is never that size |
//! | `no_traps` | From everywhere a body reaches, it can get back to the start, at some size |
//! | `same_everywhere` | The answers of a fixed set of chunks hash as they are kept in the repo |
//! | `nans_unseen` | The same answers with every NaN made one and with the machine's own |
//! | `never_too_long` | No distance is longer than the way to the surface |
//! | `surfaces_have_an_inside` | Wherever the distance comes to nought, something is solid right by it |
//! | `sketch_materials` | Blue is water, grey is stone rock/cliff, brown is land (not water), within a tolerance |
//! | `sketch_places` | Every orange mark is reached by the tiny alone, and every purple mark by giants alone |

use crate::chunk::{CELLS, Key};
use crate::reach::{HIGHEST, LOWEST, Reach, Stand, World};
use crate::sketch::{Pen, Sketch};
use crate::wasm::{Compiled, Limits};
use crate::{Answer, Description, Material, Region, Result, SIZES, START, Size, WIDTH, chunk, hash, numbers, rings};

/// A law kept, or broken and why.
pub type Verdict = std::result::Result<(), String>;

/// Sizes from the tiny to a giant, each about three times the one before: the three sizes and
/// those between, which a player passes through growing and shrinking.
pub const LADDER: [f32; 7] = [0.017, 0.054, 0.17, 0.54, 1.7, 5.4, 17.0];

/// A length in words: in metres, centimetres or millimetres, as suits it.
pub fn length(m: f32) -> String {
    let a = m.abs();
    if a >= 1.0 {
        format!("{m:.2} m")
    } else if a >= 0.01 {
        format!("{:.1} cm", m * 100.0)
    } else {
        format!("{:.2} mm", m * 1000.0)
    }
}

/// A point in words, to a precision that suits a viewer `size` tall.
pub fn at(p: [f32; 3], size: f32) -> String {
    let digits = if size < 0.1 { 4 } else if size < 5.0 { 2 } else { 1 };
    // Nought without a sign, however near it was on either side.
    let c = p.map(|c| {
        let s = format!("{c:.digits$}");
        if s.trim_start_matches('-').chars().all(|ch| ch == '0' || ch == '.') { s.trim_start_matches('-').to_string() } else { s }
    });
    format!("({}, {}, {})", c[0], c[1], c[2])
}

/// Where the laws look closely: the start, on the ground the south border says is there, and every
/// place the area describes.
pub fn spots(d: &Description) -> Vec<[f32; 3]> {
    let ground = d.borders.south.ground.get(START[0] as usize).copied().unwrap_or(0.0);
    std::iter::once([START[0] as f32, ground, START[1] as f32]).chain(d.places.iter().map(|p| p.at)).collect()
}

/// Rays for looking at an area as a viewer `size` tall sees it: half from about the spots, half
/// from anywhere over the area, up to three heights above the ground; looking every way but mostly
/// down. Each its origin and its direction.
fn rays(spots: &[[f32; 3]], size: f32, n: usize, next: &mut impl FnMut() -> f32) -> Vec<([f32; 3], [f32; 3])> {
    (0..n)
        .map(|k| {
            let o = if k % 2 == 0 {
                let s = spots[(k / 2) % spots.len()];
                [s[0] + (next() * 2.0 - 1.0) * 3.0 * size, s[1] + next() * 2.0 * size, s[2] + (next() * 2.0 - 1.0) * 3.0 * size]
            } else {
                [next() * WIDTH, next() * (3.0 * size + 3.0) - 1.0, next() * WIDTH]
            };
            let mut d = [next() * 2.0 - 1.0, next() * 1.3 - 1.0, next() * 2.0 - 1.0];
            let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
            d = d.map(|c| c / l);
            (o, d)
        })
        .collect()
}

fn along(ray: &([f32; 3], [f32; 3]), t: f32) -> [f32; 3] {
    [0, 1, 2].map(|a| ray.0[a] + ray.1[a] * t)
}

fn answers(area: &mut dyn Region, points: &[[f32; 3]], size: f32) -> Result<Vec<Answer>> {
    let mut out = Vec::new();
    chunk::sample(area, points, size, &mut out)?;
    Ok(out)
}

/// How far along each ray a ball of radius `r` goes before it touches a surface, as a viewer `size`
/// tall sees it, within `far`: a look that cannot see through gaps narrower than the ball.
fn thick(area: &mut dyn Region, rays: &[([f32; 3], [f32; 3])], size: f32, r: f32, far: f32) -> Result<Vec<Option<f32>>> {
    let mut t = vec![0.0f32; rays.len()];
    let mut hit = vec![None; rays.len()];
    let mut active: Vec<usize> = (0..rays.len()).collect();
    for _ in 0..400 {
        if active.is_empty() {
            break;
        }
        let points: Vec<_> = active.iter().map(|&k| along(&rays[k], t[k])).collect();
        let out = answers(area, &points, size)?;
        let mut still = Vec::new();
        for (j, &k) in active.iter().enumerate() {
            let d = out[j].distance;
            if d < r * 1.02 {
                hit[k] = Some(t[k]);
            } else {
                t[k] += d - r;
                if t[k] <= far {
                    still.push(k);
                }
            }
        }
        active = still;
    }
    // A ray still going after so many steps is creeping along a surface: it touches it.
    for k in active {
        hit[k] = Some(t[k]);
    }
    Ok(hit)
}

/// **Sizes agree**: what a smaller viewer sees is the larger one's world with more in it, never
/// another world (CONCEPT.md: "the crack a giant sees is where the tiny find the cave").
///
/// For each size of `LADDER` and the next one up, `rays` looks from the larger's open air, each a
/// ball as wide as the larger's detail (a twentieth of its height, so neither sees into gaps the
/// larger cannot), as each of the two sees the area. Where one stops at a surface, the other must
/// see a surface there too, within a tenth of the larger's height. So detail may roughen what the
/// larger sees, and open cracks and caves into it behind openings too small for the larger, but not
/// move it, raise it, or take it away. On failure, says the worst case and how many there were.
pub fn sizes_agree(area: &mut dyn Region, description: &Description, seed: u32, rays_a_pair: usize) -> Result<Verdict> {
    let spots = spots(description);
    let mut next = numbers(seed);
    let (mut worst, mut broken, mut looked): (Option<(f32, String)>, usize, usize) = (None, 0, 0);
    for pair in LADDER.windows(2) {
        let (small, large) = (pair[0], pair[1]);
        let (r, tolerance) = (large / 20.0, large / 10.0);
        // From where the larger sees open air.
        let all = rays(&spots, large, rays_a_pair, &mut next);
        let origins: Vec<_> = all.iter().map(|r| r.0).collect();
        let clear = answers(area, &origins, large)?;
        let rays: Vec<_> = all.into_iter().zip(&clear).filter(|(_, a)| a.distance > 2.0 * r).map(|(r, _)| r).collect();
        looked += rays.len();
        let far = 6.0 * large;
        let (by_small, by_large) = (thick(area, &rays, small, r, far)?, thick(area, &rays, large, r, far)?);
        // Each stop, asked of the other size.
        let mut stops = [Vec::new(), Vec::new()];
        for (k, (s, l)) in by_small.iter().zip(&by_large).enumerate() {
            if let Some(t) = s {
                stops[0].push((k, along(&rays[k], *t)));
            }
            if let Some(t) = l {
                stops[1].push((k, along(&rays[k], *t)));
            }
        }
        let mut bad = std::collections::HashSet::new();
        for (which, (seen_by, asked)) in [(small, large), (large, small)].into_iter().enumerate() {
            let points: Vec<_> = stops[which].iter().map(|s| s.1).collect();
            let other = answers(area, &points, asked)?;
            for (&(k, p), a) in stops[which].iter().zip(&other) {
                let over = a.distance - (r + tolerance);
                if over <= 0.0 {
                    continue;
                }
                bad.insert(k);
                let why = if which == 0 {
                    format!(
                        "at {} a {} viewer sees a surface that a {} viewer does not: to the larger, the nearest surface there is at \
                         least {} away",
                        at(p, small),
                        length(seen_by),
                        length(asked),
                        length(a.distance - r)
                    )
                } else {
                    format!(
                        "at {} a {} viewer sees a surface that is not there for a {} viewer: to the smaller, the nearest surface \
                         there is at least {} away",
                        at(p, small),
                        length(seen_by),
                        length(asked),
                        length(a.distance - r)
                    )
                };
                if worst.as_ref().is_none_or(|(w, _)| over / large > *w) {
                    worst = Some((over / large, format!("{why}, more than the {} allowed", length(tolerance))));
                }
            }
        }
        broken += bad.len();
    }
    Ok(match worst {
        None => Ok(()),
        Some((_, why)) => Err(format!(
            "sizes do not agree: {why} ({broken} of {looked} looks found such a place; seed {seed}). What a smaller viewer sees must \
             be the larger's world with more in it: detail may roughen a surface by up to a tenth of the larger's height, and open \
             cracks and caves behind openings too small for the larger to see, but never move, raise or take away what the larger \
             sees. Give a detail only where `View::sees` says it is seen, faded in by it, and work out the shapes the larger sees \
             the same way at every size."
        )),
    })
}

/// **Distances never too long** (found in step 2): no answer's distance is longer than the way to
/// the surface, at any size of `LADDER`, so the engine can step as far as it says. Searched two
/// ways: `rays` marched as the engine marches them, each step checked for landing on the far side
/// of a surface; and about every surface they find, points asked how far it is, against the way to
/// it. A distance may be up to `TOO_LONG` over, for rounding.
pub fn never_too_long(area: &mut dyn Region, description: &Description, seed: u32, rays_a_size: usize) -> Result<Verdict> {
    const TOO_LONG: f32 = 0.01;
    let spots = spots(description);
    let mut next = numbers(seed);
    let (mut worst, mut broken, mut looked): (Option<(f32, String)>, usize, usize) = (None, 0, 0);
    let report = |ratio: f32, size: f32, p: [f32; 3], said: f32, way: f32, worst: &mut Option<(f32, String)>| {
        if worst.as_ref().is_none_or(|(w, _)| ratio > *w) {
            *worst = Some((
                ratio,
                format!(
                    "at {}, for a {} viewer, the area answers a distance of {}, but a surface is at most {} away: {:.2} times too long",
                    at(p, size),
                    length(size),
                    length(said),
                    length(way),
                    ratio
                ),
            ));
        }
    };
    for &size in &LADDER {
        let (close, slack) = (0.001 * size, 1e-4 * size);
        let rays = rays(&spots, size, rays_a_size, &mut next);
        looked += rays.len();
        let far = 6.0 * size;
        let mut t = vec![0.0f32; rays.len()];
        let mut last: Vec<Option<(f32, f32)>> = vec![None; rays.len()];
        let mut surfaces = Vec::new();
        let mut active: Vec<usize> = (0..rays.len()).collect();
        let mut bad = std::collections::HashSet::new();
        // The rays, each stepped by the distance it is told; a step landing beyond a surface
        // (further than rounding) is a distance too long, by as much as the surface is nearer.
        let mut overshot = Vec::new();
        for _ in 0..200 {
            if active.is_empty() {
                break;
            }
            let points: Vec<_> = active.iter().map(|&k| along(&rays[k], t[k])).collect();
            let out = answers(area, &points, size)?;
            let mut still = Vec::new();
            for (j, &k) in active.iter().enumerate() {
                let d = out[j].distance;
                if let Some((t0, d0)) = last[k]
                    && d0.signum() != d.signum()
                    && d.abs() > slack + close
                {
                    overshot.push((k, t0, d0, t[k]));
                    continue;
                }
                if d.abs() < close {
                    surfaces.push((k, t[k], d));
                    continue;
                }
                last[k] = Some((t[k], d));
                t[k] += d.abs();
                if t[k] <= far {
                    still.push(k);
                }
            }
            active = still;
        }
        // Each overshoot narrowed to where the surface is crossed.
        for (k, t0, d0, mut t1) in overshot {
            let mut t_in = t0;
            for _ in 0..16 {
                let mid = (t_in + t1) * 0.5;
                let d = answers(area, &[along(&rays[k], mid)], size)?[0].distance;
                if d.signum() == d0.signum() { t_in = mid } else { t1 = mid }
            }
            let way = t1 - t0;
            let ratio = d0.abs() / way.max(1e-9);
            if d0.abs() > way * (1.0 + TOO_LONG) + slack {
                bad.insert(k);
                report(ratio, size, along(&rays[k], t0), d0.abs(), way, &mut worst);
            }
        }
        // About each surface found (a surface for certain: the answer's sign changes across it),
        // points asked how far it is.
        let confirm: Vec<_> = surfaces.iter().map(|&(k, t, _)| along(&rays[k], t + 3.0 * close)).collect();
        let beyond = answers(area, &confirm, size)?;
        let mut near = Vec::new();
        for (&(k, t, d), b) in surfaces.iter().zip(&beyond) {
            let approach = last[k].map_or(d, |l| l.1);
            if b.distance.signum() != approach.signum() {
                let q = along(&rays[k], t + 1.5 * close);
                for _ in 0..4 {
                    let reach = next() * 0.3 * size;
                    let v = [next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0];
                    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
                    near.push((k, [0, 1, 2].map(|a| q[a] + v[a] / l * reach), reach + 1.5 * close));
                }
            }
        }
        let points: Vec<_> = near.iter().map(|n| n.1).collect();
        let out = answers(area, &points, size)?;
        for (&(k, p, way), a) in near.iter().zip(&out) {
            if a.distance.abs() > way * (1.0 + TOO_LONG) + slack {
                bad.insert(k);
                report(a.distance.abs() / way, size, p, a.distance.abs(), way, &mut worst);
            }
        }
        broken += bad.len();
    }
    Ok(match worst {
        None => Ok(()),
        Some((ratio, why)) => Err(format!(
            "distances are too long: {why} ({broken} of {looked} rays found such a place; seed {seed}). A distance may be less than \
             the true one, never more: the engine steps that far without looking, so it would step through the surface. Scaling a \
             distance never moves its surface: multiply the part that is too long by {:.2} or less (steep detail, sums that are not \
             distances, a bound answered close to what it bounds).",
            (1.0 / ratio * 0.95).min(0.95)
        )),
    })
}

/// **Surfaces have an inside** (found in step 3): wherever the area's distance comes to nought,
/// there is solid right by it. Where a shape cut out ends exactly at the surface it is cut from (a
/// pit cut down from the ground's very surface, a door cut flush with a wall's face), the distance
/// is nought across its mouth: a surface with nothing inside it, which a ray stops at and a body
/// stands on or walks into. Rays at each size of `LADDER` stop wherever the distance says they
/// touch; about each stop, points a little way off each way are asked whether one is inside.
pub fn surfaces_have_an_inside(area: &mut dyn Region, description: &Description, seed: u32, rays_a_size: usize) -> Result<Verdict> {
    let spots = spots(description);
    let mut next = numbers(seed);
    let (mut worst, mut broken, mut looked): (Option<(f32, String)>, usize, usize) = (None, 0, 0);
    for &size in &LADDER {
        let close = 0.001 * size;
        let rays = rays(&spots, size, rays_a_size, &mut next);
        looked += rays.len();
        let far = 6.0 * size;
        let mut t = vec![0.0f32; rays.len()];
        let mut stops = Vec::new();
        let mut active: Vec<usize> = (0..rays.len()).collect();
        for _ in 0..200 {
            if active.is_empty() {
                break;
            }
            let points: Vec<_> = active.iter().map(|&k| along(&rays[k], t[k])).collect();
            let out = answers(area, &points, size)?;
            let mut still = Vec::new();
            for (j, &k) in active.iter().enumerate() {
                let d = out[j].distance;
                if d < close {
                    // Started inside, or came to a surface.
                    if t[k] > 0.0 && d > -close {
                        stops.push(k);
                    }
                    continue;
                }
                t[k] += d;
                if t[k] <= far {
                    still.push(k);
                }
            }
            active = still;
        }
        // About each stop, fifteen points three times as far off as touching is: on along the ray,
        // each way along the axes, and to the corners of a cube about it (at an edge, the points
        // along the axes lie on its faces).
        let off = 3.0 * close;
        let mut probes = Vec::new();
        for &k in &stops {
            let q = along(&rays[k], t[k]);
            probes.push(along(&rays[k], t[k] + off));
            for a in 0..3 {
                for sign in [-1.0, 1.0] {
                    let mut p = q;
                    p[a] += sign * off;
                    probes.push(p);
                }
            }
            for c in 0..8 {
                probes.push([0, 1, 2].map(|a| q[a] + if c >> a & 1 == 1 { off } else { -off }));
            }
        }
        let out = answers(area, &probes, size)?;
        for (n, &k) in stops.iter().enumerate() {
            let nearest = out[n * 15..n * 15 + 15].iter().map(|a| a.distance).fold(f32::INFINITY, f32::min);
            if nearest >= 0.0 {
                broken += 1;
                let q = along(&rays[k], t[k]);
                if worst.as_ref().is_none_or(|(w, _)| nearest / size > *w) {
                    worst = Some((
                        nearest / size,
                        format!(
                            "at {}, for a {} viewer, the distance comes to nought, but nothing within {} of it is inside anything",
                            at(q, size),
                            length(size),
                            length(off)
                        ),
                    ));
                }
            }
        }
    }
    Ok(match worst {
        None => Ok(()),
        Some((_, why)) => Err(format!(
            "a surface has no inside: {why} ({broken} of {looked} rays stopped at such a place; seed {seed}). A ray stops there and a \
             body stands on it or walks into it, though nothing is there. It is where a shape cut out ends exactly at the surface of \
             what it is cut from: make what is cut out reach a little beyond it (a pit from above the ground, a door through more \
             than the wall's thickness), and answer a bound only well away from what it bounds."
        )),
    })
}

/// A way in words: where it starts, where it changes size, and a few points between, up to the
/// stand at its end.
fn way(world: &World, way: &[Stand]) -> String {
    let mut marks = vec![0];
    for i in 1..way.len() {
        if way[i].size != way[i - 1].size {
            marks.push(i - 1);
            marks.push(i);
        }
    }
    let n = way.len().saturating_sub(1);
    marks.extend((1..4).map(|k| n * k / 4));
    marks.push(n);
    marks.sort();
    marks.dedup();
    let mut words = Vec::new();
    let mut size = None;
    for i in marks {
        let s = way[i];
        let p = world.point(s);
        if size != Some(s.size) {
            words.push(format!("as {} {}", a_size(s.size), at(p, s.size.height())));
            size = Some(s.size);
        } else {
            words.push(at(p, s.size.height()));
        }
    }
    words.join(", ")
}

fn a_size(size: Size) -> &'static str {
    ["a tiny person", "a person", "a giant"][size as usize]
}

fn never(size: Size) -> &'static str {
    ["never tiny", "never a person", "never a giant"][size as usize]
}

fn nobody_who_is_never(size: Size) -> String {
    format!("nobody who is {}", never(size))
}

/// The nearest a search came to a point as `size`: the stand and how far it is from the point.
fn nearest(world: &World, reach: &Reach, size: Size, p: [f32; 3]) -> Option<(Stand, f32)> {
    reach
        .stands
        .iter()
        .filter(|s| s.size == size)
        .map(|&s| {
            let q = world.point(s);
            (s, ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2) + (q[2] - p[2]).powi(2)).sqrt())
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
}

/// **Gates are real**: each place the area describes is reached by the size it names (changing size
/// on the way as the player may), and by nobody who is never that size (`reach`, whose rulings
/// these are: the tiny are searched about their own places, and elsewhere go where a person goes;
/// so a place for a person is held only to giants). On failure, says the first place that fails
/// and the shortest way that shows it.
/// `all` is what `world` reaches being any size.
pub fn gates_are_real(world: &mut World, all: &Reach, description: &Description) -> Result<Verdict> {
    let mut without = std::collections::HashMap::new();
    for place in &description.places {
        let (name, size) = (&place.name, place.size);
        let Some(own) = world.at_place(place, size)? else {
            return Ok(Err(format!(
                "a gate is not real: \"{name}\", meant for {}, is not somewhere they can stand: at {} there is no floor within {} \
                 of it with room for them. A place is a point on a floor, with room above it for the size it is meant for.",
                a_size(size),
                at(place.at, size.height()),
                length(crate::reach::AT_PLACE * size.height())
            )));
        };
        if !all.has(own) {
            let near = match nearest(world, all, size, place.at) {
                Some((s, far)) => format!(
                    "the nearest {} gets is {}, {} from it, by: {}",
                    a_size(size),
                    at(world.point(s), size.height()),
                    length(far),
                    way(world, &all.way(s))
                ),
                None => match all.about.iter().find(|a| &a.0 == name) {
                    Some((_, stands, 0)) => format!(
                        "out from it, there are {stands} places {} can stand, and none of them is one a person can come to, \
                         shrink and walk from: the way in is shut",
                        a_size(size)
                    ),
                    _ => format!("{} gets nowhere near it", a_size(size)),
                },
            };
            let unsettled = if all.unsettled.contains(name) { " (there was too much about it to search all of)" } else { "" };
            return Ok(Err(format!(
                "a gate is not real: \"{name}\", meant for {}, is not reached by them from the start, even changing size on the \
                 way{unsettled}: {near}.",
                a_size(size)
            )));
        }
        if let std::collections::hash_map::Entry::Vacant(e) = without.entry(size) {
            let others: Vec<Size> = Size::ALL.into_iter().filter(|&s| s != size).collect();
            e.insert(world.reach(&others)?);
        }
        let reach = &without[&size];
        for other in Size::ALL.into_iter().filter(|&s| s != size) {
            if let Some(s) = world.at_place(place, other)?
                && reach.has(s)
            {
                return Ok(Err(format!(
                    "a gate is not real: \"{name}\", meant for {} alone, is reached by {} who is {}: {}. It must be reached by \
                     {}: a way in only {} fits through or climbs.",
                    a_size(size),
                    a_size(other),
                    never(size),
                    way(world, &reach.way(s)),
                    nobody_who_is_never(size),
                    a_size(size)
                )));
            }
        }
    }
    Ok(Ok(()))
}

/// **No traps**: from everywhere a body reaches from the start, it can get back to the start, at
/// some size (growing where there is room, shrinking anywhere). On failure, says the trap nearest
/// the start and the way into it.
/// `reach` is what `world` reaches being any size.
pub fn no_traps(world: &World, reach: &Reach) -> Result<Verdict> {
    let back = reach.back();
    let trapped: Vec<_> = (0..reach.stands.len()).filter(|&i| !back[i]).collect();
    let Some(&first) = trapped.iter().min_by_key(|&&i| reach.way(reach.stands[i]).len()) else { return Ok(Ok(())) };
    let s = reach.stands[first];
    Ok(Err(format!(
        "a trap: {} who comes to {} cannot get back to the start, at any size ({} places like it). They come there by: {}. \
         Every place a body can get to, it must be able to leave: a way out it can climb (a step of a quarter of its height, a \
         jump of a third), or room to grow.",
        a_size(s.size),
        at(world.point(s), s.size.height()),
        trapped.len(),
        way(world, &reach.way(s))
    )))
}

/// The chunks whose answers are kept (`same_everywhere`): at each size's finest level, the chunk
/// about each spot.
pub fn kept_chunks(description: &Description) -> Vec<Key> {
    let mut keys = Vec::new();
    for spot in spots(description) {
        for &(_, size) in &SIZES {
            let level = rings::finest(size);
            let side = 2f64.powi(level) * CELLS as f64;
            let key = Key::new(level, spot.map(|c| (c as f64 / side).floor() as i32));
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    }
    keys
}

/// The hash of each chunk's answers at its corners (33 a side), asked for the viewer it is made for.
pub fn chunk_hashes(area: &mut dyn Region, keys: &[Key]) -> Result<Vec<(Key, u64)>> {
    let mut hashes = Vec::new();
    for &key in keys {
        let (o, v) = (key.origin(), key.voxel());
        let n = CELLS + 1;
        let points: Vec<[f32; 3]> =
            (0..n * n * n).map(|i| [i % n, i / n % n, i / (n * n)]).map(|c| [0, 1, 2].map(|a| (o[a] + c[a] as f64 * v) as f32)).collect();
        hashes.push((key, hash(&answers(area, &points, key.size())?)));
    }
    Ok(hashes)
}

/// The kept hashes in words, one chunk a line: its level, its place on the level's grid, its hash.
pub fn hashes_text(hashes: &[(Key, u64)]) -> String {
    hashes.iter().map(|(k, h)| format!("{} {} {} {} {h:016x}\n", k.level, k.at[0], k.at[1], k.at[2])).collect()
}

/// **The same everywhere**: the area's chunks hash as `kept` (`hashes_text`, lines starting `#`
/// left out) says they did. A change to the area, the SDK, the toolchain or the engine that changes
/// what the area answers is a change of its world, and is to be made on purpose, writing the hashes
/// again.
pub fn same_everywhere(area: &mut dyn Region, kept: &str) -> Result<Verdict> {
    let mut keys = Vec::new();
    let mut wanted = Vec::new();
    for line in kept.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let w: Vec<&str> = line.split_whitespace().collect();
        let n = |i: usize| w.get(i).and_then(|s| s.parse::<i32>().ok());
        let (Some(level), Some(x), Some(y), Some(z), Some(h)) = (n(0), n(1), n(2), n(3), w.get(4).and_then(|s| u64::from_str_radix(s, 16).ok()))
        else {
            return Ok(Err(format!("the kept hashes cannot be read: \"{line}\"")));
        };
        keys.push(Key::new(level, [x, y, z]));
        wanted.push(h);
    }
    if keys.is_empty() {
        return Ok(Err("no hashes are kept".into()));
    }
    let hashes = chunk_hashes(area, &keys)?;
    for ((key, h), want) in hashes.iter().zip(wanted) {
        if *h != want {
            let o = key.origin().map(|c| c as f32);
            return Ok(Err(format!(
                "the world is not the same: the chunk at {} ({} voxels, for a {} viewer) hashes {h:016x}, not {want:016x} as kept",
                at(o, key.size()),
                length(key.voxel() as f32),
                length(key.size())
            )));
        }
    }
    Ok(Ok(()))
}

/// **NaNs unseen**, the law that stands in for NaN canonicalization (spike/RESULTS.md): the area
/// gives the same answers, bit for bit, with every NaN made the one NaN and with the machine's own.
/// A NaN's bits differ from one processor to the next (on x86 the machine's NaN is not the one NaN,
/// so the difference shows here); an area whose answers depend on them would make a different world
/// on another machine. On failure, says the first point and size where the answers differ.
pub fn nans_unseen(module: &[u8], seed: u64, points: &[[f32; 3]]) -> Result<Verdict> {
    let mut one = Compiled::new(module, Limits { canonical_nans: true, ..Limits::default() })?.instantiate(seed)?;
    let mut own = Compiled::new(module, Limits::default())?.instantiate(seed)?;
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for &(size, height) in &SIZES {
        for batch in points.chunks(4096) {
            one.sample(batch, height, &mut a)?;
            own.sample(batch, height, &mut b)?;
            if let Some(i) = (0..batch.len()).find(|&i| !same(a[i], b[i])) {
                let [x, y, z] = batch[i];
                return Ok(Err(format!(
                    "the area looks at a NaN's bits: at ({x}, {y}, {z}) for a {} ({height} m) it answers {:?} with every NaN made \
                     one and {:?} with this machine's own",
                    size.name(),
                    a[i],
                    b[i]
                )));
            }
        }
    }
    if one.props()? != own.props()? {
        return Ok(Err("the area's props differ between canonical NaNs and this machine's own".into()));
    }
    Ok(Ok(()))
}

/// The same answer, bit for bit.
pub fn same(a: Answer, b: Answer) -> bool {
    a.distance.to_bits() == b.distance.to_bits() && a.material == b.material
}

/// How far in metres a feature or place in an area may lie from where the sketch drew it.
pub const SKETCH_TOLERANCE: f32 = 3.0;

/// **Sketch materials** (POC_PLAN.md, step 6): where the sketch draws blue (water), grey (rock,
/// cliff) or brown (land, ground), the area's surface seen from above matches within
/// `SKETCH_TOLERANCE` metres (so coastlines may curve naturally and small details like lily pads
/// on water are allowed, while missing ponds, misplaced seas or grassy cliffs are caught).
pub fn sketch_materials(area: &mut dyn Region, sketch: &Sketch) -> Result<Verdict> {
    if sketch.regions.is_empty() {
        return Ok(Ok(()));
    }
    const N: usize = 64;
    let size = Size::Person.height();
    let close = 0.05f32;
    let least = 0.05f32;

    // March 64 × 64 columns straight down from above (as `picture::plan`), stopping at the first
    // surface (including water).
    let mut y = vec![HIGHEST; N * N];
    let mut hit: Vec<Option<(f32, Material)>> = vec![None; N * N];
    let mut active: Vec<usize> = (0..N * N).collect();
    for _ in 0..300 {
        if active.is_empty() {
            break;
        }
        let pts: Vec<[f32; 3]> = active.iter().map(|&k| [(k % N) as f32 + 0.5, y[k], (k / N) as f32 + 0.5]).collect();
        let out = answers(area, &pts, size)?;
        let mut still = Vec::with_capacity(active.len());
        for (j, &k) in active.iter().enumerate() {
            let a = out[j];
            if a.distance < close {
                hit[k] = Some((y[k], a.material));
            } else {
                y[k] -= a.distance.max(least);
                if y[k] >= LOWEST {
                    still.push(k);
                }
            }
        }
        active = still;
    }

    // For grey (rock, cliff): a column counts as stone if its top surface is Stone, OR if it sits
    // on a cliff drop (>= 1.5 m steeper than a neighbour) whose solid face halfway down is Stone.
    let mut is_stone: Vec<bool> = hit.iter().map(|h| matches!(h, Some((_, Material::Stone)))).collect();
    let mut cliff_probes: Vec<(usize, [f32; 3])> = Vec::new();
    for j in 0..N {
        for i in 0..N {
            let k = j * N + i;
            if is_stone[k] {
                continue;
            }
            let Some((h_here, _)) = hit[k] else { continue };
            let mut min_nb = h_here;
            for (di, dj) in [(-1isize, 0), (1, 0), (0, -1), (0, 1)] {
                let (ni, nj) = (i as isize + di, j as isize + dj);
                if ni >= 0
                    && nj >= 0
                    && ni < N as isize
                    && nj < N as isize
                    && let Some((h_nb, _)) = hit[nj as usize * N + ni as usize]
                {
                    min_nb = min_nb.min(h_nb);
                }
            }
            if h_here - min_nb >= 1.5 {
                cliff_probes.push((k, [i as f32 + 0.5, (h_here + min_nb) * 0.5, j as f32 + 0.5]));
            }
        }
    }
    if !cliff_probes.is_empty() {
        let pts: Vec<[f32; 3]> = cliff_probes.iter().map(|p| p.1).collect();
        let out = answers(area, &pts, size)?;
        for (&(k, _), a) in cliff_probes.iter().zip(&out) {
            if a.distance <= 0.5 && a.material == Material::Stone {
                is_stone[k] = true;
            }
        }
    }

    // For brown checks, precompute which grid cells are within SKETCH_TOLERANCE of any Blue cell.
    let r = SKETCH_TOLERANCE.ceil() as isize;
    let r2 = SKETCH_TOLERANCE * SKETCH_TOLERANCE;
    let grid_pen: Vec<Pen> = (0..N * N).map(|k| sketch.pen_at((k % N) as f32 + 0.5, (k / N) as f32 + 0.5)).collect();
    let near_blue = |i: usize, j: usize| -> bool {
        for dj in -r..=r {
            for di in -r..=r {
                if (di * di + dj * dj) as f32 > r2 {
                    continue;
                }
                let (ni, nj) = (i as isize + di, j as isize + dj);
                if ni >= 0 && nj >= 0 && ni < N as isize && nj < N as isize && grid_pen[nj as usize * N + ni as usize] == Pen::Blue {
                    return true;
                }
            }
        }
        false
    };

    let has_nearby = |i: usize, j: usize, pred: &dyn Fn(usize) -> bool| -> bool {
        for dj in -r..=r {
            for di in -r..=r {
                if (di * di + dj * dj) as f32 > r2 {
                    continue;
                }
                let (ni, nj) = (i as isize + di, j as isize + dj);
                if ni >= 0 && nj >= 0 && ni < N as isize && nj < N as isize && pred(nj as usize * N + ni as usize) {
                    return true;
                }
            }
        }
        false
    };

    let nearest_matching = |i: usize, j: usize, pred: &dyn Fn(usize) -> bool| -> Option<f32> {
        let mut best = f32::INFINITY;
        for nj in 0..N {
            for ni in 0..N {
                if pred(nj * N + ni) {
                    let d = ((ni as f32 - i as f32).powi(2) + (nj as f32 - j as f32).powi(2)).sqrt();
                    if d < best {
                        best = d;
                    }
                }
            }
        }
        best.is_finite().then_some(best)
    };

    for (ridx, region) in sketch.regions.iter().enumerate() {
        let mut cells: Vec<(usize, usize)> = Vec::new();
        for j in 0..N {
            for i in 0..N {
                if sketch.region_at(i as f32 + 0.5, j as f32 + 0.5) == Some(ridx) {
                    cells.push((i, j));
                }
            }
        }
        if cells.is_empty() {
            let ci = region.centre[0].floor().clamp(0.0, (N - 1) as f32) as usize;
            let cj = region.centre[1].floor().clamp(0.0, (N - 1) as f32) as usize;
            cells.push((ci, cj));
        }

        let mut misses: Vec<(usize, usize)> = Vec::new();
        match region.pen {
            Pen::Blue => {
                let is_water = |k: usize| matches!(hit[k], Some((_, Material::Water)));
                for &(i, j) in &cells {
                    if !has_nearby(i, j, &is_water) {
                        misses.push((i, j));
                    }
                }
                let allowed = (cells.len() / 20).max(1);
                if misses.len() > allowed && misses.len() * 100 >= cells.len() * 5 {
                    let &(wi, wj) = misses
                        .iter()
                        .max_by(|&&(a, b), &&(c, d)| {
                            nearest_matching(a, b, &is_water)
                                .unwrap_or(999.0)
                                .total_cmp(&nearest_matching(c, d, &is_water).unwrap_or(999.0))
                        })
                        .unwrap();
                    let found = hit[wj * N + wi].map_or("empty air", |(_, m)| m.name());
                    let far = match nearest_matching(wi, wj, &is_water) {
                        Some(d) => format!("{:.1} m from the nearest water", d),
                        None => "there is no water anywhere in the area".into(),
                    };
                    let pct = (misses.len() * 100) / cells.len().max(1);
                    return Ok(Err(format!(
                        "the sketch's materials do not match: where the sketch has water (blue) around ({:.1}, {:.1}), {pct}% of it has \
                         no water within {SKETCH_TOLERANCE:.1} m (at ({:.1}, {:.1}) the surface is {found}; {far}). Put water where the \
                         sketch is blue, within {SKETCH_TOLERANCE:.1} m.",
                        region.centre[0],
                        region.centre[1],
                        wi as f32 + 0.5,
                        wj as f32 + 0.5
                    )));
                }
            }
            Pen::Grey => {
                let is_rock = |k: usize| is_stone[k];
                for &(i, j) in &cells {
                    if !has_nearby(i, j, &is_rock) {
                        misses.push((i, j));
                    }
                }
                let allowed = (cells.len() / 20).max(1);
                if misses.len() > allowed && misses.len() * 100 >= cells.len() * 5 {
                    let &(wi, wj) = misses
                        .iter()
                        .max_by(|&&(a, b), &&(c, d)| {
                            nearest_matching(a, b, &is_rock)
                                .unwrap_or(999.0)
                                .total_cmp(&nearest_matching(c, d, &is_rock).unwrap_or(999.0))
                        })
                        .unwrap();
                    let found = hit[wj * N + wi].map_or("empty air", |(_, m)| m.name());
                    let far = match nearest_matching(wi, wj, &is_rock) {
                        Some(d) => format!("{:.1} m from the nearest stone", d),
                        None => "there is no stone anywhere in the area".into(),
                    };
                    let pct = (misses.len() * 100) / cells.len().max(1);
                    return Ok(Err(format!(
                        "the sketch's materials do not match: where the sketch has rock or cliff (grey) around ({:.1}, {:.1}), {pct}% \
                         of it has no stone within {SKETCH_TOLERANCE:.1} m (at ({:.1}, {:.1}) the surface is {found}; {far}). Make the \
                         surface (or cliff face) stone where the sketch is grey, within {SKETCH_TOLERANCE:.1} m.",
                        region.centre[0],
                        region.centre[1],
                        wi as f32 + 0.5,
                        wj as f32 + 0.5
                    )));
                }
            }
            Pen::Brown => {
                let is_land = |k: usize| matches!(hit[k], Some((_, m)) if m != Material::Water);
                // A brown cell fails if it has no land within tolerance, OR if it is water well
                // away from any blue water drawn on the sketch.
                let mut flooded: Vec<(usize, usize)> = Vec::new();
                for &(i, j) in &cells {
                    if !has_nearby(i, j, &is_land) {
                        misses.push((i, j));
                    } else if matches!(hit[j * N + i], Some((_, Material::Water))) && !near_blue(i, j) {
                        flooded.push((i, j));
                    }
                }
                let allowed = (cells.len() / 20).max(1);
                if misses.len() > allowed || (flooded.len() > 20 && flooded.len() * 100 >= cells.len() * 3) {
                    let bad = if !misses.is_empty() { &misses } else { &flooded };
                    let &(wi, wj) = bad
                        .iter()
                        .max_by(|&&(a, b), &&(c, d)| {
                            nearest_matching(a, b, &is_land)
                                .unwrap_or(999.0)
                                .total_cmp(&nearest_matching(c, d, &is_land).unwrap_or(999.0))
                        })
                        .unwrap();
                    let found = hit[wj * N + wi].map_or("empty air", |(_, m)| m.name());
                    let pct = (bad.len() * 100) / cells.len().max(1);
                    return Ok(Err(format!(
                        "the sketch's materials do not match: where the sketch has land (brown) around ({:.1}, {:.1}), {pct}% of it is \
                         not land (at ({:.1}, {:.1}) the surface is {found}). Keep dry land (not water) where the sketch is brown, \
                         within {SKETCH_TOLERANCE:.1} m.",
                        region.centre[0],
                        region.centre[1],
                        wi as f32 + 0.5,
                        wj as f32 + 0.5
                    )));
                }
            }
            _ => {}
        }
    }
    Ok(Ok(()))
}

/// **Sketch places** (POC_PLAN.md, step 6): every orange mark on the sketch has a place for the
/// tiny within `SKETCH_TOLERANCE` metres reached by the tiny and by nobody who is never tiny;
/// every purple mark has a place for giants within `SKETCH_TOLERANCE` metres reached by giants and
/// by nobody who is never a giant.
pub fn sketch_places(world: &mut World, all: &Reach, description: &Description, sketch: &Sketch) -> Result<Verdict> {
    if sketch.places.is_empty() {
        return Ok(Ok(()));
    }
    let mut without = std::collections::HashMap::new();
    for mark in &sketch.places {
        let size = mark.size;
        let pen = if size == Size::Tiny { "orange" } else { "purple" };
        let matching: Vec<_> = description
            .places
            .iter()
            .filter(|p| p.size == size && mark.distance_to(sketch, p.at[0], p.at[2]) <= SKETCH_TOLERANCE)
            .collect();
        if matching.is_empty() {
            let nearest = description
                .places
                .iter()
                .filter(|p| p.size == size)
                .map(|p| (p, mark.distance_to(sketch, p.at[0], p.at[2])))
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(p, d)| format!(" (the nearest place for {} is \"{}\" at {}, {:.1} m away)", a_size(size), p.name, at(p.at, size.height()), d))
                .unwrap_or_default();
            return Ok(Err(format!(
                "the sketch's places do not match: the sketch has an {pen} mark (for {} alone) at ({:.1}, {:.1}), but the area \
                 describes no place for {} within {SKETCH_TOLERANCE:.1} m of it{nearest}. Describe a place for {} there (in \
                 `describe().places`), reached by {} and by {}.",
                a_size(size),
                mark.centre[0],
                mark.centre[1],
                a_size(size),
                a_size(size),
                a_size(size),
                nobody_who_is_never(size)
            )));
        }
        if let std::collections::hash_map::Entry::Vacant(e) = without.entry(size) {
            let others: Vec<Size> = Size::ALL.into_iter().filter(|&s| s != size).collect();
            e.insert(world.reach(&others)?);
        }
        let reach_without = &without[&size];
        let mut last_why = String::new();
        let mut ok = false;
        for place in matching {
            let Some(own) = world.at_place(place, size)? else {
                last_why = format!(
                    "\"{}\" at {} has no floor with room for {}",
                    place.name,
                    at(place.at, size.height()),
                    a_size(size)
                );
                continue;
            };
            if !all.has(own) {
                last_why = format!("\"{}\" at {} is not reached by {} from the start", place.name, at(place.at, size.height()), a_size(size));
                continue;
            }
            let mut intruded = None;
            for other in Size::ALL.into_iter().filter(|&s| s != size) {
                if let Some(s) = world.at_place(place, other)?
                    && reach_without.has(s)
                {
                    intruded = Some((other, s));
                    break;
                }
            }
            if let Some((other, s)) = intruded {
                last_why = format!(
                    "\"{}\" is reached by {} who is {}: {}",
                    place.name,
                    a_size(other),
                    never(size),
                    way(world, &reach_without.way(s))
                );
                continue;
            }
            ok = true;
            break;
        }
        if !ok {
            return Ok(Err(format!(
                "the sketch's places do not match: the sketch's {pen} mark (for {} alone) at ({:.1}, {:.1}) is not kept: {last_why}. \
                 Make it reached by {} and by {}.",
                a_size(size),
                mark.centre[0],
                mark.centre[1],
                a_size(size),
                nobody_who_is_never(size)
            )));
        }
    }
    Ok(Ok(()))
}
