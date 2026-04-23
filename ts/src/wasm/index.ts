// WASM-SIDE — thin typed wrapper over the wasm-bindgen exports.
//
// The wasm-bindgen `--target web` output at `../../pkg/spackle_wasm.js`
// exposes four pure-function exports plus a default `init`. Each takes
// a project bundle (`Array<{path, bytes: Uint8Array}>`) — no fs
// callbacks, no I/O; Rust runs the whole generation against an
// in-memory fs.

// The web target requires an explicit `await initWasm(...)` before the
// other exports work. `loadSpackleWasm()` below caches that promise so
// subsequent callers skip the fetch. Aliased off the default export to
// dodge a name collision with the named `init` export that the crate
// also ships (used internally by `#[wasm_bindgen(start)]`).
import initWasm, {
  check as wasm_check,
  generate as wasm_generate,
  plan_hooks as wasm_plan_hooks,
  validate_slot_data as wasm_validate_slot_data,
} from "../../pkg/spackle_wasm.js";
import type {
  Bundle,
  CheckResponse,
  GenerateResponse,
  PlanHooksResponse,
  SlotData,
  ValidationResponse,
} from "./types.ts";

/** Typed wrapper over the raw WASM exports. All methods are synchronous
 * against the wasm instance; the only async step is `loadSpackleWasm()`. */
export interface SpackleWasm {
  check(projectBundle: Bundle, projectDir: string): CheckResponse;
  validateSlotData(
    projectBundle: Bundle,
    projectDir: string,
    slotData: SlotData,
  ): ValidationResponse;
  generate(
    projectBundle: Bundle,
    projectDir: string,
    outDir: string,
    slotData: SlotData,
  ): GenerateResponse;
  planHooks(
    projectBundle: Bundle,
    projectDir: string,
    outDir: string,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
  ): PlanHooksResponse;
}

let cached: Promise<SpackleWasm> | null = null;

/** Load the WASM module once per process. Subsequent calls return the
 * same client. Safe to await concurrently. */
export function loadSpackleWasm(): Promise<SpackleWasm> {
  if (!cached) cached = initialize();
  return cached;
}

async function initialize(): Promise<SpackleWasm> {
  await initWasm({
    module_or_path: new URL("../../pkg/spackle_wasm_bg.wasm", import.meta.url),
  });
  // wasm-bindgen's generated .d.ts types the JSON / JsValue exports as
  // `string` / `any`. Asserting to our response unions is the only way
  // to type this boundary — the shapes are guaranteed by the Rust
  // side (`crates/spackle-wasm/src/lib.rs`), which is the actual
  // source of truth. Runtime validation (zod etc.) would be strictly
  // defensive overhead.
  return {
    check(projectBundle, projectDir) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(wasm_check(projectBundle, projectDir)) as CheckResponse;
    },
    validateSlotData(projectBundle, projectDir, slotData) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(
        wasm_validate_slot_data(projectBundle, projectDir, JSON.stringify(slotData)),
      ) as ValidationResponse;
    },
    generate(projectBundle, projectDir, outDir, slotData) {
      // generate returns a JsValue (object), not a JSON string.
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return wasm_generate(
        projectBundle,
        projectDir,
        outDir,
        JSON.stringify(slotData),
      ) as GenerateResponse;
    },
    planHooks(projectBundle, projectDir, outDir, data, hookRan) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(
        wasm_plan_hooks(
          projectBundle,
          projectDir,
          outDir,
          JSON.stringify(data),
          hookRan === undefined ? undefined : JSON.stringify(hookRan),
        ),
      ) as PlanHooksResponse;
    },
  };
}

export type {
  Bundle,
  BundleEntry,
  CheckResponse,
  GenerateResponse,
  Hook,
  HookPlanEntry,
  PlanHooksResponse,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  ValidationResponse,
} from "./types.ts";
