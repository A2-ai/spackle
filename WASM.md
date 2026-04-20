# Spackle WASM

This repo ships **two** WASM build pipelines:

| Path | Target | Purpose | Status |
|---|---|---|---|
| **wasip2** (`[features] wasip2`) | `wasm32-wasip2` + wit-bindgen + jco | Server-side consumption. Rust does fs I/O inside WASI's sandbox; host provides subprocess spawn. | **Primary direction.** |
| wasm-pack (`[features] wasm`) | `wasm32-unknown-unknown` + wasm-bindgen | Originally scoped for a browser UI. | Deprecated — kept compiling alongside wasip2 until consumers migrate. |

**No browser UI is planned.** The eventual consumer is a TypeScript
server that wraps spackle behind request/response endpoints. The WASM
module is as self-contained as possible; the TS side is a thin harness.
See "Integrating with spackle-ui" at the bottom for the in-flight server
plan.

---

## Decision record: reference runtime

**Date:** 2026-04-20

**Spike scope:** `jco transpile --no-wasi-shim` + a minimal custom Bun
WASI shim at `poc/src/wasip2/bun-wasi.ts`. Time-boxed to ~2 hours.

**Outcome: PASS.** Both spike criteria met:
1. `bun test poc/tests/wasip2/component.bun.test.ts` — all 6 smoke tests
   pass under `bun run`.
2. `bun build --compile` produces a standalone binary that runs
   `check()` against the `proj2` fixture and exits 0 — validated the
   asset-embedded path, which `bun run` alone would miss.

**Verified reference runtimes today:**

| Runtime | Version tested | Reproduce |
|---|---|---|
| Node | 22.22.2 | `just test-wasip2` |
| Bun | 1.2.8 | `just test-wasip2-bun` |

**Reasoning.** The initial blocker was
`@bytecodealliance/preview2-shim` — the default WASI layer jco injects
eagerly imports `process.binding("tcp_wrap")` from a worker thread,
which Bun doesn't implement. `jco transpile --no-wasi-shim` disables
that injection, leaving us to provide WASI imports ourselves. Because
the component only needs a narrow slice of WASI (fs + clocks + cli +
random + io — no sockets, no HTTP), a sync Bun-native shim using
`fs.readSync` / `fs.writeSync` was tractable (~400 LOC) with no workers
and no `SharedArrayBuffer`. The compiled-binary check additionally
caught an asset-embedding drift (`.core.wasm` files must be imported
with `type: "file"`, not read via `readFile`) that wouldn't have shown
up under `bun run`. Both issues are resolved in
`poc/src/wasip2/bun.ts`.

**Revisit when:** Bun ships `process.binding("tcp_wrap")` or
`@bytecodealliance/preview2-shim` stops hard-depending on it — at that
point the custom shim becomes optional rather than required, and the
two jco builds (`wasip2-pkg/`, `wasip2-pkg-no-shim/`) could collapse to
one. Also revisit on major jco version bumps (drift in
`--no-wasi-shim` output shape would surface in the compile-mode smoke).

---

## Quickstart

```bash
# Build the wasip2 component + both jco variants (default + --no-wasi-shim).
just build-wasip2

# Node reference (preview2-shim-based jco output, 6 node:test cases).
just test-wasip2

# Bun reference (custom shim, 6 `bun test` cases + compile-mode smoke).
just test-wasip2-bun

# Deprecated wasm-pack path (still green until consumers migrate).
just build-wasm && just test-poc
```

---

## wasip2 architecture (primary)

The component exports three functions and imports one host capability:

```
┌───────────────────────────────────────────────┐
│  WASI component (compiled Rust)               │
│                                               │
│  Exports (a2ai:spackle/api):                  │
│   • check(project-dir) → JSON                 │
│   • validate-slot-data(project-dir, data)     │
│   • generate(project-dir, out-dir, data,      │
│              run-hooks) → JSON                │
│                                               │
│  Rust uses `std::fs` inside the WASI sandbox  │
│  (preopens limit what can be opened).         │
│                                               │
│  Imports (a2ai:spackle/host):                 │
│   • run-command(cmd, args, cwd, env) → result │
│     — used by hook execution                  │
└──────────────┬────────────────────────────────┘
               │ sync host call
┌──────────────▼────────────────────────────────┐
│  TypeScript harness (poc/src/wasip2/)         │
│                                               │
│   • Compile core wasm ONCE at startup         │
│   • Per request: fresh instantiation with     │
│     workspace-parent preopen                  │
│   • runCommand: Bun.spawnSync / Node          │
│     child_process.spawnSync                   │
└───────────────────────────────────────────────┘
```

