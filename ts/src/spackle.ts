// Orchestration entry — bundle-in / bundle-out.
//
// Under this design Rust is a pure compute step: the host hands it a
// project bundle (files as `{path, bytes}`), Rust runs generation
// entirely against an in-memory fs, and returns a rendered output
// bundle. The host then writes that bundle to disk (or any other
// destination) however it wants.
//
// Convenience APIs:
//   - `check(projectDir, fs)` / `checkBundle(bundle)`
//   - `validateSlotData(...)` / `validateSlotDataBundle(...)`
//   - `generate(projectDir, outDir, slotData, fs, opts)` — reads from
//     disk via DiskFs, calls wasm, writes output bundle back to disk.
//   - `generateBundle(projectBundle, slotData, opts)` — pure
//     bundle-to-bundle; for memory-fs / preview flows.
//
// Virtual-fs anchors are pinned inside the wasm crate; callers never
// supply them. `_project_name` / `_output_name` are still
// caller-controllable via the `projectName` / `outputName` options on
// the relevant entry points.

import { basename } from "node:path";

import { DiskFs } from "./host/disk-fs.ts";
import { type HookEvent, runHookPlanStream, type SpackleHooks } from "./host/hooks.ts";
import { loadSpackleWasm, type NameOverrides } from "./wasm/index.ts";
import type {
  Bundle,
  CheckResponse,
  GenerateResponse,
  PlanHooksResponse,
  RenderResponse,
  SlotData,
  ValidationResponse,
} from "./wasm/types.ts";

export interface CheckOptions {
  // Intentionally empty. Kept as a stable extension point and so the
  // function signatures stay `(..., opts?)` if we need to add knobs
  // later without a breaking change.
}

export interface GenerateOptions {
  /** Override `_project_name`. Defaults to `config.name` from
   * `spackle.toml` (and ultimately the basename of the fixed virtual
   * project dir for projects with no `name`). */
  projectName?: string;
  /** Override `_output_name`. For disk-backed `generate` / `render`
   * the default is `basename(outDir)`; for `generateBundle` /
   * `renderBundle` the default falls back to the basename of the
   * fixed virtual out dir constant. */
  outputName?: string;
}

export interface RunHooksOptions {
  /** Injected executor. Defaults to `defaultHooks()` (auto-selects
   * `BunHooks` / `NodeHooks`; throws in browser-like hosts). */
  hooks?: SpackleHooks;
  /** Working dir for spawned processes. Defaults to `outDir`. */
  cwd?: string;
  env?: Record<string, string>;
  /** Override `_project_name` for hook command templating. */
  projectName?: string;
  /** Override `_output_name` for hook command templating. Defaults
   * to `basename(outDir)` so templated `{{ _output_name }}` matches
   * the real write target. */
  outputName?: string;
}

interface DiskNameOpts {
  projectName?: string;
  outputName?: string;
}

/** For disk-backed entry points: preserve the historical default of
 * `_output_name = basename(realOutDir)` unless the caller overrides. */
function diskNames(outDir: string, opts: DiskNameOpts): NameOverrides {
  return {
    projectName: opts.projectName,
    outputName: opts.outputName ?? basename(outDir),
  };
}

/** For bundle-only entry points: no real outDir, so we just pass
 * whatever the caller supplied. Unset → wasm falls back to basename
 * of the fixed virtual out dir constant. */
function bundleNames(opts: DiskNameOpts): NameOverrides | undefined {
  if (opts.projectName === undefined && opts.outputName === undefined) return undefined;
  const out: NameOverrides = {};
  if (opts.projectName !== undefined) out.projectName = opts.projectName;
  if (opts.outputName !== undefined) out.outputName = opts.outputName;
  return out;
}

/**
 * Validate a project at `projectDir` on disk. Reads the project into a
 * bundle via DiskFs, then calls wasm `check`. Returns the parsed config
 * on success so UIs can render slot forms without re-parsing TOML.
 */
export async function check(
  projectDir: string,
  fs: DiskFs,
  _opts: CheckOptions = {},
): Promise<CheckResponse> {
  const bundle = fs.readProject(projectDir);
  return checkBundle(bundle);
}

/** Same as `check` but takes a pre-built bundle (memory-fs / preview flow). */
export async function checkBundle(bundle: Bundle): Promise<CheckResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.check(bundle);
}

/**
 * Validate slot data against a project on disk.
 */
export async function validateSlotData(
  projectDir: string,
  slotData: SlotData,
  fs: DiskFs,
  _opts: CheckOptions = {},
): Promise<ValidationResponse> {
  const bundle = fs.readProject(projectDir);
  return validateSlotDataBundle(bundle, slotData);
}

/** Bundle variant of `validateSlotData`. */
export async function validateSlotDataBundle(
  bundle: Bundle,
  slotData: SlotData,
): Promise<ValidationResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.validateSlotData(bundle, slotData);
}

/**
 * Generate a filled project from disk → bundle → (wasm) → bundle → disk.
 * DiskFs handles both the read (projectDir → bundle) and write (output
 * bundle → outDir) legs.
 *
 * Hooks are a separate step — iterate `runHooksStream()` after
 * `generate()` if the project defines any. Mirrors the native CLI's
 * two-call shape (`project.generate(...)` then
 * `project.run_hooks_stream(...)`).
 */
