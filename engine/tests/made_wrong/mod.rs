//! Areas written by hand in WebAssembly's text, for the tests: the world's functions, with an
//! answer for each point that the test writes, to be made wrong on purpose.

/// A module that keeps the world as the canonical ABI lays it out: it answers each point with what
/// `answer` writes (WebAssembly that has `$p`, the point's address, and `$at`, the answer's), and
/// describes itself as "x", with no places and level edges.
pub fn area(answer: &str) -> String {
    let le = |v: u32| v.to_le_bytes().iter().map(|b| format!("\\{b:02x}")).collect::<String>();
    // The description at 1024: its name at 2048, no places, four edges each of 65 heights at 3072.
    let edge = format!("{}{}{}", le(3072), le(65), le(0).repeat(2));
    let description = format!("{}{}{}{}{}", le(2048), le(1), le(0), le(0), edge.repeat(4));
    format!(
        r#"(module
  (import "plateau:region/host@0.1.0" "seed" (func $seed (result i64)))
  (memory (export "memory") 2 2)
  (global $next (mut i32) (i32.const 8192))
  (data (i32.const 1024) "{description}")
  (data (i32.const 2048) "x")
  (func $alloc (export "cabi_realloc") (param i32 i32 i32) (param $n i32) (result i32) (local $p i32)
    (local.set $p (global.get $next))
    (global.set $next (i32.and (i32.add (i32.add (global.get $next) (local.get $n)) (i32.const 7)) (i32.const -8)))
    (local.get $p))
  (func (export "sample") (param $points i32) (param $count i32) (param $size f32) (result i32)
    (local $i i32) (local $p i32) (local $at i32) (local $answers i32)
    (local.set $answers (call $alloc (i32.const 0) (i32.const 0) (i32.const 4) (i32.mul (local.get $count) (i32.const 8))))
    (block $done
      (loop $each
        (br_if $done (i32.ge_u (local.get $i) (local.get $count)))
        (local.set $p (i32.add (local.get $points) (i32.mul (local.get $i) (i32.const 12))))
        (local.set $at (i32.add (local.get $answers) (i32.mul (local.get $i) (i32.const 8))))
        {answer}
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (br $each)))
    (i32.store (i32.const 16) (local.get $answers))
    (i32.store (i32.const 20) (local.get $count))
    (i32.const 16))
  (func (export "cabi_post_sample") (param i32))
  (func (export "describe") (result i32) (i32.const 1024))
  (func (export "cabi_post_describe") (param i32)))
"#
    )
}

/// An answer: the distance up to the ground at y = 0, stone.
pub const GROUND: &str = "(f32.store (local.get $at) (f32.load offset=4 (local.get $p))) (i32.store8 offset=4 (local.get $at) (i32.const 4))";

/// The module of `area(answer)`.
pub fn module(answer: &str) -> Vec<u8> {
    wat::parse_str(area(answer)).unwrap()
}
