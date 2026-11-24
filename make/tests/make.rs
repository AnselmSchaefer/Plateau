//! The making loop (POC_PLAN.md, step 5) with a stand-in for Claude: it never calls Claude
//! (POC_PLAN.md, "Tests"). The stand-in answers with what each test scripts, and the tests look at
//! what it was asked and what was kept. The area answered is small, so it is judged quickly: a
//! stone block with a room for the tiny behind a slot, and a pillar only a giant steps onto.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use plateau_make::{Builder, Made, Reply, Settings, Writer, judge_element, make, make_element, source_of};

/// The area, with the pillar `PILLAR` metres high: 3 m keeps every law; half a metre, a person
/// jumps onto it.
const AREA: &str = r#"use plateau_sdk::*;

struct Blocks;

const PILLAR: f32 = 3.0;

impl Area for Blocks {
    fn new(_: u64) -> Blocks {
        Blocks
    }

    fn describe(&self) -> Description {
        Description {
            name: "Blocks".into(),
            places: vec![
                Place::new("the room in the block", Size::Tiny, point(40.0, 0.0, 30.0)),
                Place::new("the top of the pillar", Size::Giant, point(20.0, PILLAR, 20.0)),
            ],
            borders: Borders::of_ground(|_, _| 0.0),
        }
    }

    fn sample(&self, p: Point, _view: &View) -> Answer {
        let ground = Answer::new(p.y, Material::Grass);
        // A block of stone 40 cm a side; inside, a room 6 cm a side, and a slot 1.6 cm wide and
        // 3 cm high from its west face into the room, each cut from a millimetre below the ground
        // (so no lid is left where it meets it) and the slot from beyond the face.
        let block = cuboid(p - point(40.0, 0.2, 30.0), point(0.2, 0.2, 0.2));
        let room = cuboid(p - point(40.0, 0.0295, 30.0), point(0.03, 0.0305, 0.03));
        let slot = cuboid(p - point(39.85, 0.0145, 30.0), point(0.17, 0.0155, 0.008));
        let stone = cut(block, union(room, slot));
        let ground = ground.cut(union(room, slot));
        let pillar = cuboid(p - point(20.0, PILLAR * 0.5, 20.0), point(1.5, PILLAR * 0.5, 1.5));
        ground.or(Answer::new(stone, Material::Stone)).or(Answer::new(pillar, Material::Stone))
    }
}

area!(Blocks);
"#;

fn right() -> String {
    format!("Here it is.\n\n```rust\n{AREA}```\n")
}

fn low_pillar() -> String {
    right().replace("const PILLAR: f32 = 3.0;", "const PILLAR: f32 = 0.5;")
}

const BROKEN: &str = "```rust\nuse plateau_sdk::*;\nfn broken( {\n```";

/// Claude stood in for: the answers it gives, in turn, and what it was asked, with or without
/// pictures.
struct StandIn {
    answers: VecDeque<String>,
    asked: Vec<(String, Option<PathBuf>)>,
}

impl Writer for StandIn {
    fn write(&mut self, prompt: &str, pictures: Option<&Path>) -> Result<Reply, String> {
        self.asked.push((prompt.to_string(), pictures.map(Path::to_path_buf)));
        let text = self.answers.pop_front().ok_or("asked more than the test scripted")?;
        Ok(Reply { text, cost: 0.5, seconds: 1.0 })
    }
}

fn builder() -> &'static Builder {
    static BUILDER: OnceLock<Builder> = OnceLock::new();
    BUILDER.get_or_init(|| {
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
        Builder::new(&repo, &repo.join("target/plateau-make-tests/sdk")).unwrap()
    })
}

const QUICK: Settings = Settings { rays: 80, view_width: 64, plan_width: 64 };

/// The stand-in's answers made into an area, in a folder of the test's own.
fn made(test: &str, answers: &[String]) -> (Made, StandIn, PathBuf) {
    made_with_sketch(test, None, answers)
}

fn made_with_sketch(test: &str, sketch: Option<&Path>, answers: &[String]) -> (Made, StandIn, PathBuf) {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/plateau-make-tests").join(test);
    std::fs::remove_dir_all(&dir).ok();
    let mut writer = StandIn { answers: answers.iter().cloned().collect(), asked: Vec::new() };
    let made = make(&mut writer, builder(), "two blocks", sketch, &dir, QUICK, &mut |_| {}).unwrap();
    (made, writer, dir)
}

