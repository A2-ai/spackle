# Recipe: Browser-side preview

**Status: planned.**

Run spackle entirely client-side in a browser to preview template output as a user edits slot values. No server round-trip, no network — the user's own browser runs the wasm.

Example target: [`examples/wasm/browser-preview`](../../../examples/wasm/browser-preview/) — skeleton; not yet implemented.

## Sketch

Use the `web` target and `generateBundle` (no disk, no `DiskFs`):

```ts
import init, { generate } from "@a2-ai/spackle/pkg/web";

await init();  // load the .wasm

// Seed the project bundle from a gist, localStorage, or similar.
const bundle = [
    {
        path: "/project/spackle.toml",
        bytes: new TextEncoder().encode(`[[slots]]\nkey = "name"\ntype = "String"\n`),
    },
    {
        path: "/project/README.md.j2",
        bytes: new TextEncoder().encode("# {{ name }}\n"),
    },
];

const result = generate(bundle, "/project", "/output", JSON.stringify({ name: "demo" }), false);
// result: { ok: true, files: [{ path: "README.md", bytes: Uint8Array(...) }] }
```

Display the rendered output in the UI (Monaco editor, a file-tree component, etc.).

Full walkthrough (wasm loading strategies, CSP headers, bundle persistence) pending in the example directory.
