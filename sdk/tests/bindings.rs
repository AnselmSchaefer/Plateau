//! The bindings the SDK carries are what wit-bindgen makes of `wit/region.wit` with the options an
//! area's build needs (the SDK's own runtime, the export macro public). Change the WIT, then make
//! them again: `PLATEAU_BLESS=1 cargo test -p plateau-sdk --test bindings`.

use wit_bindgen_core::wit_parser::Resolve;
use wit_bindgen_core::{Files, WorldGenerator};

#[test]
fn the_bindings_carried_are_made_again_the_same_from_the_wit() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut resolve = Resolve::default();
    let (package, _) = resolve.push_dir(here.join("wit")).unwrap();
    let world = resolve.select_world(&[package], Some("region")).unwrap();
    let opts = wit_bindgen_rust::Opts { format: true, runtime_path: Some("crate::rt".into()), pub_export_macro: true, ..Default::default() };
    let mut files = Files::default();
    opts.build().generate(&mut resolve, world, &mut files).unwrap();
    let (_, made) = files.iter().find(|(name, _)| *name == "region.rs").expect("region.rs made");
    let path = here.join("src/bindings.rs");
    if std::env::var_os("PLATEAU_BLESS").is_some() {
        std::fs::write(&path, made).unwrap();
    }
    let carried = std::fs::read(&path).unwrap_or_default();
    assert!(made == carried.as_slice(), "src/bindings.rs is not what wit/region.wit makes: make it again (see this file)");
}
