// Shared typed wrapper over the wasm-bindgen exports. Entry points supply
// their own WASM module location so server runtimes can keep the existing
// package-relative URL while browser bundlers can import the .wasm asset.

import type {
  Bundle,
  CheckResponse,
  GenerateResponse,
  PlanHooksResponse,
  RenderResponse,
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
  check(projectBundle: unknown): string;
  validateSlotData(projectBundle: unknown, slotDataJson: string): string;
  generate(
    projectBundle: unknown,
    slotDataJson: string,
    projectName?: string | null,
    outputName?: string | null,
  ): unknown;
  render(
    projectBundle: unknown,
    slotDataJson: string,
    projectName?: string | null,
    outputName?: string | null,
  ): unknown;
  planHooks(
    projectBundle: unknown,
    dataJson: string,
    hookRanJson?: string | null,
    projectName?: string | null,
    outputName?: string | null,
  ): string;
}

/**
 * Override the `_project_name` / `_output_name` Tera vars the renderer
 * injects. The bundle layout itself is a private invariant of the wasm
 * crate (entries always live under a fixed prefix); these are the
 * caller's only knobs on what shows up in rendered output.
 *
 * Each field is independently optional: unset means the default from
 * the wasm side (project: `config.name` or the basename of the fixed
 * virtual project dir; output: basename of the fixed virtual out dir,
 * or whatever the higher-level disk-backed wrappers in `spackle.ts`
 * decide to forward — typically `basename(realOutDir)`).
 */
export interface NameOverrides {
  projectName?: string;
  outputName?: string;
}

/** Typed wrapper over the raw WASM exports. All methods are synchronous
 * against the wasm instance; the only async step is `loadSpackleWasm()`. */
export interface SpackleWasm {
  check(projectBundle: Bundle): CheckResponse;
  validateSlotData(projectBundle: Bundle, slotData: SlotData): ValidationResponse;
  generate(projectBundle: Bundle, slotData: SlotData, names?: NameOverrides): GenerateResponse;
  /** Dynamic render-with-data, diagnostics-first. Never throws / never
   * returns `ok: false`. Empty `diagnostics` ⇒ clean render. */
  render(projectBundle: Bundle, slotData: SlotData, names?: NameOverrides): RenderResponse;
  planHooks(
    projectBundle: Bundle,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
    names?: NameOverrides,
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
    check(projectBundle) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(raw.check(projectBundle)) as CheckResponse;
    },
    validateSlotData(projectBundle, slotData) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(
        raw.validateSlotData(projectBundle, JSON.stringify(slotData)),
      ) as ValidationResponse;
    },
    generate(projectBundle, slotData, names) {
      // generate returns a JsValue (object), not a JSON string.
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return raw.generate(
        projectBundle,
        JSON.stringify(slotData),
        names?.projectName ?? undefined,
        names?.outputName ?? undefined,
      ) as GenerateResponse;
    },
    render(projectBundle, slotData, names) {
      // render returns a JsValue (object), not a JSON string.
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return raw.render(
        projectBundle,
        JSON.stringify(slotData),
        names?.projectName ?? undefined,
        names?.outputName ?? undefined,
      ) as RenderResponse;
    },
    planHooks(projectBundle, data, hookRan, names) {
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      return JSON.parse(
        raw.planHooks(
          projectBundle,
          JSON.stringify(data),
          hookRan === undefined ? undefined : JSON.stringify(hookRan),
          names?.projectName ?? undefined,
          names?.outputName ?? undefined,
        ),
      ) as PlanHooksResponse;
    },
  };
}
