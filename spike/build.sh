#!/bin/bash
# Builds the spike's hand-written area (spike/garden/area.rs) to WebAssembly twice, optimized and
# not, into spike/modules/, as Caprice builds a theme (themes/build.sh there): rustc alone, one file,
# fixed flags, the source path remapped so the bytes do not depend on where the repo is.
set -euo pipefail
cd "$(dirname "$0")/.."
fixed=(--edition 2024 --target wasm32-unknown-unknown --crate-type cdylib --crate-name area -C codegen-units=1
    -C panic=abort -C strip=debuginfo -C link-arg=--max-memory=67108864 --remap-path-prefix "$PWD/spike/garden=/area")
rustc "${fixed[@]}" -C opt-level=3 spike/garden/area.rs -o spike/modules/garden-O.wasm
rustc "${fixed[@]}" -C opt-level=0 spike/garden/area.rs -o spike/modules/garden-O0.wasm
for m in spike/modules/*.wasm; do echo "$m: $(wc -c < "$m") bytes"; done
