# Spackle WASM Proof of Concept

## What this is

A Bun-based TypeScript PoC (`poc/`) that compiles spackle's core logic to
WASM and drives it end-to-end, including hook execution via `Bun.spawn`.
Every WASM export is exercised by automated `bun test` cases so we know
the artifact is consumable from TypeScript before we attach it to a
release.

```bash
just poc          # builds WASM + runs scripts/demo.ts
just test-poc     # builds WASM + runs bun test
# or manually:
just build-wasm   # wasm-pack → poc/pkg/
cd poc && bun test
cd poc && bun run scripts/demo.ts
```

## Architecture: why this doesn't call `Project::generate()` directly

Spackle's native API splits generation into two calls:
`Project::generate()` (copy files + render templates) and
`Project::run_hooks_stream()` (execute post-generation shell commands).
Both require filesystem access; hooks additionally require process
spawning. Neither is available in WASM (`wasm32-unknown-unknown` has
no OS).

Instead, this PoC splits the work between two runtimes:

```
┌─────────────────────────────────┐
│  WASM (spackle compiled)        │
│                                 │
│  Pure computation only:         │
│  • parse_config    — toml → json│
│  • validate_config — structure  │
│  • check_project   — + templates│
│  • validate_slot_data           │
│  • render_templates — tera      │
│  • evaluate_hooks  — plan only  │
└────────────┬────────────────────┘
             │ JSON strings
┌────────────▼────────────────────┐
│  TypeScript / Bun (host)        │
│                                 │
│  All I/O:                       │
│  • read spackle.toml from disk  │
│  • read .j2 template files      │
│  • write rendered output files  │
│  • (future) spawn hook commands │
└─────────────────────────────────┘
```

The TypeScript host reads files, passes their contents as strings into
the WASM module for computation, gets JSON results back, then writes
output files and (optionally) runs hooks via `Bun.spawn`.

## Where the WASM↔TS line lives

The `poc/src/` layout is the canonical reference — every file is on one
side of the line and says so in its header:

```
poc/src/
├── wasm/                   // WASM-SIDE — pure computation. No I/O.
│   ├── types.ts            // interfaces mirroring Rust structs
│   ├── index.ts            // loadSpackleWasm() — singleton init + typed client
│   └── ...
├── host/                   // HOST-SIDE — requires Node/Bun I/O.
│   ├── fs.ts               // readSpackleConfig, walkTemplates,
│   │                       // writeRenderedFiles, copyNonTemplates
│   └── hooks.ts            // executeHookPlan (Bun.spawn)
└── spackle.ts              // ORCHESTRATION — composes wasm + host
                            // into check() and generate()
```

`poc/src/spackle.ts#generate` is the single place where the two sides
meet. Every line is annotated `// WASM` or `// HOST` so the boundary is
visible at a glance:

```ts
const toml = await readSpackleConfig(projectDir);              // HOST
const config = wasm.parseConfig(toml);                         // WASM
const templates = await walkTemplates(projectDir, config.ignore); // HOST
const rendered = wasm.renderTemplates(templates, fullData, configJson); // WASM
// Copy first, then render — matches native: templates win on path collisions.
await copyNonTemplates(projectDir, outDir, config.ignore, fullData, wasm); // HOST
await writeRenderedFiles(outDir, rendered);                    // HOST
const plan = wasm.evaluateHooks(configJson, fullData);         // WASM
for await (const r of executeHookPlan(plan, outDir)) { ... }   // HOST
```

`generate()` matches native `Project::generate` semantics by default:

- **Output-dir protection** — errors if `outDir` already exists. Opt-in
  with `{ overwrite: true }`.
- **Template-error fail-fast** — throws on the first render failure
  before writing anything. Opt-in to partial writes with
  `{ allowTemplateErrors: true }` (useful for UIs that want to surface
  every failure at once).
- **Copy → render order** — non-templates copied first, templates
  written second so they overwrite on collision.

If you are building the eventual `spackle-web/` server, lift these
functions as-is — `wasm/` and `host/` are import-stable boundaries,
`spackle.ts` is the reference implementation, and the bun tests are
your regression suite.

## What the PoC exercises

| Step | WASM function | What it proves |
|------|---------------|----------------|
| 1 | `parse_config(toml)` | Config parsing works in WASM — slots, hooks, ignore list |
| 2 | `validate_config(toml)` | Structure validation (dup keys, slot type checks) |
| 3 | `validate_slot_data(config, data)` | Slot value validation against the parsed config |
| 4 | `render_templates(templates, data, config)` | Tera template rendering in memory — variable substitution, filename templating, `.j2` stripping |
| 5 | File writes | TypeScript writes rendered content to `poc/output/` |
| 6 | `evaluate_hooks(config, data)` | Hook execution plan — which hooks would run, with `hook_ran_*` state injection for conditionals |

