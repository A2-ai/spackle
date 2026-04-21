# Recipe: Next.js API route

**Status: planned.**

Use `@a2-ai/spackle` inside a Next.js API route (app router or pages router) for server-side project generation.

Example target: [`examples/wasm/nextjs-route`](../../../examples/wasm/nextjs-route/) — skeleton; not yet implemented.

## Sketch (app router)

```ts
// app/api/generate/route.ts
import { NextResponse } from "next/server";
import { DiskFs, generate } from "@a2-ai/spackle";

export async function POST(req: Request) {
    const { templatePath, slotData } = await req.json();
    const fs = new DiskFs({ workspaceRoot: "/var/workspace" });
    const result = await generate(
        templatePath,
        `/var/workspace/gen/${crypto.randomUUID()}`,
        slotData,
        fs,
    );
    return NextResponse.json(result);
}
```

Full walkthrough (Edge runtime vs Node runtime, file persistence, wasm bundling under webpack) pending in the example directory.
