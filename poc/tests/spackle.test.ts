// End-to-end tests — exercise check/validateSlotData/generate through
// both DiskFs (real disk + workspace root) and MemoryFs (in-memory).
// Same fixtures as the prior suite.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { cp, mkdtemp, realpath, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
    DiskFs,
    MemoryFs,
    check,
    generate,
    validateSlotData,
} from "../src/spackle.ts";

const FIXTURES = resolve(import.meta.dir, "..", "..", "tests", "data");

async function workspace(fixture: string) {
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-")));
    const projectDir = join(root, "project");
    await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
    const outDir = join(root, "output");
    return { root, projectDir, outDir };
}

async function memoryProject(
    fixtureSubpaths: string[],
    fixtureRoot: string,
    virtualRoot: string,
): Promise<MemoryFs> {
    const mem = new MemoryFs();
    for (const sub of fixtureSubpaths) {
        const content = await readFile(join(fixtureRoot, sub));
        mem.insertFile(join(virtualRoot, sub), new Uint8Array(content));
    }
    return mem;
}

describe("spackle (DiskFs)", () => {
    const cleanup: string[] = [];
    beforeEach(() => void (cleanup.length = 0));
    afterEach(async () => {
        for (const p of cleanup) await rm(p, { recursive: true, force: true });
    });

    test("check: happy path returns parsed config", async () => {
        const ws = await workspace("proj2");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });
        const res = await check(ws.projectDir, fs);

        expect(res.valid).toBe(true);
        if (res.valid) {
            expect(res.config.slots.map((s) => s.key)).toContain("defined_field");
        }
    });

    test("check: bad_template surfaces template errors", async () => {
        const ws = await workspace("bad_template");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });
        const res = await check(ws.projectDir, fs);

        expect(res.valid).toBe(false);
        if (!res.valid) {
            expect(res.errors.join(" ")).toContain("invalid_slot");
        }
    });

    test("validateSlotData: accepts good data, rejects wrong type", async () => {
        const ws = await workspace("proj1");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        const ok = await validateSlotData(
            ws.projectDir,
            { slot_1: "hello", slot_2: "42", slot_3: "true" },
            fs,
        );
        expect(ok.valid).toBe(true);

        const bad = await validateSlotData(
            ws.projectDir,
            { slot_1: "hello", slot_2: "not-a-number", slot_3: "true" },
            fs,
        );
        expect(bad.valid).toBe(false);
    });

    test("generate: writes rendered + copied files to outDir", async () => {
        const ws = await workspace("proj2");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        const res = await generate(ws.projectDir, ws.outDir, { defined_field: "hi" }, fs);
        expect(res.ok).toBe(true);
        if (res.ok) {
            const originals = res.rendered.map((r) => r.original_path);
            expect(originals).toContain("good.j2");

            const rendered = await readFile(join(ws.outDir, "good"), "utf8");
            expect(rendered).toBe("hi");

            const copied = await readFile(join(ws.outDir, "subdir", "file.txt"), "utf8");
            expect(copied).toContain("{{ undefined_field }}");
        }
    });

    test("generate: runHooks=true is explicitly unsupported", async () => {
        const ws = await workspace("proj2");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        const res = await generate(
            ws.projectDir,
            ws.outDir,
            { defined_field: "hi" },
            fs,
            { runHooks: true },
        );
        expect(res.ok).toBe(false);
        if (!res.ok) {
            expect(res.error).toContain("hooks are unsupported");
        }
    });
});

describe("spackle (MemoryFs)", () => {
    test("check + generate end-to-end without touching disk", async () => {
        const mem = await memoryProject(
            ["spackle.toml", "good.j2", "subdir/file.txt"],
            join(FIXTURES, "proj2"),
            "/project",
        );

        const checkRes = await check("/project", mem);
        expect(checkRes.valid).toBe(true);

        const genRes = await generate(
            "/project",
            "/output",
            { defined_field: "mem" },
            mem,
        );
        expect(genRes.ok).toBe(true);
        if (genRes.ok) {
            const snap = mem.snapshot();
            const rendered = snap.files["/output/good"];
            expect(rendered).toBeDefined();
            expect(new TextDecoder().decode(rendered)).toBe("mem");

            const copied = snap.files["/output/subdir/file.txt"];
            expect(copied).toBeDefined();
            expect(new TextDecoder().decode(copied)).toContain("{{ undefined_field }}");
        }
    });
});
