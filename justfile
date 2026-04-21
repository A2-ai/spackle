setup:
    lefthook install

run *args="":
    cargo run -p spackle-cli {{args}}

test:
    cargo test --workspace

# Smoke-compile the wasm crate on wasm32-unknown-unknown to catch
# wasm-target regressions without running wasm-pack.
check-wasm-target:
    cargo build -p spackle-wasm --target wasm32-unknown-unknown

install:
    cargo install --path=cli

# --- WASM (@a2-ai/spackle-wasm) ---
#
# `crates/spackle-wasm` is the cdylib that exposes the three bundle-in /
# bundle-out exports. `wasm/` is the published TypeScript package that
# wraps it. See `WASM.md` for architecture, `docs/wasm/` for consumer docs.

# Build all three wasm-pack targets (nodejs, web, bundler) into
# `wasm/pkg/<target>/`. One command; consumes the script.
build-wasm:
    cd wasm && bun run scripts/build.ts

# Build the TS dist with typings. Run AFTER build-wasm since the dist
# imports from pkg/nodejs/.
build-wasm-ts: build-wasm
    cd wasm && bunx tsc -p tsconfig.build.json

# Run the wasm package's demo script. Builds wasm first.
wasm-demo: build-wasm
    cd wasm && bun install && bun run scripts/demo.ts

# Run the wasm package's bun test suite. Builds wasm first.
test-wasm-pkg: build-wasm
    cd wasm && bun install && bun test

# Legacy wasip2/component-model detour has been archived to
# `archive/wasip2-detour/` — not built, not tested. See SUMMARY.md for
# the retirement rationale.
