// Browser bundler entry over the wasm-bindgen exports.
//
// Importing the .wasm file with `?url` lets Vite/Rollup own the asset URL in
// dev and production instead of relying on a runtime /node_modules path.

import initWasm, {
  check as wasmCheck,
  generate as wasmGenerate,
  plan_hooks as wasmPlanHooks,
  validate_slot_data as wasmValidateSlotData,
} from "../../pkg/spackle_wasm.js";
import wasmUrl from "../../pkg/spackle_wasm_bg.wasm?url";
import { createSpackleWasmLoader } from "./runtime.ts";

export const { loadSpackleWasm, configureSpackleWasm } = createSpackleWasmLoader(
  {
    initWasm,
    check: wasmCheck,
    validateSlotData: wasmValidateSlotData,
    generate: wasmGenerate,
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
