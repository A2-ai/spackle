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
import { type HookEvent, runHookPlanStream, type SpackleHooks } from "./host/hooks.ts";
import { loadSpackleWasm } from "./wasm/index.ts";
import type {
  Bundle,
  CheckResponse,
  GenerateDiskResponse,
  GenerateResponse,
  GenerateStreamEntry,
  GenerateStreamEvent,
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
 * Generate a filled project, streaming each rendered file/dir to disk
 * as Rust produces it.
 *
 * Reads the project bundle from `projectDir` (eagerly — the input read
 * is the documented remaining ceiling), then drives the wasm streaming
 * generate with a callback that synchronously writes each entry under
 * `outDir`. Peak memory is bounded by one entry, not by the rendered
 * output.
 *
 * Returns counts (`files`, `dirs`) on success — not a materialized
 * bundle. Callers that want the rendered output in memory should call
 * `generateBundle` instead.
 *
 * Failure semantics match native `Project::generate`: validation
 * failures (bad config, slot data type mismatch, malformed bundle)
 * fail BEFORE `outDir` is created on disk. Mid-stream failures (a
 * template tera error after the first write, or a host-callback
 * throw) leave whatever was already written — there's no rollback,
 * matching native CLI behavior. Pick a fresh `outDir` per run.
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
): Promise<GenerateDiskResponse> {
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const virtualOut = opts.virtualOutDir ?? DEFAULT_VIRTUAL_OUTPUT;
  const wasm = await loadSpackleWasm();
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualProject });

  // Containment + AlreadyExists check WITHOUT creating outDir — defer
  // creation until the first streamed entry. That way wasm-side
  // validation failures (bad slot data, bad config, bad bundle) leave
  // no empty outDir on disk, matching native `Project::generate` which
  // only creates the destination as part of `copy::copy`.
  const absOut = fs.assertOutDirAvailable(outDir);

  let files = 0;
  let dirs = 0;
  const result = wasm.generate(bundle, virtualProject, virtualOut, slotData, (event) => {
    fs.writeEntry(absOut, event);
    if (event.kind === "file") files++;
    else dirs++;
  });
  if (!result.ok) {
    return { ok: false, error: result.error };
  }

  // Empty-project parity: native `copy::copy` unconditionally calls
  // `create_dir_all(dest)` so a project with zero files still produces
  // an empty outDir. Streaming generate skips the out_root event, so
  // ensure outDir on success — mkdir recursive is idempotent so this
  // is a no-op when writeEntry already created it.
  fs.ensureOutDir(absOut);

  return { ok: true, files, dirs };
}

/**
 * Bundle-to-bundle variant of `generate` — buffers the streamed entries
 * into a `Bundle` and returns the legacy `{ ok, files, dirs }` shape.
 * For preview / in-process consumers that want the rendered tree in
 * memory; use `generateStream` for async-iter ergonomics or `generate`
 * for true-streaming disk writes.
 *
 * Dedupes and sorts to preserve the **final-tree** semantics the old
 * buffered path had (which drained an in-memory map): when both
 * `copy::copy` and `template::fill` write to the same output path
 * (e.g., a project with `foo` and `foo.j2` both rendering to `foo`),
 * the second write wins — matching what lands on disk under streaming
 * `generate(...)` because the second `writeFileSync` overwrites the
 * first. Output is sorted by path so consumers can rely on stable
 * order regardless of HashMap iteration in the underlying walk.
 *
 * NOTE: this internally buffers — peak memory is the same as the
 * pre-streaming API. Streaming benefits the disk path; in-memory
 * consumers always pay full output size.
 */
