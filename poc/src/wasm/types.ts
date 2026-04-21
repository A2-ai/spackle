// WASM-SIDE — types that cross the wasm-bindgen boundary.
//
// Hand-maintained to match the JSON shapes returned by Rust's
// `*_with_fs` exports in `src/wasm.rs`. Keep in sync with that file.

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

/** Response from `check()`. On success, includes the parsed config so
 * callers can render forms without re-parsing TOML. */
export type CheckResponse =
    | { valid: true; config: SpackleConfig; errors: [] }
    | { valid: false; errors: string[] };

/** Response from `validateSlotData()`. */
export type ValidationResponse =
    | { valid: true }
    | { valid: false; errors: string[] };

/** One rendered/copied entry in a generate response. */
export interface RenderedSummary {
    original_path: string;
    rendered_path: string;
}

/** Response from `generate()`. Hooks are unsupported in this milestone —
 * calling `generate()` with `runHooks = true` always returns
 * `{ ok: false, error: "..." }`. */
export type GenerateResponse =
    | { ok: true; rendered: RenderedSummary[] }
    | { ok: false; error: string };
