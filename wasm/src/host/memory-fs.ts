// MemoryFs — host-side in-memory bundle holder.
//
// No longer a `SpackleFs` callback adapter. Under the bundle-in /
// bundle-out design this is a pure TS data structure: a map of path →
// bytes that can be converted to/from the wasm `Bundle` shape. Useful
// for preview flows (generate-without-touching-disk) and for tests that
// want to assert on the rendered tree without filesystem interaction.

import { isAbsolute } from "node:path";

import type { Bundle } from "../wasm/types.ts";

export interface MemoryFsSeed {
    /** Absolute-path → file bytes (or a string, UTF-8 encoded). */
    files?: Record<string, Uint8Array | string>;
}

export class MemoryFs {
    private readonly files = new Map<string, Uint8Array>();

    constructor(seed?: MemoryFsSeed) {
        for (const [p, content] of Object.entries(seed?.files ?? {})) {
            this.insertFile(p, content);
        }
    }

    /** Build a MemoryFs from a Bundle. Useful for snapshotting the
     * output of `generate()` for inspection in tests. */
    static fromBundle(bundle: Bundle, prefix = ""): MemoryFs {
        const mem = new MemoryFs();
        for (const entry of bundle) {
            const absPath = prefix ? `${prefix}/${entry.path}`.replace(/\/+/g, "/") : entry.path;
            mem.insertFile(absPath, entry.bytes);
        }
        return mem;
    }

    /** Seed helper — primarily for tests. Paths must be absolute. */
    insertFile(path: string, content: Uint8Array | string): void {
        if (!isAbsolute(path)) {
            throw new Error(`MemoryFs: path must be absolute: ${path}`);
        }
        const bytes =
            typeof content === "string" ? new TextEncoder().encode(content) : content;
        this.files.set(path, bytes);
    }

    /** Serialize to the wasm input Bundle shape. Paths are emitted as-is. */
    toBundle(): Bundle {
        return [...this.files.entries()]
            .map(([path, bytes]) => ({ path, bytes }))
            .sort((a, b) => a.path.localeCompare(b.path));
    }

    /** Access raw bytes by path. Returns undefined if absent. */
    get(path: string): Uint8Array | undefined {
        return this.files.get(path);
    }

    /** True iff a file at `path` is present. */
    has(path: string): boolean {
        return this.files.has(path);
    }

    /** Read out the current contents — primarily for tests and snapshots. */
    snapshot(): { files: Record<string, Uint8Array> } {
        return { files: Object.fromEntries(this.files) };
    }
}

export type { Bundle } from "../wasm/types.ts";
