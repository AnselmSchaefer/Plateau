//! Phase I (WORLD_PLAN.md): `plateau:region@0.2.0` gives each placement its params (`host.params`),
//! checked and saved with it; `0.1.0` modules keep working, given none.

mod made_wrong;

use plateau_engine::check;
use plateau_engine::compose::PlacedElement;
use plateau_engine::edit::WorldSave;
use plateau_engine::{Limits, Region, Wasm};

/// A module of `plateau:region@0.2.0` that answers every point with the sum of its params plus a
/// hundred times how many there are, in stone: what it was given, read back through `sample`.
fn echo() -> Vec<u8> {
    let wat = r#"(module
  (import "plateau:region/host@0.2.0" "seed" (func $seed (result i64)))
  (import "plateau:region/host@0.2.0" "params" (func $params (param i32)))
  (memory (export "memory") 2 2)
  (global $next (mut i32) (i32.const 8192))
  (func $alloc (export "cabi_realloc") (param i32 i32 i32) (param $n i32) (result i32) (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.and (i32.add (i32.add (global.get $next) (local.get $n)) (i32.const 7)) (i32.const -8)))
    (local.get $p))
  (func (export "sample") (param $points i32) (param $count i32) (param $size f32) (result i32)
    (local $i i32) (local $at i32) (local $answers i32) (local $ptr i32) (local $len i32) (local $sum f32)
    (call $params (i32.const 32))
    (local.set $ptr (i32.load (i32.const 32)))
    (local.set $len (i32.load (i32.const 36)))
    (local.set $sum (f32.mul (f32.convert_i32_u (local.get $len)) (f32.const 100)))
    (block $done
      (loop $each
        (br_if $done (i32.ge_u (local.get $i) (local.get $len)))
        (local.set $sum (f32.add (local.get $sum) (f32.load (i32.add (local.get $ptr) (i32.mul (local.get $i) (i32.const 4))))))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $each)))
    (local.set $i (i32.const 0))
    (local.set $answers (call $alloc (i32.const 0) (i32.const 0) (i32.const 4) (i32.mul (local.get $count) (i32.const 8))))
    (block $done
      (loop $each
        (br_if $done (i32.ge_u (local.get $i) (local.get $count)))
        (local.set $at (i32.add (local.get $answers) (i32.mul (local.get $i) (i32.const 8))))
        (f32.store (local.get $at) (local.get $sum))
        (i32.store8 offset=4 (local.get $at) (i32.const 4))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $each)))
    (i32.store (i32.const 16) (local.get $answers))
    (i32.store (i32.const 20) (local.get $count))
    (i32.const 16))
  (func (export "cabi_post_sample") (param i32))
  (func (export "describe") (result i32) (i32.const 0))
  (func (export "cabi_post_describe") (param i32)))"#;
    wat::parse_str(wat).unwrap()
}

fn answer(wasm: &mut Wasm) -> f32 {
    let mut out = Vec::new();
    wasm.sample(&[[1.0, 2.0, 3.0]], 1.7, &mut out).unwrap();
    out[0].distance
}

#[test]
fn a_placement_s_params_reach_its_module() {
    let bytes = echo();
    assert_eq!(check::check(&bytes), Ok(()));
    let mut given = Wasm::with_params(&bytes, 7, &[1.5, 2.0, -0.25], Limits::default()).unwrap();
    assert_eq!(answer(&mut given), 303.25);
    // Asked again, the same: each call is given its own list.
    assert_eq!(answer(&mut given), 303.25);
    let mut none = Wasm::new(&bytes, 7, Limits::default()).unwrap();
    assert_eq!(answer(&mut none), 0.0);
}

#[test]
fn params_beyond_the_limits_are_refused() {
    let bytes = echo();
    let many = vec![1.0; plateau_engine::wasm::MOST_PARAMS + 1];
    let e = Wasm::with_params(&bytes, 7, &many, Limits::default()).err().unwrap().to_string();
    assert!(e.contains("more than the 64"), "{e}");
    let e = Wasm::with_params(&bytes, 7, &[1.0, f32::NAN], Limits::default()).err().unwrap().to_string();
    assert!(e.contains("param 1"), "{e}");
    let most = vec![0.5; plateau_engine::wasm::MOST_PARAMS];
    assert_eq!(answer(&mut Wasm::with_params(&bytes, 7, &most, Limits::default()).unwrap()), 6432.0);
}

