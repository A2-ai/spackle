// Unit tests for the path-containment hardening in bun-wasi.ts.
// Exercises resolveChild() indirectly via the Descriptor API — each
// failure scenario is a path that a hostile template could plausibly
// create (symlink out, sibling-prefix collision, dot-dot traversal).

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { mkdtemp, rm, realpath, mkdir, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { createWasiImports, WasiError } from "../../src/wasip2/bun-wasi";

describe("Bun WASI shim — path containment", () => {
    let root: string;

    beforeEach(async () => {
        root = await realpath(await mkdtemp(join(tmpdir(), "spackle-contain-")));
    });
    afterEach(async () => {
        await rm(root, { recursive: true, force: true });
    });

    function rootDescriptor() {
        const wasi = createWasiImports({ preopens: { [root]: root } });
        const [desc] = (wasi as any)["wasi:filesystem/preopens"].getDirectories()[0];
        return desc as {
            statAt(flags: { symlinkFollow?: boolean }, path: string): unknown;
            openAt(
                flags: { symlinkFollow?: boolean },
                path: string,
                openFlags: Record<string, boolean>,
                descFlags: Record<string, boolean>,
            ): unknown;
        };
    }

    test("absolute path rejected", () => {
        const desc = rootDescriptor();
        expect(() => desc.statAt({}, "/etc/passwd")).toThrow();
    });

    test("dot-dot escape rejected even when path.join normalizes it", async () => {
        // Create a sibling dir that would be reached via `../sibling`.
        const sibling = join(root, "..", "spackle-contain-sibling");
        await mkdir(sibling, { recursive: true });
        try {
            const desc = rootDescriptor();
            let threw: WasiError | null = null;
            try {
                desc.statAt({}, "../spackle-contain-sibling");
            } catch (e) {
                threw = e as WasiError;
            }
            expect(threw).not.toBeNull();
            expect(threw?.code).toMatch(/not-permitted|no-entry/);
        } finally {
            await rm(sibling, { recursive: true, force: true });
        }
    });

    test("symlink pointing outside the workspace is rejected", async () => {
        // Target outside the workspace (a sibling tmp dir).
        const outsideRoot = await realpath(
            await mkdtemp(join(tmpdir(), "spackle-contain-outside-")),
        );
        await writeFile(join(outsideRoot, "secret"), "sensitive");

        try {
            // Plant a symlink inside the workspace that resolves outside it.
            await symlink(outsideRoot, join(root, "escape"));

            const desc = rootDescriptor();
            let threw: WasiError | null = null;
            try {
                desc.statAt({}, "escape/secret");
            } catch (e) {
                threw = e as WasiError;
            }
            expect(threw).not.toBeNull();
            expect(threw?.code).toBe("not-permitted");
        } finally {
            await rm(outsideRoot, { recursive: true, force: true });
        }
    });

    test("legitimate nested path still resolves", async () => {
        await mkdir(join(root, "sub"), { recursive: true });
        await writeFile(join(root, "sub", "file.txt"), "ok");
        const desc = rootDescriptor();
        // Doesn't throw → containment check passes for the canonical path.
        expect(() => desc.statAt({}, "sub/file.txt")).not.toThrow();
    });

    test("create flow (parent canonicalized) permits creating a new file", () => {
        const desc = rootDescriptor();
        // openAt with create=true for a not-yet-existing path — should
        // succeed because the parent (root) canonicalizes cleanly.
        expect(() =>
            desc.openAt({}, "newfile.txt", { create: true }, { read: true, write: true }),
        ).not.toThrow();
    });
});
