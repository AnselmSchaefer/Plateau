//! The making loop (POC_PLAN.md, step 5) with a stand-in for Claude: it never calls Claude
//! (POC_PLAN.md, "Tests"). The stand-in answers with what each test scripts, and the tests look at
//! what it was asked and what was kept. The area answered is small, so it is judged quickly: a
//! stone block with a room for the tiny behind a slot, and a pillar only a giant steps onto.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use plateau_engine::atlas::Side;
use plateau_engine::edit::{Edit, EditLog, Edited, Shape, surface_normal};
use plateau_engine::site::{SiteContext, SiteProbe};
use plateau_engine::{Material, SIZES, body};
use plateau_make::{
    Builder, FitLaw, Made, Neighbor, Reply, SEED, Settings, Site, Writer, element_guide_at, judge_element, judge_element_at, load_neighbors, make, make_element,
    make_element_at, make_with_neighbors, source_of,
};

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
        let ground_mat = strata(-p.y, &[(Material::Grass, 0.08), (Material::Soil, 0.6), (Material::Stone, 0.0)]);
        let ground = Answer::new(p.y, ground_mat);
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

    // Also verify all seven carried showcase elements pass `judge_element`.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    for name in ["tree", "bridge", "lantern", "hill", "boat", "pillar", "hobbit-door"] {
        let bytes = std::fs::read(root.join(format!("elements/modules/{name}.wasm"))).unwrap();
        let judged = judge_element(&bytes, QUICK);
        assert!(judged.kept(), "{name} broke element laws: {:?}", judged.broken);
    }
}

#[test]
fn make_element_at_cavity_holds_element_to_cavity_fit_and_shows_pictures() {
    // Carve a round doorway into a soil block in a flat world and measure its SiteContext.
    let person = SIZES[1].1;
    let mut edits = vec![Edit::add(Shape::Box { half: [3.0, 2.5, 3.0], round: 0.1 }, [32.0, 1.5, 46.0], 0.0, Material::Soil, person)];
    let r = 0.75 * person;
    for k in 0..8 {
        edits.push(Edit::carve(Shape::Sphere { radius: r }, [32.0, 1.15, 49.0 - 0.45 * r * k as f32], 0.35 * r, None, person));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let garden_bytes = std::fs::read(root.join("areas/modules/garden.wasm")).unwrap();
    let garden = plateau_engine::Wasm::new(&garden_bytes, SEED, plateau_engine::Limits::default()).unwrap();
    let mut world = Edited::new(garden, std::sync::Arc::new(EditLog::from_edits(edits).unwrap()));

    let from = [32.0f32, 1.6, 53.0];
    let hit = body::look(&mut world, from.map(|c| c as f64), [0.0, 0.0, -1.0], person, person * 45.0).unwrap().unwrap();
    let normal = surface_normal(&mut world, hit, person).unwrap();
    let context = SiteContext::measure(&mut world, &SiteProbe { target: hit.map(|c| c as f32), normal, from: Some(from), size: person }).unwrap();
    assert!(context.cavity.is_some(), "round doorway measured");

    // Verify element_guide_at embeds both the fitting section and the hobbit-door example.
    let guide_text = element_guide_at(&root).unwrap();
    assert!(guide_text.contains("### Fitting an opening") && guide_text.contains("struct HobbitDoor"));

    // The carried door makes itself for the site it is given (`host.params`, the site's measures):
    // as it is, it fits. Round 1: its frame made to stop short of the outline -> leaves a gap
    // round the opening's rim -> breaks `cavity fit`.
    let base_door = std::fs::read_to_string(root.join("elements/hobbit-door/element.rs")).unwrap();
    assert!(base_door.contains("const BEYOND: f32 = 0.08;"));
    let fitted_door = base_door.clone();
    let too_small_door = base_door.replace("const BEYOND: f32 = 0.08;", "const BEYOND: f32 = -0.25;");

    let site = Site { context, world: None, law: FitLaw::Auto };
    let dir = root.join("target/plateau-make-tests/element-cavity-fit");
    std::fs::remove_dir_all(&dir).ok();
    let mut writer = StandIn {
        answers: vec![
            format!("```rust\n{too_small_door}```"),
            format!("```rust\n{fitted_door}```"),
            format!("```rust\n{fitted_door}```"),
        ]
        .into(),
        asked: Vec::new(),
    };
    let made = make_element_at(&mut writer, builder(), "a round green hobbit door", &site, &dir, QUICK, &mut |_| {}).unwrap();
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "breaks: cavity fit"), ("laws", "keeps every law"), ("look", "keeps every law")]);
    assert!(made.kept.is_some() && made.looked);
    // Round 1 got the measured opening contour and bare site pictures.
    assert!(writer.asked[0].0.contains("const OPENING: [(f32, f32); 48]") && writer.asked[0].0.contains("site-ahead.png"), "{}", writer.asked[0].0);
    assert_eq!(writer.asked[0].1.as_deref(), Some(dir.join("site").as_path()));
    // Round 2 got the exact `cavity fit` gap feedback.
    assert!(writer.asked[1].0.contains("**cavity fit**") && writer.asked[1].0.contains("a gap of") && writer.asked[1].0.contains("o'clock"), "{}", writer.asked[1].0);
    // Round 3 (`look`) got both shut (`fitted-ahead.png`) and open (`pressed-ahead.png`) renders.
    assert!(writer.asked[2].0.contains("fitted-ahead.png") && writer.asked[2].0.contains("pressed-ahead.png"), "{}", writer.asked[2].0);
    assert_eq!(writer.asked[2].1.as_deref(), Some(dir.join("pictures").as_path()));
    for f in ["element.rs", "element.wasm", "element.site", "site/site-ahead.png", "pictures/fitted-ahead.png", "pictures/pressed-ahead.png"] {
        assert!(dir.join(f).exists(), "missing expected output {f}");
    }
}

