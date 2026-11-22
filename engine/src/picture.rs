//! Pictures of an area, ray-marched on the CPU from its own answers, headless (POC_PLAN.md,
//! "Pictures", step 4): the same on any machine, in tests, and for Claude to be shown. Every ray
//! steps as far as the area says the nearest surface is, asked at the viewer's size, so a picture
//! shows the world that size sees. Lit by a low sun, with soft shadows, darkened in hollows.
//!
//! - **Views** (`Camera`): from an eye, hazed with distance. `views` gives an area's fixed views
//!   from what it describes: a giant over the whole area, each size at the start, and each place
//!   as the size it is for sees it, from wherever there is a clear sight of it.
//! - **Plans** (`Plan`): straight down, north at the top and east to the right, cut away above a
//!   height, so what is under a roof (a cave in a wall) shows; what is cut shows flat and dark.
//! - **Reach maps** (`reached`, `tint`): a plan coloured by what each size opens (`reach`): where
//!   only those who have been a size get to, in that size's colour (`COLOURS`); where anyone gets
//!   to, as it is; where nobody can stand, darkened; the places ringed in the colour of the size
//!   they are for. Plateau's long exposure.

use crate::body::Body;
use crate::reach::{Reach, Reaches, SAME_FLOOR, World, cell};
use crate::{Answer, Description, Material, Region, Result, SIZES, START, Size};

/// Where a picture is taken from, and by whom.
#[derive(Clone, Copy, Debug)]
pub struct Camera {
    pub eye: [f32; 3],
    pub at: [f32; 3],
    /// How wide the picture sees, in degrees.
    pub fov: f32,
    /// The viewer's height: the world is asked at it, and the picture's reach is scaled to it.
    pub size: f32,
}

/// A view straight down onto a square of the area, `half` metres from its middle each way, cut
/// away above `top`, as a viewer `size` tall sees it.
#[derive(Clone, Copy, Debug)]
pub struct Plan {
    pub middle: [f32; 2],
    pub half: f32,
    pub top: f32,
    pub size: f32,
}

impl Plan {
    /// Where a point across the ground falls on the plan, in pixels of a plan `width` wide, if on it.
    pub fn pixel(&self, x: f32, z: f32, width: usize) -> Option<(usize, usize)> {
        let k = width as f32 / (2.0 * self.half);
        let (i, j) = ((x - self.middle[0] + self.half) * k, (z - self.middle[1] + self.half) * k);
        (i >= 0.0 && j >= 0.0 && i < width as f32 && j < width as f32).then_some((i as usize, j as usize))
    }
}

/// A ray: where from, which way.
type Ray = ([f32; 3], [f32; 3]);

/// What a picture is taken as.
#[derive(Clone, Copy, Debug)]
enum Shot {
    View(Camera),
    Plan(Plan),
}

impl Shot {
    fn size(&self) -> f32 {
        match self {
            Shot::View(c) => c.size,
            Shot::Plan(p) => p.size,
        }
    }
}

/// A material's colour, linear RGB.
pub fn colour(m: Material) -> [f32; 3] {
    match m {
        Material::Soil => [0.20, 0.12, 0.06],
        Material::Grass => [0.13, 0.30, 0.06],
        Material::Sand => [0.55, 0.46, 0.30],
        Material::Mud => [0.13, 0.09, 0.05],
        Material::Stone => [0.42, 0.40, 0.37],
        Material::Wood => [0.38, 0.22, 0.10],
        Material::Leaf => [0.10, 0.35, 0.08],
        Material::Moss => [0.20, 0.38, 0.05],
        Material::Water => [0.04, 0.12, 0.22],
    }
}

const SUN: [f32; 3] = [0.48, 0.70, 0.53];

/// The picture `camera` takes, `width` by `height` pixels, sRGB, row by row from the top. Each
/// thread asks its own region, made by `make`.
pub fn picture<R: Region>(make: &(dyn Fn() -> Result<R> + Sync), camera: &Camera, width: usize, height: usize) -> Result<Vec<[u8; 3]>> {
    shoot(make, Shot::View(*camera), width, height)
}

/// The plan `plan`, `width` pixels square.
pub fn plan<R: Region>(make: &(dyn Fn() -> Result<R> + Sync), plan: &Plan, width: usize) -> Result<Vec<[u8; 3]>> {
    shoot(make, Shot::Plan(*plan), width, width)
}

