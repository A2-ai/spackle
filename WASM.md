# Spackle WASM Proof of Concept

## What this is

A minimal TypeScript script (`poc/index.ts`) that exercises spackle's core
logic compiled to WASM. Run it with:

```bash
just poc          # builds WASM + runs the script
# or manually:
just build-wasm   # wasm-pack → poc/pkg/
cd poc && bun ./index.ts
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
output files. Hook execution is planned via WASM (`evaluate_hooks`
returns which hooks would run and with what commands) but actual
subprocess spawning is not yet implemented in the PoC — that is a
future step for the full web app.

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

Seven `#[wasm_bindgen]` exports, all JSON-in/JSON-out:

| Export | Purpose |
|--------|---------|
| `init()` | Panic hook for browser/bun console |
| `parse_config(toml)` | → `{ name, ignore, slots, hooks }` |
| `validate_config(toml)` | → `{ valid, errors }` |
| `check_project(toml, templates_json)` | Full check: config + slots + template refs |
| `validate_slot_data(config_json, slot_data_json)` | → `{ valid, errors }` |
| `render_templates(templates_json, slot_data_json, config_json)` | → `[{ original_path, rendered_path, content }]` |
| `evaluate_hooks(config_json, slot_data_json)` | → `[{ key, command, should_run, skip_reason, template_errors }]` |

**Tests:**
- `cargo test` (no features): 39 tests, all existing + new in-memory
  function tests. No WASM-specific tests run.
- `cargo test --features wasm` (or `just test-wasm`): 41 tests, adds
  `check_project` JSON contract test and `evaluate_hooks` template-error
  semantics test.

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

## Next steps

### 1. Scaffold `spackle-web/` from vibestack-starter

Clone the vibestack-starter template into `spackle-web/`. Strip the demo
routes (`files-loader`, `files-cache`, `server-fn`). Rename references
from `vibestack-starter` to `spackle-web`. Run `bun install`.

### 2. Typed WASM wrapper (`spackle-web/src/server/spackleWasm.ts`)

Create a server-side module that:

- Loads the WASM module **once** on first call (not per-request).
- Defines Zod schemas for every WASM return shape (`SpackleConfig`,
  `ValidationResult`, `RenderedTemplate[]`, `HookPlanEntry[]`).
- Exports typed async methods — `parseConfig`, `checkProject`,
  `validateSlotData`, `renderTemplates`, `evaluateHooks` — that call
  the raw WASM exports internally and parse the JSON through Zod.
- No raw JSON strings leak past this boundary. The rest of the server
  works with typed TypeScript objects.

The `just build-wasm` output (`poc/pkg/` or a new `spackle-web/pkg/`)
is the input to this module.

### 3. Server functions

TanStack Start `createServerFn` handlers that compose the typed WASM
wrapper with host I/O:

- **`getProjectInfo`** — read `spackle.toml` from disk → `parseConfig`
- **`checkProject`** — read `spackle.toml` + walk `.j2` files → `checkProject`
- **`validateSlotData`** — `parseConfig` → `validateSlotData`
- **`generateProject`** — read templates → `renderTemplates` → write
  output files to disk → `evaluateHooks` → spawn commands via
  `Bun.spawn` for each hook with `should_run: true`
- **`listTemplates`** — read the template root directory

### 4. Routes

File-based routes following vibestack-starter conventions:

- `src/routes/index.tsx` — template list (cards, "All" / "My templates")
- `src/routes/templates/$slug.tsx` — template detail, slot form, generate
- `src/routes/templates/$slug.edit.tsx` — file tree editor, save as new
  version

### 5. Hook execution

The PoC currently plans hooks (`evaluate_hooks`) but does not execute
them. The server function in step 3 (`generateProject`) will:

1. Call `evaluateHooks` to get the plan.
2. For each entry with `should_run: true`, call `Bun.spawn(entry.command, { cwd: outDir })`.
3. Capture stdout/stderr and stream status back to the client (SSE or
   polling — TBD based on TanStack Start capabilities).
