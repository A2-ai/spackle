// TS-side orchestrator. Walks the project on disk, calls the wasm
// per-file primitives (`check`, `renderFile`, `renderPath`,
// `validateSlotData`, `planHooks`) as it goes, and writes output via
// `DiskFs`. Static asset bytes stream disk-direct through `pipeline()`
// and never cross the wasm boundary.

import { existsSync, readFileSync, readdirSync } from "node:fs";
import { basename as pathBasename, join as pathJoin } from "node:path";

import { DiskFs } from "./host/disk-fs.ts";
import { type HookEvent, runHookPlanStream, type SpackleHooks } from "./host/hooks.ts";
import { loadSpackleWasm, type SpackleWasm } from "./wasm/index.ts";
import type {
  Bundle,
  CheckResponse,
  Diagnostic,
  GenerateResponse,
  HookPlanEntry,
  PlanHooksResponse,
  RenderResponse,
  SlotData,
  SpackleConfig,
  ValidationResponse,
} from "./wasm/types.ts";

const DEFAULT_VIRTUAL_PROJECT = "/project";

const CONFIG_FILE = "spackle.toml";
const TEMPLATE_EXTS = [".j2", ".tera"] as const;

function hasTemplateExt(name: string): boolean {
  return TEMPLATE_EXTS.some((ext) => name.endsWith(ext));
}

function stripTemplateExt(name: string): string {
  for (const ext of TEMPLATE_EXTS) {
    if (name.endsWith(ext)) return name.slice(0, -ext.length);
  }
  return name;
}

export interface CheckOptions {
  /** Virtual path the project bundle is rooted at. Defaults to
   * `/project`; rarely needs overriding. */
  virtualProjectDir?: string;
}

