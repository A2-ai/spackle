setup:
    lefthook install

run *args="":
    cargo run -p spackle-cli {{args}}

test:
    cargo test --workspace

test-wasm:
    cargo test --workspace --features wasm

install:
    cargo install --path=cli

# --- WASM (wasm-bindgen primary path) ---
#
# Rust drives generation through a JS-provided `SpackleFs` adapter.
# See `poc/src/host/{disk-fs,memory-fs,spackle-fs}.ts` for the reference
# adapters and `src/wasm_fs.rs` for the Rust bridge.

build-wasm:
    wasm-pack build --target nodejs --out-dir poc/pkg --features wasm

poc: build-wasm
    cd poc && bun install && bun run scripts/demo.ts

test-poc: build-wasm
    cd poc && bun install && bun test tests/spackle.test.ts tests/disk-fs.test.ts tests/memory-fs.test.ts

# Legacy wasip2/component-model detour has been archived to
# `archive/wasip2-detour/` — not built, not tested. See SUMMARY.md for
# the retirement rationale.