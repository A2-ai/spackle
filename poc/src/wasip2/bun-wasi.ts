// Minimal WASI preview2 host implementation for Bun. Used only to run
// the spackle wasip2 component — not a general-purpose WASI layer.
//
// Covers only the interfaces the component actually imports. Relies on
// Bun's sync fs primitives (`fs.openSync`, `fs.readSync`, etc.) so there
// are no worker threads, no SharedArrayBuffer, and no `process.binding`
// calls. That's the whole point: sidesteps the `preview2-shim` +
// `tcp_wrap` compatibility gap on Bun.
//
// Shape:
//   - `createWasiImports(preopens)` returns an object mirroring what jco's
//     generated `instantiate()` expects under the 'wasi:*' keys.
//   - Each descriptor is a userspace struct holding a host path + flags.
//     No real fds — we call fs.* per operation.
//   - Paths inside the component are resolved against the virtual preopen
//     name; mapped to the host path at descriptor creation.

import * as fs from "node:fs";
import { randomFillSync } from "node:crypto";
import {
    join as pathJoin,
    sep as pathSep,
    dirname as pathDirname,
    basename as pathBasename,
} from "node:path";

/* eslint-disable @typescript-eslint/no-explicit-any */

// --- error types ---

// jco's host-side runtime inspects `err.payload` to encode result<T,E>
// errors. If that property is missing and the error is an Error subclass,
// it's RE-THROWN instead of becoming a WIT error variant. So `payload`
// must be the variant value itself — for `wasi:filesystem/types` that's
// the error-code enum string ("no-entry", "access", etc.).
export class WasiError extends Error {
    public readonly payload: string;
    constructor(public code: string, msg?: string) {
        super(msg ?? code);
        this.payload = code;
    }
}

function codeFromNode(err: any): string {
    switch (err?.code) {
        case "EACCES":
            return "access";
        case "EAGAIN":
        case "EWOULDBLOCK":
            return "would-block";
        case "EBADF":
            return "bad-descriptor";
        case "EBUSY":
            return "busy";
        case "EEXIST":
            return "exist";
        case "EFBIG":
            return "file-too-large";
        case "EINVAL":
            return "invalid";
        case "EIO":
            return "io";
        case "EISDIR":
            return "is-directory";
        case "ELOOP":
            return "loop";
        case "EMFILE":
        case "ENFILE":
            return "insufficient-memory";
        case "ENAMETOOLONG":
            return "name-too-long";
        case "ENOENT":
            return "no-entry";
        case "ENOMEM":
            return "insufficient-memory";
        case "ENOSPC":
            return "insufficient-space";
        case "ENOTDIR":
            return "not-directory";
        case "ENOTEMPTY":
            return "not-empty";
        case "EPERM":
            return "not-permitted";
        case "EPIPE":
            return "pipe";
        case "EROFS":
            return "read-only";
        case "ESPIPE":
            return "invalid-seek";
        case "EXDEV":
            return "cross-device";
        default:
            return "io";
    }
}

// --- streams ---

class InputStream {
    private readonly hostPath: string;
    private fd: number | null = null;
    private position: bigint;

    constructor(hostPath: string, startOffset: bigint) {
        this.hostPath = hostPath;
        this.position = startOffset;
    }

    private ensureOpen(): number {
        if (this.fd === null) {
            this.fd = fs.openSync(this.hostPath, "r");
        }
        return this.fd;
    }

    read(len: bigint): Uint8Array {
        return this.blockingRead(len);
    }

    blockingRead(len: bigint): Uint8Array {
        const fd = this.ensureOpen();
        const max = Number(len);
        if (max === 0) return new Uint8Array(0);
        const buf = Buffer.alloc(max);
        const bytesRead = fs.readSync(fd, buf, 0, max, Number(this.position));
        if (bytesRead === 0) {
            // WIT models EOF via a StreamError (tag: 'closed'). jco's host-
            // side runtime converts thrown errors with a `.payload` matching
            // the variant into a StreamError.
            const err = new Error("stream closed") as any;
            err.payload = { tag: "closed" };
            throw err;
        }
        this.position += BigInt(bytesRead);
        return buf.subarray(0, bytesRead);
    }

    [Symbol.dispose]() {
        if (this.fd !== null) {
            try {
                fs.closeSync(this.fd);
            } catch {
                // best-effort close
            }
            this.fd = null;
        }
    }
}

class OutputStream {
    private readonly hostPath: string;
    private readonly mode: "write" | "append";
    private fd: number | null = null;
    private position: bigint;

