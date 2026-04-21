// WASM-SIDE — types that cross the wasm-bindgen boundary.
//
// Hand-maintained to match the shapes emitted by Rust's bundle-in /
// bundle-out exports in `crates/spackle-wasm/src/lib.rs`. Keep in sync.

export type SlotType = "String" | "Number" | "Boolean";

export interface Slot {
    key: string;
    type?: SlotType;
    name?: string;
    description?: string;
    default?: string;
    needs?: string[];
}

export interface Hook {
    key: string;
    command: string[];
    /** Optional conditional (tera template string, evaluated as bool). */
    if?: string | null;
    needs?: string[];
    name?: string | null;
    description?: string | null;
    /** Whether the hook runs by default when no per-hook override is set. */
    default?: boolean | null;
}

export interface SpackleConfig {
    name: string | null;
    ignore: string[];
    slots: Slot[];
    hooks: Hook[];
}

/** Slot values the caller supplies. Always string-valued — Rust parses
 * / coerces against each slot's declared type. */
export type SlotData = Record<string, string>;

/** A single file in a project input or generated output bundle. Paths
 * are virtual — absolute from the bundle's root for inputs (e.g.
 * `/project/spackle.toml`) and relative from `outDir` for outputs
 * (e.g. `src/main.rs`). */
export interface BundleEntry {
    path: string;
    bytes: Uint8Array;
}

/** The shape spackle-wasm takes as input (check / validate / generate)
 * and returns as output (generate's rendered subtree). */
export type Bundle = BundleEntry[];

/** Response from `check()`. On success, includes the parsed config so
 * callers can render forms without re-parsing TOML. */
export type CheckResponse =
    | { valid: true; config: SpackleConfig; errors: [] }
    | { valid: false; errors: string[] };

/** Response from `validateSlotData()`. */
export type ValidationResponse =
    | { valid: true }
    | { valid: false; errors: string[] };

/** Response from `generate()`.
 *
 * `files` carries the rendered output subtree with paths **relative to
 * outDir**. `dirs` carries the directory subtree (also relative) so
 * empty dirs survive the bundle round-trip — the native `copy` pass
 * `create_dir_all`s every directory entry it walks, and we match that
 * behavior host-side by mkdir'ing each dir even if no files live under
 * it.
 *
 * Hooks are unsupported in this milestone — calling `generate()` with
 * `runHooks = true` always returns `{ ok: false, error: "..." }`. */
export type GenerateResponse =
    | { ok: true; files: Bundle; dirs: string[] }
    | { ok: false; error: string };
