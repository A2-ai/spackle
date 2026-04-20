# poc — reference integrations

**This is not a shipped library.** It's reference TypeScript code showing
how to load and call the spackle wasip2 component from a JS runtime.
Consumers building their own integration should **lift and adapt** this
code; it's not versioned or packaged as a dependency.

For the full wasip2 architecture, the adapter contract that any loader
must honor, and the spike that produced this reference, see
[`../WASM.md`](../WASM.md).

## Supported runtime matrix

| Runtime | Version | Status | Command | Loader |
|---|---|---|---|---|
| Node | 22.22.2 | ✓ verified | `just test-wasip2` | `src/wasip2/index.ts` |
| Bun | 1.2.8 | ✓ verified (incl. `bun build --compile`) | `just test-wasip2-bun` | `src/wasip2/bun.ts` |

Older versions may work but aren't exercised by CI. Anything not listed
is undocumented territory — a consumer would port the reference.

## Directory layout

```
poc/
├── src/
│   ├── wasm/            # wasm-pack reference (DEPRECATED, retained for parity)
│   ├── host/            # host helpers shared by the wasm-pack path
│   ├── spackle.ts       # wasm-pack orchestration entry
│   └── wasip2/          # wasip2 component reference (primary)
│       ├── index.ts     # Node loader (uses @bytecodealliance/preview2-shim)
│       ├── bun.ts       # Bun loader (uses the custom shim below)
│       └── bun-wasi.ts  # minimal Bun-native WASI import layer
├── tests/
│   ├── e2e.test.ts          # wasm-pack e2e (bun test)
│   ├── host.test.ts         # wasm-pack host helpers (bun test)
│   ├── wasm.test.ts         # wasm-pack JSON contract (bun test)
│   └── wasip2/
│       ├── component.test.mjs       # Node smoke tests (node --test)
│       ├── component.bun.test.ts    # Bun smoke tests (bun test)
│       └── smoke-compile.ts         # compile-mode smoke for `bun build --compile`
├── wasip2-pkg/          # generated — default jco output (preview2-shim)
├── wasip2-pkg-no-shim/  # generated — jco --no-wasi-shim output (for Bun)
└── pkg/                 # generated — wasm-pack output (deprecated)
```

`wasip2-pkg*/` and `pkg/` are gitignored — regenerate with the build
commands below.

## Commands

```bash
# Rebuild everything the reference depends on.
just build-wasip2        # cargo-component + BOTH jco variants
just build-wasm          # wasm-pack (deprecated)

# Run the reference on each runtime.
just test-wasip2         # Node path: 6 smoke tests
just test-wasip2-bun     # Bun path: 6 smoke tests + bun build --compile smoke
just test-poc            # wasm-pack path: 36 existing tests
```

## Known caveats

See [`../WASM.md`](../WASM.md) for full treatment. Short list:

1. **Node and Bun need different jco outputs.** Node uses the default
   `wasip2-pkg/` (preview2-shim); Bun uses `wasip2-pkg-no-shim/` +
   `bun-wasi.ts` (because preview2-shim's worker thread calls
   `process.binding("tcp_wrap")`, which Bun doesn't implement).
2. **Tera builtins are disabled under wasip2.** `slugify`, `date`,
   `filesizeformat`, `urlencode`, and rand-based filters are not
   available to templates rendered through the component. Fixtures
   here don't use them.
3. **`run-command` is synchronous** — host implementations (`Bun.spawnSync`
   / `child_process.spawnSync`) block the event loop for the duration
   of each hook. Document this to your consumers; a real server running
   long-lived hooks concurrently will want a worker thread or
   (eventually) async WIT.
4. **`bun build --compile` needs asset imports, not `readFile`.** The
   Bun loader uses `import x from "./core.wasm" with { type: "file" }`
   so compile-mode bundles the `.core.wasm` files into the binary. A
   plain `readFile` path works under `bun run` but fails silently in
   compile-mode — the compile-mode smoke in `test-wasip2-bun` is what
   catches that drift.

5. **jco's host-side runtime requires `.payload` on thrown errors.**
   When a WASI host function (e.g. `Descriptor.statAt`) throws, jco's
   generated code calls `getErrorPayload(e)`. If `e instanceof Error` and
   has no `.payload` property, the error is **re-thrown** instead of
   being encoded as the WIT error variant. Meaning: a plain
   `throw new Error("no such file")` from your shim becomes an uncaught
   exception in the component, not a `result<T, error-code>` error.
   `bun-wasi.ts` handles this by defining `WasiError` with an explicit
   `payload: string` field matching the WIT error-code enum values
   (`"no-entry"`, `"access"`, etc.). Any new host function added to a
   shim must follow the same pattern or it'll crash on expected errors.

6. **Path containment in `bun-wasi.ts` uses canonicalize-then-check.**
   `resolveChild` runs `fs.realpathSync` on the target (or its parent
   for create flows), then checks the result is inside the preopen root
   with a separator-terminated prefix match (`startsWith(root + path.sep)`).
   This defends against sibling-prefix collisions, `../` traversal, and
   symlink escapes inside the workspace. **Residual hazard:** TOCTOU —
   between canonicalize and the actual `openSync`, an attacker with
   write access to the workspace could swap a file for a symlink.
   Userspace JS can't fully close this; servers accepting untrusted
   templates should layer OS-level sandboxing (Landlock on Linux, App
   Sandbox on macOS, a container). See `tests/wasip2/bun-wasi-containment.test.ts`
   for the positive and negative cases the shim enforces.

## Porting to another runtime

Start from the runtime closest to yours:

- Has Node-style `@bytecodealliance/preview2-shim` support → copy
  `src/wasip2/index.ts`, adjust paths, replace `child_process.spawnSync`
  with your runtime's sync subprocess primitive.
- Doesn't → copy `src/wasip2/bun.ts` + `bun-wasi.ts`. The custom WASI
  shim is narrow (fs + clocks + cli + random + io); reimplement any
  runtime-specific pieces (sync fs calls, asset embedding for
  compile/bundle modes, subprocess spawn) for your target.

The adapter contract (WIT imports, env merging, path handling,
preopens) is documented in [`../WASM.md`](../WASM.md). Honor it and a
new runtime's loader should behave identically to these two.
