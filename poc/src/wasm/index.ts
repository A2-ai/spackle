// WASM-SIDE — thin typed wrapper over the wasm-bindgen exports.
//
// The wasm-pack output at `../../pkg/spackle.js` exposes three
// fs-backed functions plus `init`. Each takes a `SpackleFs` object
// (see `../host/spackle-fs.ts`) that the component calls back into
// for filesystem operations. No direct disk access on the Rust side.
//
// `loadSpackleWasm()` memoizes the initialization Promise so concurrent
// callers share it.

// wasm-pack's `--target nodejs` output instantiates the module eagerly
// at import time (see the tail of `pkg/spackle.js`). No init step, no
// default export — just the named exports below.
import {
    check_with_fs,
    generate_with_fs,
    validate_slot_data_with_fs,
} from "../../pkg/spackle.js";
import type { SpackleFs } from "../host/spackle-fs";
import type {
    CheckResponse,
    GenerateResponse,
    ValidationResponse,
} from "./types.ts";

/** Typed wrapper over the raw WASM exports. All methods are synchronous
 * against the wasm instance; the only async step is `loadSpackleWasm()`. */
export interface SpackleWasm {
    check(projectDir: string, fs: SpackleFs): CheckResponse;
    validateSlotData(
        projectDir: string,
        slotData: Record<string, string>,
        fs: SpackleFs,
    ): ValidationResponse;
    generate(
        projectDir: string,
        outDir: string,
        slotData: Record<string, string>,
        runHooks: boolean,
        fs: SpackleFs,
    ): GenerateResponse;
}

let cached: Promise<SpackleWasm> | null = null;

/** Load the WASM module once per process. Subsequent calls return the
 * same client. Safe to await concurrently. Kept async for symmetry with
 * `--target web` output (which DOES need an explicit init), so callers
 * can switch targets without changing the orchestration code. */
export function loadSpackleWasm(): Promise<SpackleWasm> {
    if (!cached) cached = initialize();
    return cached;
}

async function initialize(): Promise<SpackleWasm> {
    return {
        check(projectDir, fs) {
            return JSON.parse(check_with_fs(projectDir, fs)) as CheckResponse;
        },
        validateSlotData(projectDir, slotData, fs) {
            return JSON.parse(
                validate_slot_data_with_fs(projectDir, JSON.stringify(slotData), fs),
            ) as ValidationResponse;
        },
        generate(projectDir, outDir, slotData, runHooks, fs) {
            return JSON.parse(
                generate_with_fs(
                    projectDir,
                    outDir,
                    JSON.stringify(slotData),
                    runHooks,
                    fs,
                ),
            ) as GenerateResponse;
        },
    };
}

export type {
    CheckResponse,
    GenerateResponse,
    Hook,
    RenderedSummary,
    Slot,
    SlotData,
    SlotType,
    SpackleConfig,
    ValidationResponse,
} from "./types.ts";
