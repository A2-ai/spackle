// Orchestration entry — bundle-in / bundle-out.
//
// Under this design Rust is a pure compute step: the host hands it a
// project bundle (files as `{path, bytes}`), Rust runs generation
// entirely against an in-memory fs, and returns a rendered output
// bundle. The host then writes that bundle to disk (or any other
// destination) however it wants.
//
// Convenience APIs:
//   - `check(projectDir, fs)` / `checkBundle(bundle, virtualDir)`
//   - `validateSlotData(...)` / `validateSlotDataBundle(...)`
//   - `generate(projectDir, outDir, slotData, fs, opts)` — reads from
//     disk via DiskFs, calls wasm, writes output bundle back to disk.
//   - `generateBundle(projectBundle, projectDir, outDir, slotData, opts)`
//     — pure bundle-to-bundle; for memory-fs / preview flows.

import { DiskFs } from "./host/disk-fs.ts";
import { runHookPlan, type RunHooksResponse, type SpackleHooks } from "./host/hooks.ts";
import { loadSpackleWasm } from "./wasm/index.ts";
import type {
  Bundle,
  CheckResponse,
  GenerateResponse,
  PlanHooksResponse,
  SlotData,
  ValidationResponse,
} from "./wasm/types.ts";

const DEFAULT_VIRTUAL_PROJECT = "/project";
const DEFAULT_VIRTUAL_OUTPUT = "/output";

export interface CheckOptions {
  /** Virtual path the project bundle is rooted at. Defaults to
   * `/project`; rarely needs overriding. */
  virtualProjectDir?: string;
}

export interface GenerateOptions extends CheckOptions {
  /** Virtual path used for the generate output root. Defaults to
   * `/output`. */
  virtualOutDir?: string;
}

export interface RunHooksOptions extends CheckOptions {
  /** Injected executor. Defaults to `defaultHooks()` (auto-selects
   * `BunHooks` / `NodeHooks`; throws in browser-like hosts). */
  hooks?: SpackleHooks;
  /** Working dir for spawned processes. Defaults to `outDir`. */
  cwd?: string;
  env?: Record<string, string>;
}

/**
 * Validate a project at `projectDir` on disk. Reads the project into a
 * bundle via DiskFs, then calls wasm `check`. Returns the parsed config
 * on success so UIs can render slot forms without re-parsing TOML.
 */
export async function check(
  projectDir: string,
  fs: DiskFs,
  opts: CheckOptions = {},
): Promise<CheckResponse> {
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualDir });
  return checkBundle(bundle, virtualDir);
}

/** Same as `check` but takes a pre-built bundle (memory-fs / preview flow). */
export async function checkBundle(
  bundle: Bundle,
  virtualProjectDir: string = DEFAULT_VIRTUAL_PROJECT,
): Promise<CheckResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.check(bundle, virtualProjectDir);
}

/**
 * Validate slot data against a project on disk.
 */
export async function validateSlotData(
  projectDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: CheckOptions = {},
): Promise<ValidationResponse> {
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualDir });
  return validateSlotDataBundle(bundle, slotData, virtualDir);
}

/** Bundle variant of `validateSlotData`. */
export async function validateSlotDataBundle(
  bundle: Bundle,
  slotData: SlotData,
  virtualProjectDir: string = DEFAULT_VIRTUAL_PROJECT,
): Promise<ValidationResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.validateSlotData(bundle, virtualProjectDir, slotData);
}

/**
 * Generate a filled project from disk → bundle → (wasm) → bundle → disk.
 * DiskFs handles both the read (projectDir → bundle) and write (output
 * bundle → outDir) legs.
 *
 * Hooks are a separate step — call `runHooks()` after `generate()` if
 * the project defines any. Mirrors the native CLI's two-call shape
 * (`project.generate(...)` then `project.run_hooks_stream(...)`).
 */
