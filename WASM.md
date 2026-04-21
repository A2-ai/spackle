# Spackle WASM architecture

Spackle compiles to `wasm32-unknown-unknown` via wasm-bindgen. A JS
host provides a filesystem adapter implementing the `SpackleFs`
contract; Rust uses that adapter to read config, walk templates,
render outputs, and write generated files. The host does not pre-read
or post-write — its only job is to back the adapter.

For a running log of how we got here (four phases of refactor), see
[`SUMMARY.md`](SUMMARY.md). For reference adapters + runnable demo, see
[`poc/README.md`](poc/README.md). For the in-flight spec, see
[`plan.md`](plan.md).

---

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  JS host (Node or Bun)                                   │
│                                                          │
│  Implements SpackleFs:                                   │
│   • readFile, writeFile, createDirAll                    │
│   • listDir, copyFile, exists, stat                      │
│                                                          │
│  Reference impls in poc/src/host/:                       │
│   • DiskFs   — local disk rooted at a workspace          │
│   • MemoryFs — in-memory, preview/testing use            │
└────────────┬─────────────────────────────────────────────┘
             │  sync JS method calls
             │  (bytes + typed errors across the bridge)
┌────────────▼─────────────────────────────────────────────┐
│  wasm-bindgen boundary (src/wasm.rs)                     │
│                                                          │
│  Exports:                                                │
│   • check_with_fs(projectDir, fs)                        │
│   • validate_slot_data_with_fs(projectDir, json, fs)     │
│   • generate_with_fs(projectDir, outDir, json,           │
│                      runHooks, fs)                       │
│                                                          │
│  JsFs adapter (src/wasm_fs.rs) implements                │
│  FileSystem by calling back into the JS fs object.       │
└────────────┬─────────────────────────────────────────────┘
             │  FileSystem trait (core is backend-agnostic)
┌────────────▼─────────────────────────────────────────────┐
│  Rust core (spackle lib)                                 │
│                                                          │
│  Drives the whole generate flow:                         │
│   • config::load_dir(fs, projectDir)                     │
│   • copy::copy(fs, project, out, ignore, data)           │
│   • template::fill(fs, project, out, data)               │
│                                                          │
│  Every fs op goes through the trait — fs.read_file,      │
│  fs.write_file, fs.list_dir, etc. No std::fs in wasm.    │
└──────────────────────────────────────────────────────────┘
```

**Native CLI** uses the same core with a different backend: `StdFs`
wraps `std::fs` + `walkdir`. Selecting the backend is the only
difference between the two deployments; generation logic is shared.

---

## The adapter contract (`SpackleFs`)

TypeScript shape (from `poc/src/host/spackle-fs.ts`):

```ts
type SpackleFileType = "file" | "directory" | "symlink" | "other";

type SpackleFsErrorKind =
    | "not-found"
    | "permission-denied"
    | "already-exists"
    | "not-a-directory"
    | "is-a-directory"
    | "invalid-path"
    | "other";

interface SpackleFs {
    readFile(path: string): Uint8Array;
    writeFile(path: string, content: Uint8Array): void;
    createDirAll(path: string): void;
    listDir(path: string): Array<{ name: string; type: SpackleFileType }>;
    copyFile(src: string, dst: string): void;
    exists(path: string): boolean;
    stat(path: string): { type: SpackleFileType; size: number | bigint };
}
```

### Method semantics

- **All methods are synchronous.** The bridge is built around direct
  wasm-bindgen function calls — no `await`, no Promise. If an adapter
  returns a Promise, `JsFs` sees the Promise object itself, not the
  resolved value, and decoding will fail.
- **`readFile` / `writeFile` / `copyFile`** round-trip whole files.
  Streaming isn't supported; templates and config fit comfortably in
  memory per-file.
- **`createDirAll`** is recursive like `mkdir -p`. Existing target
  is a no-op, not an error.
- **`listDir`** returns immediate children only. Rust handles
  recursion through `listDir` + `stat` combined.
- **`exists`** is best-effort. Returns `false` on any error — matches
  `Path::exists` semantics in std. Don't rely on it as a security
  boundary; use `stat` and catch the `not-found` kind.
- **`stat`** surfaces `file-type` and `size`. Size may be either
  `number` or `bigint` — `JsFs` accepts both.

### Error contract

Adapters throw `{ kind: SpackleFsErrorKind, message: string }` on
failure. Not a plain `Error` — the `kind` field is what Rust needs to
map to `io::ErrorKind` via `src/wasm_fs_kind.rs`. Unknown kinds or
untyped errors collapse to `io::ErrorKind::Other` with the stringified
message, which works but loses the type info downstream code uses
(`Project::generate`'s `AlreadyExists` branch, for example).

Use the `fsError(kind, message)` helper exported from
`poc/src/host/spackle-fs.ts` to construct errors with the right
shape. Or for Node/Bun adapters, use `errorKindFromNodeCode(e.code)`
to map `ENOENT`/`EACCES`/etc. automatically.

### Path contract

- **Paths are absolute strings.** Adapters reject relative paths with
  `invalid-path`. `JsFs` never passes relative paths; the contract
  still pins this because any host-side path sanitization needs to
  enforce it.
- **Containment is the adapter's responsibility.** `DiskFs` rejects
  paths that canonicalize outside its `workspaceRoot`. Custom
  adapters must implement their own containment semantics (or
  decide they don't need any, like `MemoryFs`).
- **Symlink handling.** `stat` surfaces symlinks without following
  (`type: "symlink"`); `listDir` same. `readFile` / `writeFile`
  follow symlinks transparently (via the underlying fs APIs). If
  containment matters, canonicalize before the containment check.

See `poc/src/host/disk-fs.ts` for a reference implementation that
handles all of this, including the mkdir-p-aware canonicalize
strategy flagged in SUMMARY.md as load-bearing.

---

## wasm exports

Three fs-backed functions plus `init`. All signatures in
`src/wasm.rs`.

### `check_with_fs(projectDir, fs)`

Validates a project: loads `spackle.toml`, checks slot structure,
validates template references against slot keys.

**Response:**

```json
// success
{ "valid": true, "config": { "name": ..., "ignore": [...], "slots": [...], "hooks": [...] }, "errors": [] }

