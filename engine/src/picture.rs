//! Pictures of an area, ray-marched on the CPU from its own answers, headless (POC_PLAN.md,
//! "Pictures"): the same on any machine, in tests, and for Claude to be shown. Every ray steps as
//! far as the area says the nearest surface is, asked at the viewer's size, so a picture shows the
//! world that size sees. Lit by a low sun, with soft shadows, and hazed with distance.

use crate::{Answer, Material, Region, Result};

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
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
    let rows = height.div_ceil(threads);
    let mut pixels = vec![[0u8; 3]; width * height];
    std::thread::scope(|s| {
        let bands: Vec<_> = pixels
            .chunks_mut(rows * width)
            .enumerate()
            .map(|(k, band)| s.spawn(move || -> Result<()> { draw(&mut make()?, camera, width, height, k * rows, band) }))
            .collect();
        bands.into_iter().try_for_each(|b| b.join().expect("a band of the picture panicked"))
    })?;
    Ok(pixels)
}

fn draw(area: &mut dyn Region, camera: &Camera, width: usize, height: usize, first: usize, band: &mut [[u8; 3]]) -> Result<()> {
    let size = camera.size;
    let forward = normal(sub(camera.at, camera.eye));
    let right = normal(cross(forward, [0.0, 1.0, 0.0]));
    let up = cross(right, forward);
    let half = (camera.fov.to_radians() * 0.5).tan();
    let pixel = 2.0 * half / width as f32;
    let far = (size * 400.0).min(300.0);
    let rays: Vec<[f32; 3]> = (0..band.len())
        .map(|i| {
            let (x, y) = ((i % width) as f32 + 0.5, (first + i / width) as f32 + 0.5);
            let (u, v) = ((x / width as f32 * 2.0 - 1.0) * half, (1.0 - y / height as f32 * 2.0) * half * height as f32 / width as f32);
            normal(add(forward, add(scale(right, u), scale(up, v))))
        })
        .collect();
    // March every ray together, a batch of points at a time.
    let mut t = vec![size * 0.05; rays.len()];
    let mut hit: Vec<Option<Answer>> = vec![None; rays.len()];
    let mut active: Vec<usize> = (0..rays.len()).collect();
    let mut out = Vec::new();
    for _ in 0..300 {
        if active.is_empty() {
            break;
        }
        let points: Vec<_> = active.iter().map(|&i| along(camera.eye, rays[i], t[i])).collect();
        sample(area, &points, size, &mut out)?;
        let mut still = Vec::with_capacity(active.len());
        for (k, &i) in active.iter().enumerate() {
            let a = out[k];
            if a.distance < t[i] * pixel * 0.5 {
                hit[i] = Some(a);
            } else {
                t[i] += a.distance.max(t[i] * pixel * 0.25);
                if t[i] < far {
                    still.push(i);
                }
            }
        }
        active = still;
    }
    // At each hit, which way the surface faces and how much sun reaches it.
    let hits: Vec<usize> = (0..rays.len()).filter(|&i| hit[i].is_some()).collect();
    let at: Vec<[f32; 3]> = hits.iter().map(|&i| along(camera.eye, rays[i], t[i])).collect();
    let step: Vec<f32> = hits.iter().map(|&i| (t[i] * pixel).max(size * 0.001)).collect();
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
    let light = shadows(area, &at, &normals, &step, size)?;
    // How open each hit is to the sky: nearer surfaces a little way out along its normal darken
    // it, so cracks and caves are dark.
    let mut open = vec![1.0f32; hits.len()];
    for k in 1..=5 {
        let reach: Vec<f32> = hits.iter().map(|&i| (t[i] * 0.03).max(size * 0.005) * k as f32).collect();
        let points: Vec<_> = (0..hits.len()).map(|j| add(at[j], scale(normals[j], reach[j]))).collect();
        sample(area, &points, size, &mut out)?;
        for j in 0..hits.len() {
            open[j] -= 1.5 * (reach[j] - out[j].distance).max(0.0) / reach[j] / (1 << k) as f32;
        }
    }
    for (i, pixel) in band.iter_mut().enumerate() {
        let d = rays[i];
        let sky = sky(d);
        let colour = match hits.binary_search(&i) {
            Ok(k) => {
                let (n, a) = (normals[k], hit[i].unwrap());
                let base = colour(a.material);
                let sun = dot(n, SUN).max(0.0) * light[k];
                let ambient = (0.30 + 0.15 * n[1]) * open[k].clamp(0.0, 1.0);
                let mut c = base.map(|b| b * (ambient + 1.6 * sun));
                if a.material == Material::Water {
                    let fresnel = 0.1 + 0.6 * (1.0 + dot(d, n)).powi(4);
                    c = mix(c, sky, fresnel);
                }
                let haze = 1.0 - (-t[i] / (size * 150.0)).exp();
                mix(c, sky, haze)
            }
            Err(_) => sky,
        };
        *pixel = colour.map(|c| (c.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0 + 0.5) as u8);
    }
    Ok(())
}

/// How much of the sun reaches each point, from 0 to 1, softened at shadows' edges.
fn shadows(area: &mut dyn Region, at: &[[f32; 3]], normals: &[[f32; 3]], step: &[f32], size: f32) -> Result<Vec<f32>> {
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
            if light[k] > 0.02 && t[k] < reach {
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
