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

- Enforces a workspace-root containment boundary; refuses paths that escape via `containProject` / `assertOutDirAvailable` / `containedJoin`.
- Per-file disk helpers (`readFile`, `writeFile`, `streamCopy`) that the `generate` orchestrator drives directly — static files traverse `pipeline(createReadStream, createWriteStream)` so GB-scale assets never sit fully in memory.
- Matches native `spackle generate`'s "outDir must not pre-exist" contract via `assertOutDirAvailable`.

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

You can mix: build an in-memory bundle from disk yourself (or via a custom reader from S3/git/etc.), and drive the per-file wasm primitives directly via `loadSpackleWasm()` to render it. There's no bundled `generateBundle` wrapper — see [`docs/ts/custom-host.md`](../docs/ts/custom-host.md) for the composition pattern.

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
- **Template render is buffered.** `generate(...)` walks the project disk-direct and stream-copies static files through `pipeline()`, so GB-scale static assets are fine. Template bodies still buffer fully in memory (Tera produces a `String` per template) — typical templates are KB-scale, so this rarely binds in practice, but very large `.j2` files remain a ceiling proportional to their rendered size.