export async function generate(
  projectDir: string,
  outDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: GenerateOptions = {},
): Promise<GenerateResponse> {
  const bundle = fs.readProject(projectDir);
  const wasm = await loadSpackleWasm();
  const result = wasm.generate(bundle, slotData, diskNames(outDir, opts));
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
  const wasm = await loadSpackleWasm();
  return wasm.generate(projectBundle, slotData, bundleNames(opts));
}

/**
 * Render a project with diagnostics-first semantics. Unlike `generate`,
 * `render` **never throws / never returns `ok: false`** — it always
 * returns a `RenderResponse` carrying a (possibly partial) bundle plus
 * the full `diagnostics` array spanning config / slot / hook / copy /
 * render stages. Empty `diagnostics` ⇒ clean render.
 *
 * Use `render` for live UI previews where the user wants to see every
 * problem at once; use `generate` for write-to-disk workflows that
 * should abort on the first error.
 */
export async function render(
  projectDir: string,
  outDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: GenerateOptions = {},
): Promise<RenderResponse> {
  const bundle = fs.readProject(projectDir);
  const wasm = await loadSpackleWasm();
  const result = wasm.render(bundle, slotData, diskNames(outDir, opts));
  // No `result.ok` discriminant — write whatever rendered, regardless of
  // diagnostics. Caller decides what to do with the diagnostics array.
  if (result.files.length > 0 || result.dirs.length > 0) {
    fs.writeOutput(outDir, { files: result.files, dirs: result.dirs });
  }
  return result;
}

/** Bundle-to-bundle variant of `render` — for MemoryFs / preview flows. */
export async function renderBundle(
  projectBundle: Bundle,
  slotData: SlotData,
  opts: GenerateOptions = {},
): Promise<RenderResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.render(projectBundle, slotData, bundleNames(opts));
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
 * pre-inject. `outputName` defaults to `basename(outDir)` for
 * disk-backed callers.
 */
export async function planHooks(
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  fs: DiskFs,
  opts: {
    hookRan?: Record<string, boolean>;
    projectName?: string;
    outputName?: string;
  } = {},
): Promise<PlanHooksResponse> {
  const bundle = fs.readProject(projectDir);
  return planHooksBundle(bundle, data, opts.hookRan, diskNames(outDir, opts));
}

/** Bundle variant of `planHooks`. */
export async function planHooksBundle(
  projectBundle: Bundle,
  data: Record<string, string>,
  hookRan?: Record<string, boolean>,
  names?: NameOverrides,
): Promise<PlanHooksResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.planHooks(projectBundle, data, hookRan, names);
}

/**
 * Run the project's hooks, yielding `HookEvent`s as they occur. Reads
 * the project into a bundle, plans via wasm, and executes host-side via
 * `opts.hooks ?? defaultHooks()`.
 *
 * Mirrors native `Project::run_hooks_stream` at `src/lib.rs:246`:
 * `data` is the full data map (slots + hook toggles). `_project_name` /
 * `_output_name` are injected wasm-side. `outputName` defaults to
 * `basename(outDir)`.
 *
 * Event protocol: see `HookEvent`. Consumers `for await` the generator
 * and update their UI on each `hook_start` / `hook_end` transition;
 * this is the hook for SSE bridging — bridge by yielding each event as
 * an SSE frame. Terminal `template_errors` / `plan_error` events end
 * the iterator (no throws for expected failure modes).
 *
 * Failure semantics match native: non-zero exit yields a `hook_end`
 * with `kind: "failed"` but the run continues; chained
 * `hook_ran_<key>` conditionals re-evaluate against actual outcomes via
 * an in-loop re-plan (visible as a `replan` event). Template rendering
 * failures are a hard abort before any execution (terminal
 * `template_errors` event).
 */
export function runHooksStream(
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  fs: DiskFs,
  opts: RunHooksOptions = {},
): AsyncGenerator<HookEvent> {
  const bundle = fs.readProject(projectDir);
  const names = diskNames(outDir, opts);
  // Wrap the planner call in an async bridge so the generator can start
  // yielding events without awaiting wasm load outside the iterator.
  async function* inner(): AsyncGenerator<HookEvent> {
    const wasm = await loadSpackleWasm();
    yield* runHookPlanStream((b, d, hookRan) => wasm.planHooks(b, d, hookRan, names), {
      bundle,
      outDir,
      data,
      hooks: opts.hooks,
      cwd: opts.cwd ?? outDir,
      env: opts.env,
    });
  }
  return inner();
}

export type {
  ConfigureSpackleWasmOptions,
  NameOverrides,
  SpackleWasm,
  SpackleWasmModuleSource,
} from "./wasm/index.ts";
export { configureSpackleWasm, loadSpackleWasm } from "./wasm/index.ts";
export {
  BUNDLE_PROJECT_ROOT,
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
  runHookPlanStream,
  type HookEvent,
  type HookExecuteResult,
  type HookRunResult,
  type HooksEnv,
  type RunHookPlanOptions,
  type SpackleHooks,
  type TemplateErrorDetail,
} from "./host/hooks.ts";
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
  RenderResponse,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  ValidationResponse,
} from "./wasm/types.ts";