export async function generateBundle(
  projectBundle: Bundle,
  slotData: SlotData,
  opts: GenerateOptions = {},
): Promise<GenerateResponse> {
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const virtualOut = opts.virtualOutDir ?? DEFAULT_VIRTUAL_OUTPUT;
  const wasm = await loadSpackleWasm();

  // Dedupe via Map<path, BundleEntry>: later writes replace earlier
  // ones, mirroring disk-write last-wins semantics. Set<string> for
  // dirs is just dedup; create_dir_all events fire repeatedly for the
  // same ancestor as different files traverse it.
  const fileMap = new Map<string, GenerateStreamEntry & { kind: "file" }>();
  const dirSet = new Set<string>();
  const result = wasm.generate(projectBundle, virtualProject, virtualOut, slotData, (event) => {
    if (event.kind === "file") {
      fileMap.set(event.path, event);
    } else {
      dirSet.add(event.path);
    }
  });
  if (!result.ok) {
    return { ok: false, error: result.error };
  }

  const files: Bundle = Array.from(fileMap.values(), (e) => ({
    path: e.path,
    bytes: e.bytes,
  })).toSorted((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const dirs = Array.from(dirSet).toSorted();
  return { ok: true, files, dirs };
}

/**
 * Streaming variant — yields each rendered entry as an async generator
 * event, with terminal `done` / `error` events. Mirrors the
 * `runHooksStream` shape.
 *
 * MEMORY NOTE: this does NOT reduce peak memory. The wasm call is
 * synchronous, so the host callback runs to completion before this
 * generator can `yield` — entries pile up in an internal queue, then
 * stream out. The value of this API is ergonomics (preview, progress
 * UI), not memory. For true-streaming disk writes that bound peak
 * memory at one entry, call `generate(projectDir, outDir, ...)`.
 */
export async function* generateStream(
  projectBundle: Bundle,
  slotData: SlotData,
  opts: GenerateOptions = {},
): AsyncGenerator<GenerateStreamEvent> {
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const virtualOut = opts.virtualOutDir ?? DEFAULT_VIRTUAL_OUTPUT;
  const wasm = await loadSpackleWasm();

  const queue: GenerateStreamEntry[] = [];
  const result = wasm.generate(projectBundle, virtualProject, virtualOut, slotData, (event) =>
    queue.push(event),
  );

  for (const event of queue) {
    yield event;
  }
  if (result.ok) {
    yield { kind: "done" };
  } else {
    yield { kind: "error", error: result.error };
  }
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
 * Run the project's hooks, yielding `HookEvent`s as they occur. Reads
 * the project into a bundle, plans via wasm, and executes host-side via
 * `opts.hooks ?? defaultHooks()`.
 *
 * Mirrors native `Project::run_hooks_stream` at `src/lib.rs:246`:
 * `data` is the full data map (slots + hook toggles). `_project_name` /
 * `_output_name` are injected wasm-side.
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
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = fs.readProject(projectDir, { virtualRoot: virtualDir });
  // Wrap the planner call in an async bridge so the generator can start
  // yielding events without awaiting wasm load outside the iterator.
  async function* inner(): AsyncGenerator<HookEvent> {
    const wasm = await loadSpackleWasm();
    yield* runHookPlanStream(
      (b, pdir, odir, d, hookRan) => wasm.planHooks(b, pdir, odir, d, hookRan),
      {
        bundle,
        projectDir: virtualDir,
        outDir,
        data,
        hooks: opts.hooks,
        cwd: opts.cwd ?? outDir,
        env: opts.env,
      },
    );
  }
  return inner();
}

export type {
  ConfigureSpackleWasmOptions,
  SpackleWasm,
  SpackleWasmModuleSource,
} from "./wasm/index.ts";
export { configureSpackleWasm, loadSpackleWasm } from "./wasm/index.ts";
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
  GenerateDiskResponse,
  GenerateResponse,
  GenerateStreamDirEvent,
  GenerateStreamEntry,
  GenerateStreamEvent,
  GenerateStreamFileEvent,
  Hook,
  HookPlanEntry,
  PlanHooksResponse,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  ValidationResponse,
} from "./wasm/types.ts";
