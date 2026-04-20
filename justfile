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

# --- WASM (wasm-pack path, deprecated but still built) ---

build-wasm:
    wasm-pack build --target web --out-dir poc/pkg --features wasm

poc: build-wasm
    cd poc && bun run scripts/demo.ts

test-poc: build-wasm
    cd poc && bun install && bun test tests/wasm.test.ts tests/host.test.ts tests/e2e.test.ts

# --- WASI Preview 2 component (cargo-component + jco) ---

# Compile the Rust side to a WASI component, then transpile TWICE:
# - `wasip2-pkg/`: default jco output (uses @bytecodealliance/preview2-shim).
#   Node-compatible; Bun-incompatible due to preview2-shim's tcp_wrap dep.
# - `wasip2-pkg-no-shim/`: --no-wasi-shim output. Pairs with the custom
#   Bun WASI shim at `poc/src/wasip2/bun-wasi.ts` — Bun-compatible.
# See WASM.md § "Decision record: reference runtime" for context.
build-wasip2:
    cargo component build --release --features wasip2 --no-default-features
    cd poc && bun x jco transpile ../target/wasm32-wasip1/release/spackle.wasm \
        -o wasip2-pkg --instantiation async --name spackle
    cd poc && bun x jco transpile ../target/wasm32-wasip1/release/spackle.wasm \
        -o wasip2-pkg-no-shim --instantiation async --name spackle --no-wasi-shim

# Node reference: runs the 6 smoke tests via node:test, using the
# preview2-shim-based jco output.
test-wasip2: build-wasip2
    cd poc && bun install && node --experimental-strip-types --test tests/wasip2/component.test.mjs

# Bun reference: runs the 6 component smoke tests + shim containment
# unit tests via `bun test`, then the compile-mode smoke (ensures
# `bun build --compile` produces a working binary — catches drift that
# `bun run` alone wouldn't).
test-wasip2-bun: build-wasip2
    cd poc && bun install && bun test tests/wasip2/component.bun.test.ts tests/wasip2/bun-wasi-containment.test.ts
    cd poc && bun build --compile tests/wasip2/smoke-compile.ts --outfile /tmp/spackle-bun-smoke
    /tmp/spackle-bun-smoke {{justfile_directory()}}/tests/data/proj2