export interface GenerateOptions extends CheckOptions {
  /** Ignored by disk flows — `_output_name` is derived from `outDir`'s
   * basename. Reserved for a future bundle-input composition. */
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

// --- walk ---

interface DiskWalkEntry {
  /** Relative to `projectDir`, forward-slash-separated. */
  relPath: string;
  /** Absolute path on disk. */
  absPath: string;
  kind: "file" | "dir";
}

/**
 * Depth-first walk under `projectDir`. Yields dirs before their
 * contents so the orchestrator can mkdir parents before writing
 * children. Symlinks are skipped (not followed, not emitted).
 *
 * Entries whose basename is `spackle.toml` are suppressed from the
 * yield stream, but a directory with that name is still recursed
 * into. Native `template::fill` walks the full project regardless of
 * basename, so a template under `spackle.toml/` must still surface
 * for rendering; only the entry itself (and non-template descendants
 * — handled by the orchestrator) drops out.
 *
 * Ignore is NOT applied here — native applies it only in the copy
 * stage, so a template in an ignored subtree still renders. Callers
 * decide per-entry whether to honor `ignore`.
 */
function* walkDisk(projectDir: string): Generator<DiskWalkEntry> {
  function* visit(absDir: string, relPrefix: string): Generator<DiskWalkEntry> {
    for (const entry of readdirSync(absDir, { withFileTypes: true })) {
      if (entry.isSymbolicLink()) continue;
      const name = entry.name;

      const abs = pathJoin(absDir, name);
      const rel = relPrefix === "" ? name : `${relPrefix}/${name}`;

      if (name === CONFIG_FILE) {
        if (entry.isDirectory()) yield* visit(abs, rel);
        continue;
      }

      if (entry.isDirectory()) {
        yield { relPath: rel, absPath: abs, kind: "dir" };
        yield* visit(abs, rel);
      } else if (entry.isFile()) {
        yield { relPath: rel, absPath: abs, kind: "file" };
      }
    }
  }
  yield* visit(projectDir, "");
}

/** True when any segment of `relPath` matches an entry in `ignore`. */
function isIgnoredByBasename(relPath: string, ignore: readonly string[]): boolean {
  if (ignore.length === 0) return false;
  for (const segment of relPath.split("/")) {
    if (ignore.includes(segment)) return true;
  }
  return false;
}

/**
 * True when any ancestor segment of `relPath` (i.e. any segment
 * except the final basename) is `spackle.toml`. Mirrors native
 * `copy::copy_collect`'s skipped_ancestors check: a path under a
 * `spackle.toml/` directory is skipped from copy, but templates
 * inside still render.
 */
function hasConfigFileAncestor(relPath: string): boolean {
  const segments = relPath.split("/");
  for (let i = 0; i < segments.length - 1; i++) {
    if (segments[i] === CONFIG_FILE) return true;
  }
  return false;
}

/**
 * Build a check-input bundle: `spackle.toml` + every template body +
 * empty-bytes placeholders for every other file. `copy::validate_paths`
 * inspects path strings but never reads bytes, so the empty placeholders
 * give it the structure it needs without pulling GB-scale static asset
 * bytes across the wasm boundary.
 */
function buildCheckBundle(projectDir: string, virtualRoot: string): Bundle {
  const out: Bundle = [];
  const tomlAbs = pathJoin(projectDir, CONFIG_FILE);
  if (existsSync(tomlAbs)) {
    out.push({
      path: `${virtualRoot}/${CONFIG_FILE}`,
      bytes: new Uint8Array(readFileSync(tomlAbs)),
    });
  }
  for (const entry of walkDisk(projectDir)) {
    if (entry.kind !== "file") continue;
    const virtPath = `${virtualRoot}/${entry.relPath}`;
    if (hasTemplateExt(entry.relPath)) {
      out.push({ path: virtPath, bytes: new Uint8Array(readFileSync(entry.absPath)) });
    } else {
      // Path-only — `copy::validate_paths` reads paths, never bytes.
      out.push({ path: virtPath, bytes: new Uint8Array() });
    }
  }
  return out;
}

/**
 * `spackle.toml`-only bundle for `validateSlotData` and `planHooks`,
 * neither of which walks the tree. Empty bundle if the file is
 * missing — the wasm call surfaces that as a config diagnostic.
 */
function buildConfigOnlyBundle(projectDir: string, virtualRoot: string): Bundle {
  const tomlAbs = pathJoin(projectDir, CONFIG_FILE);
  if (!existsSync(tomlAbs)) return [];
  return [
    {
      path: `${virtualRoot}/${CONFIG_FILE}`,
      bytes: new Uint8Array(readFileSync(tomlAbs)),
    },
  ];
}

// --- slot data injection ---

function get_output_name(outDir: string): string {
  // `path.basename` handles trailing separators the same way Rust's
  // `Path::file_name` does — "/tmp/out/" → "out", "/" → "".
  const base = pathBasename(outDir);
  return base !== "" ? base : "project";
}

function get_project_name(config: SpackleConfig | null, projectDir: string): string {
  if (config?.name) return config.name;
  const base = pathBasename(projectDir);
  const dot = base.lastIndexOf(".");
  return dot > 0 ? base.slice(0, dot) : base;
}

function injectSpecials(slotData: SlotData, projectName: string, outputName: string): SlotData {
  return {
    ...slotData,
    _project_name: projectName,
    _output_name: outputName,
  };
}

// --- check / validateSlotData ---

/**
 * Static project validation. Returns the parsed config on success so
 * UIs can render slot forms without re-parsing TOML.
 */
export async function check(
  projectDir: string,
  fs: DiskFs,
  opts: CheckOptions = {},
): Promise<CheckResponse> {
  const absProject = fs.containProject(projectDir);
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = buildCheckBundle(absProject, virtualDir);
  return checkBundle(bundle, virtualDir);
}

/** Pass-through wrapper over `wasm.check` for hosts that build the
 * bundle themselves. */
export async function checkBundle(
  bundle: Bundle,
  virtualProjectDir: string = DEFAULT_VIRTUAL_PROJECT,
): Promise<CheckResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.check(bundle, virtualProjectDir);
}

/** Validate slot data against a project on disk. */
export async function validateSlotData(
  projectDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: CheckOptions = {},
): Promise<ValidationResponse> {
  const absProject = fs.containProject(projectDir);
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = buildConfigOnlyBundle(absProject, virtualDir);
  return validateSlotDataBundle(bundle, slotData, virtualDir);
}

/** Bundle pass-through of `validateSlotData`. */
export async function validateSlotDataBundle(
  bundle: Bundle,
  slotData: SlotData,
  virtualProjectDir: string = DEFAULT_VIRTUAL_PROJECT,
): Promise<ValidationResponse> {
  const wasm = await loadSpackleWasm();
  return wasm.validateSlotData(bundle, virtualProjectDir, slotData);
}

// --- generate (disk-direct, fail-fast) ---

/**
 * Walk `projectDir` and write the filled project to `outDir`. Static
 * files stream through `pipeline()`; templated bodies render via wasm
 * and write via `DiskFs.writeFile`. Returns counts; the rendered tree
 * is on disk by the time the promise resolves.
 *
 * Fail-fast: the first per-entry failure short-circuits the rest of
 * the walk. Whatever was already written stays — there's no rollback.
 * Config / slot data errors fail before `outDir` is created.
 *
 * Hooks are a separate step — call `runHooksStream()` after
 * `generate` if the project defines any.
 */