    constructor(hostPath: string, mode: "write" | "append", startOffset: bigint) {
        this.hostPath = hostPath;
        this.mode = mode;
        this.position = startOffset;
    }

    private ensureOpen(): number {
        if (this.fd === null) {
            const flags = this.mode === "append" ? "a" : "w";
            this.fd = fs.openSync(this.hostPath, flags);
        }
        return this.fd;
    }

    checkWrite(): bigint {
        // Max writable bytes — we accept arbitrary sizes.
        return BigInt(Number.MAX_SAFE_INTEGER);
    }

    write(contents: Uint8Array): void {
        const fd = this.ensureOpen();
        const written = fs.writeSync(
            fd,
            contents,
            0,
            contents.length,
            this.mode === "append" ? null : Number(this.position),
        );
        this.position += BigInt(written);
    }

    blockingWriteAndFlush(contents: Uint8Array): void {
        this.write(contents);
        this.blockingFlush();
    }

    blockingFlush(): void {
        if (this.fd !== null) {
            try {
                fs.fsyncSync(this.fd);
            } catch {
                // some filesystems don't support fsync — ignore
            }
        }
    }

    [Symbol.dispose]() {
        if (this.fd !== null) {
            try {
                fs.closeSync(this.fd);
            } catch {
                // best-effort
            }
            this.fd = null;
        }
    }
}

// --- directory entry stream ---

class DirectoryEntryStream {
    private idx = 0;
    constructor(private readonly entries: Array<{ name: string; type: string }>) {}

    readDirectoryEntry(): { type: string; name: string } | undefined {
        if (this.idx >= this.entries.length) return undefined;
        const e = this.entries[this.idx++];
        return { type: e.type, name: e.name };
    }
}

// --- descriptor ---

function statTypeFromDirent(de: fs.Dirent): string {
    if (de.isDirectory()) return "directory";
    if (de.isFile()) return "regular-file";
    if (de.isSymbolicLink()) return "symbolic-link";
    if (de.isBlockDevice()) return "block-device";
    if (de.isCharacterDevice()) return "character-device";
    if (de.isFIFO()) return "fifo";
    if (de.isSocket()) return "socket";
    return "unknown";
}

function statTypeFromStats(st: fs.Stats): string {
    if (st.isDirectory()) return "directory";
    if (st.isFile()) return "regular-file";
    if (st.isSymbolicLink()) return "symbolic-link";
    if (st.isBlockDevice()) return "block-device";
    if (st.isCharacterDevice()) return "character-device";
    if (st.isFIFO()) return "fifo";
    if (st.isSocket()) return "socket";
    return "unknown";
}

function datetimeFromMs(ms: number): { seconds: bigint; nanoseconds: number } {
    const seconds = BigInt(Math.floor(ms / 1000));
    const nanoseconds = Math.floor((ms - Number(seconds) * 1000) * 1e6);
    return { seconds, nanoseconds };
}

function toDescriptorStat(st: fs.Stats) {
    return {
        type: statTypeFromStats(st),
        linkCount: BigInt(st.nlink),
        size: BigInt(st.size),
        dataAccessTimestamp: datetimeFromMs(st.atimeMs),
        dataModificationTimestamp: datetimeFromMs(st.mtimeMs),
        statusChangeTimestamp: datetimeFromMs(st.ctimeMs),
    };
}

class Descriptor {
    // `hostPath` must be canonical (symlinks resolved). The factory
    // canonicalizes preopen roots before constructing the root descriptor,
    // and `resolveChild()` canonicalizes every child path before passing
    // it to the child Descriptor constructor.
    constructor(
        public readonly hostPath: string,
        public readonly isPreopenRoot = false,
    ) {}

