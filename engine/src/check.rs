//! The engine's check of an area's module, every time it is loaded, never trusted (POC_PLAN.md,
//! step 1): what it may import and must export, its memory, and the WebAssembly it may use. The
//! reasons are in words, for Claude to read when it made the area.

use wasmparser::ValType::{F32, I32, I64};
use wasmparser::{CompositeInnerType, ExternalKind, Parser, Payload, TypeRef, ValType, Validator, WasmFeatures};

/// The most an area's module may be.
pub const MOST_BYTES: usize = 4 << 20;
/// The most memory it may declare it can take, in 64 KiB pages: 64 MiB.
pub const MOST_PAGES: u64 = 1024;
pub const MOST_MEMORY: usize = (MOST_PAGES as usize) << 16;

/// Where an area's imports come from: the world's `host`.
pub const HOST: &str = "plateau:region/host@0.1.0";

type Signature = (&'static str, &'static [ValType], &'static [ValType]);

/// What an area may import (from `HOST`) and must export, as the canonical ABI flattens the
/// world's functions: their parameters and results.
const IMPORTS: [Signature; 1] = [("seed", &[], &[I64])];
const EXPORTS: [Signature; 5] = [
    ("sample", &[I32, I32, F32], &[I32]),
    ("cabi_post_sample", &[I32], &[]),
    ("describe", &[], &[I32]),
    ("cabi_post_describe", &[I32], &[]),
    ("cabi_realloc", &[I32, I32, I32, I32], &[I32]),
];

/// The WebAssembly an area may use, for good: WebAssembly 2.0 without SIMD, what every engine runs
/// (a browser's too) and what `wasm32-unknown-unknown` turns on by default. No threads: an area
/// gives the same answers every time. A toolchain that turns on more makes a new version of the
/// world, never a change to this one.
pub fn features() -> WasmFeatures {
    WasmFeatures::WASM2.difference(WasmFeatures::SIMD)
}

/// Whether `module` may be run as an area, and if not, each reason why, in words.
pub fn check(module: &[u8]) -> Result<(), Vec<String>> {
    if module.len() > MOST_BYTES {
        return Err(vec![format!("it is {} KB, more than the {} KB an area may be", module.len() / 1024, MOST_BYTES / 1024)]);
    }
    if let Err(e) = Validator::new_with_features(features()).validate_all(module) {
        return Err(vec![format!("it is not the WebAssembly an area may use (2.0, without SIMD or threads): {}", e.message())]);
    }
    let mut why = Vec::new();
    // Each function's type, imported ones first.
    let (mut types, mut funcs, mut exported, mut memories) = (Vec::new(), Vec::new(), Vec::new(), 0);
    let signature = |types: &Vec<Option<(Vec<ValType>, Vec<ValType>)>>, t: u32| types.get(t as usize).cloned().flatten();
    for payload in Parser::new(0).parse_all(module) {
        let Ok(payload) = payload else { return Err(vec!["it could not be read".into()]) };
        match payload {
            Payload::TypeSection(r) => {
                for group in r.into_iter().flatten() {
                    for ty in group.into_types() {
                        types.push(match ty.composite_type.inner {
                            CompositeInnerType::Func(f) => Some((f.params().to_vec(), f.results().to_vec())),
                            _ => None,
                        });
                    }
                }
            }
            Payload::ImportSection(r) => {
                for import in r.into_imports().flatten() {
                    let wanted = IMPORTS.iter().find(|i| import.module == HOST && i.0 == import.name);
                    match (import.ty, wanted) {
                        (TypeRef::Func(t), Some((name, params, results))) => {
                            if signature(&types, t) != Some((params.to_vec(), results.to_vec())) {
                                why.push(format!("it asks the engine for {name} in another way than the engine answers"));
                            }
                            funcs.push(t);
                        }
                        (TypeRef::Func(t), None) => {
                            why.push(format!("it asks for {} of {}, which the engine does not give", import.name, import.module));
                            funcs.push(t);
                        }
                        _ => why.push(format!("it asks for {} of {}, which is not a function the engine gives", import.name, import.module)),
                    }
                }
            }
            Payload::FunctionSection(r) => funcs.extend(r.into_iter().flatten()),
            Payload::MemorySection(r) => {
                for m in r.into_iter().flatten() {
                    memories += 1;
                    match m.maximum {
                        _ if m.shared || m.memory64 => why.push("its memory is shared or 64-bit".into()),
                        None => why.push("it does not say how much memory it may take".into()),
                        Some(most) if most > MOST_PAGES => {
                            why.push(format!("it may take {} MB of memory, more than the {} MB an area may", most / 16, MOST_PAGES / 16))
                        }
                        _ => {}
                    }
                }
            }
            Payload::ExportSection(r) => {
                for export in r.into_iter().flatten() {
                    exported.push(export.name.to_string());
                    let wanted = EXPORTS.iter().find(|e| e.0 == export.name);
                    match (export.kind, wanted) {
                        (ExternalKind::Func, Some((name, params, results))) => {
                            let t = funcs.get(export.index as usize).and_then(|&t| signature(&types, t));
                            if t != Some((params.to_vec(), results.to_vec())) {
                                why.push(format!("its {name} is not what the engine calls"));
                            }
                        }
                        (ExternalKind::Memory, _) if export.name == "memory" => {}
                        _ => why.push(format!("it gives {}, which the engine does not use", export.name)),
                    }
                }
            }
            _ => {}
        }
    }
    if memories != 1 {
        why.push("it does not have one memory of its own".into());
    }
    for name in EXPORTS.iter().map(|e| e.0).chain(["memory"]) {
        if !exported.iter().any(|e| e == name) {
            why.push(format!("it does not give {name}, which the engine calls"));
        }
    }
    if why.is_empty() { Ok(()) } else { Err(why) }
}