export async function generate(
  projectDir: string,
  outDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: GenerateOptions = {},
): Promise<GenerateResponse> {
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const absProject = fs.containProject(projectDir);

  // AlreadyExists is the first thing native checks; match that order
  // so a pre-existing outDir fails before any wasm work.
  const absOut = fs.assertOutDirAvailable(outDir);

  const wasm = await loadSpackleWasm();

  const checkInput = buildCheckBundle(absProject, virtualProject);
  const checkRes = wasm.check(checkInput, virtualProject);
  const fatal = checkRes.diagnostics.find((d) => d.severity === "error");
  if (fatal) return { ok: false, error: `${fatal.path ?? "spackle.toml"}: ${fatal.message}` };

  const configBundle = buildConfigOnlyBundle(absProject, virtualProject);
  const slotRes = wasm.validateSlotData(configBundle, virtualProject, slotData);
  if (!slotRes.valid) {
    return { ok: false, error: slotRes.errors.join("; ") };
  }

  const projectName = get_project_name(checkRes.config, absProject);
  const outputName = get_output_name(outDir);
  const data = injectSpecials(slotData, projectName, outputName);
  const ignore = checkRes.config?.ignore ?? [];

  let files = 0;
  let dirs = 0;
  for (const entry of walkDisk(absProject)) {
    // Classify on the **source** filename, not the rendered path. A
    // static file whose templated name renders to `foo.j2` is still
    // a copy, not a template (e.g. `{{ name }}` with `name = "foo.j2"`).
    const isTemplate = entry.kind === "file" && hasTemplateExt(entry.relPath);

    // Dirs and static files get skipped by both filters; templates
    // render regardless of either (native `template::fill` walks the
    // full tree).
    if (!isTemplate) {
      if (isIgnoredByBasename(entry.relPath, ignore)) continue;
      if (hasConfigFileAncestor(entry.relPath)) continue;
    }

    const pathRes = wasm.renderPath(entry.relPath, data);
    if (pathRes.diagnostics.length > 0) {
      const d = pathRes.diagnostics[0];
      if (d) return { ok: false, error: `${d.path ?? entry.relPath}: ${d.message}` };
    }
    const renderedRel = pathRes.path;

    if (entry.kind === "dir") {
      fs.ensureOutDir(fs.containedJoin(absOut, renderedRel));
      dirs++;
      continue;
    }

    if (isTemplate) {
      const body = fs.readFile(entry.absPath);
      const renderRes = wasm.renderFile(body, data, entry.relPath);
      if (renderRes.diagnostics.length > 0) {
        const d = renderRes.diagnostics[0];
        if (d) return { ok: false, error: `${d.path ?? entry.relPath}: ${d.message}` };
      }
      const dstAbs = fs.containedJoin(absOut, stripTemplateExt(renderedRel));
      fs.writeFile(dstAbs, renderRes.bytes);
      files++;
    } else {
      const dstAbs = fs.containedJoin(absOut, renderedRel);
      try {
        // Sequential: parallel copies would explode the FD budget on
        // large projects.
        // oxlint-disable-next-line eslint/no-await-in-loop
        await fs.streamCopy(entry.absPath, dstAbs);
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        return { ok: false, error: `${entry.relPath}: ${msg}` };
      }
      files++;
    }
  }

  // An all-ignored / empty project still produces an empty `outDir`.
  fs.ensureOutDir(absOut);

  return { ok: true, files, dirs };
}

// --- render (disk-direct, diagnostics-first) ---

/**
 * Diagnostics-first preview against a disk project. Walks
 * `projectDir`, accumulates rendered + copied files into a `Bundle`,
 * and collects every per-file failure into `diagnostics`. Never
 * throws / never returns `ok: false`; per-file failures don't abort
 * the walk. Use `generate` for fail-fast disk writes.
 *
 * `hookPlan` is included unless the config didn't parse.
 */