    /**
     * Resolve a relative path against this descriptor, returning the
     * canonical host path with containment enforced.
     *
     * Two-layer defense:
     *
     * 1. **Prefix-sep containment.** `resolved === hostPath` or
     *    `resolved.startsWith(hostPath + path.sep)`. Naive prefix check
     *    (`startsWith(hostPath)`) would allow sibling collisions like
     *    `/workspace` vs `/workspace-evil`; adding the separator closes
     *    that.
     * 2. **Symlink resolution.** `realpath`-resolve the target (or its
     *    parent, for create-flow paths). A symlink inside the workspace
     *    pointing to `/etc/passwd` would pass a raw prefix check; the
     *    post-canonicalization check rejects it.
     *
     * Known residual hazard: TOCTOU. Between the realpath check here and
     * the subsequent `fs.openSync` in the caller, an attacker with write
     * access to the workspace could swap a file for a symlink. Closing
     * this fully requires kernel-level sandboxing (Landlock on Linux,
     * App Sandbox on macOS, etc.) — out of scope for a userspace WASI
     * shim. Production servers that accept untrusted templates should
     * layer OS-level sandboxing around the process.
     */
    private resolveChild(
        path: string,
        opts: { forCreate?: boolean } = {},
    ): string {
        if (path.startsWith("/")) {
            throw new WasiError("not-permitted", `absolute path rejected: ${path}`);
        }
        // path.join normalizes `..` segments; that alone doesn't prevent
        // all escapes but narrows the attack surface before canonicalize.
        const joined = pathJoin(this.hostPath, path);

        let canonical: string;
        if (opts.forCreate) {
            // Target may not exist yet — canonicalize the parent (which
            // must already exist under a preopen) and rejoin the basename.
            const parent = pathDirname(joined);
            try {
                canonical = pathJoin(fs.realpathSync(parent), pathBasename(joined));
            } catch (e) {
                throw new WasiError(codeFromNode(e));
            }
        } else {
            try {
                canonical = fs.realpathSync(joined);
            } catch (e) {
                throw new WasiError(codeFromNode(e));
            }
        }

        if (
            canonical !== this.hostPath &&
            !canonical.startsWith(this.hostPath + pathSep)
        ) {
            throw new WasiError(
                "not-permitted",
                `path escape: ${canonical} not contained in ${this.hostPath}`,
            );
        }
        return canonical;
    }

    readViaStream(offset: bigint): InputStream {
        return new InputStream(this.hostPath, offset);
    }

    writeViaStream(offset: bigint): OutputStream {
        return new OutputStream(this.hostPath, "write", offset);
    }

    appendViaStream(): OutputStream {
        return new OutputStream(this.hostPath, "append", 0n);
    }

    getFlags() {
        return { read: true, write: true };
    }

