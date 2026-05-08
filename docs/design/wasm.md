# Spackle WASM — contributor architecture

Internal notes for people modifying the wasm path. Consumer-facing docs live under [`/docs/ts/`](docs/wasm/).

For the running implementation log, see [`SUMMARY.md`](SUMMARY.md).

---

## One-paragraph architecture

`crates/spackle-wasm/` is a `cdylib` crate that depends on `spackle` via path. It exposes four `#[wasm_bindgen]` functions — `check`, `validate_slot_data`, `generate`, `plan_hooks` — that take a **project bundle** (`Array<{path, bytes: Uint8Array}>`), hydrate an in-process `MemoryFs` from it, and run the requested operation against that fs through the generic `spackle::fs::FileSystem` trait. `check` / `validate_slot_data` / `plan_hooks` return their result as a serialized envelope. `generate` is **streaming-only**: it takes a host callback (`on_entry`) and emits each output file/dir through it as Rust produces them, returning just an `{ok}` envelope; the rendered tree never accumulates Rust-side. Rust never touches the host filesystem; the TS host (`ts/`) reads projects into bundles, picks a sink for the streamed output (memory buffer, async iterator, or sync disk write), and spawns hook subprocesses on its side. Fundamentally: Rust is a pure compute step.

```
┌───────────────────────────────────────────────────────┐
│  TS host  (ts/src/)                                   │
│                                                       │
│  DiskFs   — readProject(dir) → Bundle                 │
│           — prepareOutDir(dir) → string               │
│           — writeEntry(dir, event)  ← streaming sink  │
│           — writeOutput(dir, Bundle)                  │
│  MemoryFs — toBundle() / fromBundle(bundle)           │
│  (plain TS classes; not passed to wasm)               │
│                                                       │
│  generate(projectDir, outDir, …, fs) — disk streaming │
│  generateBundle(bundle, …) — buffer to {files, dirs}  │
│  generateStream(bundle, …) — async-iter ergonomics    │
└─────────────────────┬─────────────────────────────────┘
                      │ bundle in (eager input read);
                      │ output entries fed back through
                      │ on_entry callback while wasm runs
┌─────────────────────▼─────────────────────────────────┐
│  wasm-bindgen layer  (crates/spackle-wasm/src/lib.rs) │
│                                                       │
│  pub fn check(bundle, project_dir) -> String          │
│  pub fn validate_slot_data(bundle, project_dir,       │
│                             slot_data_json) -> String │
│  pub fn generate(bundle, project_dir, out_dir,        │
│                  slot_data_json,                      │
│                  on_entry: js_sys::Function)          │
│                   -> JsValue {ok | ok+error}          │
│  pub fn plan_hooks(bundle, project_dir, out_dir,      │
│                    data_json, hook_ran_json?)         │
│                   -> String {ok, plan | error}        │
└─────────────────────┬─────────────────────────────────┘
                      │ CallbackFs<JsCallbackSink>
                      │ impls FileSystem (write_file /
                      │ create_dir_all → JS callback)
┌─────────────────────▼─────────────────────────────────┐
│  spackle core  (src/) — UNCHANGED                     │
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
├── src/                       # spackle core (rlib only — no wasm deps)
├── cli/                       # spackle-cli (uses StdFs)
├── crates/
│   └── spackle-wasm/          # cdylib, wasm-bindgen exports + MemoryFs
│       ├── src/lib.rs         # four #[wasm_bindgen] exports + init
│       ├── src/memory_fs.rs   # MemoryFs impls spackle::fs::FileSystem
│       └── src/callback_fs.rs # CallbackFs — streaming sink for `generate`
├── scripts/
│   └── build-wasm.sh        # cargo build (wasm32) → wasm-bindgen --target web → wasm-opt
├── ts/                      # @a2-ai/spackle npm-shaped TS package
│   ├── src/                 # TS orchestration + host helpers
│   ├── tests/               # bun test (end-to-end via pkg/)
│   ├── scripts/             # demo.ts
│   └── pkg/                 # wasm-bindgen web-target output (flat — no subdirs)
├── docs/ts/                 # consumer-facing docs
├── examples/                # one full bun-script + framework stubs
└── tests/                   # Rust integration + fixtures/
```

---

## The bundle contract

A **bundle** is `Array<{path: string, bytes: Uint8Array}>`. Paths in an **input** bundle (passed to all four exports) are absolute from the caller's virtual root (typical: `/project/spackle.toml`).

