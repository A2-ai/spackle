// End-to-end tests — exercise check/validateSlotData/generate through
// the bundle-in / bundle-out API. DiskFs covers the disk-backed flow,
// checkBundle / generateBundle covers the in-memory flow.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { cp, mkdtemp, realpath, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
    DiskFs,
    MemoryFs,
    check,
    checkBundle,
    generate,
    generateBundle,
    validateSlotData,
} from "../src/spackle.ts";

const FIXTURES = resolve(import.meta.dir, "..", "..", "tests", "fixtures");

async function workspace(fixture: string) {
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-")));
    const projectDir = join(root, "project");
    await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
    const outDir = join(root, "output");
    return { root, projectDir, outDir };
}

async function bundleFromDisk(
    fixtureSubpaths: string[],
    fixtureRoot: string,
    virtualRoot: string,
) {
    const entries = [];
    for (const sub of fixtureSubpaths) {
        const content = await readFile(join(fixtureRoot, sub));
        entries.push({ path: `${virtualRoot}/${sub}`, bytes: new Uint8Array(content) });
    }
    return entries;
}

describe("spackle (DiskFs)", () => {
    const cleanup: string[] = [];
    beforeEach(() => void (cleanup.length = 0));
    afterEach(async () => {
        for (const p of cleanup) await rm(p, { recursive: true, force: true });
    });

    test("check: happy path returns parsed config", async () => {
        const ws = await workspace("basic_project");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });
        const res = await check(ws.projectDir, fs);

        expect(res.valid).toBe(true);
        if (res.valid) {
            const keys = res.config.slots.map((s) => s.key);
            expect(keys).toContain("greeting");
            expect(keys).toContain("target");
            expect(keys).toContain("filename");
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
        const ws = await workspace("typed_slots");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        const ok = await validateSlotData(
            ws.projectDir,
            { name: "hello", count: "42", enabled: "true" },
            fs,
        );
        expect(ok.valid).toBe(true);

        const bad = await validateSlotData(
            ws.projectDir,
            { name: "hello", count: "not-a-number", enabled: "true" },
            fs,
        );
        expect(bad.valid).toBe(false);
    });

    test("generate: writes rendered + copied files to outDir", async () => {
        const ws = await workspace("basic_project");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        const res = await generate(
            ws.projectDir,
            ws.outDir,
            { greeting: "hi", target: "world", filename: "notes" },
            fs,
        );
        expect(res.ok).toBe(true);
        if (res.ok) {
            const paths = res.files.map((f) => f.path).sort();
            expect(paths).toContain("README.md");

            const readme = await readFile(join(ws.outDir, "README.md"), "utf8");
            expect(readme).toContain("HI, world!");

            // Static file copied verbatim (tokens not interpolated).
            const copied = await readFile(join(ws.outDir, "docs", "static.md"), "utf8");
            expect(copied).toContain("{{ greeting }}");

            // `drafts/` is in the ignore list and must not be copied.
            await expect(readFile(join(ws.outDir, "drafts", "ignored.md"))).rejects.toThrow();
        }
    });

    test("generate: refuses a pre-existing outDir (native AlreadyExists parity)", async () => {
        const ws = await workspace("basic_project");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        // Pre-create the out dir so writeOutput sees it already present.
        await import("node:fs/promises").then((mod) =>
            mod.mkdir(ws.outDir, { recursive: true }),
        );

        await expect(
            generate(
                ws.projectDir,
                ws.outDir,
                { greeting: "hi", target: "world", filename: "notes" },
                fs,
            ),
        ).rejects.toThrow(/already exists/);
    });

    test("generate: runHooks=true is explicitly unsupported", async () => {
        const ws = await workspace("basic_project");
        cleanup.push(ws.root);
        const fs = new DiskFs({ workspaceRoot: ws.root });

        const res = await generate(
            ws.projectDir,
            ws.outDir,
            { greeting: "hi", target: "world", filename: "notes" },
            fs,
            { runHooks: true },
        );
        expect(res.ok).toBe(false);
        if (!res.ok) {
            expect(res.error).toContain("hooks are unsupported");
        }
    });
});

describe("spackle (bundle-only / MemoryFs)", () => {
    test("checkBundle + generateBundle end-to-end without touching disk", async () => {
        const bundle = await bundleFromDisk(
            [
                "spackle.toml",
                "README.md.j2",
                "docs/static.md",
                "src/{{ filename }}.txt.j2",
            ],
            join(FIXTURES, "basic_project"),
            "/project",
        );

        const checkRes = await checkBundle(bundle, "/project");
        expect(checkRes.valid).toBe(true);

        const genRes = await generateBundle(
            bundle,
            { greeting: "hey", target: "mem", filename: "notes" },
            { virtualProjectDir: "/project", virtualOutDir: "/output" },
        );
        expect(genRes.ok).toBe(true);
        if (genRes.ok) {
            // Output bundle paths are RELATIVE to out_dir.
            const snap = MemoryFs.fromBundle(genRes.files, "/output").snapshot();
            const readme = snap.files["/output/README.md"];
            expect(readme).toBeDefined();
            expect(new TextDecoder().decode(readme)).toContain("HEY, mem!");

            const copied = snap.files["/output/docs/static.md"];
            expect(copied).toBeDefined();
            expect(new TextDecoder().decode(copied)).toContain("{{ greeting }}");
        }
    });
});
