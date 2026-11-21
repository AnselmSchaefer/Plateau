#!/bin/bash
# Builds each area of areas/ (areas/<name>/area.rs) to WebAssembly as the engine will when Claude
# makes one (POC_PLAN.md, "Compiling"): the repo's rustc (rust-toolchain.toml), the SDK as one rlib,
# the area as one file, fixed flags, the output always named area.wasm (wasm-ld writes the name
# into the module) and the paths remapped (a path is in the module, where a panic would say it
# happened), so the same source gives the same bytes wherever the repo is. Into areas/modules/
# <name>.wasm, or the folder given. `-C strip=debuginfo` keeps the component-type sections the
# SDK's bindings put in, so every area can still be made a component of `plateau:region@0.1.0`.
set -euo pipefail
cd "$(dirname "$0")/.."
into=$(realpath "${1:-areas/modules}")
out=$(mktemp -d "${TMPDIR:-/tmp}/areas.XXXXXX")
trap 'rm -rf "$out"' EXIT
fixed=(--edition 2024 --target wasm32-unknown-unknown -O -C codegen-units=1 -C panic=abort -C strip=debuginfo)
rustc "${fixed[@]}" --crate-type rlib --crate-name plateau_sdk --remap-path-prefix "$PWD/sdk=/sdk" \
    "$PWD/sdk/src/lib.rs" -o "$out/libplateau_sdk.rlib"
for dir in areas/*/; do
    [ -f "$dir/area.rs" ] || continue
    name=$(basename "$dir")
    mkdir -p "$out/$name"
    # 64 MiB at most, declared in the module (engine::check::MOST_MEMORY).
    rustc --crate-name area "${fixed[@]}" --crate-type cdylib -C link-arg=--max-memory=67108864 \
        --remap-path-prefix "$PWD/areas/$name=/area" --extern plateau_sdk="$out/libplateau_sdk.rlib" \
        "$PWD/areas/$name/area.rs" -o "$out/$name/area.wasm"
    cp "$out/$name/area.wasm" "$into/$name.wasm"
    echo "$into/$name.wasm: $(wc -c < "$into/$name.wasm") bytes"
done