#[test]
fn make_element_at_open_ground_catches_floating_and_keeps_grounded_element() {
    let person = SIZES[1].1;
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let garden_bytes = std::fs::read(root.join("areas/modules/garden.wasm")).unwrap();
    let mut garden = plateau_engine::Wasm::new(&garden_bytes, SEED, plateau_engine::Limits::default()).unwrap();
    let from = [46.0f32, 1.6, 48.0];
    let hit = body::look(&mut garden, from.map(|c| c as f64), [0.0, -0.6, -0.8], person, person * 45.0).unwrap().unwrap();
    let normal = surface_normal(&mut garden, hit, person).unwrap();
    let context = SiteContext::measure(&mut garden, &SiteProbe { target: hit.map(|c| c as f32), normal, from: Some(from), size: person }).unwrap();
    assert!(context.cavity.is_none());

    let floating = r#"use plateau_sdk::*;
struct FloatingBench;
impl Element for FloatingBench {
    fn new(_: u64) -> Self { FloatingBench }
    fn name(&self) -> &str { "Floating Bench" }
    fn scene(&self) -> Scene {
        let mut m = MeshBuilder::new();
        m.cuboid(point(0.0, 1.1, 0.0), point(0.8, 0.2, 0.4), Material::Wood);
        let mut sc = SceneBuilder::new();
        let mesh = sc.add_mesh(m.build());
        sc.place(mesh, point(0.0, 0.0, 0.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
        sc.build()
    }
}
element!(FloatingBench);
"#;
    let grounded = floating.replace("point(0.0, 1.1, 0.0), point(0.8, 0.2, 0.4)", "point(0.0, 0.25, 0.0), point(0.8, 0.40, 0.4)");

    let site = Site { context, world: None, law: FitLaw::Auto };
    let dir = root.join("target/plateau-make-tests/element-ground-fit");
    std::fs::remove_dir_all(&dir).ok();
    let mut writer = StandIn {
        answers: vec![format!("```rust\n{floating}```"), format!("```rust\n{grounded}```"), format!("```rust\n{grounded}```")].into(),
        asked: Vec::new(),
    };
    let made = make_element_at(&mut writer, builder(), "a wooden garden bench", &site, &dir, QUICK, &mut |_| {}).unwrap();
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "breaks: ground fit"), ("laws", "keeps every law"), ("look", "keeps every law")]);
    assert!(writer.asked[1].0.contains("**ground fit**") && writer.asked[1].0.contains("floats"), "{}", writer.asked[1].0);
    assert!(dir.join("site/site-plan.png").exists() && dir.join("pictures/fitted-plan.png").exists());
}

