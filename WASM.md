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
