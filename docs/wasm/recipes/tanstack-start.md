# Recipe: TanStack Start server

**Status: planned.**

Use `@a2-ai/spackle-wasm` inside a TanStack Start server route to generate project trees on demand (e.g., scaffold-as-a-service, template preview, per-request fixture generation).

Example target: [`examples/wasm/tanstack-start-server`](../../../examples/wasm/tanstack-start-server/) — skeleton; not yet implemented.

## Sketch

```ts
// app/routes/api/generate.ts
import { DiskFs, generate } from "@a2-ai/spackle-wasm";
import { createServerFn } from "@tanstack/start";

export const generateProject = createServerFn("POST", async (input: {
    templatePath: string;
    slotData: Record<string, string>;
}) => {
    const fs = new DiskFs({ workspaceRoot: "/var/workspace" });
    return generate(
        input.templatePath,
        `/var/workspace/gen/${crypto.randomUUID()}`,
        input.slotData,
        fs,
    );
});
```

Full walkthrough (install, deployment considerations, streaming back to the client) pending in the example directory.
