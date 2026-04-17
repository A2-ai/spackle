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

# --- WASM ---

build-wasm:
    wasm-pack build --target web --out-dir poc/pkg --features wasm

poc: build-wasm
    cd poc && bun run scripts/demo.ts

test-poc: build-wasm
    cd poc && bun install && bun test