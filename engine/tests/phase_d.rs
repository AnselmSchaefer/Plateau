//! Phase D (`WORLD_PLAN.md`): saves that mean the same thing everywhere. A save names each module it
//! was made of relative to the world's root and by its bytes' hash; a module rebuilt differently
//! is refused by name (or let through and reported); a save carried elsewhere still opens; saves of
//! version 1 still load and are written back as version 2.

use std::path::{Path, PathBuf};

use plateau_engine::compose::{ElementCatalog, PlacedElement};
use plateau_engine::edit::{Edit, SavedTile, Shape, WorldSave};
use plateau_engine::modules::{ModuleCheck, relative_to_root, sha256_hex};
use plateau_engine::{Limits, Material, OpenWorld};

const PERSON: f32 = 1.7;
const GARDEN: &str = "areas/modules/garden.wasm";
const TREE: &str = "elements/modules/tree.wasm";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// A fresh folder standing in for a world's root, with the garden and the tree copied into it.
fn world_root(name: &str) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("phase_d").join(name);
    if root.exists() {
        std::fs::remove_dir_all(&root).unwrap();
    }
    for module in [GARDEN, TREE] {
        let to = root.join(module);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        std::fs::copy(repo_root().join(module), to).unwrap();
    }
    root
}

/// A module's bytes with a custom section appended: still a valid module, but other bytes.
fn rebuilt(path: &Path) {
    let mut bytes = std::fs::read(path).unwrap();
    let name = b"rebuilt";
    bytes.extend_from_slice(&[0x00, (1 + name.len()) as u8, name.len() as u8]);
    bytes.extend_from_slice(name);
    std::fs::write(path, bytes).unwrap();
}

fn tree_at(id: u32, module: &str) -> PlacedElement {
    PlacedElement {
        id,
        module: module.into(),
        seed: 218,
        pos: [41.5, 0.03, 45.7],
        yaw: 0.0,
        rot: None,
        scale: 1.0,
        stamp_size: PERSON,
        after_edits: 1,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: Some("local".into()),
    }
}

/// A save as `play` makes one: its tile named by an absolute path with `..` in it.
fn played_save(root: &Path) -> WorldSave {
    let lawn = vec![Edit::add(Shape::Sphere { radius: 1.2 }, [38.0, 0.0, 50.0], 0.3, Material::Soil, PERSON)];
    let mut save = WorldSave::with_elements("The walled garden", 7, lawn, vec![tree_at(1, TREE)]);
    let roundabout = root.join("areas").join("..").join(GARDEN);
    save.tiles = vec![SavedTile { at: [0, 0], module: roundabout.to_string_lossy().into_owned(), seed: 7 }];
    save
}

#[test]
fn a_sealed_save_names_its_modules_relative_to_the_root_with_their_hashes() {
    let root = world_root("sealed");
    let mut save = played_save(&root);
    save.seal(&root).unwrap();

    assert_eq!(save.tiles[0].module, GARDEN);
    assert_eq!(save.elements[0].module, TREE);
    let names: Vec<&str> = save.modules.iter().map(|m| m.path.as_str()).collect();
    assert_eq!(names, [GARDEN, TREE]);
    let garden_hash = sha256_hex(&std::fs::read(root.join(GARDEN)).unwrap());
    assert_eq!(save.recorded_hash(GARDEN), Some(garden_hash.as_str()));

    let text = save.to_text();
    assert!(text.starts_with("plateau-world 2\n"), "{text}");
    assert!(text.contains(&format!("module {GARDEN} sha256 {garden_hash}\n")), "{text}");
    assert!(!text.contains(&root.to_string_lossy().into_owned()), "no absolute paths left: {text}");
    let again = WorldSave::from_text(&text).unwrap();
    assert_eq!(again.modules, save.modules);
    assert_eq!(again.to_text(), text, "round-trips exactly");
}

#[test]
fn a_rebuilt_module_is_refused_by_name_or_let_through_and_reported() {
    let root = world_root("rebuilt");
    let mut save = played_save(&root);
    let path = root.join("worlds/garden.plateau");
    save.save_sealed(&path, &root).unwrap();
    let saved = WorldSave::load(&path).unwrap();
    assert!(OpenWorld::open(saved.clone(), &root, None, PERSON, Limits::default()).unwrap().changed_modules.is_empty());

    rebuilt(&root.join(GARDEN));
    let refused = OpenWorld::open(saved.clone(), &root, None, PERSON, Limits::default()).err().expect("a changed area is refused");
    let message = format!("{refused:#}");
    assert!(message.contains(GARDEN) && message.contains("changed since this world was saved"), "{message}");
    assert_eq!(saved.changed_modules(&root), [GARDEN]);

    let open = OpenWorld::open_checked(saved, &root, None, PERSON, Limits::default(), ModuleCheck::AcceptChanged).unwrap();
    assert_eq!(open.changed_modules, [GARDEN]);
}

#[test]
fn a_rebuilt_element_is_refused_by_the_catalog() {
    let root = world_root("element");
    let mut save = played_save(&root);
    save.seal(&root).unwrap();
    rebuilt(&root.join(TREE));

    assert!(format!("{:#}", OpenWorld::open(save.clone(), &root, None, PERSON, Limits::default()).err().unwrap()).contains(TREE));

    let mut catalog = ElementCatalog::new(&root, Limits::default());
    catalog.expect_hashes(&save.modules, ModuleCheck::Strict);
    assert!(!catalog.discover().iter().any(|e| e.module == TREE), "the palette does not offer the changed tree as the saved one");
    assert!(catalog.get_or_load(TREE).is_err());

    catalog.expect_hashes(&save.modules, ModuleCheck::AcceptChanged);
    assert!(catalog.get_or_load(TREE).is_ok());
    assert_eq!(catalog.changed_modules(), [TREE]);
}

