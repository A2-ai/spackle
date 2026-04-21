# Getting started

`@a2-ai/spackle-wasm` is the spackle templating engine compiled to WebAssembly, with a TypeScript orchestration layer for Node, Bun, and browsers. Install, point it at a project directory, hand it slot values, and it writes a filled project tree.

## Install

`@a2-ai/spackle-wasm` is **not published to the npm registry**. It's distributed as a tarball asset on each [GitHub release](https://github.com/a2-ai/spackle/releases).

```bash
# From a pinned release asset:
bun add https://github.com/a2-ai/spackle/releases/download/<tag>/a2-ai-spackle-<version>.tgz

# Or directly from git (builds from source — needs wasm-pack + bun on the
# install host):
bun add git+ssh://git@github.com/a2-ai/spackle.git#<ref>
```

`npm install <same-url>` also works. Pin by tag or commit SHA for reproducibility.

## Minimal Bun example

```ts
import { DiskFs, generate } from "@a2-ai/spackle-wasm";

const fs = new DiskFs({ workspaceRoot: "/var/workspace" });

const result = await generate(
    "/var/workspace/my-template",
    "/var/workspace/out",
    { project_name: "hello" },
    fs,
);

if (result.ok) {
    console.log(`Wrote ${result.files.length} files.`);
} else {
    console.error(result.error);
}
```

`DiskFs` handles reading the project into a bundle, calling wasm, and writing the output bundle back to disk. The `workspaceRoot` option is a containment boundary — both `projectDir` and `outDir` must resolve under it, or `DiskFs` refuses the call.

## What's a "project"?

A spackle project is a directory containing:

- `spackle.toml` — config: slot declarations, ignore patterns, optional hooks.
- Template files ending in `.j2` — rendered with Tera.
- Non-`.j2` files — copied verbatim.
- File/dir names can contain `{{ slot }}` placeholders — rendered too.

See the fixture at `tests/fixtures/basic_project/` in the repo for a minimal example, or the runnable [`examples/wasm/bun-script`](../../examples/wasm/bun-script/) for a complete flow.

## Next

- [API reference](./api.md) — full shapes for `check`, `validateSlotData`, `generate`, bundle entries, responses.
- [Runtime targets](./runtime-targets.md) — which `pkg/{nodejs,web,bundler}` to import from your environment.
- [Custom host](./custom-host.md) — swap the disk-based bundle reader for S3, git, or anything else.
