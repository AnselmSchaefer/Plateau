//! Pictures of an area from its module (`picture`, POC_PLAN.md, step 4): its fixed views, its
//! plans, and its reach maps, the plans coloured by who can stand where (`reach`).
//! `cargo run --release -p plateau-engine --example look [module] [folder] [--seed N]`: the walled
//! garden into `target/pictures` unless told otherwise (`areas/garden/pictures/` are these). For
//! the garden, also the views of the crack and the cave chosen by hand in step 1.

use plateau_engine::picture::{Camera, map, picture, plan, plans, tint, views};
use plateau_engine::reach::World;
use plateau_engine::{Compiled, Limits, Region};

fn main() -> plateau_engine::Result<()> {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let (mut module, mut into, mut seed) = (here.join("areas/modules/garden.wasm"), None, 7u64);
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--seed" => seed = args.next().and_then(|s| s.parse().ok()).expect("--seed takes a number"),
            _ if arg.ends_with(".wasm") => module = arg.into(),
            _ => into = Some(std::path::PathBuf::from(arg)),
        }
    }
    let into = into.unwrap_or(here.join("target/pictures"));
    std::fs::create_dir_all(&into)?;
    let compiled = Compiled::new(&std::fs::read(&module)?, Limits::default())?;
    let make = || compiled.instantiate(seed);
    let mut area = make()?;
    let described = area.describe()?;
    let write = |name: &str, pixels: &[[u8; 3]], width: usize, height: usize, started: std::time::Instant| -> plateau_engine::Result<()> {
        let path = into.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path)?), width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header()?.write_image_data(pixels.as_flattened())?;
        println!("{}: {:.1} s", path.display(), started.elapsed().as_secs_f32());
        Ok(())
    };

    let mut shots = views(&mut area, &described)?;
    if module.ends_with("garden.wasm") {
        let cave = described.places.iter().find(|p| p.name.contains("cave")).unwrap().at;
        let f = cave[1];
        shots.extend([
            ("person-at-the-gate", Camera { eye: [32.0, 1.6, 60.0], at: [32.0, 1.2, 40.0], fov: 75.0, size: 1.7 }),
            ("person-under-the-bench", Camera { eye: [36.0, 1.6, 26.0], at: [40.0, 4.0, 18.0], fov: 75.0, size: 1.7 }),
            ("person-at-the-crack", Camera { eye: [52.0, 1.0, 31.0], at: [53.7, 0.1, 30.0], fov: 60.0, size: 1.7 }),
            ("tiny-at-the-crack", Camera { eye: [53.55, f + 0.02, 30.05], at: [53.75, f + 0.03, 30.0], fov: 75.0, size: 0.017 }),
            ("tiny-in-the-cave", Camera { eye: [cave[0], f + 0.015, cave[2] + 0.2], at: [53.75, f + 0.03, 30.0], fov: 80.0, size: 0.017 }),
            ("tiny-on-the-pond", Camera { eye: [22.0, -0.3 + 0.05, 33.0], at: [22.0, -0.45, 35.0], fov: 75.0, size: 0.017 }),
        ].map(|(n, c)| (n.to_string(), c)));
    }
    let (width, height) = (640, 360);
    for (name, camera) in &shots {
        let started = std::time::Instant::now();
        write(name, &picture(&make, camera, width, height)?, width, height, started)?;
    }

    // Who can stand where, once for every plan.
    let started = std::time::Instant::now();
    let mut world = World::new(&mut area, &described.places);
    let reaches = world.reaches()?;
    println!("where each size can go: {:.1} s", started.elapsed().as_secs_f32());
    let places: Vec<_> = described.places.iter().map(|p| (p.size, p.at)).collect();
    let side = 640;
    for (name, p, band) in plans(&described) {
        let started = std::time::Instant::now();
        let mut pixels = plan(&make, &p, side)?;
        write(&name, &pixels, side, side, started)?;
        let started = std::time::Instant::now();
        let m = map(&world, &reaches, &p, band, side);
        tint(&mut pixels, &m, &p, side, &places);
        write(&name.replacen("plan", "reach", 1), &pixels, side, side, started)?;
    }
    Ok(())
}
