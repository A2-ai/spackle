// Bun-runtime version of the wasip2 smoke tests. Uses the custom Bun
// WASI shim in `src/wasip2/bun-wasi.ts` instead of the jco default
// `@bytecodealliance/preview2-shim`. Same 6 scenarios as the Node tests
// at `tests/wasip2/component.test.mjs` — proves runtime parity.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { mkdtemp, cp, rm, realpath, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
    checkProject,
    validateSlotData,
    generateProject,
} from "../../src/wasip2/bun";

const FIXTURES = resolve(import.meta.dir, "..", "..", "..", "tests", "data");

async function makeWorkspace(fixture: string) {
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-wasip2-bun-")));
    const projectDir = join(root, "project");
    await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
    const outDir = join(root, "output");
    return { workspaceRoot: root, projectDir, outDir };
}

describe("wasip2 component (Bun)", () => {
    const cleanup: string[] = [];
    beforeEach(() => void (cleanup.length = 0));
    afterEach(async () => {
        for (const p of cleanup) {
            await rm(p, { recursive: true, force: true });
        }
    });

    test("check: happy path returns parsed config", async () => {
        const ws = await makeWorkspace("proj2");
        cleanup.push(ws.workspaceRoot);

        const res = await checkProject(ws);

        expect(res.valid).toBe(true);
        if (res.valid) {
            expect(res.config.slots.map((s) => s.key)).toContain("defined_field");
        }
    });

    test("check: template ref error surfaces in errors array", async () => {
        const ws = await makeWorkspace("bad_template");
        cleanup.push(ws.workspaceRoot);

        const res = await checkProject(ws);
        expect(res.valid).toBe(false);
        if (!res.valid) {
            expect(res.errors.length).toBeGreaterThan(0);
            expect(res.errors.join(" ")).toContain("invalid_slot");
        }
    });

    test("validate_slot_data: accepts good data, rejects wrong type", async () => {
        const ws = await makeWorkspace("proj1");
        cleanup.push(ws.workspaceRoot);

        const ok = await validateSlotData({
            ...ws,
            slotData: { slot_1: "hello", slot_2: "42", slot_3: "true" },
        });
        expect(ok.valid).toBe(true);

        const bad = await validateSlotData({
            ...ws,
            slotData: { slot_1: "hello", slot_2: "not-a-number", slot_3: "true" },
        });
        expect(bad.valid).toBe(false);
    });

    test("generate: writes rendered + copied files to outDir", async () => {
        const ws = await makeWorkspace("proj2");
        cleanup.push(ws.workspaceRoot);

        const res = await generateProject({
            ...ws,
            slotData: { defined_field: "hello" },
            runHooks: false,
        });

        expect(res.ok).toBe(true);
        if (res.ok) {
            const originals = res.rendered.map((r) => r.original_path);
            expect(originals).toContain("good.j2");

            const rendered = await readFile(join(ws.outDir, "good"), "utf8");
            expect(rendered).toBe("hello");

            const copied = await readFile(
                join(ws.outDir, "subdir", "file.txt"),
                "utf8",
            );
            expect(copied).toContain("{{ undefined_field }}");
        }
    });

    test("generate: runHooks=true invokes host runCommand and captures output", async () => {
        const ws = await makeWorkspace("hook");
        cleanup.push(ws.workspaceRoot);

        const res = await generateProject({
            ...ws,
            slotData: {},
            runHooks: true,
        });

        expect(res.ok).toBe(true);
        if (res.ok) {
            const byKey = new Map(res.hook_results.map((r) => [r.hook_key, r]));
            const h1 = byKey.get("hook_1")!;
            expect(h1.kind).toBe("completed");
            if (h1.kind === "completed") {
                const stdout = Buffer.from(h1.stdout).toString();
                const stderr = Buffer.from(h1.stderr).toString();
                expect(stdout).toContain("This is logged to stdout");
                expect(stderr).toContain("This is logged to stderr");
            }
        }
    });

    test("generate: hook_ran conditional chain re-evaluates correctly", async () => {
        const ws = await makeWorkspace("hook_ran_cond");
        cleanup.push(ws.workspaceRoot);

        const res = await generateProject({
            ...ws,
            slotData: {},
            runHooks: true,
        });

        expect(res.ok).toBe(true);
        if (res.ok) {
            const byKey = new Map(res.hook_results.map((r) => [r.hook_key, r]));
            expect(byKey.get("hook_1")?.kind).toBe("completed");
            expect(byKey.get("dep_hook_should_run")?.kind).toBe("completed");
        }
    });
});
