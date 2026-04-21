// Walk through every public entry in `../src/spackle.ts` so you can
// eyeball the output after a wasm-pack build.
//
// Run: `just poc` or `bun run scripts/demo.ts`

import { cp, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";

import { DiskFs, MemoryFs, check, generate, validateSlotData } from "../src/spackle.ts";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const FIXTURES = join(REPO_ROOT, "tests", "data");

/**
 * Make a throwaway workspace seeded with a fixture. Returns the
 * workspace root + resolved projectDir / outDir under it.
 */
async function workspace(fixture: string) {
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-demo-")));
    const projectDir = join(root, "project");
    await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
    const outDir = join(root, "output");
    return { root, projectDir, outDir };
}

// --- check: proj2 (clean) + bad_default_slot_val ---

for (const fixture of ["proj2", "bad_default_slot_val"]) {
    const ws = await workspace(fixture);
    try {
        console.log(`=== check(${fixture}) — DiskFs ===`);
        const fs = new DiskFs({ workspaceRoot: ws.root });
        const result = await check(ws.projectDir, fs);
        console.log(
            `  valid=${result.valid}`,
            !result.valid ? `errors=${JSON.stringify(result.errors)}` : "",
        );
        if (result.valid) {
            console.log(
                `  name=${result.config.name ?? "(unnamed)"}`,
                `slots=${result.config.slots.length}`,
                `hooks=${result.config.hooks.length}`,
            );
        }
        console.log();
    } finally {
        await rm(ws.root, { recursive: true, force: true });
    }
}

// --- validateSlotData: proj1 happy + bad-type ---

{
    const ws = await workspace("proj1");
    try {
        const fs = new DiskFs({ workspaceRoot: ws.root });
        console.log("=== validateSlotData(proj1, good) ===");
        const ok = await validateSlotData(
            ws.projectDir,
            { slot_1: "hello", slot_2: "42", slot_3: "true" },
            fs,
        );
        console.log(`  valid=${ok.valid}`);

        console.log("=== validateSlotData(proj1, wrong-type) ===");
        const bad = await validateSlotData(
            ws.projectDir,
            { slot_1: "hello", slot_2: "not-a-number", slot_3: "true" },
            fs,
        );
        console.log(
            `  valid=${bad.valid}`,
            !bad.valid ? `errors=${JSON.stringify(bad.errors)}` : "",
        );
        console.log();
    } finally {
        await rm(ws.root, { recursive: true, force: true });
    }
}

// --- generate: proj2 with DiskFs (writes to disk) ---

{
    const ws = await workspace("proj2");
    try {
        const fs = new DiskFs({ workspaceRoot: ws.root });
        console.log(`=== generate(proj2, DiskFs) → ${relative(process.cwd(), ws.outDir)} ===`);
        const result = await generate(
            ws.projectDir,
            ws.outDir,
            { defined_field: "hello" },
            fs,
        );
        if (result.ok) {
            for (const r of result.rendered) {
                console.log(`  ${r.original_path} → ${r.rendered_path}`);
            }
        } else {
            console.log(`  FAILED: ${result.error}`);
        }
        console.log();
    } finally {
        await rm(ws.root, { recursive: true, force: true });
    }
}

// --- generate: proj2 with MemoryFs (ephemeral preview) ---

{
    const proj2Src = join(FIXTURES, "proj2");
    const mem = new MemoryFs();
    // Seed MemoryFs with the project files (DiskFs would read directly).
    // For a real preview flow, a server would populate MemoryFs from a
    // template store (git, S3, etc.).
    mem.insertFile(
        "/project/spackle.toml",
        await Bun.file(join(proj2Src, "spackle.toml")).text(),
    );
    mem.insertFile("/project/good.j2", await Bun.file(join(proj2Src, "good.j2")).text());
    mem.insertDir("/project/subdir");
    mem.insertFile(
        "/project/subdir/file.txt",
        await Bun.file(join(proj2Src, "subdir", "file.txt")).text(),
    );

    console.log("=== generate(proj2, MemoryFs) — in-memory preview ===");
    const result = await generate("/project", "/output", { defined_field: "hello" }, mem);
    if (result.ok) {
        for (const r of result.rendered) {
            console.log(`  ${r.original_path} → ${r.rendered_path}`);
        }
        const snap = mem.snapshot();
        const outputs = Object.keys(snap.files).filter((p) => p.startsWith("/output/"));
        console.log(`  in-memory outputs: ${outputs.length} file(s)`);
        for (const p of outputs.sort()) {
            const preview = new TextDecoder().decode(snap.files[p]).slice(0, 40);
            console.log(`    ${p}  ${JSON.stringify(preview)}`);
        }
    } else {
        console.log(`  FAILED: ${result.error}`);
    }
    console.log();
}

// --- generate with runHooks=true → unsupported error ---

{
    const ws = await workspace("proj2");
    try {
        const fs = new DiskFs({ workspaceRoot: ws.root });
        console.log("=== generate(proj2, runHooks=true) — expect unsupported ===");
        const result = await generate(
            ws.projectDir,
            ws.outDir,
            { defined_field: "hello" },
            fs,
            { runHooks: true },
        );
        console.log(`  ok=${result.ok}`, !result.ok ? `error=${result.error}` : "");
    } finally {
        await rm(ws.root, { recursive: true, force: true });
    }
}

console.log("\nDone.");
