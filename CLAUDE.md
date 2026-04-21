# spackle — repo guide for Claude

Project templating tool. Rust core + native CLI + WebAssembly surface for JS hosts.

## Workspace layout

```
spackle/
├── src/                  # spackle core (rlib). Generic over `F: FileSystem`.
├── cli/                  # spackle-cli (uses StdFs). Installed binary.
├── crates/
│   └── spackle-wasm/     # cdylib, wasm-bindgen exports + Rust MemoryFs.
│                         # Depends on `spackle` via path.
├── ts/                   # @a2-ai/spackle TS package (npm-shaped, GitHub-distributed).
│   ├── src/              # TS orchestration + host helpers.
│   ├── src/wasm/         # Internal wasm-bindgen wrapper subsystem (keep named wasm/).
│   ├── pkg/              # wasm-pack outputs {nodejs,web,bundler} — gitignored.
│   └── dist/             # tsc emit for the npm package entry — gitignored.
├── docs/
│   ├── configuration.md  # core spackle config
│   └── ts/               # consumer-facing TS package docs
├── examples/             # cli/ (stub), wasm/bun-script/ (runnable), others stubs
├── tests/                # Rust integration tests + fixtures/
└── archive/wasip2-detour/  # Archived WASI experiment. NOT built, NOT tested.
                           # Leave alone.
```

`WASM.md` is the contributor-architecture doc for the wasm path.

## Key commands

```bash
# Native (Rust)
cargo test --workspace          # ~98 tests across spackle / cli / spackle-wasm
just check-wasm-target          # compile spackle-wasm on wasm32-unknown-unknown

# WASM (@a2-ai/spackle package)
just build-wasm                 # wasm-pack × 3 targets → ts/pkg/{nodejs,web,bundler}
just build-wasm-ts              # above + tsc emit to ts/dist/
just test-wasm-pkg              # builds wasm, then bun test in ts/
just wasm-demo                  # runs ts/scripts/demo.ts
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

**Not published to npm.** The wasm TS package ships as a tarball attached to each GitHub release (`a2-ai-spackle-<version>.tgz`, from `bun pm pack` in the release workflow). Install via release asset URL or git+ssh. See `ts/README.md`.

## Non-obvious invariants (don't accidentally break)

- **No `std::fs` in the wasm binary.** `StdFs` is `#[cfg(not(target_arch = "wasm32"))]`. `spackle-wasm` uses `MemoryFs` only.
- **`canonicalize()` removed from core.** `Project::get_name` / `get_output_name` use `file_stem()` / `file_name()` directly. Canonicalization happens host-side (`DiskFs` does it for containment).
- **Bundle output paths are relative to `outDir`.** Host prepends its real disk root. Simplifies the contract; don't change back to absolute.
- **Output bundle carries `files` AND `dirs`.** Empty dirs must survive the round-trip to match native `copy::copy`'s `create_dir_all` behavior. Dropping `dirs` silently regresses parity.
- **`DiskFs.writeOutput` refuses a pre-existing `outDir`.** Matches native `GenerateError::AlreadyExists`. Don't weaken this without matching native too.
- **Path containment uses `path.resolve` + prefix check, not `split("/")` blocklists.** Handles platform-specific separators transparently.
- **`slugify` appears in `pkg/*/spackle_wasm.d.ts`.** Incidental tera export. Ignore.
- **wasm-pack's nodejs target inits eagerly; web/bundler need `await init()` first.** The TS wrapper layer only uses nodejs; browser consumers reach for the explicit subpath (`@a2-ai/spackle/pkg/web`).

## Hooks

Unsupported in the wasm path this milestone. `generate(..., { runHooks: true })` returns an explicit `{ ok: false, error: "hooks are unsupported..." }`. Native CLI still runs hooks via `hook::run_hooks_stream`. Pure planning (`hook::evaluate_hook_plan`) lives in core. Placeholder types at `ts/src/host/hooks.ts`.

## Development practices

### Rust

- **Table-driven tests where appropriate.** When a function has multiple input/output cases that share shape, collect them into a single `Vec<(input, expected)>` (or struct-per-case) and assert in a loop. Keeps related cases visible together and keeps failure messages identifying the case. One-off edge cases stay as individual `#[test]` fns — don't force everything into tables.
- **`cargo fmt` before committing.** The workspace uses default rustfmt config.

### TypeScript (`ts/`)

- **Lint with `oxlint`**: `bun run lint` (check), `bun run lint:fix` (apply).
- **Format with `oxfmt`**: `bun run fmt` (apply), `bun run fmt:check` (verify).
- Config lives at `ts/.oxlintrc.json` and `ts/.oxfmtrc.json`. Run from inside `ts/`.
- Both should pass before pushing. CI runs `bunx tsc --noEmit` separately for type-checking.

### Don't adopt external crates for core abstractions casually

The `FileSystem` trait is hand-rolled, not borrowed from `vfs` or similar. Adopting a widely-used abstraction is defensible — but only when it fits the usage pattern better than what we have. For whole-file reads/writes we prefer byte-buffer shapes (`read_file → Vec<u8>`) over stream shapes (`Box<dyn SeekAndRead>`).

## Before editing

- If you're touching the wasm path, skim `WASM.md` for the architecture + invariants.
- If you're touching tests or fixtures, see `tests/fixtures/` (basic_project, bad_template, typed_slots) — the bun tests consume the same fixtures.
