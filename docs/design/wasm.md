# Spackle WASM — contributor architecture

Internal notes for people modifying the wasm path. Consumer-facing docs live under [`/docs/ts/`](../ts/).

---

## One-paragraph architecture

`crates/spackle-wasm/` is a `cdylib` crate that depends on `spackle` via path. It exposes five `#[wasm_bindgen]` functions — `check`, `validate_slot_data`, `render_file`, `render_path`, `plan_hooks` — that handle Tera-flavored compute primitives. The TS host (`ts/`) owns the project walk, the ignore filter, and all disk I/O; it calls into wasm only when it needs Tera + spackle's templating semantics. **Static asset bytes never enter wasm** — `generate` walks `projectDir` disk-direct, calls `render_path` on each relative path, decides per file whether to stream-copy (non-template) or `render_file` (template), and writes output via `DiskFs.writeFile` / `DiskFs.streamCopy`. The native CLI (`crates/spackle-cli/`) threads `StdFs` through the same core primitives; `copy::copy` calls `io::copy(open_read, open_write)` so GB-scale static templates stream there too.

```
┌────────────────────────────────────────────────────────────┐
│  TS host  (ts/src/)                                        │
│                                                            │
│  spackle.ts          — orchestrator. Walks projectDir.     │
│  host/disk-fs.ts     — workspaceRoot containment + per-    │
│                        file disk I/O (writeFile,           │
│                        streamCopy via pipeline()).         │
│  host/memory-fs.ts   — in-memory bundle holder for         │
│                        preview / test flows.               │
│  host/hooks.ts       — host-side subprocess executor.      │
└──────────────────────┬─────────────────────────────────────┘
                       │ per-file calls; static bytes never enter wasm
                       │ (Uint8Array for the few templates we need to render)
┌──────────────────────▼─────────────────────────────────────┐
│  wasm-bindgen layer  (crates/spackle-wasm/src/lib.rs)      │
│                                                            │
│  pub fn check(bundle, project_dir) -> String               │
│    → { config, diagnostics[] }                             │
│  pub fn validate_slot_data(bundle, project_dir,            │
│                             slot_data_json) -> String      │
│  pub fn render_file(template_bundle, target_path,          │
│                     slot_data_json) -> JsValue             │
│    → { bytes: Uint8Array, diagnostics[] }                  │
│  pub fn render_path(path_template,                         │
│                     slot_data_json) -> JsValue             │
│    → { path: string, diagnostics[] }                       │
│  pub fn plan_hooks(bundle, project_dir, out_dir,           │
│                    data_json, hook_ran_json?)              │
│                   -> String {ok, plan | error}             │
└──────────────────────┬─────────────────────────────────────┘
                       │ MemoryFs (config-only) for check/validate/plan
┌──────────────────────▼─────────────────────────────────────┐
│  spackle core  (src/)                                      │
│                                                            │
│  spackle::{check, render}     ← top-level free fns         │
│    → CheckReport / RenderReport with Diagnostic[]          │
│  diagnostic::{Diagnostic, ...} ← structured shape          │
│  Project::{check, generate}    ← native entrypoints        │
│  template::fill<F: FileSystem>                             │
│  template::render_in_memory    ← per-template render       │
│  copy::{copy, copy_collect}<F: FileSystem>                 │
│    — uses open_read/open_write + io::copy for streaming    │
│  config::load_dir<F: FileSystem>                           │
│  slot::validate                                            │
│  hook::{validate_config, evaluate_hook_plan}               │
└────────────────────────────────────────────────────────────┘
```

`spackle::render` (the diagnostics-first project-level pipeline) still exists in core for native callers; the wasm path no longer drives it. The TS `render` orchestrator gets the same shape by composing the per-file primitives host-side over a disk walk.

## Why per-file primitives

Whole-project generation used to flow through wasm via a `MemoryFs` containing the entire project bundle. That worked for KB–MB templates but fell apart for projects with GB-scale static assets:

- **wasm32's 4GB linear-memory ceiling.** A 3GB static asset alone fills two-thirds of the address space.
- **Bundle copy overhead.** Each project file crossed the wasm boundary as `Uint8Array` (JS → Rust copy), got cloned into `MemoryFs`'s `HashMap`, and the output got cloned out again. ~3–4× peak memory per file.

