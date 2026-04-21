// Shared types + error helpers for the `SpackleFs` contract.
//
// Both the Rust side (`src/wasm_fs.rs`) and any host adapter implement
// this shape. Adapters MUST:
// - treat every method as synchronous
// - throw `{ kind, message }` objects on error (not plain `Error`)
// - never return a Promise
//
// Violations are handled gracefully on the Rust side (collapse to
// `io::ErrorKind::Other`) but the caller loses type info on the kind.

export type SpackleFileType = "file" | "directory" | "symlink" | "other";

export type SpackleFsErrorKind =
    | "not-found"
    | "permission-denied"
    | "already-exists"
    | "not-a-directory"
    | "is-a-directory"
    | "invalid-path"
    | "other";

export interface SpackleFsError {
    kind: SpackleFsErrorKind;
    message: string;
}

export interface SpackleFileEntry {
    name: string;
    type: SpackleFileType;
}

export interface SpackleFileStat {
    type: SpackleFileType;
    size: number | bigint;
}

export interface SpackleFs {
    readFile(path: string): Uint8Array;
    writeFile(path: string, content: Uint8Array): void;
    createDirAll(path: string): void;
    listDir(path: string): SpackleFileEntry[];
    copyFile(src: string, dst: string): void;
    exists(path: string): boolean;
    stat(path: string): SpackleFileStat;
}

/**
 * Construct a typed `SpackleFsError`. Adapters should throw this (not
 * a plain `Error`) so the Rust bridge can decode the `kind` field.
 */
export function fsError(kind: SpackleFsErrorKind, message: string): SpackleFsError {
    return { kind, message };
}

/** Type-guard for distinguishing thrown SpackleFsErrors from other exceptions. */
export function isSpackleFsError(x: unknown): x is SpackleFsError {
    return (
        typeof x === "object" &&
        x !== null &&
        "kind" in x &&
        typeof (x as { kind: unknown }).kind === "string"
    );
}

/**
 * Map a Node/Bun fs errno code to a `SpackleFsErrorKind`. Used by
 * adapters that wrap `fs.*Sync` to preserve error-kind information
 * across the bridge.
 */
export function errorKindFromNodeCode(code: unknown): SpackleFsErrorKind {
    switch (code) {
        case "ENOENT":
            return "not-found";
        case "EACCES":
        case "EPERM":
            return "permission-denied";
        case "EEXIST":
            return "already-exists";
        case "ENOTDIR":
            return "not-a-directory";
        case "EISDIR":
            return "is-a-directory";
        default:
            return "other";
    }
}
