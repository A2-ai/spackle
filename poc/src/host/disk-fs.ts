// Local-disk `SpackleFs` backend. Rooted at a workspace directory;
// every path the component passes is canonicalized against that root
// and rejected if it escapes.
//
// Uses sync fs primitives only (`fs.readFileSync`, `fs.writeFileSync`,
// ...) — the bridge contract is synchronous.

import {
    copyFileSync,
    existsSync,
    lstatSync,
    mkdirSync,
    readFileSync,
    readdirSync,
    realpathSync,
    writeFileSync,
} from "node:fs";
import {
    basename as pathBasename,
    dirname as pathDirname,
    isAbsolute,
    resolve as pathResolve,
    sep as pathSep,
} from "node:path";

import {
    errorKindFromNodeCode,
    fsError,
    type SpackleFileEntry,
    type SpackleFileStat,
    type SpackleFileType,
    type SpackleFs,
} from "./spackle-fs";

function fileTypeFromStat(
    st: { isDirectory(): boolean; isFile(): boolean; isSymbolicLink(): boolean },
): SpackleFileType {
    if (st.isDirectory()) return "directory";
    if (st.isFile()) return "file";
    if (st.isSymbolicLink()) return "symlink";
    return "other";
}

/**
 * Throws a typed `SpackleFsError` wrapping a Node/Bun fs exception. The
 * `code` field on thrown Node errors (ENOENT, EACCES, ...) drives the
 * kind mapping; the error message is preserved for operator-debug.
 */
function throwFromNode(op: string, path: string, e: unknown): never {
    const code = (e as { code?: string } | null)?.code;
    const message = (e as { message?: string } | null)?.message ?? String(e);
    throw fsError(
        errorKindFromNodeCode(code),
        `${op}(${path}): ${message}`,
    );
}

export interface DiskFsOptions {
    /**
     * Absolute path to the workspace root. The adapter refuses any path
     * that canonicalizes outside this root. Must exist on disk (so we
     * can canonicalize it at construction time).
     */
    workspaceRoot: string;
}

export class DiskFs implements SpackleFs {
    private readonly root: string;

    constructor(opts: DiskFsOptions) {
        if (!isAbsolute(opts.workspaceRoot)) {
            throw fsError(
                "invalid-path",
                `DiskFs workspaceRoot must be absolute: ${opts.workspaceRoot}`,
            );
        }
        try {
            this.root = realpathSync(opts.workspaceRoot);
        } catch (e) {
            const code = (e as { code?: string } | null)?.code;
            throw fsError(
                errorKindFromNodeCode(code),
                `DiskFs: workspaceRoot not accessible: ${opts.workspaceRoot}`,
            );
        }
    }