### Why `wasm32-wasip2` + wit-bindgen + jco

- **wasm-pack doesn't target WASI.** wasm-bindgen's JS glue is browser-
  oriented — no filesystem, no syscalls. For server-side we wanted
  actual fs access.
- **Component model (wasip2)** gives us a clean WIT interface with
  imports + exports. Rust side uses `wit-bindgen` (via `cargo-component`);
  TS side uses `jco transpile` to produce portable JS that drives the
  core wasm.
- **Portable artifact, runtime-specific glue.** The `.wasm` component is
  standard and runs under any component-model-capable JS runtime *in
  principle*. In practice each runtime needs a WASI import layer, and
  the default jco-injected layer (`@bytecodealliance/preview2-shim`)
  works on Node but not Bun. We ship two jco builds — default for Node
  (`poc/wasip2-pkg/`) and `--no-wasi-shim` for the custom Bun shim
  (`poc/wasip2-pkg-no-shim/`). See the decision record above.

### Preopen & instantiation model

Preopens are configured when the WASI context is built, and the
component sees the filesystem through that lens. For a server where
`projectDir` and `outDir` vary per request:

- **Compile core wasm once** at process startup (the expensive step).
- **Instantiate per request** with a fresh WASI context. Cheap.
- **Preopen the workspace parent**, not `outDir` itself. `Project::generate`
  creates `outDir` and errors if it already exists — you can't preopen
  a path that doesn't exist. The server maintains a workspace root;
  both `projectDir` and `outDir` live under it.

See `poc/src/wasip2/index.ts` for the Node reference loader, and
`poc/src/wasip2/bun.ts` + `bun-wasi.ts` for the Bun reference loader
(includes the custom WASI shim).

### Hook execution (host-imported subprocess)

WASI of any version can't spawn processes. The Rust side calls a WIT-
imported `run-command(cmd, args, cwd, env) -> command-result`, and the
TS harness implements it with `Bun.spawnSync` (or `node:child_process`
`spawnSync`).

`src/hook_wasip2.rs` loops over the hook plan, invokes `run-command`
per hook, and threads `hook_ran_<key>` state forward across iterations
(so conditionals like `if = "{{ hook_ran_foo }}"` see the real outcome).

### Security posture (server responsibility)

The component puts primitives in place. The server wraps them:

- **Path constraints** — canonicalize `projectDir` and reject anything
  outside the workspace root before calling in. WASI's preopen sandbox
  is belt-and-suspenders: the component cannot open files outside the
  granted dirs even if an attacker tricked the Rust code.
- **Hook command policy** — hooks come from the template's `spackle.toml`.
  If the template is user-submitted, *any hook is arbitrary code exec
  on the server*. Run hooks in a constrained subprocess (non-root user,
  bounded cwd, no network) when serving untrusted templates. The
  `run-hooks: false` parameter is a first-class option for templates
  the server can't vouch for.

### Known gotchas

1. **Bun cannot use `@bytecodealliance/preview2-shim` (default jco output).**
   The shim's I/O layer eagerly imports `process.binding("tcp_wrap")`
   from a Worker thread — Bun doesn't implement that binding, the Worker
   dies on load, and every subsequent `synckit` call throws
   `Worker has been terminated`. **Worked around** by transpiling with
   `jco transpile --no-wasi-shim` into `poc/wasip2-pkg-no-shim/` and
   using the custom Bun WASI shim at `poc/src/wasip2/bun-wasi.ts` (see
   decision record). The default `poc/wasip2-pkg/` output is Node-only;
   the `wasip2-pkg-no-shim/` output + custom shim is Bun-only. Both are
   built by `just build-wasip2`.