    getType(): string {
        try {
            return statTypeFromStats(fs.statSync(this.hostPath));
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    readDirectory(): DirectoryEntryStream {
        try {
            const entries = fs
                .readdirSync(this.hostPath, { withFileTypes: true })
                .map((de) => ({ name: de.name, type: statTypeFromDirent(de) }));
            return new DirectoryEntryStream(entries);
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    createDirectoryAt(path: string): void {
        // forCreate=true: the target directory does not exist yet; we can
        // only canonicalize the parent (which must).
        const target = this.resolveChild(path, { forCreate: true });
        try {
            fs.mkdirSync(target, { recursive: false });
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    stat() {
        try {
            return toDescriptorStat(fs.statSync(this.hostPath));
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    statAt(pathFlags: { symlinkFollow?: boolean }, path: string) {
        const target = this.resolveChild(path);
        try {
            const st = pathFlags.symlinkFollow
                ? fs.statSync(target)
                : fs.lstatSync(target);
            return toDescriptorStat(st);
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    openAt(
        _pathFlags: { symlinkFollow?: boolean },
        path: string,
        openFlags: {
            create?: boolean;
            directory?: boolean;
            exclusive?: boolean;
            truncate?: boolean;
        },
        _descriptorFlags: { read?: boolean; write?: boolean },
    ): Descriptor {
        // Create flow (file or dir may not exist) vs open-existing take
        // different canonicalization paths — forCreate canonicalizes the
        // parent, the default canonicalizes the target itself.
        const target = this.resolveChild(path, { forCreate: !!openFlags.create });
        if (openFlags.create) {
            // Ensure parent exists, create the file if missing.
            try {
                if (openFlags.directory) {
                    fs.mkdirSync(target, { recursive: false });
                } else {
                    const fd = fs.openSync(
                        target,
                        openFlags.exclusive ? "wx" : openFlags.truncate ? "w" : "a+",
                    );
                    fs.closeSync(fd);
                }
            } catch (e: any) {
                if (
                    !openFlags.exclusive &&
                    (e.code === "EEXIST" || e.code === "EISDIR")
                ) {
                    // fine — it already exists and we didn't require exclusive
                } else {
                    throw new WasiError(codeFromNode(e));
                }
            }
        } else if (openFlags.truncate) {
            try {
                fs.truncateSync(target, 0);
            } catch (e) {
                throw new WasiError(codeFromNode(e));
            }
        }
        return new Descriptor(target);
    }

    readlinkAt(path: string): string {
        const target = this.resolveChild(path);
        try {
            return fs.readlinkSync(target);
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    metadataHash() {
        try {
            const st = fs.statSync(this.hostPath);
            return {
                lower: BigInt(st.ino) & 0xffffffffffffffffn,
                upper: BigInt(st.dev) & 0xffffffffffffffffn,
            };
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }

    metadataHashAt(pathFlags: { symlinkFollow?: boolean }, path: string) {
        const target = this.resolveChild(path);
        try {
            const st = pathFlags.symlinkFollow
                ? fs.statSync(target)
                : fs.lstatSync(target);
            return {
                lower: BigInt(st.ino) & 0xffffffffffffffffn,
                upper: BigInt(st.dev) & 0xffffffffffffffffn,
            };
        } catch (e) {
            throw new WasiError(codeFromNode(e));
        }
    }
}

// --- filesystemErrorCode ---

function filesystemErrorCode(err: unknown): string | undefined {
    if (err instanceof WasiError) return err.code;
    if (err && typeof err === "object" && "code" in err) {
        return codeFromNode(err);
    }
    return undefined;
}

// --- the import object builder ---

export interface CreateImportsOptions {
    preopens: Record<string, string>; // virtual path -> host path
    env?: Record<string, string | undefined>;
}

export function createWasiImports(opts: CreateImportsOptions) {
    // Canonicalize preopen host paths up front. Child descriptors created
    // via `openAt` are already canonical (resolveChild canonicalizes);
    // the containment check compares against `this.hostPath`, so the
    // root must be canonical too or every child will appear to "escape."
    // On macOS `/tmp → /private/tmp` is the canonical example.
    const preopenDescriptors = Object.entries(opts.preopens).map(
        ([virtualPath, hostPath]) => {
            let canonical: string;
            try {
                canonical = fs.realpathSync(hostPath);
            } catch (e) {
                throw new Error(
                    `preopen path does not exist or is not accessible: ${hostPath} (${(e as any)?.code ?? "unknown"})`,
                );
            }
            return [new Descriptor(canonical, true), virtualPath] as [
                Descriptor,
                string,
            ];
        },
    );

    const env = opts.env ?? process.env;

    return {
        "wasi:cli/environment": {
            getEnvironment: () =>
                Object.entries(env).map(
                    ([k, v]) => [k, v ?? ""] as [string, string],
                ),
            getArguments: () => [],
            initialCwd: () => undefined,
        },
        "wasi:cli/exit": {
            exit: (_code: boolean) => {
                // `exit` in WIT takes a result. We just throw so the generator
                // unwinds — we never want the component to actually exit the
                // host process.
                const err = new Error("component called exit") as any;
                err.payload = { tag: "exit" };
                throw err;
            },
        },
        "wasi:cli/stderr": {
            getStderr: () => new OutputStream("/dev/null", "append", 0n),
        },
        "wasi:cli/stdout": {
            getStdout: () => new OutputStream("/dev/null", "append", 0n),
        },
        "wasi:cli/stdin": {
            getStdin: () => new InputStream("/dev/null", 0n),
        },
        "wasi:cli/terminal-input": { TerminalInput: class {} },
        "wasi:cli/terminal-output": { TerminalOutput: class {} },
        "wasi:cli/terminal-stderr": { getTerminalStderr: () => undefined },
        "wasi:cli/terminal-stdin": { getTerminalStdin: () => undefined },
        "wasi:cli/terminal-stdout": { getTerminalStdout: () => undefined },
        "wasi:clocks/monotonic-clock": {
            now: () => process.hrtime.bigint(),
            resolution: () => 1n,
            subscribeInstant: () => ({}),
            subscribeDuration: () => ({}),
        },
        "wasi:clocks/wall-clock": {
            now: () => datetimeFromMs(Date.now()),
            resolution: () => ({ seconds: 0n, nanoseconds: 1_000_000 }),
        },
        "wasi:filesystem/preopens": {
            getDirectories: () => preopenDescriptors,
        },
        "wasi:filesystem/types": {
            Descriptor,
            DirectoryEntryStream,
            filesystemErrorCode,
        },
        "wasi:io/error": {
            Error: class {
                toDebugString() {
                    return "io error";
                }
            },
        },
        "wasi:io/streams": {
            InputStream,
            OutputStream,
        },
        "wasi:random/random": {
            getRandomBytes: (len: bigint) => {
                const n = Number(len);
                const out = new Uint8Array(n);
                randomFillSync(out);
                return out;
            },
            getRandomU64: () => {
                const out = new Uint8Array(8);
                randomFillSync(out);
                const view = new DataView(out.buffer);
                return view.getBigUint64(0, true);
            },
        },
    };
}
