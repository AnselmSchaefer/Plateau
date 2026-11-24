//! An area's module as the engine takes it: checked on load, refused saying why, its answers read
//! out of the canonical ABI with every number checked, its limits kept. Through modules written by
//! hand (`made_wrong`).

mod made_wrong;

use made_wrong::{GROUND, area, module};
use plateau_engine::check::check;
use plateau_engine::{Limits, Material, Region, Wasm, laws};

fn refused(wat: &str) -> Vec<String> {
    check(&wat::parse_str(wat).unwrap()).expect_err("the module should be refused")
}

#[test]
fn a_module_that_keeps_the_world_is_taken_and_answered() {
    let mut wasm = Wasm::new(&module(GROUND), 7, Limits::default()).unwrap();
    let mut out = Vec::new();
    wasm.sample(&[[1.0, 2.5, 3.0], [4.0, -1.0, 6.0]], 1.7, &mut out).unwrap();
    assert_eq!(out.iter().map(|a| (a.distance, a.material)).collect::<Vec<_>>(), [(2.5, Material::Stone), (-1.0, Material::Stone)]);
    let described = wasm.describe().unwrap();
    assert_eq!((described.name.as_str(), described.places.len(), described.borders.west.ground.len()), ("x", 0, 65));
    // Asked again, it answers again (this module never frees what it takes: three times fits).
    for _ in 0..3 {
        wasm.sample(&[[0.0, 1.0, 0.0]; 1000], 0.017, &mut out).unwrap();
    }
    assert_eq!(out.len(), 1000);
}