export async function render(
  projectDir: string,
  outDir: string,
  slotData: SlotData,
  fs: DiskFs,
  opts: GenerateOptions = {},
): Promise<RenderResponse> {
  const absProject = fs.containProject(projectDir);
  const virtualProject = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;

  const wasm = await loadSpackleWasm();

  const checkInput = buildCheckBundle(absProject, virtualProject);
  const checkRes = wasm.check(checkInput, virtualProject);
  const diagnostics: Diagnostic[] = [...checkRes.diagnostics];

  if (!checkRes.config) {
    return { files: [], dirs: [], diagnostics, hookPlan: null };
  }

  // Slot data errors become diagnostics, not a hard abort — Tera
  // substitutes empty strings for missing keys so the preview can
  // still partially render.
  const configBundle = buildConfigOnlyBundle(absProject, virtualProject);
  const slotRes = wasm.validateSlotData(configBundle, virtualProject, slotData);
  if (!slotRes.valid) {
    for (const message of slotRes.errors) {
      diagnostics.push({ severity: "error", source: "slot_data", message });
    }
  }

  const projectName = get_project_name(checkRes.config, absProject);
  const outputName = get_output_name(outDir);
  const data = injectSpecials(slotData, projectName, outputName);
  const ignore = checkRes.config.ignore ?? [];

  const fileMap = new Map<string, Uint8Array>();
  const dirSet = new Set<string>();
  for (const entry of walkDisk(absProject)) {
    const isTemplate = entry.kind === "file" && hasTemplateExt(entry.relPath);
    // Dirs and static files get filtered by ignore and by
    // spackle.toml-ancestor; templates render regardless.
    if (!isTemplate) {
      if (isIgnoredByBasename(entry.relPath, ignore)) continue;
      if (hasConfigFileAncestor(entry.relPath)) continue;
    }

    const pathRes = wasm.renderPath(entry.relPath, data);
    diagnostics.push(...pathRes.diagnostics);
    // On render_name failure, `path` falls back to the input so the
    // walk continues against a stable path.
    const renderedRel = pathRes.path;

    if (entry.kind === "dir") {
      dirSet.add(renderedRel);
      continue;
    }

    if (isTemplate) {
      const body = fs.readFile(entry.absPath);
      const renderRes = wasm.renderFile(body, data, entry.relPath);
      diagnostics.push(...renderRes.diagnostics);
      if (renderRes.diagnostics.length === 0) {
        fileMap.set(stripTemplateExt(renderedRel), renderRes.bytes);
      }
    } else {
      // Static bytes go into the preview bundle. GB-scale assets
      // should use `generate` (which streams) — `render` buffers.
      fileMap.set(renderedRel, fs.readFile(entry.absPath));
    }
  }

  const planRes = wasm.planHooks(configBundle, virtualProject, outDir, data);
  let hookPlan: HookPlanEntry[] | null = null;
  if (planRes.ok) {
    hookPlan = planRes.plan;
    for (const entry of planRes.plan) {
      for (const msg of entry.template_errors ?? []) {
        diagnostics.push({
          severity: "error",
          source: "hook_config",
          message: msg,
          ref: entry.key,
          path: CONFIG_FILE,
          code: "hook::template_render_failed",
        });
      }
    }
  } else {
    diagnostics.push({ severity: "error", source: "hook_config", message: planRes.error });
  }

  const files: Bundle = [...fileMap.entries()]
    .map(([path, bytes]) => ({ path, bytes }))
    .toSorted((a, b) => (a.path < b.path ? -1 : a.path > b.path ? 1 : 0));
  const dirs = [...dirSet].toSorted();
  return { files, dirs, diagnostics, hookPlan };
}

// --- hooks ---

/**
 * Inspect the hook plan without executing. Returns the resolved plan
 * (templated commands, should-run flags, skip reasons).
 */
export async function planHooks(
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  fs: DiskFs,
  opts: CheckOptions & { hookRan?: Record<string, boolean> } = {},
): Promise<PlanHooksResponse> {
  const absProject = fs.containProject(projectDir);
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = buildConfigOnlyBundle(absProject, virtualDir);
  return planHooksBundle(bundle, virtualDir, outDir, data, opts.hookRan);
}

/** Bundle pass-through of `planHooks`. */
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
 * Run the project's hooks, yielding `HookEvent`s as they occur.
 * Executes host-side via `opts.hooks ?? defaultHooks()`.
 *
 * `data` is the full data map (slot values + hook toggles keyed by
 * the hook's raw `key`). `_project_name` / `_output_name` are
 * injected wasm-side.
 *
 * Non-zero exit continues the run; template render failures are a
 * terminal `template_errors` event.
 */
export function runHooksStream(
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  fs: DiskFs,
  opts: RunHooksOptions = {},
): AsyncGenerator<HookEvent> {
  const absProject = fs.containProject(projectDir);
  const virtualDir = opts.virtualProjectDir ?? DEFAULT_VIRTUAL_PROJECT;
  const bundle = buildConfigOnlyBundle(absProject, virtualDir);
  async function* inner(): AsyncGenerator<HookEvent> {
    const wasm: SpackleWasm = await loadSpackleWasm();
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

// --- re-exports ---

export type {
  ConfigureSpackleWasmOptions,
  SpackleWasm,
  SpackleWasmModuleSource,
} from "./wasm/index.ts";
export { configureSpackleWasm, loadSpackleWasm } from "./wasm/index.ts";
export { DiskFs, type DiskFsOptions } from "./host/disk-fs.ts";
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
  RenderFileResponse,
  RenderPathResponse,
  RenderResponse,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  ValidationResponse,
} from "./wasm/types.ts";
