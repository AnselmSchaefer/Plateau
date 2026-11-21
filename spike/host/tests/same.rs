//! The same everywhere (CONCEPT.md, "Laws"), as the spike can check it: the area's module gives
//! the answers its source gives played natively, bit for bit, optimized or not.

use spike_host::{Area, Limits, Native, SIZES, Wasm, hash, module, points};

fn hashes(area: &mut dyn Area, points: &[[f32; 3]]) -> Vec<u64> {
    let mut out = Vec::new();
    SIZES
        .iter()
        .map(|&(_, size)| {
            let mut all = Vec::new();
            for batch in points.chunks(1000) {
                area.sample(batch, size, &mut out).unwrap();
                all.extend_from_slice(&out);
            }
            hash(&all)
        })
        .collect()
}

#[test]
fn the_module_answers_as_the_source_does_at_every_size_and_both_levels() {
    let points = points(32);
    let native = hashes(&mut Native { seed: 7 }, &points);
    for level in ["O", "O0"] {
        let mut wasm = Wasm::new(&module(level).unwrap(), 7, Limits::default()).unwrap();
        assert_eq!(hashes(&mut wasm, &points), native, "the module built at -{level}");
    }
}

#[test]
fn each_size_sees_its_own_detail() {
    let native = hashes(&mut Native { seed: 7 }, &points(16));
    assert!(native[0] != native[1] && native[1] != native[2], "{native:x?}");
}

#[test]
fn a_module_that_runs_out_of_fuel_is_stopped() {
    let limits = Limits { fuel: Some(10_000), ..Limits::default() };
    let mut wasm = Wasm::new(&module("O").unwrap(), 7, limits).unwrap();
    let mut out = Vec::new();
    assert!(wasm.sample(&points(8), 0.017, &mut out).is_err());
}

#[test]
fn the_garden_never_looks_at_a_nans_bits() {
    for level in ["O", "O0"] {
        let verdict = spike_host::nans_unseen(&module(level).unwrap(), 7, &points(24)).unwrap();
        assert_eq!(verdict, Ok(()), "the module built at -{level}");
    }
}

/// An area that makes a NaN from its input (so that nothing can fold it ahead) and answers with
/// its sign bit: x86's own NaN has it set, the one NaN has not.
const LOOKS_AT_NANS: &str = r#"
(module
  (import "host" "seed" (func $seed (result i64)))
  (memory (export "memory") 2)
  (global $next (mut i32) (i32.const 1024))
  (func (export "alloc") (param $n i32) (result i32) (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.and (i32.add (i32.add (global.get $next) (local.get $n)) (i32.const 7)) (i32.const -8)))
    (local.get $p))
  (func (export "sample") (param $points i32) (param $count i32) (param $size f32) (param $out i32)
    (local $i i32) (local $zero f32) (local $at i32)
    (block $done
      (loop $each
        (br_if $done (i32.ge_u (local.get $i) (local.get $count)))
        (local.set $zero (f32.mul (f32.load (i32.add (local.get $points) (i32.mul (local.get $i) (i32.const 12)))) (f32.const 0)))
        (local.set $at (i32.add (local.get $out) (i32.mul (local.get $i) (i32.const 8))))
        (f32.store (local.get $at)
          (f32.convert_i32_u (i32.shr_u (i32.reinterpret_f32 (f32.div (local.get $zero) (local.get $zero))) (i32.const 31))))
        (i32.store offset=4 (local.get $at) (i32.const 2))
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $each)))))
"#;

#[cfg(target_arch = "x86_64")]
#[test]
fn an_area_that_looks_at_a_nans_bits_breaks_the_law() {
    let module = wat::parse_str(LOOKS_AT_NANS).unwrap();
    let verdict = spike_host::nans_unseen(&module, 7, &points(4)).unwrap();
    let message = verdict.expect_err("the law should fail");
    assert!(message.contains("looks at a NaN's bits") && message.contains("tiny"), "{message}");
}
