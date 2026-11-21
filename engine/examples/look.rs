//! Pictures of the walled garden from its module, at each size (`picture`): what step 1 has to
//! look at. `cargo run --release -p plateau-engine --example look [folder]`, into
//! `target/pictures` unless a folder is given (`areas/garden/pictures/` are these).

use plateau_engine::picture::{Camera, picture};
use plateau_engine::{Compiled, Limits, Region};

fn main() -> plateau_engine::Result<()> {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let into = std::env::args().nth(1).map_or(here.join("target/pictures"), Into::into);
    std::fs::create_dir_all(&into)?;
    let compiled = Compiled::new(&std::fs::read(here.join("areas/modules/garden.wasm"))?, Limits::default())?;
    let make = || compiled.instantiate(7);
    let described = make()?.describe()?;
    let cave = described.places.iter().find(|p| p.name.contains("cave")).unwrap().at;
    let f = cave[1];
    let views = [
        ("giant-over-the-garden", Camera { eye: [4.0, 24.0, 70.0], at: [34.0, 0.0, 30.0], fov: 70.0, size: 17.0 }),
        ("person-at-the-gate", Camera { eye: [32.0, 1.6, 60.0], at: [32.0, 1.2, 40.0], fov: 75.0, size: 1.7 }),
        ("person-under-the-bench", Camera { eye: [36.0, 1.6, 26.0], at: [40.0, 4.0, 18.0], fov: 75.0, size: 1.7 }),
        ("person-at-the-crack", Camera { eye: [52.0, 1.0, 31.0], at: [53.7, 0.1, 30.0], fov: 60.0, size: 1.7 }),
        ("tiny-at-the-crack", Camera { eye: [53.55, f + 0.02, 30.05], at: [53.75, f + 0.03, 30.0], fov: 75.0, size: 0.017 }),
        ("tiny-in-the-cave", Camera { eye: [cave[0], f + 0.015, cave[2] + 0.2], at: [53.75, f + 0.03, 30.0], fov: 80.0, size: 0.017 }),
        ("tiny-on-the-pond", Camera { eye: [22.0, -0.3 + 0.05, 33.0], at: [22.0, -0.45, 35.0], fov: 75.0, size: 0.017 }),
    ];
    let (width, height) = (640, 360);
    for (name, camera) in views {
        let started = std::time::Instant::now();
        let pixels = picture(&make, &camera, width, height)?;
        let path = into.join(format!("{name}.png"));
        let mut encoder = png::Encoder::new(std::io::BufWriter::new(std::fs::File::create(&path)?), width as u32, height as u32);
        encoder.set_color(png::ColorType::Rgb);
        encoder.write_header()?.write_image_data(pixels.as_flattened())?;
        println!("{}: {:.1} s", path.display(), started.elapsed().as_secs_f32());
    }
    Ok(())
}