export async function generate(
  projectDir: string,
  outDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: GenerateOptions = {},
): Promise<GenerateResponse> {
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const virtualOut = opts.virtualOutDir ?? DEFAULT_VIRTUAL_OUTPUT;
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualProject });
  const result = await generateBundle(bundle, slotData, {
    virtualProjectDir: virtualProject,
    virtualOutDir: virtualOut,
  });
  if (result.ok) {
    fs.writeOutput(outDir, { files: result.files, dirs: result.dirs });
  }
  return result;
}

/** Bundle-to-bundle variant of `generate` — for MemoryFs / preview
 * flows that never touch disk. */
export async function generateBundle(
  projectBundle: Bundle,
  slotData: SlotData,
  opts: GenerateOptions = {},
): Promise<GenerateResponse> {
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const virtualOut = opts.virtualOutDir ?? DEFAULT_VIRTUAL_OUTPUT;
  const wasm = await loadSpackleWasm();
  return wasm.generate(projectBundle, virtualProject, virtualOut, slotData);
}

/**
 * Inspect the hook plan without executing. Reads the project into a
 * bundle, calls the wasm planner, and returns the resolved plan
 * (templated commands, should-run flags, skip reasons, template
 * errors). Useful for UIs that want to preview what would run.
 *
 * `data` is the full data map matching native `Project::run_hooks_stream`:
 * slot values plus hook toggles keyed by the hook's own `key`
 * (e.g. `{ "format_code": "false" }` to disable that hook).
 * `_project_name` and `_output_name` are injected wasm-side — don't
 * pre-inject.
 */
export async function planHooks(
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  fs: DiskFs,
  opts: CheckOptions & { hookRan?: Record<string, boolean> } = {},
): Promise<PlanHooksResponse> {
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualDir });
  return planHooksBundle(bundle, virtualDir, outDir, data, opts.hookRan);
}

/** Bundle variant of `planHooks`. */
export async function planHooksBundle(
  projectBundle: Bundle,
  virtualProjectDir: string,
  outDir: string,
  data: Record<string, string>,
  hookRan?: Record<string, boolean>,
): Promise<PlanHooksResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.planHooks(projectBundle, virtualProjectDir, outDir, data, hookRan);
}

/**
 * Run the project's hooks. Reads the project into a bundle, plans via
 * wasm, and executes host-side via `opts.hooks ?? defaultHooks()`.
 *
 * Mirrors native `Project::run_hooks_stream` at `src/lib.rs:246`:
 * `data` is the full data map (slots + hook toggles). `_project_name` /
 * `_output_name` are injected wasm-side.
 *
 * Failure semantics match native: non-zero exit yields a `failed`
 * result but the run continues; chained `hook_ran_<key>` conditionals
 * re-evaluate against actual outcomes via an in-loop re-plan. Template
 * rendering failures are a hard abort before any execution (returns
 * `{ ok: false, error, templateErrors }`).
 */
export async function runHooks(
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  fs: DiskFs,
  opts: RunHooksOptions = {},
): Promise<RunHooksResponse> {
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualDir });
  const wasm = await loadSpackleWasm();
  return runHookPlan((b, pdir, odir, d, hookRan) => wasm.planHooks(b, pdir, odir, d, hookRan), {
    bundle,
    projectDir: virtualDir,
    outDir,
    data,
    hooks: opts.hooks,
    cwd: opts.cwd ?? outDir,
    env: opts.env,
  });
}

export type { SpackleWasm } from "./wasm/index.ts";
export { loadSpackleWasm } from "./wasm/index.ts";
export {
  DiskFs,
  type DiskFsOptions,
  type ReadProjectOptions,
  type WriteOutputInput,
} from "./host/disk-fs.ts";
export { MemoryFs, type MemoryFsSeed } from "./host/memory-fs.ts";
export {
  BunHooks,
  NodeHooks,
  defaultHooks,
  detectHooksEnv,
  formatArgv,
  parseShellLine,
  runHookPlan,
  type HookExecuteResult,
  type HookRunResult,
  type HooksEnv,
  type RunHookPlanOptions,
  type RunHooksResponse,
  type SpackleHooks,
  type TemplateErrorDetail,
} from "./host/hooks.ts";
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
} from "./wasm/types.ts";