    /**
     * Canonicalize + contain. Target path must end up under
     * `this.root` or we throw `invalid-path`.
     *
     * ============================================================
     * LOAD-BEARING DESIGN CHOICE — read before editing this method.
     * ============================================================
     *
     * There are two modes:
     *
     * - **Default (read / stat flows).** Call `realpathSync(target)`
     *   on the whole path. Target must exist. A full canonicalize
     *   catches symlinks anywhere in the path — the strongest
     *   containment check we can make without kernel-level sandboxing.
     *
     * - **`forCreate` mode (write / mkdir / copy-destination flows).**
     *   The target probably doesn't exist yet; neither do several of
     *   its ancestors (think `mkdir -p /root/a/b/c`). `realpathSync`
     *   on a nonexistent path throws `ENOENT`, so naive "canonicalize
     *   the target's parent" fails too (the parent may also not exist).
     *
     *   The chosen strategy: **walk up until we hit an existing
     *   ancestor, canonicalize that, and rejoin the remaining
     *   segments lexically.** This is a deliberate tradeoff:
     *
     *     - Catches symlink escapes inside any _existing_ prefix of
     *       the path (the realistic attack surface — an attacker
     *       has to plant the symlink somewhere that already exists).
     *     - Does NOT catch symlinks that will be created later as
     *       part of the path. We consider that a TOCTOU problem;
     *       OS-level sandboxing (Landlock on Linux, etc.) is the
     *       real fix and is out of scope for userspace JS.
     *     - Allows legitimate multi-level creation (`mkdir -p`
     *       semantics) — single-ancestor canonicalization would
     *       reject those as not-found.
     *
     * If you refactor this method, preserve both properties: existing
     * prefix is fully canonicalized, multi-level creation works.
     * `poc/tests/disk-fs.test.ts` pins these behaviors — run those
     * tests before landing any change here.
     */
    private resolve(path: string, opts: { forCreate?: boolean } = {}): string {
        if (!isAbsolute(path)) {
            throw fsError("invalid-path", `path must be absolute: ${path}`);
        }

        // First, resolve `..` segments lexically so we don't feed them
        // into realpath (which would error on nonexistent intermediates).
        const lexical = pathResolve(path);

        let canonical: string;
        if (opts.forCreate) {
            // Walk up from the target until we hit an existing ancestor,
            // then canonicalize that and rejoin the remaining segments.
            let existing = lexical;
            const tail: string[] = [];
            while (!existsSync(existing)) {
                tail.unshift(pathBasename(existing));
                const parent = pathDirname(existing);
                if (parent === existing) {
                    // Walked to root — nothing existed. Unusual on a real
                    // disk; surface as not-found rather than hang.
                    throw fsError(
                        "not-found",
                        `DiskFs.resolve(${path}): no existing ancestor`,
                    );
                }
                existing = parent;
            }
            try {
                const canonicalExisting = realpathSync(existing);
                canonical = tail.length
                    ? `${canonicalExisting}${pathSep}${tail.join(pathSep)}`
                    : canonicalExisting;
            } catch (e) {
                const code = (e as { code?: string } | null)?.code;
                throw fsError(
                    errorKindFromNodeCode(code),
                    `DiskFs.resolve(${path}): ${(e as Error).message ?? String(e)}`,
                );
            }
        } else {
            // Target must exist; full canonicalize catches any symlink
            // escape on the whole path.
            try {
                canonical = realpathSync(lexical);
            } catch (e) {
                const code = (e as { code?: string } | null)?.code;
                throw fsError(
                    errorKindFromNodeCode(code),
                    `DiskFs.resolve(${path}): ${(e as Error).message ?? String(e)}`,
                );
            }
        }

        if (canonical !== this.root && !canonical.startsWith(this.root + pathSep)) {
            throw fsError(
                "invalid-path",
                `path escapes workspaceRoot: ${canonical} not under ${this.root}`,
            );
        }
        return canonical;
    }

    readFile(path: string): Uint8Array {
        const p = this.resolve(path);
        try {
            return readFileSync(p);
        } catch (e) {
            throwFromNode("readFile", p, e);
        }
    }

    writeFile(path: string, content: Uint8Array): void {
        const p = this.resolve(path, { forCreate: true });
        try {
            writeFileSync(p, content);
        } catch (e) {
            throwFromNode("writeFile", p, e);
        }
    }

    createDirAll(path: string): void {
        const p = this.resolve(path, { forCreate: true });
        try {
            // `recursive: true` already treats an existing directory as
            // a no-op (Node fs contract: "If the directory already
            // exists, no error is thrown"). EEXIST only surfaces when
            // the path exists as something OTHER than a directory — a
            // genuine structural error we want the caller to see.
            mkdirSync(p, { recursive: true });
        } catch (e) {
            throwFromNode("createDirAll", p, e);
        }
    }

    listDir(path: string): SpackleFileEntry[] {
        const p = this.resolve(path);
        try {
            return readdirSync(p, { withFileTypes: true }).map((de) => ({
                name: de.name,
                type: fileTypeFromStat(de),
            }));
        } catch (e) {
            throwFromNode("listDir", p, e);
        }
    }

    copyFile(src: string, dst: string): void {
        const sp = this.resolve(src);
        const dp = this.resolve(dst, { forCreate: true });
        try {
            copyFileSync(sp, dp);
        } catch (e) {
            throwFromNode("copyFile", dp, e);
        }
    }

    exists(path: string): boolean {
        try {
            const p = this.resolve(path);
            return existsSync(p);
        } catch {
            // Containment violations or canonicalize errors surface as
            // "doesn't exist from the adapter's point of view" — matches
            // the trait's best-effort contract.
            return false;
        }
    }

    stat(path: string): SpackleFileStat {
        // Containment check: `resolve` canonicalizes through symlinks
        // and throws `invalid-path` if the target escapes workspaceRoot.
        // We call it for the side effect, not for its return value — if
        // we used the canonical path for the actual lstat, we'd lose
        // visibility of the _argument_ being a symlink (realpath strips
        // symlinks, then lstat sees only the target's type).
        //
        // Instead, lstat the lexical form so a symlink query reports
        // `type: "symlink"` — matches `listDir`'s Dirent-based typing
        // and keeps walkers from recursing into symlink targets.
        this.resolve(path);
        const lexical = pathResolve(path);
        try {
            const st = lstatSync(lexical);
            return {
                type: fileTypeFromStat(st),
                size: st.size,
            };
        } catch (e) {
            throwFromNode("stat", lexical, e);
        }
    }
}