// failure
{ "valid": false, "errors": ["<error msg>", ...] }
```

The parsed config surfaces on success so UIs can render forms for
slot data without re-parsing TOML host-side.

### `validate_slot_data_with_fs(projectDir, slotDataJson, fs)`

Validates submitted slot data against the project's config. Rust
reads the config via `fs` — the host never handles TOML.

**Response:**

```json
// success
{ "valid": true }

// failure
{ "valid": false, "errors": [...] }
```

### `generate_with_fs(projectDir, outDir, slotDataJson, runHooks, fs)`

Runs the full generation: loads config, walks templates, renders,
writes rendered + copied files to `outDir`.

**Response:**

```json
// success
{ "ok": true, "rendered": [{ "original_path": "...", "rendered_path": "..." }, ...] }

// failure
{ "ok": false, "error": "<msg>" }
```

**`runHooks = true` is currently unsupported** — returns an explicit
`{ "ok": false, "error": "hooks are unsupported in this milestone..." }`.
See "Hooks" below for the plan.

### `init()`

Sets up the panic hook so wasm panics surface with useful messages in
the host console. Called automatically on module load under
`wasm-pack build --target nodejs`; keep calling explicitly if a
future build uses `--target web`.

---

## Hooks (deferred)

Post-generation hooks (run `npm install`, `git init`, etc. after
rendering) are not supported in the current milestone. The reason
they're deferred, not dropped: hooks need to interact with the real
server environment — spawn real subprocesses, touch real paths — and
designing that interface properly requires the fs adapter model to
settle first. Retrofitting hooks into a sandboxed Rust-side view
(the WASI approach we set aside) creates more friction than it
removes.

The planned shape is a second JS-provided interface — call it
`SpackleHooks` — that mirrors `SpackleFs` but exposes
`runCommand(cmd, args, cwd, env) -> CommandResult`. The same
wasm-bindgen pattern will pick it up. No ETA; the hooks milestone
will be scoped separately when the adapter model has shaken out.

For now: `generate_with_fs(..., runHooks = true, ...)` returns an
explicit unsupported error. Downstream callers observe it, don't
silently lose hooks.

---

## Known behavior + caveats

1. **`--target nodejs` has no `init()` call from the host.** wasm-pack's
   nodejs output instantiates the wasm module eagerly at module load
   time. The `loadSpackleWasm()` loader in `poc/src/wasm/index.ts` keeps
   an async shape for symmetry with `--target web` output, but doesn't
   await anything underneath.
2. **`Path::canonicalize` is gone from the lib.** `Project::get_name`
   and `get_output_name` use `file_stem` / `file_name` directly. If a
   CLI caller passes a path like `.`, the output name will be `.` —
   not the current working directory's name as it would have been
   before. Document in the CLI if it's a problem; don't reintroduce
   canonicalize in the lib.
3. **`slugify` is an incidental export.** wasm-pack's bindings surface
   `slugify` because tera pulls in the `slug` crate which has its own
   wasm-bindgen exports. Harmless; not part of our contract.
4. **Path encoding is UTF-8 only.** `JsFs` converts Rust `Path`s via
   `to_string_lossy`. Non-UTF8 filenames will be mangled. Low
   probability in template source trees; document as a known limit.

---

## Layout

| Path | Role |
|---|---|
| `src/fs.rs` | `FileSystem` trait + `StdFs` (native) + `MockFs` (tests) + `walk()` helper |
| `src/wasm_fs.rs` | `JsFs` adapter — wasm32-only, implements `FileSystem` over a JS object |
| `src/wasm_fs_kind.rs` | Pure error-kind mapping (native-testable) |
| `src/wasm.rs` | wasm-bindgen exports — builds `JsFs` from the passed JS object, calls into the lib |
| `src/{lib,config,copy,template}.rs` | Generation logic, all generic over `FileSystem` |
| `poc/src/host/spackle-fs.ts` | Shared TS contract + error helpers |
| `poc/src/host/disk-fs.ts` | Reference disk-backed adapter |
| `poc/src/host/memory-fs.ts` | Reference in-memory adapter |
| `poc/src/wasm/index.ts` | Typed wrapper around the wasm-bindgen exports |
| `poc/src/spackle.ts` | Thin orchestrator — public entry points |
| `archive/wasip2-detour/` | Snapshot of the abandoned wasip2/component-model approach (not built, not tested) |
