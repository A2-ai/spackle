// In-memory `SpackleFs` backend. Backing store is a Map keyed by
// canonical absolute path; directories are tracked separately in a Set.
//
// Useful for:
// - preview/ephemeral flows (generate outputs without touching disk)
// - unit-testing fs-bridge behavior without temp directories
//
// Path semantics: all paths must be absolute and canonical on the way in.
// No containment model — the MemoryFs IS the sandbox (nothing outside
// it exists). Callers that want to enforce a virtual workspace root can
// wrap this adapter.

import {
    basename as pathBasename,
    dirname as pathDirname,
    isAbsolute,
} from "node:path";

import {
    fsError,
    type SpackleFileEntry,
    type SpackleFileStat,
    type SpackleFs,
} from "./spackle-fs";

export interface MemoryFsSeed {
    /** Absolute-path → file bytes. Parent dirs auto-created. */
    files?: Record<string, Uint8Array | string>;
    /** Absolute-path → explicit directory (besides parents of seeded files). */
    dirs?: string[];
}

export class MemoryFs implements SpackleFs {
    private readonly files = new Map<string, Uint8Array>();
    private readonly dirs = new Set<string>(["/"]);

    constructor(seed?: MemoryFsSeed) {
        for (const d of seed?.dirs ?? []) {
            this.insertDir(d);
        }
        for (const [p, content] of Object.entries(seed?.files ?? {})) {
            this.insertFile(p, content);
        }
    }

    private requireAbsolute(path: string): void {
        if (!isAbsolute(path)) {
            throw fsError("invalid-path", `MemoryFs: path must be absolute: ${path}`);
        }
    }

    /** Create every ancestor directory of the given absolute path. */
    private insertDirRecursive(path: string): void {
        let current = path;
        while (current && current !== "/") {
            this.dirs.add(current);
            current = pathDirname(current);
        }
        this.dirs.add("/");
    }

    /** Public seed helper — primarily for tests. */
    insertDir(path: string): void {
        this.requireAbsolute(path);
        this.insertDirRecursive(path);
    }

    /** Public seed helper — primarily for tests. */
    insertFile(path: string, content: Uint8Array | string): void {
        this.requireAbsolute(path);
        const bytes =
            typeof content === "string" ? new TextEncoder().encode(content) : content;
        const parent = pathDirname(path);
        this.insertDirRecursive(parent);
        this.files.set(path, bytes);
    }

    /** Read out the current contents — primarily for tests. */
    snapshot(): { files: Record<string, Uint8Array>; dirs: string[] } {
        return {
            files: Object.fromEntries(this.files),
            dirs: [...this.dirs],
        };
    }

    readFile(path: string): Uint8Array {
        this.requireAbsolute(path);
        const bytes = this.files.get(path);
        if (bytes === undefined) {
            throw fsError("not-found", `MemoryFs.readFile: ${path}`);
        }
        return bytes;
    }

    writeFile(path: string, content: Uint8Array): void {
        this.requireAbsolute(path);
        const parent = pathDirname(path);
        if (!this.dirs.has(parent)) {
            throw fsError(
                "not-found",
                `MemoryFs.writeFile: parent does not exist: ${parent}`,
            );
        }
        this.files.set(path, content);
    }

    createDirAll(path: string): void {
        this.requireAbsolute(path);
        // Walk every ancestor (including the target itself) and make
        // sure no entry exists as a file. Turning a file into a dir
        // would be silent corruption — mkdir -p doesn't do that either,
        // it errors with ENOTDIR / EEXIST.
        let current = path;
        while (current && current !== "/") {
            if (this.files.has(current)) {
                throw fsError(
                    "already-exists",
                    `MemoryFs.createDirAll: path exists as a file: ${current}`,
                );
            }
            const parent = pathDirname(current);
            if (parent === current) break;
            current = parent;
        }
        this.insertDirRecursive(path);
    }

    listDir(path: string): SpackleFileEntry[] {
        this.requireAbsolute(path);
        if (!this.dirs.has(path)) {
            throw fsError("not-found", `MemoryFs.listDir: ${path}`);
        }
        const out: SpackleFileEntry[] = [];
        for (const p of this.files.keys()) {
            if (pathDirname(p) === path) {
                out.push({ name: pathBasename(p), type: "file" });
            }
        }
        for (const d of this.dirs) {
            if (d !== path && pathDirname(d) === path) {
                out.push({ name: pathBasename(d), type: "directory" });
            }
        }
        return out;
    }

    copyFile(src: string, dst: string): void {
        this.requireAbsolute(src);
        this.requireAbsolute(dst);
        const bytes = this.files.get(src);
        if (bytes === undefined) {
            throw fsError("not-found", `MemoryFs.copyFile src: ${src}`);
        }
        const parent = pathDirname(dst);
        if (!this.dirs.has(parent)) {
            throw fsError(
                "not-found",
                `MemoryFs.copyFile: destination parent does not exist: ${parent}`,
            );
        }
        this.files.set(dst, bytes);
    }

    exists(path: string): boolean {
        if (!isAbsolute(path)) return false;
        return this.files.has(path) || this.dirs.has(path);
    }

    stat(path: string): SpackleFileStat {
        this.requireAbsolute(path);
        const bytes = this.files.get(path);
        if (bytes !== undefined) {
            return { type: "file", size: bytes.length };
        }
        if (this.dirs.has(path)) {
            return { type: "directory", size: 0 };
        }
        throw fsError("not-found", `MemoryFs.stat: ${path}`);
    }
}