#[test]
fn a_save_carried_elsewhere_still_opens() {
    let root = world_root("carried");
    let mut save = played_save(&root);
    let here = root.join("deep/down/world.plateau");
    save.save_sealed(&here, &root).unwrap();

    // The same root found in another place, the save beside it in yet another.
    let moved = root.with_file_name("carried-moved");
    if moved.exists() {
        std::fs::remove_dir_all(&moved).unwrap();
    }
    std::fs::rename(&root, &moved).unwrap();
    let elsewhere = moved.join("elsewhere.plateau");
    std::fs::rename(moved.join("deep/down/world.plateau"), &elsewhere).unwrap();

    let open = OpenWorld::open(WorldSave::load(&elsewhere).unwrap(), &moved, None, PERSON, Limits::default()).unwrap();
    assert!(open.changed_modules.is_empty());
    assert_eq!(open.save.elements.len(), 1);
}

#[test]
fn version_one_saves_still_load_and_are_written_as_version_two() {
    let v1 = "plateau-world 1\narea The walled garden\nseed 7\ntile 0 0 areas/modules/garden.wasm seed 7\n\
              sdf add sphere 32 0 48 1.5 blend 0.3 mat soil size 1.7\n";
    let save = WorldSave::from_text(v1).unwrap();
    assert_eq!(save.version, 1);
    assert!(save.modules.is_empty(), "nothing recorded: taken as it is");
    let open = OpenWorld::open(save.clone(), &repo_root(), None, PERSON, Limits::default()).unwrap();
    assert!(open.changed_modules.is_empty());

    let mut again = save;
    again.seal(&repo_root()).unwrap();
    let text = again.to_text();
    assert!(text.starts_with("plateau-world 2\n"));
    assert!(text.contains(&format!("module {GARDEN} sha256 ")));

    // Hashes belong to version 2, written well.
    let hash = "0".repeat(64);
    assert!(WorldSave::from_text(&format!("{v1}module {GARDEN} sha256 {hash}\n")).is_err());
    let v2 = v1.replace("plateau-world 1", "plateau-world 2");
    assert!(WorldSave::from_text(&format!("{v2}module {GARDEN} sha256 {hash}\n")).is_ok());
    assert!(WorldSave::from_text(&format!("{v2}module {GARDEN} sha256 XYZ\n")).is_err());
    assert!(WorldSave::from_text(&format!("{v2}module {GARDEN} sha256 {hash}\nmodule {GARDEN} sha256 {hash}\n")).is_err());
    assert!(WorldSave::from_text("plateau-world 3\nseed 7\n").is_err());
}

#[test]
fn sealing_keeps_a_hash_it_cannot_read_again_and_refuses_one_it_never_had() {
    let root = world_root("unreadable");
    let mut save = played_save(&root);
    save.seal(&root).unwrap();
    let recorded = save.modules.clone();

    std::fs::remove_file(root.join(TREE)).unwrap();
    save.seal(&root).unwrap();
    assert_eq!(save.modules, recorded, "the tree's hash kept from the save it came from");

    save.elements.push(tree_at(2, "elements/modules/no-such.wasm"));
    let e = save.seal(&root).err().expect("a module with no hash and no file");
    assert!(format!("{e:#}").contains("no-such.wasm"));
}

#[test]
fn paths_outside_the_root_are_kept_as_they_are() {
    let root = repo_root();
    assert_eq!(relative_to_root(GARDEN, &root), GARDEN);
    assert_eq!(relative_to_root(&root.join("play").join("..").join(GARDEN).to_string_lossy(), &root), GARDEN);
    let outside = "/nowhere/else/area.wasm";
    assert_eq!(relative_to_root(outside, &root), outside);
}

/// The platform's maths library differs in its last bits between machines; nothing the engine
/// keeps may come from it (`engine/src/math.rs`). Pictures, for people, are exempt.
#[test]
fn the_engine_does_its_own_transcendental_maths() {
    const LIBRARY: [&str; 16] = [
        ".sin()", ".cos()", ".tan()", ".asin()", ".acos()", ".atan()", ".atan2(", ".sin_cos()", ".powf(", ".exp()", ".exp2()", ".ln()",
        ".log2()", ".log10()", ".hypot(", ".cbrt()",
    ];
    const EXEMPT: [&str; 2] = ["picture.rs", "math.rs"];
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = Vec::new();
    let mut files: Vec<PathBuf> = std::fs::read_dir(&src).unwrap().flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "rs")).collect();
    files.sort();
    for file in files {
        let name = file.file_name().unwrap().to_string_lossy().into_owned();
        if EXEMPT.contains(&name.as_str()) {
            continue;
        }
        for (n, line) in std::fs::read_to_string(&file).unwrap().lines().enumerate() {
            let code = line.split("//").next().unwrap();
            for call in LIBRARY {
                if code.contains(call) {
                    found.push(format!("{name}:{}: {call} in `{}`", n + 1, line.trim()));
                }
            }
        }
    }
    assert!(found.is_empty(), "use plateau_engine::math instead:\n{}", found.join("\n"));
}