## Changes to spackle for WASM

All changes are additive — the existing CLI and native API are untouched.

**Cargo.toml:**
- Added `cdylib` to `crate-type` (needed for wasm-pack)
- Added `wasm` feature gating `wasm-bindgen`, `serde_json`,
  `serde-wasm-bindgen`, `console_error_panic_hook`
- Moved native-only deps (`async-process`, `polyjuice`, `users`,
  `tokio`, `walkdir`, etc.) into
  `[target.'cfg(not(target_arch = "wasm32"))'.dependencies]`
- Added `getrandom` v0.2 with `js` feature to fix a transitive dep
  from `tera` → `rand`

**Source — cfg gates (`#[cfg(not(target_arch = "wasm32"))]`):**
- `lib.rs`: `copy` module, `Project::generate/check/run_hooks*`,
  `GenerateError`, `CheckError`, `RunHooksError`, `load_project`
- `hook.rs`: process-spawning imports, `run_hooks_stream`, `run_hooks`,
  `HookResult`, `HookResultKind`, `HookError`, `Error`

**Source — new in-memory entry points:**
- `config::parse(toml_str)` — parse without filesystem
- `template::render_in_memory(templates, data)` — tera rendering from
  a `HashMap<String, String>` instead of a glob
- `template::validate_in_memory(templates, slots)` — check template
  variable references against slot keys
- `hook::evaluate_hook_plan(hooks, slots, data)` — resolve needs,
  evaluate conditionals with `hook_ran_*` injection, template command
  args, surface errors as `should_run=false`

**Source — WASM bindings (`src/wasm.rs`):**

Ten `#[wasm_bindgen]` exports, all JSON-in/JSON-out:

| Export | Purpose |
|--------|---------|
| `init()` | Panic hook for browser/bun console |
| `parse_config(toml)` | → `{ name, ignore, slots, hooks }` |
| `validate_config(toml)` | → `{ valid, errors }` |
| `check_project(toml, templates_json)` | Full check: config + slots + template refs |
| `validate_slot_data(config_json, slot_data_json)` | → `{ valid, errors }` |
| `render_templates(templates_json, slot_data_json, config_json)` | → `[{ original_path, rendered_path, content }]` |
| `evaluate_hooks(config_json, slot_data_json)` | → `[{ key, command, should_run, skip_reason, template_errors }]` |
| `render_string(template, data_json)` | One-off tera render — used by the host for filename-templated non-`.j2` files |
| `get_output_name(out_dir)` | Mirrors `crate::get_output_name` — derives project name from an output dir |
| `get_project_name(config_json, project_dir)` | Mirrors `Project::get_name` — config.name ?? project_dir file_stem |

**Tests:**
- `cargo test` (no features): 39 tests, all existing + new in-memory
  function tests. No WASM-specific tests run.
- `cargo test --features wasm` (or `just test-wasm`): 44 tests, adds
  WASM JSON contract tests (`check_project`, `evaluate_hooks`,
  `get_output_name`, `get_project_name`, `render_string`).
- `bun test` (or `just test-poc`): 30 tests across
  `poc/tests/{wasm,host,e2e}.test.ts` — proves the WASM artifact is
  loadable from Bun and that orchestration produces spackle-equivalent
  output including hook subprocess execution.

## Behavioral notes

**`evaluate_hooks` matches native failure semantics.** When a hook's
command template fails to render (e.g. references an undefined variable),
the hook is marked `should_run: false` with `skip_reason: "template_error"`
and `hook_ran_<key>` is NOT flipped to `true`. Downstream hooks that
depend on `{{ hook_ran_<key> }}` correctly see `false`. This matches
`run_hooks_stream()` which treats template errors as hard errors before
execution.

**`check_project` response shape is always `{ valid, errors }`.** Even
when the input JSON is malformed, the response uses the same shape
(with the parse error in the `errors` array) so TypeScript callers
never need to branch on response structure.

**`render_templates` preserves the original template path.** Each
success entry has `original_path` (e.g. `{{slot_1}}.j2`) distinct from
`rendered_path` (e.g. `hello`) so callers can map output back to source.

## Why not WASI?