#[test]
fn the_source_is_the_last_rust_block_of_an_answer() {
    assert_eq!(source_of("words\n```rust\nfn a() {}\n```\nmore\n```rust\nfn b() {}\n```\n").as_deref(), Some("fn b() {}\n"));
    assert_eq!(source_of("no code"), None);
}

#[test]
fn an_area_that_keeps_every_law_at_once_is_shown_its_pictures_once_and_kept() {
    let (made, writer, dir) = made("at-once", &[right(), right()]);
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "keeps every law"), ("look", "keeps every law")]);
    assert_eq!((made.fixes, made.looked, made.cost()), (0, true, 1.0));
    assert_eq!(made.kept.as_deref(), source_of(&right()).as_deref());
    // The look: told where the pictures are, and given them to read.
    let (look, pictures) = &writer.asked[1];
    assert!(look.contains("keeps every law") && look.contains("reach.png") && look.contains("giant-over-the-area.png"), "{look}");
    assert_eq!(pictures.as_deref(), Some(dir.join("pictures").as_path()));
    assert!(writer.asked[0].1.is_none(), "no pictures to write from");
    for file in ["area.rs", "area.wasm", "pictures/reach.png", "rounds/1.rs", "rounds/2-look-answer.md"] {
        assert!(dir.join(file).exists(), "{file}");
    }
}

#[test]
fn what_does_not_build_goes_back_with_the_compiler_s_words_and_the_fix_is_kept() {
    let (made, writer, _) = made("compiler", &["No code, sorry.".into(), BROKEN.into(), right(), right()]);
    let came: Vec<_> = made.rounds.iter().map(|r| r.came.as_str()).collect();
    assert_eq!(came, ["no source", "does not build", "keeps every law", "keeps every law"]);
    assert_eq!(made.fixes, 2);
    assert!(writer.asked[1].0.contains("There is no ```rust block"), "{}", writer.asked[1].0);
    let compiler = &writer.asked[2].0;
    assert!(compiler.contains("It does not build") && compiler.contains("error"), "{compiler}");
    assert!(made.kept.is_some());
}

#[test]
fn a_broken_law_goes_back_in_its_own_words() {
    let (made, writer, _) = made("law", &[low_pillar(), right(), right()]);
    let came: Vec<_> = made.rounds.iter().map(|r| r.came.as_str()).collect();
    assert_eq!(came, ["breaks: gates are real", "keeps every law", "keeps every law"]);
    let laws = &writer.asked[1].0;
    assert!(laws.contains("**gates are real**") && laws.contains("\"the top of the pillar\"") && laws.contains("is reached by a person"), "{laws}");
    assert_eq!(made.kept.as_deref(), source_of(&right()).as_deref());
}

#[test]
fn after_three_fixes_nothing_is_kept_if_nothing_kept_every_law() {
    let (made, writer, dir) = made("never", &[BROKEN.into(), BROKEN.into(), low_pillar(), low_pillar()]);
    assert_eq!((made.rounds.len(), made.fixes, made.looked), (4, 3, false));
    assert!(made.kept.is_none() && !dir.join("area.rs").exists());
    assert_eq!(writer.asked.len(), 4);
}

#[test]
fn a_better_version_that_breaks_a_law_is_fixed_or_the_last_good_one_kept() {
    // Kept, looked at, made "better" but breaking a law, then fixed: the fix is kept.
    let (made, _, _) = made("better", &[right(), low_pillar(), right()]);
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "keeps every law"), ("look", "breaks: gates are real"), ("laws", "keeps every law")]);
    assert!(made.kept.is_some() && made.looked);
}

fn write_test_sketch(path: &Path) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let (w, h) = (128usize, 128usize);
    let mut rgba = vec![[140u8, 90, 43, 255]; w * h]; // all brown land
    // Orange dot at (40, 30) and purple dot at (20, 20), matching AREA.
    let paint = |rgba: &mut [[u8; 4]], cx: f32, cz: f32, r: f32, c: [u8; 4]| {
        for j in 0..h {
            for i in 0..w {
                let x = (i as f32 + 0.5) / w as f32 * 64.0;
                let z = (j as f32 + 0.5) / h as f32 * 64.0;
                if (x - cx).powi(2) + (z - cz).powi(2) <= r * r {
                    rgba[j * w + i] = c;
                }
            }
        }
    };
    paint(&mut rgba, 40.0, 30.0, 2.0, [245, 140, 20, 255]);
    paint(&mut rgba, 20.0, 20.0, 2.5, [160, 70, 215, 255]);
    let mut f = std::fs::File::create(path).unwrap();
    let mut enc = png::Encoder::new(&mut f, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(rgba.as_flattened()).unwrap();
}

