# Spackle WASM — contributor architecture

Internal notes for people modifying the wasm path. Consumer-facing docs live under [`/docs/ts/`](docs/wasm/).

For the running implementation log, see [`SUMMARY.md`](SUMMARY.md).

---

## One-paragraph architecture

`crates/spackle-wasm/` is a `cdylib` crate that depends on `spackle` via path. It exposes three `#[wasm_bindgen]` functions — `check`, `validate_slot_data`, `generate` — that take a **project bundle** (`Array<{path, bytes: Uint8Array}>`), hydrate an in-process `MemoryFs` from it, run the requested operation against that fs through the generic `spackle::fs::FileSystem` trait, and return a serialized result. `generate` additionally returns an output bundle. Rust never touches the host filesystem; the TS host (`ts/`) reads projects into bundles and writes output bundles back to disk on its side. Fundamentally: Rust is a pure compute step.

```
┌───────────────────────────────────────────────────────┐
│  TS host  (ts/src/)                                   │
│                                                       │
│  DiskFs   — readProject(dir) → Bundle                 │
│           — writeOutput(dir, Bundle)                  │
│  MemoryFs — toBundle() / fromBundle(bundle)           │
│  (plain TS classes; not passed to wasm)               │
└─────────────────────┬─────────────────────────────────┘
                      │ bundle in, bundle out
                      │ (Uint8Array across the boundary)
┌─────────────────────▼─────────────────────────────────┐
│  wasm-bindgen layer  (crates/spackle-wasm/src/lib.rs) │
│                                                       │
│  pub fn check(bundle, project_dir) -> String          │
│  pub fn validate_slot_data(bundle, project_dir,       │
│                             slot_data_json) -> String │
│  pub fn generate(bundle, project_dir, out_dir,        │
│                  slot_data_json, run_hooks)           │
│                   -> JsValue {ok, files | error}      │
└─────────────────────┬─────────────────────────────────┘
                      │ MemoryFs impls FileSystem
┌─────────────────────▼─────────────────────────────────┐
│  spackle core  (src/)                                 │
│                                                       │
│  Project::{check, generate}                           │
│  template::fill<F: FileSystem>                        │
│  copy::copy<F: FileSystem>                            │
│  config::load_dir<F: FileSystem>                      │
│  slot::validate                                       │
└───────────────────────────────────────────────────────┘
```

Native CLI (`cli/`) threads `spackle::fs::StdFs` through the same core. The only difference between the CLI and wasm paths is which `FileSystem` impl is plumbed in.

---

## Repo layout

```
spackle/
├── src/                     # spackle core (rlib only — no wasm deps)
├── cli/                     # spackle-cli (uses StdFs)
├── crates/
│   └── spackle-wasm/        # cdylib, wasm-bindgen exports + MemoryFs
│       ├── src/lib.rs       # three #[wasm_bindgen] exports + init
│       └── src/memory_fs.rs # MemoryFs impls spackle::fs::FileSystem
├── ts/                      # @a2-ai/spackle npm-shaped TS package
│   ├── src/                 # TS orchestration + host helpers
│   ├── tests/               # bun test (end-to-end via pkg/nodejs)
│   ├── scripts/             # build.ts, demo.ts
│   └── pkg/                 # wasm-pack outputs (nodejs, web, bundler)
├── docs/ts/                 # consumer-facing docs
├── examples/                # one full bun-script + framework stubs
└── tests/                   # Rust integration + fixtures/
```

---

## The bundle contract

A **bundle** is `Array<{path: string, bytes: Uint8Array}>`. Paths in an **input** bundle are absolute from the caller's virtual root (typical: `/project/spackle.toml`). Paths in the **output** bundle returned by `generate` are relative to `outDir`.

Rust deserializes bundles via `serde-wasm-bindgen` into `Vec<BundleEntry>` where `BundleEntry { path: String, bytes: Vec<u8> }` is annotated with `#[serde(with = "serde_bytes")]` so the default `Serializer::new()` emits `Uint8Array` on the return trip (and accepts it on the way in).

The `MemoryFs` (in `crates/spackle-wasm/src/memory_fs.rs`) auto-creates ancestor dirs when hydrating from the bundle, so callers only need to send file entries — they don't have to enumerate directories explicitly.

---

## Build + test locally

```bash
# Native tests (spackle + spackle-cli).
cargo test --workspace

# Build the wasm crate for inspection (outputs /tmp/smoke).
wasm-pack build crates/spackle-wasm --target nodejs --out-dir /tmp/smoke

# Full TS package build — all three wasm-pack targets into ts/pkg/{nodejs,web,bundler}.
cd ts && bun run scripts/build.ts

# Bun test suite against the nodejs-target output.
cd ts && bun test
```

---

## Adding a new wasm-pack target

`ts/scripts/build.ts` iterates a hard-coded array of targets. Add the new target to that array; the generated `pkg/<target>/` dir is automatically gitignored. To expose it as a subpath export, add it to `ts/package.json`'s `exports` map (`./pkg/<target>: "./pkg/<target>/spackle_wasm.js"`).

Consumer-facing guidance on which target to pick lives in [`/docs/ts/runtime-targets.md`](docs/wasm/runtime-targets.md).

---

## Hooks — deferred

Hook *planning* (`spackle::hook::evaluate_hook_plan`) is pure and lives in the core crate. Hook *execution* requires spawning subprocesses, which needs a host-side bridge (the same shape as a future `JsHooks` callback adapter). Not wired in this milestone: `generate(..., runHooks=true)` returns `{ ok: false, error: "hooks are unsupported in this milestone" }`. The placeholder JS types live in [`ts/src/host/hooks.ts`](ts/src/host/hooks.ts).

---

## Non-obvious invariants

- **No `std::fs` in the wasm binary.** `StdFs` is `#[cfg(not(target_arch = "wasm32"))]`. If something pulls `std::fs` into the wasm tree, the binary grows a WASI-fs import we'd rather avoid.
- **`canonicalize` is gone from the lib.** `Project::get_name` and `get_output_name` use `.file_stem()` / `.file_name()` directly. `DiskFs` canonicalizes host-side for its containment check.
- **`slugify` appears in `pkg/*/spackle_wasm.d.ts`.** Incidental export from tera's `slug` dep. Not part of our public contract; ignore.
- **Tera builtins are fully on.** No `default-features = false` dance — the `slug` cfg collision that motivated it was resolved upstream.