The previous streaming-generate PR (#44) shaved the output-side duplication via a callback FS but left the input bundle and per-file peak intact. The per-file primitive split removes both: only `.j2` template bodies cross the boundary, and only one at a time.

## Per-call shapes

### `check(bundle, project_dir) → CheckResponse`

Bundle expected to contain `spackle.toml` (real bytes) plus path-only placeholders (empty `bytes`) for every other project file. Static path-template validation needs the filename, not the contents — `copy::validate_paths` only reads paths. `.j2`/`.tera` templates that the caller wants statically validated should be passed with real bytes so `template::validate` can parse the body and catch undefined-slot refs.

`buildCheckBundle` (in `ts/src/spackle.ts`) is the host-side builder that puts this together — real bytes for templates, empty bytes for statics, never reads static asset payloads.

### `validate_slot_data(bundle, project_dir, slot_data_json) → ValidationResponse`

Bundle only needs `spackle.toml`. Doesn't walk the project tree.

### `render_file(template_bundle, target_path, slot_data_json) → { bytes, diagnostics }`

Renders one target template against an in-memory registry. `template_bundle` is a `Bundle` of `.j2` / `.tera` template bodies keyed by the same relative paths the host uses for `target_path`; entries are added to a fresh `Tera` instance via the multi-pass `build_resilient_registry` helper so Tera 2's cross-template tags (`{% include %}` / `{% extends %}`) in the target (and anything it transitively references) resolve. Tera 2 does not support `{% macro %}` / `{% import %}`. Unrelated sibling templates that fail to register don't poison the render of a clean target — only the target's own ancestry has to resolve.

Static assets must stay out of the bundle. Peak wasm memory is roughly the sum of template bodies passed in, Tera's parsed registry, and one rendered string for the target. Whole-project output never accumulates wasm-side because only `target_path` is rendered per call.

`target_path` doubles as the diagnostic `path` attribution.

`_project_name` / `_output_name` are not auto-injected here — the host already has both values when it walks the project; it injects them once into the data map rather than passing them per call.

### `render_path(path_template, slot_data_json) → { path, diagnostics }`

Renders one path / filename template. On error, `path` falls back to the input so the host can surface the offending path. Source is `render_name` for any diagnostic.

For `.j2` files with templated names like `src/{{ filename }}.txt.j2`: the host calls `render_path` on the full relative path (yielding `src/notes.txt.j2`), then strips the trailing `.j2` host-side, then calls `render_file` on the body.

### `plan_hooks(bundle, project_dir, out_dir, data_json, hook_ran_json?) → PlanHooksResponse`

Bundle only needs `spackle.toml`. Returns the resolved hook plan (templated commands, should-run flags, skip reasons, template errors). Host executes subprocesses and feeds outcomes back via `hook_ran_json` on re-plan. See the hooks section below.

## Diagnostic surface — `check` vs `render` vs `generate`

| | `check` | `render` | `generate` |
| --- | --- | --- | --- |
| Where | wasm (config + templates) | TS orchestrator | TS orchestrator |
| Needs slot data? | no | yes | yes |
| Fail-fast? | no — collects all | no — collects all (partial preview) | yes — first error aborts |
| Return shape | `{ config, diagnostics[] }` | `{ files, dirs, diagnostics[], hookPlan }` | `{ ok: true; files: number; dirs: number } \| { ok: false; error }` |
| Use case | live editor diagnostics, `spackle check` | live preview pane | write-to-disk workflows |

`check` and `render` share the same `Diagnostic` type — UIs have one rendering path for both.

### Diagnostic sources

- `config` — `spackle.toml` parse / structural error. `path = "spackle.toml"`.
- `slot_config` — bad slot default value type, etc. `ref` = slot key.
- `hook_config` — unknown `needs` reference, broken command/conditional template. `ref` = hook key.
- `slot_data` — user-supplied slot data missing / wrong type. No `path`. `ref` = slot key.
- `copy` — fs read / write / mkdir failure. Reserved for true I/O failures; Tera-sourced failures (path templating) are `render_name` instead.
- `render_body` — template body render fail.
- `render_name` — filename / path template parse or render fail.

Each diagnostic optionally carries `span: { line, column }` (best-effort, extracted from Tera's rendered error format) and a stable `code` (e.g. `"hook::template_render_failed"`).

---

## Streaming I/O on the FileSystem trait

`FileSystem` exposes both byte-buffer methods (`read_file` / `write_file`) and streaming methods (`open_read` / `open_write`).

`copy::copy` uses the streaming pair via `io::copy(reader, writer)`. For `StdFs` that wraps `File::open` / `File::create` directly — bytes flow through Rust's stack-allocated copy buffer (~8 KiB chunks) and never materialize as `Vec<u8>`. Large templates copy with constant memory.

For the in-memory `MockFs` (test utility) and `MemoryFs` (wasm crate), `open_write` returns a buffered writer that commits on drop — `io::copy` does NOT call `flush`, so commit-on-drop is load-bearing. The buffer-in-memory model is correct for tests; the wasm `MemoryFs` only ever holds a small config bundle so the same shape is fine.

---

## Repo layout

```
spackle/
├── src/                     # spackle core (rlib only — no wasm deps)
│   ├── fs.rs                # FileSystem trait + StdFs + MockFs
│   ├── copy.rs              # streaming copy via io::copy(open_read,
│   │                        # open_write)
│   ├── template.rs          # template::fill / render_in_memory
│   └── ...
├── crates/
│   ├── spackle-cli/         # spackle-cli (uses StdFs)
│   └── spackle-wasm/        # cdylib, wasm-bindgen per-file primitives
│       ├── src/lib.rs       # five #[wasm_bindgen] exports + init
│       └── src/memory_fs.rs # MemoryFs (config-only bundles)
├── scripts/
│   └── build-wasm.sh        # cargo build (wasm32) → wasm-bindgen → wasm-opt
├── ts/                      # @a2-ai/spackle npm-shaped TS package
│   ├── src/
│   │   ├── spackle.ts       # orchestrator (walk + per-file wasm calls)
│   │   ├── host/disk-fs.ts  # workspaceRoot + per-file disk I/O
│   │   ├── host/memory-fs.ts
│   │   ├── host/hooks.ts    # subprocess executor
│   │   └── wasm/            # wasm-bindgen wrapper subsystem
│   ├── tests/               # bun test
│   └── pkg/                 # wasm-bindgen output (gitignored)
├── docs/ts/                 # consumer-facing docs
├── examples/                # one full bun-script + framework stubs
└── tests/                   # Rust integration + fixtures/
```

---

## The bundle contract (now config-only)

A **bundle** is `Array<{path: string, bytes: Uint8Array}>`. After the per-file primitive split, bundles only carry config-level inputs to wasm:

- `check` bundle: `spackle.toml` (real bytes) + `.j2`/`.tera` templates (real bytes) + path-only placeholders for static files (empty `bytes`). Static asset bytes never travel.
- `validate_slot_data` / `plan_hooks` bundle: just `spackle.toml`.

Rust deserializes bundles via `serde-wasm-bindgen` into `Vec<BundleEntry>` where `BundleEntry { path: String, bytes: Vec<u8> }` is annotated with `#[serde(with = "serde_bytes")]` so the boundary stays `Uint8Array`.

`MemoryFs` (in `crates/spackle-wasm/src/memory_fs.rs`) auto-creates ancestor dirs when hydrating from the bundle, so callers only need to send file entries.

---

## Build + test locally

```bash
# First-time setup: git hooks, cargo check, bun install, wasm toolchain.
just setup                          # or: just init

# Native tests (spackle + spackle-cli + spackle-wasm).
cargo test --workspace

# Build the wasm artifact into ts/pkg/ (web target, flat layout).
just build-wasm                     # wraps scripts/build-wasm.sh

# Bun test suite for the TS package (builds wasm first).
just test-ts
```

---

## Hooks — plan in wasm, execute in host

Hook *planning* is pure and lives in wasm. The `plan_hooks` export delegates to a **local `plan_hooks_native_parity` function** — a reimplementation of `spackle::hook::evaluate_hook_plan`'s inner loop with `run_hooks_stream` ordering. Why reimplement instead of just calling core's function:

- **Template before conditional.** Native `run_hooks_stream` templates all `queued_hooks` at `src/hook.rs:412-425` BEFORE evaluating `if` expressions; `evaluate_hook_plan` in core templates AFTER the conditional, so a broken template in a hook with `if = "false"` silently skips. Our planner reorders to match native — broken templates are a hard error regardless of conditional outcome.
- **Conditional errors are `Failed`, not skipped.** The planner surfaces conditional-eval errors with `skip_reason="conditional_error: ..."`; the TS runner re-categorizes these to `{ kind: "failed" }` to match native `HookResultKind::Failed(HookError::ConditionalFailed)` at `src/hook.rs:485`.
- **Executed-hook handling.** When caller passes `hook_ran_json`, those hooks are skipped from iteration (so we don't re-plan them and don't overwrite the caller-supplied hook_ran state) but kept in the `items` set so dependent hooks' `needs` resolution still finds them.

The wrapper also injects `_project_name` + `_output_name` to match `Project::run_hooks_stream` at `src/lib.rs:253-254`.

Hook *execution* is host-side. The TS package ships `NodeHooks` (child_process.spawn) and `BunHooks` (Bun.spawn) in `ts/src/host/hooks.ts`; `defaultHooks()` auto-selects per runtime and throws in browser-like hosts. Top-level `runHooksStream(projectDir, outDir, data, fs)` is an async generator that reads `spackle.toml` into a tiny bundle, calls `plan_hooks`, iterates the plan yielding `HookEvent`s per transition, and maintains a `hookRan` map fed back into `plan_hooks` after any non-zero exit so chained conditionals re-evaluate.

**Parity invariants:**
- **Continue on failure.** Native `run_hooks_stream` at `src/hook.rs:527` uses `continue` on non-zero exit, not abort. The TS runner matches.
- **Template errors = hard abort.** The planner surfaces these as `should_run=false` + `template_errors[]`; the TS runner yields a terminal `{ type: "template_errors", error, templateErrors }` event and ends the iterator before any execution. Checked on the initial plan AND every re-plan.
- **Conditional-eval errors = failed.** Surfaced from the planner as `skip_reason="conditional_error: ..."` and re-categorized to `{ kind: "failed" }` by the runner.
- **Hook toggles keyed by raw hook `key`.** Not `hook_<key>`. `Hook::is_enabled` at `src/hook.rs:79-85` checks `data.contains_key(&self.key)`.
- **Tera features match core.** `spackle-wasm`'s tera dep uses full defaults (same as spackle core) so builtins like `| slugify` render identically in wasm and native contexts.

**Deferred:** a stateful session API (`open_session(bundle, project_dir) → SessionId` + `plan_hooks_session(session_id, ...)`) would amortize the per-call bundle parse across the plan-execute loop. Not worth the lifecycle complexity at current scale — parse is sub-millisecond, dwarfed by subprocess spawn time. Revisit when profiles show per-call parse dominating.

Consumer-facing walkthrough: [`docs/ts/hooks.md`](../ts/hooks.md).

---

## Non-obvious invariants

- **No `std::fs` in the wasm binary.** `StdFs` is `#[cfg(not(target_arch = "wasm32"))]`. If something pulls `std::fs` into the wasm tree, the binary grows a WASI-fs import we'd rather avoid.
- **`canonicalize` is gone from the lib.** `Project::get_name` and `get_output_name` use `.file_stem()` / `.file_name()` directly. `DiskFs` canonicalizes host-side for its containment check.
- **Wasm exports are per-file primitives.** Re-adding a whole-project `generate` / `render` wasm export reintroduces the 4GB linear-memory ceiling. Compose new high-level operations in TS on top of the existing primitives.
- **`slugify` appears in `pkg/spackle_wasm.d.ts`.** Incidental export from tera's `slug` dep. Not part of our public contract; ignore.
- **Tera builtins are fully on.** No `default-features = false` dance — the `slug` cfg collision that motivated it was resolved upstream.

## Follow-ups

### Optimization: reuse the Tera registry across calls

`render_file` rebuilds the Tera instance from `template_bundle` every call. For a whole-project `generate`, that's O(N²) parse work — N target renders × N templates per registry build. The constant is small (parse is ~µs per template) and dwarfed by I/O for typical projects, so this is fine at current scale.

If profiles ever show registry rebuilds dominating, the natural next step is a stateful session: `open_session(template_bundle) → SessionId` + `render_file_session(session_id, target_path, slot_data)` + `close_session`. Same shape as the deferred hook-planning session, same tradeoff (lifecycle complexity for amortized parse). Until that surfaces in a profile, the per-call rebuild is intentionally simpler.