Rust deserializes input bundles via `serde-wasm-bindgen` into `Vec<BundleEntry>` where `BundleEntry { path: String, bytes: Vec<u8> }` is annotated with `#[serde(with = "serde_bytes")]` so the default `Serializer::new()` accepts `Uint8Array` on the way in (and emits it on the way out for the streamed entries — see below).

The `MemoryFs` (in `crates/spackle-wasm/src/memory_fs.rs`) auto-creates ancestor dirs when hydrating from the bundle, so callers only need to send file entries — they don't have to enumerate directories explicitly.

## The generate streaming protocol

`generate` does not return an output bundle. Instead, the host passes a `js_sys::Function` callback as the fifth argument; Rust invokes it synchronously per output entry while the wasm call runs:

```
{ kind: "file", path: <relative>, bytes: Uint8Array }
{ kind: "dir",  path: <relative> }
```

Paths are relative to `out_dir`. Order is parent-before-child: `create_dir_all` events for ancestor directories arrive before any file underneath them, root-to-leaf, deduplicated across multiple file writes that share parents. Files within a directory arrive in whatever order `copy::copy` and `template::fill` produce them.

The `CallbackFs` impl (`crates/spackle-wasm/src/callback_fs.rs`) is the bridge:

- Source-bundle reads (`read_file`, `list_dir`, `stat`, `exists` on `/project/...` paths) delegate to an inner `MemoryFs` hydrated from the input bundle.
- `write_file(/output/<rel>, bytes)` and `create_dir_all(/output/<rel>)` are translated into `{kind, path, bytes?}` events fed to the JS callback.
- `exists(/output)` returns `false` so `Project::generate`'s `AlreadyExists` guard at `src/lib.rs:160` doesn't abort — the host is responsible for AlreadyExists semantics on the real disk before calling.
- Errors thrown by the JS callback are latched in a `RefCell<Option<String>>`; subsequent writes short-circuit so the template phase (which collects per-file errors at `src/template.rs:235-241` rather than aborting) can surface them without re-entering JS. The wasm export checks the latch after `Project::generate` returns and prefers the latched JS error over the synthesized `GenerateError`.

`Project::generate` itself is unchanged — it still writes through the `FileSystem` trait. The streaming behavior is entirely in the wasm-side `CallbackFs`; native callers (CLI) keep using `StdFs` and see no difference.

The TS package gives consumers three sinks atop this primitive:

- `generateBundle(bundle, …)` — buffers events into `{ files: Bundle; dirs: string[] }`. Same shape as the legacy buffered API; preserved for in-memory consumers (preview, in-process inspection). **Does not reduce peak memory** — the buffer holds everything.
- `generateStream(bundle, …)` — async generator that yields each entry plus terminal `done` / `error` events. Useful for progress UIs. **Also does not reduce peak memory** because the wasm call is synchronous and the queue accumulates while Rust runs.
- `generate(projectDir, outDir, …, fs)` — synchronously routes each entry to `DiskFs.writeEntry` inside the callback. **This is the only path that bounds peak memory at one entry** — bytes never accumulate host-side.

---

## Build + test locally

```bash
# First-time setup: git hooks, cargo check, bun install, wasm toolchain.
just setup                          # or: just init

# Native tests (spackle + spackle-cli).
cargo test --workspace

# Build the wasm artifact into ts/pkg/ (web target, flat layout).
just build-wasm                     # wraps scripts/build-wasm.sh

# Bun test suite for the TS package (builds wasm first).
just test-ts
```

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
- **`CallbackFs::exists(out_root)` returns `false`.** Required so `Project::generate`'s `AlreadyExists` guard at `src/lib.rs:160` lets generation proceed under streaming. The host owns AlreadyExists semantics for the real disk; the in-memory view of out_root is "not there yet" until something has been written under it.
- **Streaming aborts leave partial output on disk.** When the host callback throws (or returns an error), the wasm export latches it and surfaces a terminal `{ ok: false, error }` envelope, but any entries that already streamed to disk stay there. Matches native CLI behavior — there's no temp-dir + atomic-rename phase. Callers that need atomicity should pick a fresh `outDir` and move it themselves on success.
- **Input bundle is still eager.** `DiskFs.readProject` materializes the project before calling `wasm.generate`. Output is the streaming win; a lazy-input path is a separate, larger change deferred until profiles call for it.
