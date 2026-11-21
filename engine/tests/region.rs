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
