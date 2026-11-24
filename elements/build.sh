#!/bin/bash
# Builds each standalone element of elements/ (elements/<name>/element.rs) to WebAssembly using the
# exact same pinned toolchain, SDK rlib, fixed flags, and path remapping as areas/build.sh.
# Outputs into elements/modules/<name>.wasm (or the folder given).
set -euo pipefail
cd "$(dirname "$0")/.."
into=$(realpath "${1:-elements/modules}")
mkdir -p "$into"
out=$(mktemp -d "${TMPDIR:-/tmp}/elements.XXXXXX")
trap 'rm -rf "$out"' EXIT
fixed=(--edition 2024 --target wasm32-unknown-unknown -O -C codegen-units=1 -C panic=abort -C strip=debuginfo)
rustc "${fixed[@]}" --crate-type rlib --crate-name plateau_sdk --remap-path-prefix "$PWD/sdk=/sdk" \
    "$PWD/sdk/src/lib.rs" -o "$out/libplateau_sdk.rlib"
for dir in elements/*/; do
    [ -f "$dir/element.rs" ] || continue
    name=$(basename "$dir")
    mkdir -p "$out/$name"
    rustc --crate-name area "${fixed[@]}" --crate-type cdylib -C link-arg=--max-memory=67108864 \
        --remap-path-prefix "$PWD/elements/$name=/element" --extern plateau_sdk="$out/libplateau_sdk.rlib" \
        "$PWD/elements/$name/element.rs" -o "$out/$name/element.wasm"
    cp "$out/$name/element.wasm" "$into/$name.wasm"
    echo "$into/$name.wasm: $(wc -c < "$into/$name.wasm") bytes"
done
