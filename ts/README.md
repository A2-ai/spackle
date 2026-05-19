# @a2-ai/spackle

spackle project templating as an ESM WebAssembly module. Runs in modern browsers and Bun.

---

## Install

Not published to npm. Pick the route that matches how you're iterating, from fastest loop to most published-like:

### 1. Local dev-loop — `bun link`

`just setup` (or `just init`) already registers `@a2-ai/spackle` for linking. If you need to redo it manually:

```bash
cd ts && bun link
```

Then in the consuming project:

```bash
bun link @a2-ai/spackle
```

Rebuilds reflect immediately (the consumer points at the symlink). Best when both repos are on the same machine.

### 2. Local tarball QA — `bun pm pack`

Simulates exactly what an installed user would see. From this repo:

```bash
cd ts && bun pm pack --destination /tmp
```

Then in the consumer:

```bash
bun add /tmp/a2-ai-spackle-<version>.tgz
```

### 3. Pre-release for teammates — GitHub release asset

Tag a pre-release (e.g. `vX.Y.Z-dev.N`). The `build.yaml` workflow produces the `.tgz` and attaches it to the release. Consumers:

```bash
bun add https://github.com/a2-ai/spackle/releases/download/<tag>/a2-ai-spackle-<version>.tgz
```

### 4. S3 artifacts bucket

If a CI step pushes the tarball to the team's S3 artifacts bucket, consumers with `aws` creds that have read access:

```bash
aws s3 cp s3://<bucket>/<path>/a2-ai-spackle-<version>.tgz /tmp/
bun add /tmp/a2-ai-spackle-<version>.tgz
```

### Not supported: `bun add git+ssh://...`

`package.json` lives at `ts/`, not the repo root, and neither bun nor npm supports subpath specifiers on git URLs — so `bun add git+ssh://git@github.com/a2-ai/spackle.git#<ref>` does **not** work today. Use one of the routes above.

See [`docs/ts/getting-started.md`](../docs/ts/getting-started.md) for a usage walkthrough.

---

## Pick the right adapter

The library ships two host-side helpers. Use the one matching where your bytes actually live.

### `DiskFs` — server-side, reads/writes real files

Use when your projects live on disk and you want generated output written back to disk.

- Reads a project directory into a bundle via `readProject`.
- Streams each rendered entry to disk via `writeEntry` (the `generate(...)` orchestrator uses this under the hood).
- Writes a buffered bundle out via `writeOutput` (for `generateBundle` / `render` outputs already in memory).
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
- [Custom host](../docs/ts/custom-host.md) — bundle readers for S3 / git / anything else
- [Hooks](../docs/ts/hooks.md) — `runHooksStream` / `planHooks`, `SpackleHooks` executor contract, SSE bridging
- Runnable example: [`examples/ts/bun-script/`](../examples/ts/bun-script/)

---

## Known limitations

- **Browser hosts need a custom `SpackleHooks`.** `runHooksStream()` uses `defaultHooks()` which picks `BunHooks` / `NodeHooks` at runtime; a browser throws with a clear message. Supply a custom executor (e.g. one that posts to a backend) to run hooks there. See [hooks docs](../docs/ts/hooks.md).
- **UTF-8 paths only.**
- **Input bundle and template render pass are buffered.** The `generate(...)` write side streams to disk through `DiskFs.writeEntry`, but the input project bundle is read in full before the wasm call and spackle core renders all `.j2` files into a `Vec<RenderedFile>` before the per-file write loop. Fine for KB–MB templates; very large fixtures need an input-side / render-side streaming path that doesn't exist yet.
