# Example: bun-script

Minimal runnable example of `@a2-ai/spackle-wasm` under Bun.

Reads the fixture at `fixtures/my-template/`, fills it with slot values, writes the output to a temp dir, and prints what was written.

## Run

```bash
# From the repo root. `build-wasm-ts` transitively runs `build-wasm`
# (wasm-pack for all three targets) AND emits the TypeScript dist/
# that `@a2-ai/spackle-wasm`'s default entry resolves to. A clean clone
# needs both — `build-wasm` alone leaves `dist/` empty and the
# `file:../../../wasm` dependency link fails to resolve.
just build-wasm-ts

cd examples/wasm/bun-script
bun install
bun run generate.ts
```

## What the fixture contains

```
fixtures/my-template/
├── spackle.toml                # declares slots: name, project_type
└── {{name}}.j2                 # filename is templated; contents too
```

Run `generate.ts` with different slot values (`name`, `project_type`) to see the output shape change — filename is derived from `{{name}}`, contents include all three.