#[test]
fn with_a_neighbour_claude_gets_its_border_and_is_held_to_borders_match() {
    // Build AREA as a neighbour module on disk, then generate an area to its east (--beside west:...).
    let nb_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/plateau-make-tests/neighbor-base");
    std::fs::remove_dir_all(&nb_dir).ok();
    let nb_wasm_bytes = builder().build(AREA, &nb_dir).unwrap();
    let nb_wasm = nb_dir.join("area.wasm");
    std::fs::write(&nb_wasm, &nb_wasm_bytes).unwrap();

    let neighbor = Neighbor::load(Side::West, nb_wasm, SEED).unwrap();
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/plateau-make-tests/beside-west");
    std::fs::remove_dir_all(&dir).ok();

    // Round 1: ground lifted by 0.6 m -> breaks `borders match (west)`. Round 2: matches `AREA` (y = 0) -> keeps every law.
    let bad_border = right()
        .replace(
            "let ground_mat = strata(-p.y, &[(Material::Grass, 0.08), (Material::Soil, 0.6), (Material::Stone, 0.0)]);\n        let ground = Answer::new(p.y, ground_mat);",
            "let ground_mat = strata(0.6 - p.y, &[(Material::Grass, 0.08), (Material::Soil, 0.6), (Material::Stone, 0.0)]);\n        let ground = Answer::new(p.y - 0.6, ground_mat);",
        )
        .replace("Borders::of_ground(|_, _| 0.0)", "Borders::of_ground(|_, _| 0.6)")
        .replace("point(40.0, 0.0, 30.0)", "point(40.0, 0.6, 30.0)")
        .replace("point(40.0, 0.2, 30.0)", "point(40.0, 0.8, 30.0)")
        .replace("point(40.0, 0.0295, 30.0)", "point(40.0, 0.6295, 30.0)")
        .replace("point(39.85, 0.0145, 30.0)", "point(39.85, 0.6145, 30.0)")
        .replace("point(20.0, PILLAR, 20.0)", "point(20.0, PILLAR + 0.6, 20.0)")
        .replace("point(20.0, PILLAR * 0.5, 20.0)", "point(20.0, PILLAR * 0.5 + 0.6, 20.0)");

    let mut writer = StandIn { answers: vec![bad_border, right(), right()].into(), asked: Vec::new() };
    let made = make_with_neighbors(&mut writer, builder(), "two blocks beside west", None, &[neighbor], &dir, QUICK, &mut |_| {}).unwrap();
    let came: Vec<_> = made.rounds.iter().map(|r| (r.asked, r.came.as_str())).collect();
    assert_eq!(came, [("write", "breaks: borders match (west)"), ("laws", "keeps every law"), ("look", "keeps every law")]);

    // Round 1 prompt included the neighbour's WEST_GROUND table and border instructions.
    assert!(writer.asked[0].0.contains("const WEST_GROUND: [f32; 65]"), "{}", writer.asked[0].0);
    // Round 2 got the `borders match (west)` failure message.
    assert!(writer.asked[1].0.contains("**borders match (west)**") && writer.asked[1].0.contains("apart"), "{}", writer.asked[1].0);
    // Look round included stitched seam pictures and `neighbors.txt` round-trips cleanly.
    assert!(writer.asked[2].0.contains("seam-west.png") && writer.asked[2].0.contains("seam-view-west.png"), "{}", writer.asked[2].0);
    assert!(dir.join("pictures/seam-west.png").exists() && dir.join("pictures/seam-view-west.png").exists());
    let reloaded = load_neighbors(&dir).unwrap();
    assert_eq!(reloaded.len(), 1);
    assert_eq!(reloaded[0].side, Side::West);
}

