// Minimal bun example: read template → fill → write to a temp dir.
//
// Run: `bun run generate.ts` from this directory.

import { readFile, readdir, rm } from "node:fs/promises";
import { join } from "node:path";

import { DiskFs, generate } from "@a2-ai/spackle";

const FIXTURE = join(import.meta.dir, "fixtures", "my-template");

async function main() {
    // Workspace root contains both the template and the output. DiskFs
    // refuses any path that doesn't resolve under this boundary — so
    // scope it to this example's own directory.
    const workspaceRoot = import.meta.dir;
    const outDir = join(workspaceRoot, "output");

    // `generate` refuses a pre-existing outDir (matches native
    // spackle's AlreadyExists contract). Clean it before every run.
    await rm(outDir, { recursive: true, force: true });

    const fs = new DiskFs({ workspaceRoot });

    const result = await generate(
        FIXTURE,
        outDir,
        {
            name: "hello",
            project_type: "rust",
        },
        fs,
    );

    if (!result.ok) {
        console.error("Generation failed:", result.error);
        process.exit(1);
    }

    console.log(`Wrote ${result.files.length} file(s) to ${outDir}:`);
    for (const entry of result.files) {
        console.log(`  ${entry.path}  (${entry.bytes.length} bytes)`);
    }
    console.log("\nContents:");
    for (const name of await readdir(outDir)) {
        const body = await readFile(join(outDir, name), "utf8");
        console.log(`--- ${name} ---\n${body}`);
    }
}

main().catch((e) => {
    console.error(e);
    process.exit(1);
});
