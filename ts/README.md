# @a2-ai/spackle

spackle project templating as a WebAssembly module. Runs in Node, Bun, and browsers.

---

## Install

Not published to npm. Distributed as a GitHub release asset:

```bash
bun add https://github.com/a2-ai/spackle/releases/download/<tag>/a2-ai-spackle-<version>.tgz
```

Or install from git directly (builds from source — requires `wasm-pack` + `bun` on the host):

```bash
bun add git+ssh://git@github.com/a2-ai/spackle.git#<ref>
```

Pin by tag or commit SHA. See [`docs/ts/getting-started.md`](../docs/ts/getting-started.md) for a walkthrough.

---

## Pick the right adapter

The library ships two host-side helpers. Use the one matching where your bytes actually live.

### `DiskFs` — server-side, reads/writes real files

Use when your projects live on disk and you want generated output written back to disk.

- Reads a project directory into a bundle via `readProject`.
- Writes a generate response back out via `writeOutput`.
- Enforces a workspace-root containment boundary; refuses paths that escape.
- Matches native `spackle generate`'s "outDir must not pre-exist" contract.

**Runtimes:** Node, Bun, Deno (compat mode). Needs `node:fs` — **not available in browsers**.

**Typical contexts:** CLI-like scripts, server-side scaffold-as-a-service, CI pipelines that produce artifacts.

### `MemoryFs` — in-memory, no disk

Use for preview flows, tests, or anywhere disk access is unavailable/undesirable.

- Pure TS `Map<path, bytes>` — no `node:fs` imports.
- Build bundles via `insertFile` / the `files:` seed option.
- Round-trip output via `MemoryFs.fromBundle(result.files, prefix)` for snapshotting.

**Runtimes:** anywhere — Node, Bun, browsers, service workers, Cloudflare Workers, Deno.

**Typical contexts:** browser-side live preview as a user edits slot values, sandboxed test fixtures, edge runtimes.

### Both together

You can mix: read a project from disk with `DiskFs.readProject`, inspect or mutate the bundle, hand it to `generateBundle` (the memory-only variant), then decide at the end whether to write via `DiskFs.writeOutput` or keep it in-memory.

---

## Docs

- [Getting started](../docs/ts/getting-started.md) — install + minimal example
- [API reference](../docs/ts/api.md) — shapes, options, response types
- [Runtime targets](../docs/ts/runtime-targets.md) — nodejs vs web vs bundler
- [Custom host](../docs/ts/custom-host.md) — bundle readers for S3 / git / anything else
- [Hooks](../docs/ts/hooks.md) — `runHooksStream` / `planHooks`, `SpackleHooks` executor contract, SSE bridging
- Runnable example: [`examples/wasm/bun-script/`](../examples/wasm/bun-script/)

---

## Known limitations

- **Browser hosts need a custom `SpackleHooks`.** `runHooksStream()` uses `defaultHooks()` which picks `BunHooks` / `NodeHooks` at runtime; a browser throws with a clear message. Supply a custom executor (e.g. one that posts to a backend) to run hooks there. See [hooks docs](../docs/ts/hooks.md).
- **UTF-8 paths only.**
- **Whole-project marshalling.** Bundles live in memory during the call; fine for KB–MB templates, no streaming path yet.
