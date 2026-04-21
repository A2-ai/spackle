# Runtime targets

`@a2-ai/spackle` ships three wasm-pack outputs under `pkg/`. Pick the one matching your environment:

| Target | Module format | Instantiation | Import path |
|---|---|---|---|
| **nodejs** | CommonJS | Eager at import time (`fs.readFileSync`) | `@a2-ai/spackle` (default entry) |
| **web** | ESM | Async — call `init()` before exports | `@a2-ai/spackle/pkg/web` |
| **bundler** | ESM | Bundler handles `.wasm` loading | `@a2-ai/spackle/pkg/bundler` |

## Node / Bun / Deno-in-compat-mode

Use the default import — the TS wrapper layer handles everything:

```ts
import { generate, DiskFs } from "@a2-ai/spackle";
```

Under the hood this imports from `pkg/nodejs/`, which eagerly instantiates the `.wasm` at import time via `fs.readFileSync`. No ceremony; calls are ready immediately.

## Browser (direct)

For browsers that fetch the `.wasm` directly, use the web subpath:

```ts
import init, { generate } from "@a2-ai/spackle/pkg/web";

await init();  // loads and instantiates the .wasm — must await before use
// now `generate(...)` etc. are callable
```

The web target's `.js` glue uses `fetch()` to load the `.wasm`. Serve `spackle_wasm_bg.wasm` alongside your JS bundle with the correct MIME type (`application/wasm`).

## Bundler pipelines (webpack, vite, esbuild)

Some bundlers can inline `.wasm` as part of the build. Use the bundler subpath:

```ts
import { generate } from "@a2-ai/spackle/pkg/bundler";
```

The bundler target emits ESM with a `.wasm` import the bundler resolves. Each bundler has its own configuration for `.wasm` handling — consult your bundler's docs.

## Which target does the TS wrapper use?

The high-level orchestration layer (`check`, `generate`, `validateSlotData`, `DiskFs`, `MemoryFs`) only wraps the **nodejs** target. If you need the TS layer plus another target's raw exports, import from both:

```ts
import { DiskFs } from "@a2-ai/spackle";                   // high-level, nodejs target
import { generate as rawGenerate } from "@a2-ai/spackle/pkg/web";  // raw web export
```