fn shoot<R: Region>(make: &(dyn Fn() -> Result<R> + Sync), shot: Shot, width: usize, height: usize) -> Result<Vec<[u8; 3]>> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = height.div_ceil(threads);
    let mut pixels = vec![[0u8; 3]; width * height];
    std::thread::scope(|s| {
        let bands: Vec<_> = pixels
            .chunks_mut(rows * width)
            .enumerate()
            .map(|(k, band)| s.spawn(move || -> Result<()> { draw(&mut make()?, &shot, width, height, k * rows, band) }))
            .collect();
        bands.into_iter().try_for_each(|b| b.join().expect("a band of the picture panicked"))
    })?;
    Ok(pixels)
}

fn draw(area: &mut dyn Region, shot: &Shot, width: usize, height: usize, first: usize, band: &mut [[u8; 3]]) -> Result<()> {
    let size = shot.size();
    // Each ray: where from, which way; how wide a pixel is at a distance along it (`spread.0` and
    // `spread.1` times the distance); how far it goes; how far out hollows are looked for.
    let pixels = (first..first + band.len() / width).flat_map(|y| (0..width).map(move |x| (x as f32 + 0.5, y as f32 + 0.5)));
    let (rays, spread, far, start): (Vec<Ray>, (f32, f32), f32, f32) = match *shot {
        Shot::View(camera) => {
            let forward = normal(sub(camera.at, camera.eye));
            let right = normal(cross(forward, [0.0, 1.0, 0.0]));
            let up = cross(right, forward);
            let half = (camera.fov.to_radians() * 0.5).tan();
            let rays = pixels
                .map(|(x, y)| {
                    let (u, v) = ((x / width as f32 * 2.0 - 1.0) * half, (1.0 - y / height as f32 * 2.0) * half * height as f32 / width as f32);
                    (camera.eye, normal(add(forward, add(scale(right, u), scale(up, v)))))
                })
                .collect();
            (rays, (0.0, 2.0 * half / width as f32), (size * 400.0).min(300.0), size * 0.05)
        }
        Shot::Plan(plan) => {
            let pixel = 2.0 * plan.half / width as f32;
            let rays = pixels
                .map(|(x, y)| ([plan.middle[0] - plan.half + x * pixel, plan.top, plan.middle[1] - plan.half + y * pixel], [0.0, -1.0, 0.0]))
                .collect();
            (rays, (pixel, 0.0), plan.top - crate::reach::LOWEST, 0.0)
        }
    };
    let wide = |t: f32| spread.0 + spread.1 * t;
    let hollows = |t: f32| if matches!(shot, Shot::View(_)) { t * 0.03 } else { spread.0 * 6.0 };
    // A plan's light is not stopped by what it cuts away.
    let roof = if let Shot::Plan(p) = shot { p.top } else { f32::INFINITY };
    // March every ray together, a batch of points at a time. A plan's ray that starts inside
    // something is where the plan cuts it.
    let mut t = vec![start; rays.len()];
    let mut hit: Vec<Option<Answer>> = vec![None; rays.len()];
    let mut cut = vec![false; rays.len()];
    let mut active: Vec<usize> = (0..rays.len()).collect();
    let mut out = Vec::new();
    for _ in 0..300 {
        if active.is_empty() {
            break;
        }
        let points: Vec<_> = active.iter().map(|&i| along(rays[i].0, rays[i].1, t[i])).collect();
        sample(area, &points, size, &mut out)?;
        let mut still = Vec::with_capacity(active.len());
        for (k, &i) in active.iter().enumerate() {
            let a = out[k];
            if a.distance < wide(t[i]) * 0.5 {
                hit[i] = Some(a);
                cut[i] = t[i] == start && a.distance < 0.0 && matches!(shot, Shot::Plan(_));
            } else {
                t[i] += a.distance.max(wide(t[i]) * 0.25);
                if t[i] < far {
                    still.push(i);
                }
            }
        }
        active = still;
    }
    // At each hit, which way the surface faces and how much sun reaches it.
    let hits: Vec<usize> = (0..rays.len()).filter(|&i| hit[i].is_some()).collect();
    let at: Vec<[f32; 3]> = hits.iter().map(|&i| along(rays[i].0, rays[i].1, t[i])).collect();
    let step: Vec<f32> = hits.iter().map(|&i| wide(t[i]).max(size * 0.001)).collect();
    let mut normals = vec![[0.0f32; 3]; hits.len()];
    for axis in 0..3 {
        let shifted = |sign: f32| -> Vec<[f32; 3]> {
            at.iter().zip(&step).map(|(p, e)| {
                let mut q = *p;
                q[axis] += sign * e;
                q
            }).collect()
        };
        let (mut plus, mut minus) = (Vec::new(), Vec::new());
        sample(area, &shifted(1.0), size, &mut plus)?;
        sample(area, &shifted(-1.0), size, &mut minus)?;
        for k in 0..hits.len() {
            normals[k][axis] = plus[k].distance - minus[k].distance;
        }
    }
    let normals: Vec<_> = normals.into_iter().map(normal).collect();
    let light = shadows(area, &at, &normals, &step, size, roof)?;
    // How open each hit is to the sky: nearer surfaces a little way out along its normal darken
    // it, so cracks and caves are dark.
    let mut open = vec![1.0f32; hits.len()];
    for k in 1..=5 {
        let reach: Vec<f32> = hits.iter().map(|&i| hollows(t[i]).max(size * 0.005) * k as f32).collect();
        let points: Vec<_> = (0..hits.len()).map(|j| add(at[j], scale(normals[j], reach[j]))).collect();
        sample(area, &points, size, &mut out)?;
        for j in 0..hits.len() {
            open[j] -= 1.5 * (reach[j] - out[j].distance).max(0.0) / reach[j] / (1 << k) as f32;
        }
    }
    for (i, pixel) in band.iter_mut().enumerate() {
        let d = rays[i].1;
        let sky = sky(d);
        let colour = match hits.binary_search(&i) {
            Ok(_) if cut[i] => colour(hit[i].unwrap().material).map(|c| c * 0.35),
            Ok(k) => {
                let (n, a) = (normals[k], hit[i].unwrap());
                let base = colour(a.material);
                let sun = dot(n, SUN).max(0.0) * light[k];
                let ambient = (0.30 + 0.15 * n[1]) * open[k].clamp(0.0, 1.0);
                // A lamp at the eye, lighting what the sun does not, within a few of the
                // viewer's heights: a room the tiny stand in is otherwise lit by the hollows'
                // dim ambient alone, a brown fog (found in step 5, by Claude, of its own areas).
                let lamp = match shot {
                    Shot::View(_) => 0.9 * (1.0 - light[k]) * (-dot(d, n)).max(0.0) / (1.0 + (t[i] / (4.0 * size)).powi(2)),
                    Shot::Plan(_) => 0.0,
                };
                let mut c = base.map(|b| b * (ambient + 1.6 * sun + lamp));
                if a.material == Material::Water {
                    let fresnel = 0.1 + 0.6 * (1.0 + dot(d, n)).powi(4);
                    c = mix(c, sky, fresnel);
                }
                let haze = if matches!(shot, Shot::View(_)) { 1.0 - (-t[i] / (size * 150.0)).exp() } else { 0.0 };
                mix(c, sky, haze)
            }
            Err(_) => sky,
        };
        *pixel = colour.map(|c| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8);
    }
    Ok(())
}

