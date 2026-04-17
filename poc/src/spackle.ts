// ORCHESTRATION — composes WASM compute with host I/O.
//
// This file is *the* reference for how spackle's native entry points map
// onto the WASM/host split. Each line is annotated so the boundary is
// obvious when reading. If you're hunting for where spackle spends its
// time, start here.

import { mkdir, stat } from "node:fs/promises";
import { loadSpackleWasm } from "./wasm/index.ts";
import {
  copyNonTemplates,
  readSpackleConfig,
  walkTemplates,
  writeRenderedFiles,
} from "./host/fs.ts";
import { executeHookPlan, type HookOutcome } from "./host/hooks.ts";
import type {
  HookPlanEntry,
  RenderedTemplate,
  SlotData,
  SpackleConfig,
  ValidationResult,
} from "./wasm/types.ts";

export interface CheckResult extends ValidationResult {
  config: SpackleConfig | null;
}

export interface GenerateOptions {
  /** If true, executes hooks via Bun.spawn after rendering. Defaults to
   * false — callers often want to plan+render without side effects. */
  runHooks?: boolean;
  /** If true, write into `outDir` even if it already exists. Defaults to
   * false — matches native `Project::generate` which errors with
   * `AlreadyExists` to protect existing work. */
  overwrite?: boolean;
  /** If true, proceed past template render failures, writing only the
   * successful entries. Defaults to false — matches native
   * `Project::generate` which iterates render results and returns
   * `GenerateError::FileError` on the FIRST `Err` encountered. Set to
   * true for UI flows that want to surface every failure at once via
   * `result.rendered`. */
  allowTemplateErrors?: boolean;
}

/** Thrown by `generate()` when a template fails to render and
 * `allowTemplateErrors` was not set. `message` references only the first
 * failing entry (matching native observable behavior), but the full
 * rendered batch is attached so callers can introspect every failure if
 * they want to without re-running. */
export class TemplateRenderError extends Error {
  readonly original_path: string;
  readonly rendered: RenderedTemplate[];
  constructor(first: RenderedTemplate, all: RenderedTemplate[]) {
    super(`template render failed: ${first.original_path}: ${first.error}`);
    this.name = "TemplateRenderError";
    this.original_path = first.original_path;
    this.rendered = all;
  }
}

export interface GenerateResult {
  /** Every template the WASM layer tried to render (includes per-file
   * errors, same shape as `evaluate_hooks`). */
  rendered: RenderedTemplate[];
  /** How many files were actually written to disk (skips error entries). */
  written: number;
  /** How many non-`.j2` files were copied. */
  copied: number;
  /** The hook plan from WASM. Populated even when `runHooks: false`. */
  plan: HookPlanEntry[];
  /** Per-hook outcomes when `runHooks: true`; empty array otherwise. */
  hookOutcomes: HookOutcome[];
  /** Final slot data including the injected `_project_name` /
   * `_output_name` specials — handy for assertions. */
  slotData: SlotData;
}

/** Orchestration: full check of a project on disk (config + slot defaults
 * + template references). Mirrors `Project::check` native entry. */
export async function check(projectDir: string): Promise<CheckResult> {
  const wasm = await loadSpackleWasm();
  const toml = await readSpackleConfig(projectDir); // HOST: disk read
  const configRes = wasm.validateConfig(toml); // WASM: structure
  if (!configRes.valid) {
    return { valid: false, errors: configRes.errors, config: null };
  }
  const config = wasm.parseConfig(toml); // WASM: parse
  const templates = await walkTemplates(projectDir, config.ignore); // HOST: disk walk
  const projectRes = wasm.checkProject(toml, templates); // WASM: template refs
  return {
    valid: projectRes.valid,
    errors: projectRes.errors,
    config,
  };
}

/** Orchestration: run the full `spackle generate` workflow — validate, then
 * render templates + copy non-templates + (optionally) run hooks.
 *
 * The body reads top-to-bottom as an annotated map of WASM vs HOST work. */
export async function generate(
  projectDir: string,
  slotData: SlotData,
  outDir: string,
  opts: GenerateOptions = {},
): Promise<GenerateResult> {
  const wasm = await loadSpackleWasm();

  // 1. Load + parse config. We hold onto BOTH the raw toml (for error msgs
  //    if needed) and the JSON-serialized config (what the WASM layer wants
  //    for downstream calls — never pass `toml` as `configJson`).
  const toml = await readSpackleConfig(projectDir); // HOST: disk read
  const config = wasm.parseConfig(toml); // WASM: parse
  const configJson = JSON.stringify(config);

  // 2. Validate slot data up front (matches native behavior of failing fast).
  const slotValidation = wasm.validateSlotData(configJson, slotData); // WASM
  if (!slotValidation.valid) {
    throw new Error(
      `slot data invalid: ${(slotValidation.errors ?? []).join("; ")}`,
    );
  }

  // 3. Inject the specials that the native layer injects.
  const fullData: SlotData = {
    _project_name: wasm.getProjectName(configJson, projectDir), // WASM
    _output_name: wasm.getOutputName(outDir), // WASM
    ...slotData,
  };

  // 4. Protect the output directory unless overwrite was requested.
  //    Mirrors native `Project::generate` which returns
  //    `GenerateError::AlreadyExists` before doing any work.
  if (!opts.overwrite && (await pathExists(outDir))) { // HOST: disk stat
    throw new Error(
      `output directory already exists: ${outDir} (pass { overwrite: true } to proceed)`,
    );
  }

  // 5. Walk + render templates in memory. Nothing is written yet — we
  //    check for render errors before touching disk.
  const templates = await walkTemplates(projectDir, config.ignore); // HOST: walk disk
  const rendered = wasm.renderTemplates(templates, fullData, configJson); // WASM: render

  // 6. Fail fast on the FIRST template error unless the caller opted
  //    into "collect them all and return" semantics. Mirrors native
  //    `Project::generate`, which iterates render results and returns
  //    `GenerateError::FileError` on the first `Err` — subsequent errors
  //    never surface. The full `rendered` array is attached to the
  //    thrown error for inspection without re-running.
  if (!opts.allowTemplateErrors) {
    const firstFailure = rendered.find((r) => r.error);
    if (firstFailure) {
      throw new TemplateRenderError(firstFailure, rendered);
    }
  }

  // 7. Materialize outDir. Order below matches native `Project::generate`:
  //    copy non-templates FIRST, then write rendered templates. Templates
  //    win on path collisions — callers rely on this to override copied
  //    files with templated variants.
  await mkdir(outDir, { recursive: true }); // HOST: disk mkdir
  const copied = await copyNonTemplates( // HOST: disk copy
    projectDir,
    outDir,
    config.ignore,
    fullData,
    wasm,
  );
  const written = await writeRenderedFiles(outDir, rendered); // HOST: disk write

  // 8. Plan hooks (always) and optionally execute them.
  const plan = wasm.evaluateHooks(configJson, fullData); // WASM: plan
  const hookOutcomes: HookOutcome[] = [];
  if (opts.runHooks) {
    for await (const outcome of executeHookPlan(plan, outDir)) { // HOST: spawn
      hookOutcomes.push(outcome);
    }
  }

  return { rendered, written, copied, plan, hookOutcomes, slotData: fullData };
}

async function pathExists(path: string): Promise<boolean> {
  try {
    await stat(path);
    return true;
  } catch {
    return false;
  }
}

export type { SpackleWasm } from "./wasm/index.ts";
export { loadSpackleWasm } from "./wasm/index.ts";
export type { HookOutcome, HookResult } from "./host/hooks.ts";
