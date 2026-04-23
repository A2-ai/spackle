#!/usr/bin/env bash
# Build the `spackle-wasm` crate to web-target ESM bindings in ts/pkg/.
#
# Pipeline: cargo build (wasm32) → wasm-bindgen --target web → wasm-opt -O.

set -euo pipefail

cd "$(dirname "$0")/.."

WB_VERSION=$(cargo metadata --format-version 1 --locked \
  | jq -r '.packages[] | select(.name == "wasm-bindgen") | .version' | head -1)
if [ -z "$WB_VERSION" ]; then
  echo "could not read wasm-bindgen version from Cargo.lock" >&2
  exit 1
fi

installed=$(wasm-bindgen --version 2>/dev/null | awk '{print $2}' || true)
if [ "$installed" != "$WB_VERSION" ]; then
  echo "wasm-bindgen ${installed:-not installed} != crate $WB_VERSION. Run: just setup-wasm" >&2
  exit 1
fi

if ! command -v wasm-opt >/dev/null 2>&1; then
  echo "wasm-opt not found. Run: just setup-wasm" >&2
  exit 1
fi

cargo build --lib -p spackle-wasm --target wasm32-unknown-unknown --release

rm -rf ts/pkg
mkdir -p ts/pkg

wasm-bindgen target/wasm32-unknown-unknown/release/spackle_wasm.wasm \
  --target web --typescript --out-dir ts/pkg

wasm-opt ts/pkg/spackle_wasm_bg.wasm -O -o ts/pkg/spackle_wasm_bg.opt.wasm
mv ts/pkg/spackle_wasm_bg.opt.wasm ts/pkg/spackle_wasm_bg.wasm