/// How much of the sun reaches each point, from 0 to 1, softened at shadows' edges.
fn shadows(area: &mut dyn Region, at: &[[f32; 3]], normals: &[[f32; 3]], step: &[f32], size: f32, roof: f32) -> Result<Vec<f32>> {
    let reach = size * 30.0;
    let mut light = vec![1.0f32; at.len()];
    let mut t: Vec<f32> = step.iter().map(|e| e * 2.0).collect();
    let start: Vec<[f32; 3]> = at.iter().zip(normals).zip(step).map(|((p, n), e)| add(*p, scale(*n, e * 2.0))).collect();
    let mut active: Vec<usize> = (0..at.len()).collect();
    let mut out = Vec::new();
    for _ in 0..64 {
        if active.is_empty() {
            break;
        }
        let points: Vec<_> = active.iter().map(|&k| along(start[k], SUN, t[k])).collect();
        sample(area, &points, size, &mut out)?;
        let mut still = Vec::with_capacity(active.len());
        for (j, &k) in active.iter().enumerate() {
            let d = out[j].distance;
            light[k] = light[k].min((8.0 * d / t[k]).clamp(0.0, 1.0));
            t[k] += d.max(step[k]);
            if light[k] > 0.02 && t[k] < reach && points[j][1] < roof {
                still.push(k);
            }
        }
        active = still;
    }
    Ok(light)
}