2. **Tera `builtins` feature is off in the wasip2 build.** `builtins`
   pulls in `slug`, which unconditionally depends on wasm-bindgen on
   all wasm32 targets — incompatible with wit-bindgen. The wasip2 build
   uses `tera = { default-features = false }`, which drops the
   `slugify`, `date`, `filesizeformat`, `urlencode`, and rand-based
   filters. None of the existing fixtures use them; if a template
   needs one, it'll fail to render under the component while working
   fine natively.

3. **Sync `run-command` blocks the event loop.** Bun.spawnSync / Node
   spawnSync are fully synchronous from the host perspective. Fine for
   a PoC at one-request-at-a-time, but under concurrent load a slow
   hook blocks the whole thread. Mitigation path, in order of effort:
   (a) document `run-hooks: false` as the server default,
   (b) move hook execution into a worker thread so only the worker blocks,
   (c) migrate to async WIT when wit-bindgen + jco async paths stabilize.

---

## wasm-pack pipeline (deprecated)

Still compiles, still tested. Original PoC shape:

- Pure computation only in WASM (`parseConfig`, `validateConfig`,
  `checkProject`, `validateSlotData`, `renderTemplates`, `evaluateHooks`,
  `renderString`, `getOutputName`, `getProjectName`).
- TypeScript host does all I/O: walk template dir, read files, write
  outputs, spawn hooks.
- Release attaches an npm-style tarball (via `bun pm pack`) as a GitHub
  Release asset. This stays in place until wasip2 replaces it — the
  artifact shape for wasip2 releases (raw `.wasm` vs npm tarball of
  jco output) is still under discussion.

`poc/src/wasm/*`, `poc/src/host/*`, `poc/src/spackle.ts`, and
`poc/tests/{wasm,host,e2e}.test.ts` cover this path.

---

## Changes to spackle (shared infrastructure)

**Cargo.toml:**
- Dual features: `wasm` (wasm-bindgen) and `wasip2` (wit-bindgen).
  Mutually exclusive in practice (different targets) but orthogonal
  as cargo features.
- Native deps stay behind `[target.'cfg(not(target_arch = "wasm32"))']`
  (wasip2 and unknown both set `target_arch = "wasm32"`, so native-only
  deps are correctly excluded from both wasm builds).
- Target-specific blocks for tera and getrandom_02 so the wasm-pack
  build gets the JS crypto shim (needed) and the wasip2 build gets
  the WASI backend (getrandom's native) without pulling in wasm-bindgen.

**Source:**
- `src/api.rs` — pure JSON-in/JSON-out helpers. Shared by `src/wasm.rs`
  (wasm-bindgen wrappers) and `src/component.rs` (wit-bindgen Guest).
- `src/component.rs` — WIT `Guest` impl. Loads projects from disk,
  runs `Project::generate`, delegates hooks to `hook_wasip2::run_hooks_sync`.
- `src/hook_wasip2.rs` — sync hook executor. Reuses `Hook::evaluate_conditional`
  (made `pub(crate)`) to keep planning semantics identical to the
  native `run_hooks_stream`.
- `src/bindings.rs` — auto-generated by cargo-component from
  `wit/spackle.wit`. **Do not edit by hand.**
- Relaxed cfgs: `Project::check`, `generate`, `copy_files`,
  `render_templates`, `load_project` (and the `copy` module) are now
  `#[cfg(any(not(target_arch = "wasm32"), target_os = "wasi"))]`.
  `run_hooks_stream` / `run_hooks` stay strictly native (they need
  tokio + polyjuice).

**Tests:**
- `cargo test` — 39 tests (native only).
- `cargo test --features wasm` — 44 tests (adds wasm JSON contract tests).
- `just test-poc` — 36 `bun test` cases against the wasm-pack artifact.
- `just test-wasip2` — 6 `node --test` cases against the Node jco build.
- `just test-wasip2-bun` — 6 `bun test` cases against the `--no-wasi-shim`
  build + custom Bun shim, plus a `bun build --compile` smoke.

---

## Integrating with spackle-ui

The in-flight UI repo (a sibling `../spackle-ui/`) expects a server that
speaks HTTP/request-response backed by this component. That server
doesn't exist yet; this PR lands the component + reference integrations
(Node and Bun) that a future server would lift from. The integration
plan is authored separately.

`poc/` is **reference only** — see `poc/README.md` for the supported
runtime matrix and copy-paste commands.
