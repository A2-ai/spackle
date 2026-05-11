# spackle

Project templating tool. Rust core + native CLI + WebAssembly surface for JS hosts.

## Workspace layout

```
spackle/
├── src/                  # spackle core (rlib). Generic over `F: FileSystem`.
├── crates/
│   ├── spackle-cli/      # spackle-cli (uses StdFs). Installed binary.
│   └── spackle-wasm/     # cdylib, wasm-bindgen exports + Rust MemoryFs.
│                         # Both crates depend on `spackle` via path.
├── scripts/
│   └── build-wasm.sh     # cargo build (wasm32) → wasm-bindgen --target web → wasm-opt.
│                         # Single source of truth; called by `just build-wasm` and CI.
├── ts/                   # @a2-ai/spackle TS package (npm-shaped, GitHub-distributed).
│   ├── src/              # TS orchestration + host helpers.
│   ├── src/wasm/         # Internal wasm-bindgen wrapper subsystem (keep named wasm/).
│   ├── pkg/              # wasm-bindgen web-target output (flat; no subdirs) — gitignored.
│   └── dist/             # tsc emit for the npm package entry — gitignored.
├── docs/
│   ├── configuration.md  # core spackle config
│   └── ts/               # consumer-facing TS package docs
├── examples/             # cli/ (stub), wasm/bun-script/ (runnable), others stubs
├── tests/                # Rust integration tests + fixtures/
└── archive/wasip2-detour/  # Archived WASI experiment. NOT built, NOT tested.
                           # Leave alone.
```

`docs/design/wasm.md` is the contributor-architecture doc for the wasm path.

## Key commands

```bash
# Onboarding (first clone; also available as `just init`)
just setup                      # git hooks + cargo check + bun install + setup-wasm
just setup-wasm                 # wasm toolchain alone (wasm32 + wasm-bindgen-cli + wasm-opt)

# Native (Rust)
cargo test --workspace          # ~98 tests across spackle / spackle-cli / spackle-wasm

# Build (all / per-component)
just build                      # CLI binary + wasm + TS dist
just build-cli                  # target/release/spackle
just build-wasm                 # scripts/build-wasm.sh → ts/pkg/ (web target, flat)
just build-ts                   # wasm + tsc emit to ts/dist/ (npm package)

# TS package (@a2-ai/spackle) — test / demo
just test-ts                    # builds wasm, then bun test in ts/
just demo-ts                    # runs ts/scripts/demo.ts
```

CI: `.github/workflows/ci.yaml` runs cargo tests + bun tests. `.github/workflows/build.yaml` is the release pipeline (goreleaser CLI binaries + wasm tarball).

## Architecture in one paragraph

`spackle` (core) is generic over a `FileSystem` trait (`src/fs.rs`) — every fs-touching function takes `&impl FileSystem`. Native callers pass `StdFs`. The wasm path lives in `crates/spackle-wasm/`, which depends on `spackle` and implements its own `MemoryFs`. Rust never crosses the wasm boundary for I/O — it's a **bundle-in / bundle-out pure function**: TS host hands in `Array<{path, bytes: Uint8Array}>`, Rust generates entirely in-memory, returns `{ files, dirs }`. The TS package (`ts/`) writes the output bundle to disk (or wherever) on its side.

## Naming conventions (settled after course corrections — don't re-flip)

- **TS package dir is `ts/`**, not `wasm/`. It's a TypeScript package, not the wasm binary.
- **Consumer docs live at `docs/ts/`**, not `docs/wasm/`.
- **npm-style package name is `@a2-ai/spackle`**, no `-wasm` suffix. Not published to the npm registry.
- **Rust crate is `spackle-wasm`** — that name IS accurate (the wasm-bindgen surface crate).
- **Internal subdir `ts/src/wasm/`** stays named `wasm/` — it's the wasm-bindgen wrapper subsystem inside the TS package.

## Distribution

**Not published to npm.** The wasm TS package ships as a tarball attached to each GitHub release (`a2-ai-spackle-<version>.tgz`, from `bun pm pack` in the release workflow). `bun add git+ssh:...` does **not** work because `package.json` is at `ts/`, not the repo root, and no JS package manager supports subpath specifiers on git URLs. For dev iteration use `bun link` or a local tarball; for shared pre-releases use the GitHub release asset URL.

## Non-obvious invariants (don't accidentally break)

- **No `std::fs` in the wasm binary.** `StdFs` is `#[cfg(not(target_arch = "wasm32"))]`. `spackle-wasm` uses `MemoryFs` only.
- **`canonicalize()` removed from core.** `Project::get_name` / `get_output_name` use `file_stem()` / `file_name()` directly. Canonicalization happens host-side (`DiskFs` does it for containment).
- **Bundle output paths are relative to `outDir`.** Host prepends its real disk root. Simplifies the contract; don't change back to absolute.
- **Output bundle carries `files` AND `dirs`.** Empty dirs must survive the round-trip to match native `copy::copy`'s `create_dir_all` behavior. Dropping `dirs` silently regresses parity.
- **`DiskFs.writeOutput` refuses a pre-existing `outDir`.** Matches native `GenerateError::AlreadyExists`. Don't weaken this without matching native too.
- **Path containment uses `path.resolve` + prefix check, not `split("/")` blocklists.** Handles platform-specific separators transparently.
- **`slugify` appears in `pkg/spackle_wasm.d.ts`.** Incidental tera export. Ignore.

## Hooks

**Planned in wasm, executed host-side.** `plan_hooks` returns a resolved hook plan (templated commands, should-run flags, skip reasons); the TS host executes subprocesses and feeds results back via `runHooksStream()`, re-planning after any non-zero exit so chained conditionals see actual state. Implementation detail: `plan_hooks_native_parity` reimplements the inner loop with `run_hooks_stream` ordering (template before conditional) to match native failure semantics exactly — see `docs/design/wasm.md`.

**Session API consciously deferred.** Per-call bundle re-parse is sub-millisecond; not worth the stateful API complexity until profiles show otherwise.

**Not exposed in the wasm path:** `run_as_user` / polyjuice; hooks in `generateBundle` (bundle-only mode has no real cwd).

## Development practices

### Rust

- **Table-driven tests where appropriate.** Collect multi-case functions into a `Vec<(input, expected)>` and assert in a loop; one-off edge cases stay as individual `#[test]` fns.

### TypeScript (`ts/`)

- **Lint with `oxlint`**: `bun run lint` (check), `bun run lint:fix` (apply).
- **Format with `oxfmt`**: `bun run fmt` (apply), `bun run fmt:check` (verify).
- Config lives at `ts/.oxlintrc.json` and `ts/.oxfmtrc.json`. Run from inside `ts/`.
- Both should pass before pushing. CI runs `bunx tsc --noEmit` separately for type-checking.

### Don't adopt external crates for core abstractions casually

The `FileSystem` trait is hand-rolled, not borrowed from `vfs` or similar — fits our whole-file byte-buffer shape better than stream-oriented alternatives.

## Before editing

- If you're touching the wasm path, skim `docs/design/wasm.md` for the architecture + invariants.
- If you're touching tests or fixtures, see `tests/fixtures/` (basic_project, bad_template, typed_slots) — the bun tests consume the same fixtures.