#[test]
fn a_module_that_asks_for_anything_but_the_engine_s_host_is_refused_saying_why() {
    let wat = area(GROUND).replace(
        r#"(func $seed (result i64)))"#,
        r#"(func $seed (result i64)))
  (import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
    );
    assert_eq!(refused(&wat), ["it asks for fd_write of wasi_snapshot_preview1, which the engine does not give"]);
    let wat = area(GROUND).replace(r#""seed" (func $seed (result i64))"#, r#""seed" (func $seed (result i32))"#);
    assert_eq!(refused(&wat), ["it asks the engine for seed in another way than the engine answers"]);
}

#[test]
fn a_module_missing_what_the_engine_calls_or_giving_more_is_refused() {
    let wat = area(GROUND).replace(r#"(func (export "describe")"#, r#"(func (export "describe_all")"#);
    assert_eq!(refused(&wat), ["it gives describe_all, which the engine does not use", "it does not give describe, which the engine calls"]);
    let wat = area(GROUND).replace(r#"(func (export "cabi_post_sample") (param i32))"#, r#"(func (export "cabi_post_sample") (param i64))"#);
    assert_eq!(refused(&wat), ["its cabi_post_sample is not what the engine calls"]);
}

#[test]
fn a_module_that_may_take_too_much_memory_is_refused() {
    let unbounded = area(GROUND).replace(r#"(memory (export "memory") 2 2)"#, r#"(memory (export "memory") 2)"#);
    assert_eq!(refused(&unbounded), ["it does not say how much memory it may take"]);
    let large = area(GROUND).replace(r#"(memory (export "memory") 2 2)"#, r#"(memory (export "memory") 2 2048)"#);
    assert_eq!(refused(&large), ["it may take 128 MB of memory, more than the 64 MB an area may"]);
}

#[test]
fn a_module_using_webassembly_beyond_the_fixed_set_is_refused() {
    let simd = area(GROUND).replace(
        r#"(func (export "cabi_post_sample") (param i32))"#,
        r#"(func (export "cabi_post_sample") (param i32) (drop (v128.const i32x4 0 0 0 0)))"#,
    );
    let why = refused(&simd);
    assert!(why.len() == 1 && why[0].starts_with("it is not the WebAssembly an area may use") && why[0].contains("SIMD"), "{why:?}");
    let shared = area(GROUND).replace(r#"(memory (export "memory") 2 2)"#, r#"(memory (export "memory") 2 2 shared)"#);
    let why = refused(&shared);
    assert!(why.len() == 1 && why[0].contains("threads"), "{why:?}");
}

#[test]
fn an_answer_the_engine_cannot_read_is_an_error_not_a_world() {
    let mut out = Vec::new();
    let unknown = "(f32.store (local.get $at) (f32.const 1)) (i32.store8 offset=4 (local.get $at) (i32.const 200))";
    let mut wasm = Wasm::new(&module(unknown), 7, Limits::default()).unwrap();
    let e = wasm.sample(&[[0.0; 3]], 1.7, &mut out).unwrap_err();
    assert_eq!(e.to_string(), "the area answered material 200, which is not one of the engine's");
    let short = area(GROUND).replace("(i32.store (i32.const 20) (local.get $count))", "(i32.store (i32.const 20) (i32.const 1))");
    let mut wasm = Wasm::new(&wat::parse_str(short).unwrap(), 7, Limits::default()).unwrap();
    let e = wasm.sample(&[[0.0; 3]; 3], 1.7, &mut out).unwrap_err();
    assert_eq!(e.to_string(), "the area gave 1 answers for 3 points");
    let outside = area(GROUND).replace("(i32.store (i32.const 16) (local.get $answers))", "(i32.store (i32.const 16) (i32.const 131068))");
    let mut wasm = Wasm::new(&wat::parse_str(outside).unwrap(), 7, Limits::default()).unwrap();
    let e = wasm.sample(&[[0.0; 3]; 3], 1.7, &mut out).unwrap_err();
    assert_eq!(e.to_string(), "the area answered outside its memory");
}

#[test]
fn an_area_is_asked_only_for_the_sizes_there_are() {
    let mut wasm = Wasm::new(&module(GROUND), 7, Limits::default()).unwrap();
    let mut out = Vec::new();
    for size in [0.0, -1.0, 100.0, f32::NAN] {
        assert!(wasm.sample(&[[0.0; 3]], size, &mut out).is_err(), "{size}");
    }
}

#[test]
fn a_module_that_runs_out_of_fuel_is_stopped() {
    let forever = "(loop $ever (br $ever))";
    let mut wasm = Wasm::new(&module(forever), 7, Limits::default()).unwrap();
    let e = wasm.sample(&[[0.0; 3]], 1.7, &mut Vec::new()).unwrap_err();
    assert!(format!("{e:?}").contains("fuel"), "{e:?}");
}

/// An answer made from a NaN made from the point (so that nothing can fold it ahead): its sign bit.
/// x86's own NaN has it set, the one NaN has not.
const LOOKS_AT_NANS: &str = r#"
    (f32.store (local.get $at)
      (f32.convert_i32_u (i32.shr_u (i32.reinterpret_f32
        (f32.div (f32.mul (f32.load (local.get $p)) (f32.const 0)) (f32.mul (f32.load (local.get $p)) (f32.const 0))))
        (i32.const 31))))
    (i32.store8 offset=4 (local.get $at) (i32.const 4))"#;

#[cfg(target_arch = "x86_64")]
#[test]
fn an_area_that_looks_at_a_nans_bits_breaks_the_law() {
    let verdict = laws::nans_unseen(&module(LOOKS_AT_NANS), 7, &plateau_engine::points(64, 4.0, 1)).unwrap();
    let message = verdict.expect_err("the law should fail");
    assert!(message.contains("looks at a NaN's bits") && message.contains("tiny"), "{message}");
    assert_eq!(laws::nans_unseen(&module(GROUND), 7, &plateau_engine::points(64, 4.0, 1)).unwrap(), Ok(()));
}

#[test]
fn scene_tick_interact_and_solid_collision_work_and_are_capped() {
    use plateau_engine::body::Body;
    use plateau_engine::scene::{Combined, SceneWorld};
    use plateau_engine::{Command, Instance, Light, MeshData, PlayerState, Scene, Vertex};

    // Old modules without `scene`/`tick`/`interact` still load cleanly and return empty defaults.
    let mut old = Wasm::new(&module(GROUND), 7, Limits::default()).unwrap();
    assert_eq!(old.scene().unwrap(), Scene::default());
    assert_eq!(old.tick(0.016, &PlayerState::default()).unwrap(), Vec::new());
    assert_eq!(old.interact(&PlayerState::default()).unwrap(), Vec::new());

    // Build a real WASM module from Rust using plateau_sdk's MeshBuilder, SceneBuilder, tick & interact!
    let src = r#"
        use plateau_sdk::*;
        struct Plugin { lit: bool }
        impl Area for Plugin {
            fn new(_: u64) -> Self { Plugin { lit: false } }
            fn describe(&self) -> Description {
                Description { name: "Bridge".into(), places: vec![], borders: Borders::of_ground(|_, _| 0.0) }
            }
            fn sample(&self, p: Point, _: &View) -> Answer {
                Answer::new(p.y, Material::Grass)
            }
            fn scene(&self) -> Scene {
                let mut m = MeshBuilder::new();
                m.cuboid(point(0.0, 0.0, 0.0), point(2.0, 0.1, 1.0), Material::Wood);
                let mut sc = SceneBuilder::new();
                let deck = sc.add_mesh(m.build());
                sc.spawn(1, deck, point(32.0, 2.0, 32.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true);
                sc.light(1, point(32.0, 3.0, 32.0), [1.0, 0.8, 0.5], 50.0, 8.0);
                sc.build()
            }
            fn tick(&mut self, dt: f32, p: &PlayerState) -> Vec<Command> {
                let lift = 2.0 + sin(p.time + dt) * 0.5;
                vec![Command::SetTransform(instance(1, 0, point(32.0, lift, 32.0), Quat::IDENTITY, point(1.0, 1.0, 1.0), true))]
            }
            fn interact(&mut self, p: &PlayerState) -> Vec<Command> {
                self.lit = !self.lit;
                let intensity = if self.lit { 200.0 } else { 0.0 };
                vec![
                    Command::SetLight(light(1, p.eye, [1.0, 0.9, 0.6], intensity, 10.0)),
                    Command::ApplyImpulse(point(0.0, 100.0, 0.0)), // engine clamps to MAX_IMPULSE!
                    Command::Say(if self.lit { "lamp on".into() } else { "lamp off".into() }),
                ]
            }
        }
        area!(Plugin);
    "#;
    let tmp = std::env::temp_dir().join(format!("plateau-scene-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).unwrap();
    let rs = tmp.join("area.rs");
    let rlib = tmp.join("libplateau_sdk.rlib");
    let wasm_path = tmp.join("area.wasm");
    std::fs::write(&rs, src).unwrap();
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let fixed = ["--edition", "2024", "--target", "wasm32-unknown-unknown", "-O", "-C", "codegen-units=1", "-C", "panic=abort", "-C", "strip=debuginfo"];
    assert!(std::process::Command::new("rustc").args(fixed).args(["--crate-type", "rlib", "--crate-name", "plateau_sdk"]).arg(root.join("sdk/src/lib.rs")).arg("-o").arg(&rlib).status().unwrap().success());
    assert!(std::process::Command::new("rustc").args(["--crate-name", "area"]).args(fixed).args(["--crate-type", "cdylib", "-C", "link-arg=--max-memory=67108864", "--extern"]).arg(format!("plateau_sdk={}", rlib.display())).arg(&rs).arg("-o").arg(&wasm_path).status().unwrap().success());
    let bytes = std::fs::read(&wasm_path).unwrap();
    let _ = std::fs::remove_dir_all(&tmp);

    let mut wasm = Wasm::new(&bytes, 7, Limits::default()).unwrap();
    let sc = wasm.scene().unwrap();
    assert_eq!(sc.meshes.len(), 1);
    assert_eq!(sc.instances.len(), 1);
    assert_eq!(sc.lights.len(), 1);
    assert_eq!(sc.instances[0].id, 1);

    // Body standing above the solid procedural bridge deck lands ON the deck at y ≈ 2.1, not ground y = 0!
    let mut world = SceneWorld::from_scene(&sc);
    {
        let mut combined = Combined { region: &mut wasm, world: &world };
        let body = Body::standing(&mut combined, [32.0, 4.0, 32.0], 1.7).unwrap();
        assert!((body.feet[1] - 2.1).abs() < 0.05, "expected to stand on solid bridge at y=2.1, got {}", body.feet[1]);
    }

    // Crosshair raycast hits dynamic instance 1.
    let aim_hit = world.raycast_dynamic([32.0, 3.5, 32.0], [0.0, -1.0, 0.0], 5.0);
    assert_eq!(aim_hit.map(|(id, _)| id), Some(1));

    // Tick moves the deck; interact toggles the light, clamps impulse to MAX_IMPULSE, and says "lamp on".
    let ps = PlayerState { eye: [32.0, 3.5, 32.0], time: 1.0, ..PlayerState::default() };
    let tcmds = wasm.tick(0.016, &ps).unwrap();
    assert!(matches!(tcmds.as_slice(), [Command::SetTransform(inst)] if inst.id == 1 && inst.pos[1] > 2.2));
    world.apply(&tcmds);
    let icmds = wasm.interact(&ps).unwrap();
    assert_eq!(icmds.len(), 3);
    let applied = world.apply(&icmds);
    assert!((applied.impulse[1] - plateau_engine::wasm::MAX_IMPULSE).abs() < 1e-4, "impulse was not clamped: {:?}", applied.impulse);
    assert_eq!(applied.said.as_deref(), Some("lamp on"));
    assert_eq!(world.lights.get(&(0, 1)).map(|l| l.intensity), Some(200.0));
    // Interact again turns the light off (mutable WASM state persisted!).
    let icmds2 = wasm.interact(&ps).unwrap();
    let applied2 = world.apply(&icmds2);
    assert_eq!(applied2.said.as_deref(), Some("lamp off"));
    assert!(!world.lights.contains_key(&(0, 1)));

    // Suppress unused imports check on Vertex/MeshData/Instance/Light types.
    let _ = (Vertex { pos: [0.0; 3], normal: [0.0, 1.0, 0.0], material: Material::Stone, tint: [255; 3] }, MeshData::default(), Instance { id: 0, mesh: 0, pos: [0.0; 3], rot: [0.0, 0.0, 0.0, 1.0], scale: [1.0; 3], solid: false }, Light { id: 0, pos: [0.0; 3], color: [1.0; 3], intensity: 0.0, radius: 1.0 });
}
