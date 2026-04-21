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
import { loadSpackleWasm } from "./wasm/index.ts";
import type {
  Bundle,
  CheckResponse,
  GenerateResponse,
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
  /** Refused with an explicit unsupported error. Reserved for when
   * the hooks bridge lands. */
  runHooks?: boolean;
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
 * `runHooks = true` is unsupported in this milestone; the wasm side
 * returns `{ ok: false, error: "hooks are unsupported in this milestone" }`.
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
    runHooks: opts.runHooks,
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
  return wasm.generate(projectBundle, virtualProject, virtualOut, slotData, opts.runHooks ?? false);
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
export { throwUnsupportedHooks, type SpackleHooks, type HookResult } from "./host/hooks.ts";
export type {
  Bundle,
  BundleEntry,
  CheckResponse,
  GenerateResponse,
  Hook,
  Slot,
  SlotData,
  SlotType,
  SpackleConfig,
  ValidationResponse,
} from "./wasm/types.ts";
