# One-shot onboarding. Installs git hooks, verifies the Rust workspace
# builds, installs JS deps, installs wasm toolchain. Run once after clone.
alias init := setup

setup: setup-wasm
    lefthook install
    cargo check --workspace
    cd ts && bun install

# Wasm toolchain (wasm32 target + wasm-bindgen-cli pinned to Cargo.lock
# + wasm-opt). Split out so re-runs don't trigger the full bootstrap.
setup-wasm:
    #!/usr/bin/env bash
    set -euo pipefail
    rustup target add wasm32-unknown-unknown
    WB_VERSION=$(cargo metadata --format-version 1 --locked \
      | jq -r '.packages[] | select(.name == "wasm-bindgen") | .version' | head -1)
    cargo binstall --no-confirm wasm-bindgen-cli --version "$WB_VERSION" \
      || cargo install --locked wasm-bindgen-cli --version "$WB_VERSION"
    cargo binstall --no-confirm wasm-opt || cargo install wasm-opt

run *args="":
    cargo run -p spackle-cli {{args}}

test:
    cargo test --workspace

install:
    cargo install --path=cli

# --- Build ---
#
# `build` is the top-level catch-all: CLI binary + wasm + TS dist. The
# per-component recipes below can also be invoked individually. Layout:
#
#   build-cli   — `spackle` CLI binary (cargo release build).
#   build-wasm  — `crates/spackle-wasm` compiled to web-target wasm
#                 bundle in `ts/pkg/` (runs `scripts/build-wasm.sh`).
#   build-ts    — `@a2-ai/spackle` TS package: wasm + tsc emit to
#                 `ts/dist/`. Transitively runs `build-wasm`.

# Build everything: CLI + wasm + TS dist.
build: build-cli build-ts

# Build the `spackle` CLI binary (release profile). Output at
# `target/release/spackle`. Use `just install` to place it on PATH.
build-cli:
    cargo build --release -p spackle-cli

# Build the web-target wasm into `ts/pkg/`. Delegates to the repo-root
# shell script — same path CI uses (no `just` dependency in CI).
build-wasm:
    ./scripts/build-wasm.sh

# Build the `@a2-ai/spackle` TS package: wasm (via build-wasm dep) plus
# tsc emit to `ts/dist/`. This is what `bun pm pack` tarballs.
build-ts: build-wasm
    cd ts && bunx tsc -p tsconfig.build.json

# Run the TS package's demo script. Builds wasm first.
demo-ts: build-wasm
    cd ts && bun install && bun run scripts/demo.ts

# Run the TS package's bun test suite. Builds wasm first.
test-ts: build-wasm
    cd ts && bun install && bun test

# Legacy wasip2/component-model detour has been archived to
# `archive/wasip2-detour/` — not built, not tested. See SUMMARY.md for
# the retirement rationale.
