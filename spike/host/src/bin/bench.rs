//! The spike's measurements (POC_PLAN.md, step 0): a million samples by batch size, natively and in
//! wasmtime with fuel counted and not; the instructions a sample takes at each size; a chunk made
//! at each size. `cargo run --release -p spike-host --bin bench`.

use spike_host::mesh::{Chunk, mesh};
use spike_host::{Area, Limits, Native, SIZES, Wasm, module, points, voxel};
use std::time::Instant;

fn million(area: &mut dyn Area, points: &[[f32; 3]], batch: usize, size: f32) -> f64 {
    let mut out = Vec::new();
    let started = Instant::now();
    let mut done = 0;
    while done < 1_000_000 {
        for b in points.chunks(batch) {
            area.sample(b, size, &mut out).unwrap();
            done += b.len();
        }
    }
    started.elapsed().as_secs_f64() * 1e9 / done as f64
}

/// Where the wall's face is, as a size sees it, at (y 1.5, z 30): its detail moves it further than
/// a tiny person's chunk is wide.
fn face(area: &mut dyn Area, size: f32) -> f32 {
    let (mut outside, mut inside) = (42.0f32, 44.5f32);
    let mut out = Vec::new();
    for _ in 0..40 {
        let mid = (outside + inside) / 2.0;
        area.sample(&[[mid, 1.5, 30.0]], size, &mut out).unwrap();
        if out[0].0 < 0.0 { inside = mid } else { outside = mid }
    }
    outside
}

fn main() -> spike_host::Result<()> {
    let optimized = module("O")?;
    let unoptimized = module("O0")?;
    let points = points(64);
    let started = Instant::now();
    let _ = Wasm::new(&optimized, 7, Limits::default())?;
    println!("compiling and instantiating the module: {:.1} ms\n", started.elapsed().as_secs_f64() * 1e3);

    for &(name, size) in &SIZES {
        println!("ns a sample, {name} ({size} m), by batch:");
        println!("{:>8} {:>8} {:>10} {:>10} {:>10} {:>10}", "batch", "native", "module", "no fuel", "one NaN", "-O0");
        for batch in [1, 16, 256, 4096, 65536] {
            let native = million(&mut Native { seed: 7 }, &points, batch, size);
            let fuel = million(&mut Wasm::new(&optimized, 7, Limits::default())?, &points, batch, size);
            let free = million(&mut Wasm::new(&optimized, 7, Limits { fuel: None, ..Limits::default() })?, &points, batch, size);
            let nans = Limits { canonical_nans: true, ..Limits::default() };
            let nans = million(&mut Wasm::new(&optimized, 7, nans)?, &points, batch, size);
            let slow = million(&mut Wasm::new(&unoptimized, 7, Limits::default())?, &points, batch, size);
            println!("{batch:>8} {native:>8.1} {fuel:>10.1} {free:>10.1} {nans:>10.1} {slow:>10.1}");
        }
        println!();
    }

    println!("fuel (about one a WebAssembly instruction) a sample:");
    for &(name, size) in &SIZES {
        let limits = Limits::default();
        let mut wasm = Wasm::new(&optimized, 7, limits)?;
        let mut out = Vec::new();
        wasm.sample(&points[..65536], size, &mut out)?;
        let used = limits.fuel.unwrap() - wasm.fuel_left().unwrap();
        println!("  {name:>6}: {:.0}", used as f64 / 65536.0);
    }
    println!();

    println!("a chunk of 32 x 32 x 32 cells at the size's voxel, on the wall's face, batches of 4096 (ms):");
    println!("{:>8} {:>8} {:>10} {:>10} {:>10} {:>10}", "size", "voxel", "native", "wasm", "meshing", "triangles");
    for &(name, size) in &SIZES {
        let v = voxel(size);
        let face = face(&mut Native { seed: 7 }, size);
        let chunk = Chunk { origin: [face - 16.0 * v, 1.5 - 16.0 * v, 30.0 - 16.0 * v], voxel: v, cells: [32; 3] };
        let mut wasm = Wasm::new(&optimized, 7, Limits::default())?;
        let best = |area: &mut dyn Area| {
            (0..5).map(|_| mesh(area, chunk, size, 4096).unwrap()).min_by(|a, b| a.1.sampling.total_cmp(&b.1.sampling)).unwrap()
        };
        let (_, native) = best(&mut Native { seed: 7 });
        let (m, t) = best(&mut wasm);
        let ms = |s: f64| s * 1e3;
        println!(
            "{name:>8} {v:>8.4} {:>10.2} {:>10.2} {:>10.2} {:>10}",
            ms(native.sampling),
            ms(t.sampling),
            ms(t.meshing),
            m.indices.len() / 3
        );
    }
    Ok(())
}
