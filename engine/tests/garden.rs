//! The walled garden (`areas/garden/area.rs`), step 1's area: its module passes the engine's check
//! as it is built, is built again the same from its source, can be made a component of the world,
//! and answers as its source played natively does, bit for bit, at every size and wherever it is
//! asked (POC_PLAN.md, step 1, "Done when").

mod played;

use plateau_engine::check::check;
use plateau_engine::{Answer, Limits, Region, SIZES, Size, Wasm, hash, laws, points, points_near};

const SEED: u64 = 7;

fn module(name: &str) -> Vec<u8> {
    let path = format!("{}/../areas/modules/{name}.wasm", env!("CARGO_MANIFEST_DIR"));
    std::fs::read(&path).unwrap_or_else(|e| panic!("{path}: {e}: run areas/build.sh"))
}

fn answers(area: &mut dyn Region, points: &[[f32; 3]], size: f32) -> Vec<Answer> {
    let (mut all, mut out) = (Vec::new(), Vec::new());
    for batch in points.chunks(4096) {
        area.sample(batch, size, &mut out).unwrap();
        all.extend_from_slice(&out);
    }
    all
}

/// Where the garden is asked: all over it, and closely where its detail is (the crack and its
/// cave, the bench's seat, the lily pads, the wall's face), where the tiny look.
fn where_asked() -> Vec<[f32; 3]> {
    let mut all = points(40_000, 10.0, 1);
    all.extend(points_near([53.8, 0.1, 30.1], 0.4, 20_000, 2));
    all.extend(points_near([40.0, 4.5, 18.0], 2.0, 10_000, 3));
    all.extend(points_near([22.0, -0.3, 36.0], 5.0, 10_000, 4));
    all.extend(points_near([32.0, 1.2, 10.3], 1.0, 10_000, 5));
    all
}

#[test]
fn the_walled_garden_passes_the_check_as_it_is_built() {
    assert_eq!(check(&module("garden")), Ok(()));
}

#[test]
fn the_garden_s_module_answers_as_its_source_played_natively_at_every_size() {
    let points = where_asked();
    let mut native = played::area("garden", SEED);
    let mut wasm = Wasm::new(&module("garden"), SEED, Limits::default()).unwrap();
    // The three sizes, and sizes between, as a viewer grows or shrinks.
    for size in SIZES.map(|(_, h)| h).into_iter().chain([0.05, 0.4, 5.0]) {
        let (a, b) = (answers(&mut native, &points, size), answers(&mut wasm, &points, size));
        if let Some(i) = (0..points.len()).find(|&i| !laws::same(a[i], b[i])) {
            panic!("at {:?} for {size} m the source answers {:?} and the module {:?}", points[i], a[i], b[i]);
        }
        assert_eq!(hash(&a), hash(&b));
    }
}

#[test]
fn the_garden_describes_itself_the_same_natively_and_as_a_module() {
    let mut wasm = Wasm::new(&module("garden"), SEED, Limits::default()).unwrap();
    let described = wasm.describe().unwrap();
    assert_eq!(described, played::area("garden", SEED).describe().unwrap());
    assert_eq!(described.name, "The walled garden");
    let places: Vec<_> = described.places.iter().map(|p| (p.name.as_str(), p.size)).collect();
    assert_eq!(places, [("the cave behind the crack", Size::Tiny), ("the giant's seat", Size::Giant)]);
}

#[test]
fn each_size_sees_its_own_garden() {
    let points = where_asked();
    let mut native = played::area("garden", SEED);
    let hashes = SIZES.map(|(_, h)| hash(&answers(&mut native, &points, h)));
    assert!(hashes[0] != hashes[1] && hashes[1] != hashes[2], "{hashes:x?}");
}

#[test]
fn the_garden_never_looks_at_a_nans_bits() {
    let verdict = laws::nans_unseen(&module("garden"), SEED, &where_asked()[..20_000]).unwrap();
    assert_eq!(verdict, Ok(()));
}

#[test]
fn every_area_can_still_be_made_a_component_of_the_world() {
    for (name, _) in areas_played::AREAS {
        let component = wit_component::ComponentEncoder::default().module(&module(name)).unwrap().validate(true).encode();
        assert!(component.is_ok(), "{name}: {:?}", component.err());
    }
}

#[test]
fn the_modules_carried_are_built_again_the_same_from_their_sources() {
    let here = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let into = std::env::temp_dir().join(format!("plateau-areas-{}", std::process::id()));
    std::fs::create_dir_all(&into).unwrap();
    let built = std::process::Command::new(here.join("areas/build.sh")).arg(&into).output().unwrap();
    assert!(built.status.success(), "areas/build.sh: {}", String::from_utf8_lossy(&built.stderr));
    for (name, _) in areas_played::AREAS {
        let again = std::fs::read(into.join(format!("{name}.wasm"))).unwrap();
        assert!(again == module(name), "areas/modules/{name}.wasm is not what its source builds to: run areas/build.sh");
    }
    std::fs::remove_dir_all(&into).unwrap();
}