fn sample(area: &mut dyn Region, points: &[[f32; 3]], size: f32, out: &mut Vec<Answer>) -> Result<()> {
    out.clear();
    let mut batch = Vec::new();
    for chunk in points.chunks(4096) {
        area.sample(chunk, size, &mut batch)?;
        out.extend_from_slice(&batch);
    }
    Ok(())
}

fn sky(d: [f32; 3]) -> [f32; 3] {
    let up = d[1].max(0.0);
    mix([0.62, 0.70, 0.78], [0.22, 0.40, 0.70], up.sqrt())
}

fn along(o: [f32; 3], d: [f32; 3], t: f32) -> [f32; 3] {
    add(o, scale(d, t))
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    a.map(|c| c * k)
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn normal(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 0.0 { scale(a, 1.0 / l) } else { [0.0, 1.0, 0.0] }
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

/// Each size's colour on a reach map, sRGB: the tiny orange and giants purple, as the sketch's
/// pens for their places (CONCEPT.md, "The sketch"); persons yellow, which no pen is.
pub const COLOURS: [[u8; 3]; 3] = [[245, 140, 20], [245, 225, 60], [160, 70, 215]];

/// A name made of words for a file: lower case, words joined by hyphens.
pub fn slug(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect::<Vec<_>>().join("-")
}

/// An area's fixed views (POC_PLAN.md, step 4), each named: a giant over the whole area from the
/// south; each size standing at the start, looking in; and each place the area describes, as the
/// size it is for sees it, from the first viewpoint with a clear sight of it (`viewpoint`; a place
/// with none is left out).
pub fn views(area: &mut dyn Region, d: &Description) -> Result<Vec<(String, Camera)>> {
    let mut views = vec![("giant-over-the-area".to_string(), Camera { eye: [32.0, 42.0, 94.0], at: [32.0, 0.0, 30.0], fov: 60.0, size: SIZES[2].1 })];
    for (size, h) in SIZES {
        let body = Body::standing(area, [START[0], 40.0, START[1]], h)?;
        let eye = body.eye().map(|c| c as f32);
        let at = [eye[0], eye[1] - 0.35 * h, eye[2] - 6.0 * h];
        views.push((format!("{}-at-the-start", size.name()), Camera { eye, at, fov: 75.0, size: h }));
    }
    for place in &d.places {
        let h = place.size.height();
        let at = [place.at[0], place.at[1] + 0.4 * h, place.at[2]];
        if let Some(eye) = viewpoint(area, at, h)? {
            views.push((format!("{}-{}", place.size.name(), slug(&place.name)), Camera { eye, at, fov: 75.0, size: h }));
        }
    }
    Ok(views)
}

/// Where to see `at` from, as a viewer `size` tall: the first of points about it (as far as three
/// of their heights off, then nearer; a little above it, then higher; from the south, then round)
/// that is in the open and has a clear sight of it.
pub fn viewpoint(area: &mut dyn Region, at: [f32; 3], size: f32) -> Result<Option<[f32; 3]>> {
    let mut out = Vec::new();
    for far in [3.0, 2.0, 1.2] {
        for up in [0.3f32, 0.7] {
            for k in 0..8 {
                let round = std::f32::consts::TAU * k as f32 / 8.0;
                let way = [up.cos() * round.sin(), up.sin(), up.cos() * round.cos()];
                let eye = add(at, scale(way, far * size));
                // Along the sight, from the eye to the point, each step as far as is clear.
                let (mut t, length, mut clear) = (0.0, far * size, false);
                for _ in 0..200 {
                    sample(area, &[along(eye, scale(way, -1.0), t)], size, &mut out)?;
                    let d = out[0].distance;
                    if t == 0.0 && d < 0.25 * size {
                        break;
                    }
                    if t >= length - 0.3 * size {
                        clear = true;
                        break;
                    }
                    if d < 0.002 * size {
                        break;
                    }
                    t += d;
                }
                if clear {
                    return Ok(Some(eye));
                }
            }
        }
    }
    Ok(None)
}

/// Which sizes can stand in each pixel of a plan `width` pixels square, as bits (1 the tiny, 2 a
/// person, 4 a giant): the stands `reach` reached on the floor seen from above, each filling its
/// column (with `bodies`, as wide as a body standing there: a person's feet keep off a wall, their
/// body does not), where the finest column found at a pixel has a floor at its height with room
/// for it. The floor seen is the highest of its column between `low` and `high` (where the
/// plan is cut away): the seat of a bench, not the ground under it.
pub fn reached(world: &World, reach: &Reach, plan: &Plan, low: f32, high: f32, width: usize, bodies: bool) -> Vec<u8> {
    let mut bits = vec![0u8; width * width];
    // A span across the plan, in pixels, clamped to it: none if wholly off it.
    let k = width as f32 / (2.0 * plan.half);
    let span = |c: f32, half: f32, m: f32| {
        let (a, b) = ((c - half - m + plan.half) * k, (c + half - m + plan.half) * k);
        (b > 0.0 && a < width as f32).then(|| (a.max(0.0) as usize, (b.ceil() as usize).clamp(1, width) - 1))
    };
    for &s in &reach.stands {
        let [x, y, z] = world.point(s);
        let seen = world.column(s).iter().position(|f| f.y <= high);
        if y < low || seen != Some(s.floor as usize) {
            continue;
        }
        let half = 0.5 * cell(s.size) + if bodies { crate::body::RADIUS * s.size.height() } else { 0.0 };
        let (Some((i0, i1)), Some((j0, j1))) = (span(x, half, plan.middle[0]), span(z, half, plan.middle[1])) else { continue };
        // A stand fills a pixel only where the finest column found there (of its size or smaller)
        // has a floor at its height too, with room over it for this size: a giant on the lawn beside
        // a wall does not fill the wall, nor a cave level with the lawn; a person on a wall's top does
        // not fill the lawn below its edge.
        let finer: Vec<Size> = Size::ALL.into_iter().filter(|f| f.height() <= s.size.height()).collect();
        let near = SAME_FLOOR * s.size.height();
        for j in j0..=j1 {
            for i in i0..=i1 {
                let (px, pz) = (plan.middle[0] - plan.half + (i as f32 + 0.5) / k, plan.middle[1] - plan.half + (j as f32 + 0.5) / k);
                let floors = finer.iter().find_map(|&f| world.floors_at(f, px, pz));
                if floors.is_none_or(|fl| fl.iter().any(|f| (f.y - y).abs() <= near && f.ceiling - f.y >= 0.9 * s.size.height())) {
                    bits[j * width + i] |= 1 << s.size as u8;
                }
            }
        }
    }
    bits
}

/// What each size opens, in each pixel, as bits (as `reached`'s): `all` is who can stand there,
/// being any size (`reached`, from `World::reach` of every size); `without[s]`, who can that is
/// never size `s`. A size opens what is reached, but not by anyone who is never it. Where only the
/// tiny stand, a person is not said to open it: without persons, the tiny are not searched.
pub fn opens(all: &[u8], without: [&[u8]; 3]) -> Vec<u8> {
    (0..all.len())
        .map(|k| {
            let only_tiny = all[k] == 1;
            let mut bits = 0;
            for (s, never) in without.iter().enumerate() {
                if all[k] != 0 && never[k] == 0 && !(s == Size::Person as usize && only_tiny) {
                    bits |= 1 << s;
                }
            }
            bits
        })
        .collect()
}

/// A plan coloured by what each size opens (Plateau's long exposure; `opens`): where only those who
/// have been a size get to, it is tinted that size's colour (`COLOURS`; striped, if more than one);
/// where anyone gets to (`all`), or a body standing there reaches (`near`), it is left as it is;
/// where nobody is at the plan's heights but somebody is at others (`anywhere`), greyed; where
/// nobody can be at all, darkened. Each place is ringed in the colour of the size it is for.
pub fn tint(pixels: &mut [[u8; 3]], map: &Map, plan: &Plan, width: usize, places: &[(Size, [f32; 3])]) {
    let Map { all, near, anywhere, opens } = map;
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| [0, 1, 2].map(|c| (a[c] as f32 + (b[c] as f32 - a[c] as f32) * t) as u8);
    for (k, p) in pixels.iter_mut().enumerate() {
        let (i, j) = (k % width, k / width);
        let needs: Vec<usize> = (0..3).filter(|&s| opens[k] >> s & 1 == 1).collect();
        *p = if all[k] == 0 && near[k] == 0 && anywhere[k] != 0 {
            mix(*p, [128, 128, 128], 0.6)
        } else if all[k] == 0 && near[k] == 0 {
            p.map(|c| (c as f32 * 0.45) as u8)
        } else if needs.is_empty() {
            *p
        } else {
            mix(*p, COLOURS[needs[(i + j) / 6 % needs.len()]], 0.6)
        };
    }
    let r = (width as f32 / 80.0).max(4.0);
    for &(size, at) in places {
        let Some((ci, cj)) = plan.pixel(at[0], at[2], width) else { continue };
        let reach = (r + 3.0).ceil() as isize;
        for dj in -reach..=reach {
            for di in -reach..=reach {
                let (i, j) = (ci as isize + di, cj as isize + dj);
                if i < 0 || j < 0 || i >= width as isize || j >= width as isize {
                    continue;
                }
                let d = ((di * di + dj * dj) as f32).sqrt();
                let p = &mut pixels[j as usize * width + i as usize];
                if (d - r).abs() < 1.0 {
                    *p = COLOURS[size as usize];
                } else if (d - r).abs() < 2.2 {
                    *p = [0, 0, 0];
                }
            }
        }
    }
}

/// An area's plans (POC_PLAN.md, step 4), each named, with the heights of the floors its reach
/// map counts: the whole area as a person sees it, every floor counted; and about each place, as
/// the size it is for sees it, thirty of their heights each way (the whole area at most), cut away
/// ten of their heights above it, counting the floors from a fifth of their height (and 20 cm)
/// below it up to the cut. So a cave inside a wall shows, and the wall's top above it does not
/// count, nor the ground under a giant's seat.
pub fn plans(d: &Description) -> Vec<(String, Plan, (f32, f32))> {
    let whole = Plan { middle: [32.0, 32.0], half: 32.0, top: crate::reach::HIGHEST, size: SIZES[1].1 };
    let mut plans = vec![("plan".to_string(), whole, (f32::NEG_INFINITY, f32::INFINITY))];
    for place in &d.places {
        let h = place.size.height();
        let [x, y, z] = place.at;
        let plan = Plan { middle: [x, z], half: (30.0 * h).min(32.0), top: y + 10.0 * h, size: h };
        plans.push((format!("plan-{}-{}", place.size.name(), slug(&place.name)), plan, (y - 0.2 * h - 0.2, plan.top)));
    }
    plans
}

/// A plan's reach map, pixel by pixel, each as bits by size (1 the tiny, 2 a person, 4 a giant):
/// who can stand there being any size, at the plan's heights (`all`); whose body standing near
/// reaches it, at the plan's heights (`near`) and at any (`anywhere`); and what each size opens,
/// by where bodies reach (`opens`).
pub struct Map {
    pub all: Vec<u8>,
    pub near: Vec<u8>,
    pub anywhere: Vec<u8>,
    pub opens: Vec<u8>,
}

/// The reach map of a plan `width` pixels square, counting the floors from `low` to `high`.
pub fn map(world: &World, reaches: &Reaches, plan: &Plan, (low, high): (f32, f32), width: usize) -> Map {
    let all = reached(world, &reaches.all, plan, low, high, width, false);
    let near = reached(world, &reaches.all, plan, low, high, width, true);
    let anywhere = reached(world, &reaches.all, plan, f32::NEG_INFINITY, f32::INFINITY, width, true);
    // What each size opens, by bodies: a person's feet keep off a wall that a giant's wide column
    // reaches, but their bodies both come to it.
    let without: Vec<_> = reaches.without.iter().map(|r| reached(world, r, plan, low, high, width, true)).collect();
    let opens = opens(&near, [&without[0], &without[1], &without[2]]);
    Map { all, near, anywhere, opens }
}
