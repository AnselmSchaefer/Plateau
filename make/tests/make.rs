//! The making loop (POC_PLAN.md, step 5) with a stand-in for Claude: it never calls Claude
//! (POC_PLAN.md, "Tests"). The stand-in answers with what each test scripts, and the tests look at
//! what it was asked and what was kept. The area answered is small, so it is judged quickly: a
//! stone block with a room for the tiny behind a slot, and a pillar only a giant steps onto.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use plateau_make::{Builder, Made, Reply, Settings, Writer, make, source_of};

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
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/plateau-make-tests").join(test);
    std::fs::remove_dir_all(&dir).ok();
    let mut writer = StandIn { answers: answers.iter().cloned().collect(), asked: Vec::new() };
    let made = make(&mut writer, builder(), "two blocks", &dir, QUICK, &mut |_| {}).unwrap();
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