#[test]
fn an_element_that_reads_its_params_is_held_to_openings_like_its_site_s_too() {
    // The round doorway of the test above, measured.
    let person = SIZES[1].1;
    let mut edits = vec![Edit::add(Shape::Box { half: [3.0, 2.5, 3.0], round: 0.1 }, [32.0, 1.5, 46.0], 0.0, Material::Soil, person)];
    let r = 0.75 * person;
    for k in 0..8 {
        edits.push(Edit::carve(Shape::Sphere { radius: r }, [32.0, 1.15, 49.0 - 0.45 * r * k as f32], 0.35 * r, None, person));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..");
    let garden_bytes = std::fs::read(root.join("areas/modules/garden.wasm")).unwrap();
    let garden = plateau_engine::Wasm::new(&garden_bytes, SEED, plateau_engine::Limits::default()).unwrap();
    let mut world = Edited::new(garden, std::sync::Arc::new(EditLog::from_edits(edits).unwrap()));
    let from = [32.0f32, 1.6, 53.0];
    let hit = body::look(&mut world, from.map(|c| c as f64), [0.0, 0.0, -1.0], person, person * 45.0).unwrap().unwrap();
    let normal = surface_normal(&mut world, hit, person).unwrap();
    let context = SiteContext::measure(&mut world, &SiteProbe { target: hit.map(|c| c as f32), normal, from: Some(from), size: person }).unwrap();
    let site = Site { context, world: None, law: FitLaw::Auto };

    // The carried door makes itself from every radius it is given: it fits the doorway and the
    // four beside it, and nothing is noted of its fitting (only of its wood in soil).
    let door = std::fs::read_to_string(root.join("elements/hobbit-door/element.rs")).unwrap();
    let work = root.join("target/plateau-make-tests/param-range");
    let module = builder().build_element(&door, &work.join("door")).unwrap();
    let judged = judge_element_at(&module, &site, QUICK);
    assert!(judged.kept(), "{:?}", judged.broken);
    assert!(!judged.notes.iter().any(|n| n.contains("params")), "{:?}", judged.notes);
    assert!(judged.notes.iter().any(|n| n.contains("wood")), "{:?}", judged.notes);

    // One that reads them but makes itself round, as wide as the doorway is to its right: it fits
    // the round doorway, and none of the four beside it but the wider one.
    let lazy = door.replace("*r = params[P_RADII + k];", "*r = params[P_RADII];");
    assert_ne!(lazy, door);
    let module = builder().build_element(&lazy, &work.join("lazy")).unwrap();
    let judged = judge_element_at(&module, &site, QUICK);
    let others: Vec<&String> = judged.broken.iter().filter(|(law, _)| law == "cavity fit, other openings").map(|(_, why)| why).collect();
    assert!(judged.broken.iter().all(|(law, _)| law == "cavity fit, other openings"), "fits its own: {:?}", judged.broken);
    assert!(others.iter().any(|w| w.contains("15% higher")) && others.iter().any(|w| w.contains("15% narrower")), "{others:?}");

    // One that reads none: it fits its own, and is noted to fit no other.
    let fixed = std::fs::read(root.join("made/elements/oak-hollow-door/element.wasm")).unwrap();
    assert!(!plateau_engine::check::reads_params(&fixed));
    assert!(plateau_engine::check::reads_params(&std::fs::read(root.join("elements/modules/hobbit-door.wasm")).unwrap()));
}
