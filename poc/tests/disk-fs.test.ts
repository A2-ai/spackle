// DiskFs unit tests — focus on the containment + absolute-path contract.
// The broader "DiskFs backs a real generate" story is covered in
// spackle.test.ts; here we pin negative cases that would be invisible
// through the wasm layer.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { mkdtemp, mkdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { DiskFs, isSpackleFsError } from "../src/spackle.ts";

describe("DiskFs", () => {
    let root: string;

    beforeEach(async () => {
        root = await realpath(await mkdtemp(join(tmpdir(), "spackle-disk-")));
    });
    afterEach(async () => {
        await rm(root, { recursive: true, force: true });
    });

    test("rejects absolute paths that escape the workspaceRoot", async () => {
        const fs = new DiskFs({ workspaceRoot: root });
        let threw: unknown = null;
        try {
            fs.readFile("/etc/passwd");
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            expect(threw.kind).toBe("invalid-path");
        }
    });

    test("rejects relative paths (contract is absolute-only)", async () => {
        const fs = new DiskFs({ workspaceRoot: root });
        let threw: unknown = null;
        try {
            fs.readFile("relative/path.txt");
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            expect(threw.kind).toBe("invalid-path");
        }
    });

    test("rejects dot-dot traversal inside workspace", async () => {
        const sibling = await realpath(await mkdtemp(join(tmpdir(), "spackle-sibling-")));
        await writeFile(join(sibling, "secret"), "sensitive");
        try {
            const fs = new DiskFs({ workspaceRoot: root });
            let threw: unknown = null;
            try {
                // Construct an absolute path that lexically lives under root
                // but resolves elsewhere via `..`.
                fs.readFile(join(root, "..", "sibling-secret"));
            } catch (e) {
                threw = e;
            }
            expect(threw).not.toBeNull();
        } finally {
            await rm(sibling, { recursive: true, force: true });
        }
    });

    test("rejects symlink escape", async () => {
        const outside = await realpath(await mkdtemp(join(tmpdir(), "spackle-outside-")));
        await writeFile(join(outside, "secret"), "x");
        try {
            await symlink(outside, join(root, "escape"));
            const fs = new DiskFs({ workspaceRoot: root });
            let threw: unknown = null;
            try {
                fs.readFile(join(root, "escape", "secret"));
            } catch (e) {
                threw = e;
            }
            expect(threw).not.toBeNull();
            if (isSpackleFsError(threw)) {
                expect(threw.kind).toBe("invalid-path");
            }
        } finally {
            await rm(outside, { recursive: true, force: true });
        }
    });

    test("allows legitimate nested access + round-trips bytes", async () => {
        await mkdir(join(root, "sub"), { recursive: true });
        await writeFile(join(root, "sub", "a.txt"), "hello");

        const fs = new DiskFs({ workspaceRoot: root });

        const read = fs.readFile(join(root, "sub", "a.txt"));
        expect(new TextDecoder().decode(read)).toBe("hello");

        fs.writeFile(join(root, "sub", "b.txt"), new TextEncoder().encode("world"));
        const read2 = fs.readFile(join(root, "sub", "b.txt"));
        expect(new TextDecoder().decode(read2)).toBe("world");

        expect(fs.exists(join(root, "sub", "b.txt"))).toBe(true);
        expect(fs.exists(join(root, "nope.txt"))).toBe(false);
    });

    test("createDirAll is idempotent for an existing directory", () => {
        const fs = new DiskFs({ workspaceRoot: root });
        const target = join(root, "a", "b", "c");
        fs.createDirAll(target);
        fs.createDirAll(target); // no-op
        expect(fs.exists(target)).toBe(true);
    });

    test("createDirAll errors when the path exists as a file", async () => {
        await writeFile(join(root, "collide"), "bytes");
        const fs = new DiskFs({ workspaceRoot: root });
        let threw: unknown = null;
        try {
            fs.createDirAll(join(root, "collide"));
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            // Node surfaces this as EEXIST → we map to "already-exists".
            expect(["already-exists", "not-a-directory", "other"]).toContain(threw.kind);
        }
    });

    test("stat on a symlink surfaces type=symlink (does not follow)", async () => {
        // Plant a symlink targeting a real dir inside the workspace. A
        // walker that recurses on `type === "directory"` must NOT follow
        // symlinks — stat has to report them as-is or the walker loops.
        await mkdir(join(root, "real"), { recursive: true });
        await symlink(join(root, "real"), join(root, "link"));

        const fs = new DiskFs({ workspaceRoot: root });
        const st = fs.stat(join(root, "link"));
        expect(st.type).toBe("symlink");

        // Sanity: the target itself still stats as a directory.
        const stReal = fs.stat(join(root, "real"));
        expect(stReal.type).toBe("directory");
    });

    test("listDir surfaces dirents with type", async () => {
        await mkdir(join(root, "sub"), { recursive: true });
        await writeFile(join(root, "a"), "1");

        const fs = new DiskFs({ workspaceRoot: root });
        const entries = fs.listDir(root).sort((x, y) => x.name.localeCompare(y.name));

        expect(entries.map((e) => e.name)).toEqual(["a", "sub"]);
        expect(entries[0].type).toBe("file");
        expect(entries[1].type).toBe("directory");
    });
});
