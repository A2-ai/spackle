// Shared typed wrapper over the wasm-bindgen exports. Entry points supply
// their own WASM module location so server runtimes can keep the existing
// package-relative URL while browser bundlers can import the .wasm asset.

import type {
  Bundle,
  CheckResponse,
  GenerateResult,
  GenerateStreamEntry,
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

export type SpackleWasmModuleSource = InitInput | Promise<InitInput>;

export interface ConfigureSpackleWasmOptions {
  moduleOrPath: SpackleWasmModuleSource;
}

export interface RawWasmExports {
  initWasm: InitWasm;
  check(projectBundle: unknown, projectDir: string): string;
  validateSlotData(projectBundle: unknown, projectDir: string, slotDataJson: string): string;
  /** Streams output entries through `onEntry` synchronously while the
   * wasm call runs; returns a terminal envelope. The callback receives
   * raw `{kind, path, bytes?}` objects from serde-wasm-bindgen — the
   * typed wrapper below narrows this for callers. */
  generate(
    projectBundle: unknown,
    projectDir: string,
    outDir: string,
    slotDataJson: string,
    onEntry: (event: unknown) => void,
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
  /** Run generate, streaming each output file/dir through `onEntry` as
   * it's produced. Returns once Rust has finished walking the project.
   * Bytes are dropped after each callback returns — peak memory is
   * bounded by what the host buffers in `onEntry`, not by the size of
   * the rendered output. */
  generate(
    projectBundle: Bundle,
    projectDir: string,
    outDir: string,
    slotData: SlotData,
    onEntry: (event: GenerateStreamEntry) => void,
  ): GenerateResult;
  planHooks(
    projectBundle: Bundle,
    projectDir: string,
    outDir: string,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
  ): PlanHooksResponse;
}

export interface SpackleWasmLoaderPair {
  loadSpackleWasm: () => Promise<SpackleWasm>;
  configureSpackleWasm: (options: ConfigureSpackleWasmOptions) => void;
}

export function createSpackleWasmLoader(
  raw: RawWasmExports,
  defaultModuleOrPath?: SpackleWasmModuleSource,
): SpackleWasmLoaderPair {
  let cached: Promise<SpackleWasm> | null = null;
  let override: SpackleWasmModuleSource | null = null;

  function configureSpackleWasm(options: ConfigureSpackleWasmOptions): void {
    if (cached) {
      throw new Error("configureSpackleWasm must be called before loadSpackleWasm");
    }
    override = options.moduleOrPath;
  }

  function loadSpackleWasm(): Promise<SpackleWasm> {
    if (!cached) {
      const source = override ?? defaultModuleOrPath;
      if (source === undefined) {
        throw new Error(
          "Spackle WASM module source is not configured. Call configureSpackleWasm({ moduleOrPath }) before using @a2-ai/spackle.",
        );
      }
      cached = initialize(raw, source);
    }
    return cached;
  }

  return { loadSpackleWasm, configureSpackleWasm };
}

async function initialize(
  raw: RawWasmExports,
  moduleOrPath: SpackleWasmModuleSource,
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
    generate(projectBundle, projectDir, outDir, slotData, onEntry) {
      // generate returns a JsValue (object), not a JSON string. The
      // callback receives serde-wasm-bindgen-shaped {kind, path, bytes?}
      // objects — assert the narrowed entry shape here so consumers
      // see the typed union.
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return raw.generate(
        projectBundle,
        projectDir,
        outDir,
        JSON.stringify(slotData),
        // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
        (event: unknown) => onEntry(event as GenerateStreamEntry),
      ) as GenerateResult;
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
