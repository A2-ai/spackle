// WASM-SIDE - server/runtime-default entry over the wasm-bindgen exports.
//
// The wasm-bindgen `--target web` output at `../../pkg/spackle_wasm.js`
// exposes pure-function exports plus a default `init`. This default entry
// preserves the existing package-relative WASM URL behavior for Bun/Node-like
// runtimes. Browser bundlers should resolve the package export's `browser`
// condition to `./browser.ts`, which imports the `.wasm` asset directly.
// Standalone bundle hosts (single-file output via `bun build`, `.deb`
// payloads, etc.) where `import.meta.url` no longer resolves to the
// package root should call `configureSpackleWasm({ moduleOrPath })` with
// their own bytes / URL / module before the first `loadSpackleWasm()`.

import initWasm, {
  check as wasmCheck,
  plan_hooks as wasmPlanHooks,
  render_file as wasmRenderFile,
  render_path as wasmRenderPath,
  validate_slot_data as wasmValidateSlotData,
} from "../../pkg/spackle_wasm.js";
import { createSpackleWasmLoader } from "./runtime.ts";

export const { loadSpackleWasm, configureSpackleWasm } = createSpackleWasmLoader(
  {
    initWasm,
    check: wasmCheck,
    validateSlotData: wasmValidateSlotData,
    renderFile: wasmRenderFile,
    renderPath: wasmRenderPath,
    planHooks: wasmPlanHooks,
  },
  new URL("../../pkg/spackle_wasm_bg.wasm", import.meta.url),
);

export type {
  ConfigureSpackleWasmOptions,
  SpackleWasm,
  SpackleWasmModuleSource,
} from "./runtime.ts";
export type {
  Bundle,
  BundleEntry,
  CheckResponse,
  Diagnostic,
  DiagnosticSeverity,
  DiagnosticSource,
  DiagnosticSpan,
  GenerateResponse,
  Hook,
  HookPlanEntry,
  PlanHooksResponse,
  RenderFileResponse,
  RenderPathResponse,
  RenderResponse,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  ValidationResponse,
} from "./types.ts";
