// Browser bundler entry over the wasm-bindgen exports.
//
// Importing the .wasm file with `?url` lets Vite/Rollup own the asset URL in
// dev and production instead of relying on a runtime /node_modules path.

import initWasm, {
  check as wasmCheck,
  plan_hooks as wasmPlanHooks,
  render_file as wasmRenderFile,
  render_path as wasmRenderPath,
  validate_slot_data as wasmValidateSlotData,
} from "../../pkg/spackle_wasm.js";
import wasmUrl from "../../pkg/spackle_wasm_bg.wasm?url";
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
  wasmUrl,
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