#[test]
fn a_module_of_the_first_interface_still_loads_and_may_not_ask_for_params() {
    // `made_wrong::area` imports `seed` from `host@0.1.0`, as every module made before 0.2.0 does.
    let old = made_wrong::module(made_wrong::GROUND);
    assert_eq!(check::check(&old), Ok(()));
    let mut w = Wasm::with_params(&old, 7, &[4.0], Limits::default()).unwrap();
    let mut out = Vec::new();
    w.sample(&[[1.0, 2.0, 3.0]], 1.7, &mut out).unwrap();
    assert_eq!(out[0].distance, 2.0);

    let asks_old_for_params = made_wrong::area(made_wrong::GROUND).replace(
        r#"(import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))"#,
        r#"(import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))
  (import "plateau:region/host@0.1.0" "params" (func $params (param i32)))"#,
    );
    let why = check::check(&wat::parse_str(asks_old_for_params).unwrap()).unwrap_err();
    assert!(why.iter().any(|w| w.contains("params of plateau:region/host@0.1.0")), "{why:?}");

    let asks_both = made_wrong::area(made_wrong::GROUND).replace(
        r#"(import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))"#,
        r#"(import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))
  (import "plateau:region/host@0.2.0" "params" (func $params (param i32)))"#,
    );
    let why = check::check(&wat::parse_str(asks_both).unwrap()).unwrap_err();
    assert!(why.iter().any(|w| w.contains("two versions")), "{why:?}");

    let params_other_way = made_wrong::area(made_wrong::GROUND).replace(
        r#"(import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))"#,
        r#"(import "plateau:region/host@0.2.0" "params" (func $params (result f32)))"#,
    );
    let why = check::check(&wat::parse_str(params_other_way).unwrap()).unwrap_err();
    assert!(why.iter().any(|w| w.contains("params in another way")), "{why:?}");
}

fn door(params: Vec<f32>) -> PlacedElement {
    PlacedElement {
        id: 1,
        module: "elements/modules/hobbit-door.wasm".into(),
        seed: 3,
        pos: [10.0, 0.5, 20.0],
        yaw: 0.25,
        rot: None,
        scale: 1.0,
        stamp_size: 1.7,
        after_edits: 0,
        promoted: None,
        clips: Vec::new(),
        local_edits: Vec::new(),
        author: Some("local".into()),
        params,
    }
}

#[test]
fn params_are_saved_with_their_placement() {
    let save = WorldSave::with_elements("x", 1, Vec::new(), vec![door(vec![1.25, 0.1, -3.0, 1e-7])]);
    let text = save.to_text();
    assert!(text.contains(" params 4 1.25 0.1 -3 0.0000001\n"), "{text}");
    let back = WorldSave::from_text(&text).unwrap();
    assert_eq!(back.elements[0].params, vec![1.25, 0.1, -3.0, 1e-7]);
    assert_eq!(back, save);

    // None given, none written: a save without params reads as it did.
    let plain = WorldSave::with_elements("x", 1, Vec::new(), vec![door(Vec::new())]).to_text();
    assert!(!plain.contains("params"), "{plain}");

    let line = text.lines().find(|l| l.starts_with("element ")).unwrap();
    for wrong in [
        line.replace(" params 4 1.25 0.1 -3 0.0000001", " params 5 1.25 0.1 -3 0.0000001"),
        line.replace(" params 4 1.25 0.1 -3 0.0000001", " params 0"),
        line.replace(" params 4 1.25 0.1 -3 0.0000001", " params 1 NaN"),
        format!("{line} params 1 2"),
    ] {
        let bad = text.replace(line, &wrong);
        assert!(WorldSave::from_text(&bad).is_err(), "{wrong}");
    }
}
