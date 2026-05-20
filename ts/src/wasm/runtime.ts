// Shared typed wrapper over the wasm-bindgen exports. Entry points supply
// their own WASM module location so server runtimes can keep the existing
// package-relative URL while browser bundlers can import the .wasm asset.

import type {
  Bundle,
  CheckResponse,
  PlanHooksResponse,
  RenderFileResponse,
  RenderPathResponse,
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
  /** Returns `{ bytes, diagnostics }` as a `JsValue` object (not a
   * JSON string); the typed wrapper narrows it. */
  renderFile(templateBytes: Uint8Array, slotDataJson: string, virtualPath?: string | null): unknown;
  /** Returns `{ path, diagnostics }` as a `JsValue` object. */
  renderPath(pathTemplate: string, slotDataJson: string): unknown;
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
  /** Render a single template body. No template registry — cross-
   * template tags don't resolve. `check` rejects them so the
   * limitation surfaces consistently. `virtualPath` is optional;
   * shows up in error diagnostics for UI attribution. */
  renderFile(
    templateBytes: Uint8Array,
    slotData: SlotData,
    virtualPath?: string,
  ): RenderFileResponse;
  /** Render a single path template (e.g. `"src/{{ project }}.txt"`).
   * On error, `path` falls back to the input — branch on
   * `diagnostics`. */
  renderPath(pathTemplate: string, slotData: SlotData): RenderPathResponse;
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
    renderFile(templateBytes, slotData, virtualPath) {
      // render_file returns a JsValue object, not a JSON string.
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return raw.renderFile(
        templateBytes,
        JSON.stringify(slotData),
        virtualPath ?? null,
      ) as RenderFileResponse;
    },
    renderPath(pathTemplate, slotData) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return raw.renderPath(pathTemplate, JSON.stringify(slotData)) as RenderPathResponse;
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