WASI (WebAssembly System Interface) is a standardized set of syscalls
for WASM modules that includes filesystem access, environment variables,
and clocks. It's a reasonable question whether spackle could target WASI
instead of `wasm32-unknown-unknown` and skip the TypeScript I/O layer.

| Target | Filesystem | Processes | Clock | Maturity |
|--------|-----------|-----------|-------|----------|
| `wasm32-unknown-unknown` | no | no | no | Stable, well-supported |
| `wasm32-wasip1` | yes (sandboxed) | no | yes | Stable in Rust, runtime support varies |
| `wasm32-wasip2` (component model) | yes | no | yes | Experimental |

With WASI, the host (Bun, wasmtime, etc.) grants the WASM module access
to specific directories at startup. Rust's `std::fs` just works —
`fs::read_to_string`, `walkdir`, all of it — because the Rust standard
library has a WASI backend. `config::load_dir`, `template::fill`, and
`copy::copy` would all compile and work since they only use `std::fs` +
`walkdir` + `tera`.

**What WASI still can't do:** spawn processes. There's no `fork`/`exec`
equivalent in any WASI version. Hooks would still need the TypeScript
host regardless.

**Why we didn't go that route:**

- **wasm-pack doesn't support WASI.** It only targets
  `wasm32-unknown-unknown`. A WASI build needs a different pipeline
  (plain `cargo build` + manual JS glue or `jco` for the component
  model).
- **Bun's WASI support is experimental.** `Bun.WASI` exists but is
  flagged unstable — the API has changed between releases.
- **The split we have is cleaner.** TypeScript handling I/O and Rust
  handling computation is a natural boundary. If you give WASM
  filesystem access, the question becomes "why not just ship a native
  binary?" — and at that point you're back to the original spackle CLI.

If the WASI toolchain matures (wasm-pack support, stable Bun runtime),
revisiting this could collapse the I/O layer. But for now,
`wasm32-unknown-unknown` + TypeScript host is the pragmatic choice.

## Consuming the WASM artifact from another repo

Every tagged release attaches a tarball with all three wasm-pack targets
(`web`, `nodejs`, `bundler`) as a single archive. Install with Bun:

```bash
bun add https://github.com/a2-ai/spackle/releases/download/v<X.Y.Z>/spackle-wasm-v<X.Y.Z>.tgz
```

The tarball extracts to `web/`, `nodejs/`, and `bundler/` — import from
the one that matches your runtime. For a Bun server, `nodejs/` is
usually cleanest; for a Vite/TanStack Start app, use `bundler/` with
`vite-plugin-wasm`.

The `poc/src/wasm/index.ts` wrapper + `poc/src/host/` helpers are
designed to be lifted verbatim into a consumer repo — treat them as the
reference integration.

## Next steps (follow-up PRs)

### 1. Scaffold `spackle-web/` from vibestack-starter

Clone `vibestack-starter` into `spackle-web/`. Strip the demo routes
(`files-loader`, `files-cache`, `server-fn`). Rename references from
`vibestack-starter` to `spackle-web`. Run `bun install`.

### 2. Lift the PoC helpers into `spackle-web/src/`

- `poc/src/wasm/*` → `spackle-web/src/server/wasm/*` (server-side, loaded
  once per process).
- `poc/src/host/*` → `spackle-web/src/server/host/*`.
- `poc/src/spackle.ts` → `spackle-web/src/server/spackle.ts` — called
  from TanStack Start `createServerFn` handlers.

Consider adding Zod schemas on top of the hand-written interfaces in
`wasm/types.ts` when the WASM boundary starts serving client-triggered
requests (runtime validation on untrusted input).

### 3. Server functions

TanStack Start `createServerFn` handlers that wrap the orchestration:

- **`getProjectInfo`** — `check()` on a project directory.
- **`validateSlotData`** — proxy for `wasm.validateSlotData`.
- **`generateProject`** — `generate(..., { runHooks: true })`, streaming
  each `HookOutcome` back to the client over SSE.
- **`listTemplates`** — read the template root directory.

### 4. Routes

File-based routes following vibestack-starter conventions:

- `src/routes/index.tsx` — template list.
- `src/routes/templates/$slug.tsx` — template detail, slot form, generate.
- `src/routes/templates/$slug.edit.tsx` — file tree editor.

### 5. Streaming hook output to the client

`executeHookPlan` already yields one outcome per hook as an async
iterator — hook it up to an SSE stream or `ReadableStream` from the
server function so the UI can render hook stdout as it arrives, rather
than waiting for the whole batch.
