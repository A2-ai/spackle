# @a2-ai/spackle-wasm

spackle project templating, compiled to WebAssembly. Drop it into Node, Bun, or a browser and generate filled project trees from TOML-configured templates.

---

## Install

`@a2-ai/spackle-wasm` is **not published to npm**. It ships as a tarball attached to each [GitHub release](https://github.com/a2-ai/spackle/releases) of the `spackle` repo, produced by the release pipeline (`wasm-pack` × 3 targets + TS dist, packed via `bun pm pack`).

Install from a release asset URL:

```bash
bun add https://github.com/a2-ai/spackle/releases/download/<tag>/a2-ai-spackle-<version>.tgz
# npm works the same way:
npm install https://github.com/a2-ai/spackle/releases/download/<tag>/a2-ai-spackle-<version>.tgz
```

Or directly from the git repo (builds from source — needs `wasm-pack` + `bun` on the install host):

```bash
bun add git+ssh://git@github.com/a2-ai/spackle.git#<ref>
```

Pin by tag / commit SHA for reproducibility.

---

## Quickstart

```ts
import { DiskFs, generate } from "@a2-ai/spackle-wasm";

const fs = new DiskFs({ workspaceRoot: "/var/workspace" });

const result = await generate(
    "/var/workspace/my-template",
    "/var/workspace/generated/abc-123",
    { project_name: "hello", owner: "you" },
    fs,
);

if (result.ok) {
    console.log(`Wrote ${result.files.length} files`);
} else {
    console.error(result.error);
}
```

In-memory preview (no disk I/O):

```ts
import { MemoryFs, generateBundle } from "@a2-ai/spackle-wasm";

const bundle = new MemoryFs({
    files: {
        "/project/spackle.toml": "...",
        "/project/README.md.j2": "# {{ project_name }}",
    },
}).toBundle();

const result = await generateBundle(bundle, { project_name: "demo" });
```

---

## Architecture in one paragraph

Rust runs the full spackle generation pipeline against an in-process virtual filesystem. The TS host reads a project tree into a bundle (`Array<{path, bytes: Uint8Array}>`), calls wasm, gets back the rendered output bundle, and writes it to disk (or wherever). No callbacks cross the wasm boundary — it's a pure compute step. The `DiskFs` / `MemoryFs` classes are pure TS helpers for moving bundles in and out.

For contract details, see:

- [`/docs/wasm/getting-started.md`](../docs/wasm/getting-started.md) — install + minimal example
- [`/docs/wasm/api.md`](../docs/wasm/api.md) — full API reference
- [`/docs/wasm/runtime-targets.md`](../docs/wasm/runtime-targets.md) — Node vs browser vs bundler
- [`/docs/wasm/custom-host.md`](../docs/wasm/custom-host.md) — implement a bundle reader for S3 / git / custom storage
- [`/docs/wasm/hooks.md`](../docs/wasm/hooks.md) — hook execution status (deferred)

---

## Runtime targets

Three wasm-pack outputs ship in `pkg/`:

| Target | Import path | When to use |
|---|---|---|
| nodejs | `@a2-ai/spackle-wasm` (default) | Node, Bun, Deno-compat |
| web | `@a2-ai/spackle-wasm/pkg/web` | Browsers fetching the `.wasm` directly |
| bundler | `@a2-ai/spackle-wasm/pkg/bundler` | webpack / vite pipelines |

Default import path (no suffix) hits the nodejs target.

---

## Known limitations

- **Hooks unsupported** this milestone. `generate(..., { runHooks: true })` returns `{ ok: false, error: "hooks are unsupported in this milestone" }`.
- **UTF-8 paths only.** The bundle boundary doesn't round-trip non-UTF-8 filenames.
- **Whole-project marshalling.** Input and output bundles live in memory during the call. Fine for typical templates (KB–MB); very large fixtures should consider streaming (not yet offered).
