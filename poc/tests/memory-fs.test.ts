// MemoryFs unit tests — focus on the in-memory backing semantics.
// Pairs with disk-fs.test.ts for negative containment cases; here we
// verify state tracking (files vs dirs), listing, and the seed helpers.

import { describe, expect, test } from "bun:test";

import { MemoryFs, isSpackleFsError } from "../src/spackle.ts";

describe("MemoryFs", () => {
    test("round-trips files", () => {
        const fs = new MemoryFs();
        fs.insertDir("/ws");
        fs.writeFile("/ws/a.txt", new TextEncoder().encode("hello"));
        expect(new TextDecoder().decode(fs.readFile("/ws/a.txt"))).toBe("hello");
    });

    test("rejects relative paths", () => {
        const fs = new MemoryFs();
        let threw: unknown = null;
        try {
            fs.readFile("relative");
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            expect(threw.kind).toBe("invalid-path");
        }
    });

    test("writeFile requires parent directory", () => {
        const fs = new MemoryFs();
        let threw: unknown = null;
        try {
            fs.writeFile("/missing/parent/file.txt", new Uint8Array());
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            expect(threw.kind).toBe("not-found");
        }
    });

    test("createDirAll creates ancestors", () => {
        const fs = new MemoryFs();
        fs.createDirAll("/a/b/c");
        expect(fs.exists("/a")).toBe(true);
        expect(fs.exists("/a/b")).toBe(true);
        expect(fs.exists("/a/b/c")).toBe(true);
    });

    test("createDirAll errors when target exists as a file", () => {
        const fs = new MemoryFs({ files: { "/collide": "bytes" } });
        let threw: unknown = null;
        try {
            fs.createDirAll("/collide");
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            expect(threw.kind).toBe("already-exists");
        }
    });

    test("createDirAll errors when an ancestor exists as a file", () => {
        const fs = new MemoryFs({ files: { "/a": "bytes" } });
        let threw: unknown = null;
        try {
            fs.createDirAll("/a/b/c");
        } catch (e) {
            threw = e;
        }
        expect(threw).not.toBeNull();
        if (isSpackleFsError(threw)) {
            expect(threw.kind).toBe("already-exists");
        }
    });

    test("listDir surfaces files and subdirectories", () => {
        const fs = new MemoryFs({
            dirs: ["/ws/sub"],
            files: {
                "/ws/a.txt": "A",
                "/ws/b.txt": "B",
            },
        });

        const entries = fs.listDir("/ws").sort((x, y) => x.name.localeCompare(y.name));
        expect(entries).toEqual([
            { name: "a.txt", type: "file" },
            { name: "b.txt", type: "file" },
            { name: "sub", type: "directory" },
        ]);
    });

    test("copyFile + stat", () => {
        const fs = new MemoryFs({ files: { "/src/a.bin": new Uint8Array([1, 2, 3]) } });
        fs.createDirAll("/dst");
        fs.copyFile("/src/a.bin", "/dst/a.bin");

        const st = fs.stat("/dst/a.bin");
        expect(st.type).toBe("file");
        expect(st.size).toBe(3);
    });

    test("seed helpers initialize state correctly", () => {
        const fs = new MemoryFs({
            files: { "/a/b.txt": "hi" },
            dirs: ["/a/sub"],
        });
        // Parent dir of seeded file is auto-created.
        expect(fs.exists("/a")).toBe(true);
        expect(fs.exists("/a/b.txt")).toBe(true);
        expect(fs.exists("/a/sub")).toBe(true);
    });

    test("snapshot exposes current state", () => {
        const fs = new MemoryFs();
        fs.insertDir("/x");
        fs.insertFile("/x/y.txt", "data");
        const snap = fs.snapshot();
        expect(Object.keys(snap.files)).toContain("/x/y.txt");
        expect(snap.dirs).toContain("/x");
    });
});
