// Shared typed wrapper over the wasm-bindgen exports. Entry points supply
// their own WASM module location so server runtimes can keep the existing
// package-relative URL while browser bundlers can import the .wasm asset.

import type {
  Bundle,
  CheckResponse,
  GenerateResponse,
  PlanHooksResponse,
  SlotData,
  ValidationResponse,
} from "./types.ts";

type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;
type InitWasm = (
  moduleOrPath?:
    | { module_or_path: InitInput | Promise<InitInput> }
    | InitInput
    | Promise<InitInput>,
) => Promise<unknown>;

export interface RawWasmExports {
  initWasm: InitWasm;
  check(projectBundle: unknown, projectDir: string): string;
  validateSlotData(projectBundle: unknown, projectDir: string, slotDataJson: string): string;
  generate(
    projectBundle: unknown,
    projectDir: string,
    outDir: string,
    slotDataJson: string,
  ): unknown;
  planHooks(
    projectBundle: unknown,
    projectDir: string,
    outDir: string,
    dataJson: string,
    hookRanJson?: string | null,
  ): string;
}

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

export function createSpackleWasmLoader(
  raw: RawWasmExports,
  moduleOrPath: InitInput | Promise<InitInput>,
): () => Promise<SpackleWasm> {
  let cached: Promise<SpackleWasm> | null = null;

  return function loadSpackleWasm(): Promise<SpackleWasm> {
    if (!cached) cached = initialize(raw, moduleOrPath);
    return cached;
  };
}

async function initialize(
  raw: RawWasmExports,
  moduleOrPath: InitInput | Promise<InitInput>,
): Promise<SpackleWasm> {
  await raw.initWasm({ module_or_path: moduleOrPath });
  // wasm-bindgen's generated .d.ts types the JSON / JsValue exports as
  // `string` / `any`. Asserting to our response unions is the only way
  // to type this boundary: the shapes are guaranteed by the Rust side
  // (`crates/spackle-wasm/src/lib.rs`), which is the source of truth.
  return {
    check(projectBundle, projectDir) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(raw.check(projectBundle, projectDir)) as CheckResponse;
    },
    validateSlotData(projectBundle, projectDir, slotData) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(
        raw.validateSlotData(projectBundle, projectDir, JSON.stringify(slotData)),
      ) as ValidationResponse;
    },
    generate(projectBundle, projectDir, outDir, slotData) {
      // generate returns a JsValue (object), not a JSON string.
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return raw.generate(
        projectBundle,
        projectDir,
        outDir,
        JSON.stringify(slotData),
      ) as GenerateResponse;
    },
    planHooks(projectBundle, projectDir, outDir, data, hookRan) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(
        raw.planHooks(
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
