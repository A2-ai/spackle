# Spackle WASM — contributor architecture

Internal notes for people modifying the wasm path. Consumer-facing docs live under [`/docs/ts/`](docs/wasm/).

For the running implementation log, see [`SUMMARY.md`](SUMMARY.md).

---

## One-paragraph architecture

`crates/spackle-wasm/` is a `cdylib` crate that depends on `spackle` via path. It exposes four `#[wasm_bindgen]` functions — `check`, `validate_slot_data`, `generate`, `plan_hooks` — that take a **project bundle** (`Array<{path, bytes: Uint8Array}>`), hydrate an in-process `MemoryFs` from it, run the requested operation against that fs through the generic `spackle::fs::FileSystem` trait, and return a serialized result. `generate` additionally returns an output bundle; `plan_hooks` returns a resolved hook plan (templated commands + should-run + skip reasons) that the host executes. Rust never touches the host filesystem; the TS host (`ts/`) reads projects into bundles, writes output bundles back to disk, and spawns hook subprocesses on its side. Fundamentally: Rust is a pure compute step.

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
│                  slot_data_json)                      │
│                   -> JsValue {ok, files, dirs|error}  │
│  pub fn plan_hooks(bundle, project_dir, out_dir,      │
│                    data_json, hook_ran_json?)         │
│                   -> String {ok, plan | error}        │
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

## Hooks — plan in wasm, execute in host

Hook *planning* is pure and lives in wasm. The `plan_hooks` export in `crates/spackle-wasm/src/lib.rs` delegates to a **local `plan_hooks_native_parity` function** — a reimplementation of `spackle::hook::evaluate_hook_plan`'s inner loop with `run_hooks_stream` ordering. Why reimplement instead of just calling core's function:

- **Template before conditional.** Native `run_hooks_stream` templates all `queued_hooks` at `src/hook.rs:412-425` BEFORE evaluating `if` expressions; `evaluate_hook_plan` in core templates AFTER the conditional, so a broken template in a hook with `if = "false"` silently skips. Our planner reorders to match native — broken templates are a hard error regardless of conditional outcome.
- **Conditional errors are `Failed`, not skipped.** The planner surfaces conditional-eval errors with `skip_reason="conditional_error: ..."`; the TS runner re-categorizes these to `{ kind: "failed" }` to match native `HookResultKind::Failed(HookError::ConditionalFailed)` at `src/hook.rs:485`.
- **Executed-hook handling.** When caller passes `hook_ran_json`, those hooks are skipped from iteration (so we don't re-plan them and don't overwrite the caller-supplied hook_ran state) but kept in the `items` set so dependent hooks' `needs` resolution still finds them.

The wrapper also injects `_project_name` + `_output_name` to match `Project::run_hooks_stream` at `src/lib.rs:253-254`.

Hook *execution* is host-side. The TS package ships `NodeHooks` (child_process.spawn) and `BunHooks` (Bun.spawn) in `ts/src/host/hooks.ts`; `defaultHooks()` auto-selects per runtime and throws in browser-like hosts. Top-level `runHooksStream(projectDir, outDir, data, fs)` is an async generator that reads the bundle, calls `plan_hooks`, iterates the plan yielding `HookEvent`s per transition, and maintains a `hookRan` map fed back into `plan_hooks` after any non-zero exit so chained conditionals re-evaluate (matches native's inline conditional re-eval at `src/hook.rs:474-491`). The event stream is the bridge point for SSE-style live UIs.

**Parity invariants:**
- **Continue on failure.** Native `run_hooks_stream` at `src/hook.rs:527` uses `continue` on non-zero exit, not abort. The TS runner matches.
- **Template errors = hard abort.** The planner surfaces these as `should_run=false` + `template_errors[]`; the TS runner yields a terminal `{ type: "template_errors", error, templateErrors }` event and ends the iterator before any execution, matching `Error::ErrorRenderingTemplate` at `src/hook.rs:415-425`. Checked on the initial plan AND every re-plan.
- **Conditional-eval errors = failed.** Surfaced from the planner as `skip_reason="conditional_error: ..."` and re-categorized to `{ kind: "failed" }` by the runner.
- **Hook toggles keyed by raw hook `key`.** Not `hook_<key>`. `Hook::is_enabled` at `src/hook.rs:79-85` checks `data.contains_key(&self.key)`.
- **Tera features match core.** `spackle-wasm`'s tera dep uses full defaults (same as spackle core) so builtins like `| slugify` render identically in wasm and native contexts.

**Deferred:** a stateful session API (`open_session(bundle, project_dir) → SessionId` + `plan_hooks_session(session_id, ...)`) would amortize the per-call bundle parse across the plan-execute loop. Not worth the lifecycle complexity at current scale — parse is sub-millisecond, dwarfed by subprocess spawn time. Revisit when profiles show per-call parse dominating.

Consumer-facing walkthrough: [`docs/ts/hooks.md`](docs/ts/hooks.md).

---

## Non-obvious invariants

- **No `std::fs` in the wasm binary.** `StdFs` is `#[cfg(not(target_arch = "wasm32"))]`. If something pulls `std::fs` into the wasm tree, the binary grows a WASI-fs import we'd rather avoid.
- **`canonicalize` is gone from the lib.** `Project::get_name` and `get_output_name` use `.file_stem()` / `.file_name()` directly. `DiskFs` canonicalizes host-side for its containment check.
- **`slugify` appears in `pkg/*/spackle_wasm.d.ts`.** Incidental export from tera's `slug` dep. Not part of our public contract; ignore.
- **Tera builtins are fully on.** No `default-features = false` dance — the `slug` cfg collision that motivated it was resolved upstream.
