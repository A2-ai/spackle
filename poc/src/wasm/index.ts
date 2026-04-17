// WASM-SIDE — pure computation. No I/O.
//
// Singleton loader for the compiled spackle WASM module. `loadSpackleWasm()`
// returns the *typed* client; callers should never import from
// `../../pkg/spackle.js` directly.

import initWasm, {
  check_project,
  evaluate_hooks,
  get_output_name,
  get_project_name,
  parse_config,
  render_string,
  render_templates,
  validate_config,
  validate_slot_data,
} from "../../pkg/spackle.js";
import type {
  HookPlanEntry,
  RenderedTemplate,
  SlotData,
  SpackleConfig,
  TemplateInput,
  ValidationResult,
} from "./types.ts";

/** Typed wrapper over the raw WASM exports. All methods are synchronous
 * (the only async step is `loadSpackleWasm()` itself). */
export interface SpackleWasm {
  parseConfig(toml: string): SpackleConfig;
  validateConfig(toml: string): ValidationResult;
  checkProject(toml: string, templates: TemplateInput[]): ValidationResult;
  validateSlotData(configJson: string, data: SlotData): ValidationResult;
  renderTemplates(
    templates: TemplateInput[],
    data: SlotData,
    configJson: string,
  ): RenderedTemplate[];
  evaluateHooks(configJson: string, data: SlotData): HookPlanEntry[];
  renderString(template: string, data: SlotData): string;
  getOutputName(outDir: string): string;
  getProjectName(configJson: string, projectDir: string): string;
}

let cached: Promise<SpackleWasm> | null = null;

/** Load the WASM module once per process. Subsequent calls return the same
 * client. Safe to await concurrently — the first caller initializes, the
 * rest share the promise. */
export function loadSpackleWasm(): Promise<SpackleWasm> {
  if (!cached) cached = initialize();
  return cached;
}

async function initialize(): Promise<SpackleWasm> {
  await initWasm();
  return {
    parseConfig(toml) {
      return parseOrThrow<SpackleConfig>(parse_config(toml), "parseConfig");
    },
    validateConfig(toml) {
      return JSON.parse(validate_config(toml)) as ValidationResult;
    },
    checkProject(toml, templates) {
      return JSON.parse(
        check_project(toml, JSON.stringify(templates)),
      ) as ValidationResult;
    },
    validateSlotData(configJson, data) {
      return JSON.parse(
        validate_slot_data(configJson, JSON.stringify(data)),
      ) as ValidationResult;
    },
    renderTemplates(templates, data, configJson) {
      return parseOrThrow<RenderedTemplate[]>(
        render_templates(
          JSON.stringify(templates),
          JSON.stringify(data),
          configJson,
        ),
        "renderTemplates",
      );
    },
    evaluateHooks(configJson, data) {
      return parseOrThrow<HookPlanEntry[]>(
        evaluate_hooks(configJson, JSON.stringify(data)),
        "evaluateHooks",
      );
    },
    renderString(template, data) {
      return parseOrThrow<string>(
        render_string(template, JSON.stringify(data)),
        "renderString",
      );
    },
    getOutputName(outDir) {
      return parseOrThrow<string>(get_output_name(outDir), "getOutputName");
    },
    getProjectName(configJson, projectDir) {
      return parseOrThrow<string>(
        get_project_name(configJson, projectDir),
        "getProjectName",
      );
    },
  };
}

/** The WASM layer returns either a valid JSON value or `{"error": "..."}`.
 * `parseOrThrow` funnels the error envelope through a thrown Error so the
 * orchestration layer doesn't have to branch on response shape. */
function parseOrThrow<T>(raw: string, label: string): T {
  const parsed = JSON.parse(raw);
  if (parsed && typeof parsed === "object" && "error" in parsed) {
    throw new Error(`${label}: ${parsed.error}`);
  }
  return parsed as T;
}

export type {
  Hook,
  HookPlanEntry,
  RenderedTemplate,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  TemplateInput,
  ValidationResult,
} from "./types.ts";
