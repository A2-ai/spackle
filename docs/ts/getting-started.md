# Getting started

`@a2-ai/spackle` is the spackle templating engine compiled to WebAssembly (web-target ESM), with a TypeScript orchestration layer. Runs in modern browsers and Bun. Install, point it at a project directory, hand it slot values, and it writes a filled project tree.

## Install

`@a2-ai/spackle` is **not published to the npm registry**. It's distributed as a tarball asset on each [GitHub release](https://github.com/a2-ai/spackle/releases):

```bash
bun add https://github.com/a2-ai/spackle/releases/download/<tag>/a2-ai-spackle-<version>.tgz
```

`npm install <same-url>` also works. Pin by tag or commit SHA for reproducibility.

For local dev iteration (`bun link`, local tarball), pre-release versions, or installing via the team's S3 artifacts bucket, see the [install menu in `ts/README.md`](../../ts/README.md#install). Note that `bun add git+ssh://...` against this monorepo is **not** supported — `package.json` lives at `ts/`, not the repo root, and JS package managers can't subpath into git URLs.

## Minimal Bun example

```ts
import { DiskFs, generate } from "@a2-ai/spackle";

const fs = new DiskFs({ workspaceRoot: "/var/workspace" });

const result = await generate(
    "/var/workspace/my-template",
    "/var/workspace/out",
    { project_name: "hello" },
    fs,
);

if (result.ok) {
    console.log(`Wrote ${result.files} file(s), ${result.dirs} dir(s).`);
} else {
    console.error(result.error);
}
```

`generate` walks `projectDir` on disk, calls the wasm per-file primitives (`renderFile` / `renderPath`) for templates and path placeholders, and stream-copies static files through `pipeline(createReadStream, createWriteStream)` so GB-scale assets never sit fully in memory. On success the response carries **counts**, not a bundle — if you need the rendered tree in memory, call `render` (the diagnostics-first preview) or read the output back from disk after the call.

`DiskFs` enforces the `workspaceRoot` containment boundary — both `projectDir` and `outDir` must resolve under it, or `DiskFs` refuses the call — and provides the per-file I/O helpers (`writeFile`, `streamCopy`, `readFile`) the orchestrator drives.

## What's a "project"?

A spackle project is a directory containing:

- `spackle.toml` — config: slot declarations, ignore patterns, optional hooks.
- Template files ending in `.j2` — rendered with Tera.
- Non-`.j2` files — copied verbatim.
- File/dir names can contain `{{ slot }}` placeholders — rendered too.

See the fixture at `tests/fixtures/basic_project/` in the repo for a minimal example, or the runnable [`examples/ts/bun-script`](../../examples/ts/bun-script/) for a complete flow.

## Next

- [API reference](./api.md) — full shapes for `check`, `validateSlotData`, `generate`, bundle entries, responses.
- [Custom host](./custom-host.md) — swap the disk-based bundle reader for S3, git, or anything else.