#[test]
fn with_a_sketch_claude_gets_its_picture_and_shapes_and_is_held_to_its_laws() {
    let sketch_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/plateau-make-tests/test-sketch.png");
    write_test_sketch(&sketch_path);
    // First answer moves the pillar away from the purple mark (20, 20) -> breaks sketch places.
    let moved_pillar = right().replace("point(20.0, PILLAR, 20.0)", "point(50.0, PILLAR, 20.0)").replace("point(20.0, PILLAR * 0.5, 20.0)", "point(50.0, PILLAR * 0.5, 20.0)");
    let (made, writer, dir) = made_with_sketch("sketched", Some(&sketch_path), &[moved_pillar, right(), right()]);
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "breaks: sketch places"), ("laws", "keeps every law"), ("look", "keeps every law")]);
    // Round 1 got the sketch path, shapes in metres, and Read permission on the folder.
    assert!(writer.asked[0].0.contains("sketch.png") && writer.asked[0].0.contains("orange (tiny only)"), "{}", writer.asked[0].0);
    assert_eq!(writer.asked[0].1.as_deref(), Some(dir.as_path()));
    // Round 2 got the sketch places failure in its own words.
    assert!(writer.asked[1].0.contains("**sketch places**") && writer.asked[1].0.contains("purple mark"), "{}", writer.asked[1].0);
    // Round 3 (look) compares against the sketch.
    assert!(writer.asked[2].0.contains("sketch.png"), "{}", writer.asked[2].0);
    assert!(dir.join("sketch.png").exists() && dir.join("pictures/sketch.png").exists());
}

#[test]
fn a_version_that_kept_every_law_is_kept_if_the_look_fails() {
    // Only one answer scripted: the look round gets an error from the stand-in, and round 1's
    // working version is still returned and kept on disk.
    let (made, _, dir) = made("look-error", &[right()]);
    assert_eq!(made.rounds.len(), 1);
    assert!(!made.looked);
    assert_eq!(made.kept.as_deref(), source_of(&right()).as_deref());
    assert!(dir.join("area.rs").exists() && dir.join("area.wasm").exists());
}

const GOOD_ELEMENT: &str = r#"use plateau_sdk::*;

struct StoneCairn;

impl Element for StoneCairn {
    fn new(_: u64) -> Self { StoneCairn }
    fn name(&self) -> &str { "Stone Cairn" }
    fn sdf_bounds(&self) -> Option<ElementBounds> {
        Some(ElementBounds::new(point(-1.5, -1.0, -1.5), point(1.5, 2.0, 1.5), 0.2))
    }
    fn sample(&self, p: Point, _view: &View) -> Answer {
        let a = sphere(p - point(0.0, 0.35, 0.0), 0.65);
        let b = sphere(p - point(0.0, 1.05, 0.0), 0.42);
        Answer::new(smooth_union(a, b, 0.15), Material::Stone)
    }
}

element!(StoneCairn);
"#;

#[test]
fn make_element_catches_clipped_bounds_and_keeps_fixed_element() {
    let clipped = GOOD_ELEMENT.replace("point(-1.5, -1.0, -1.5), point(1.5, 2.0, 1.5)", "point(-0.3, -0.3, -0.3), point(0.3, 0.5, 0.3)");
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/plateau-make-tests/element-cairn");
    std::fs::remove_dir_all(&dir).ok();
    let mut writer = StandIn {
        answers: vec![format!("```rust\n{clipped}```"), format!("```rust\n{GOOD_ELEMENT}```")].into(),
        asked: Vec::new(),
    };
    let made = make_element(&mut writer, builder(), "a stacked stone cairn", &dir, QUICK, &mut |_| {}).unwrap();
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "breaks: element bounds containment"), ("laws", "keeps every law")]);
    assert!(made.kept.is_some());
    assert!(dir.join("element.rs").exists() && dir.join("element.wasm").exists());

    // Also verify all four carried showcase elements pass `judge_element`.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for name in ["tree", "bridge", "lantern", "hill"] {
        let bytes = std::fs::read(root.join(format!("elements/modules/{name}.wasm"))).unwrap();
        let judged = judge_element(&bytes, QUICK);
        assert!(judged.kept(), "{name} broke element laws: {:?}", judged.broken);
    }
}
