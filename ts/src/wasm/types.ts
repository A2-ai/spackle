// WASM-SIDE — types that cross the wasm-bindgen boundary.
//
// Hand-maintained to match the shapes emitted by Rust's bundle-in /
// bundle-out exports in `crates/spackle-wasm/src/lib.rs`. Keep in sync.

export type SlotType = "String" | "Number" | "Boolean";

export interface Slot {
  key: string;
  type?: SlotType;
  name?: string;
  description?: string;
  default?: string;
  needs?: string[];
}

export interface Hook {
  key: string;
  command: string[];
  /** Optional conditional (tera template string, evaluated as bool). */
  if?: string | null;
  needs?: string[];
  name?: string | null;
  description?: string | null;
  /** Whether the hook runs by default when no per-hook override is set. */
  default?: boolean | null;
}

export interface SpackleConfig {
  name: string | null;
  ignore: string[];
  slots: Slot[];
  hooks: Hook[];
}

/** Slot values the caller supplies. Always string-valued — Rust parses
 * / coerces against each slot's declared type. */
export type SlotData = Record<string, string>;

/** A single file in a project input or generated output bundle. Paths
 * are virtual — absolute from the bundle's root for inputs (e.g.
 * `/project/spackle.toml`) and relative from `outDir` for outputs
 * (e.g. `src/main.rs`). */
export interface BundleEntry {
  path: string;
  bytes: Uint8Array;
}

/** The shape spackle-wasm takes as input (check / validate / generate)
 * and returns as output (generate's rendered subtree). */
export type Bundle = BundleEntry[];

/** Response from `check()`. On success, includes the parsed config so
 * callers can render forms without re-parsing TOML. */
export type CheckResponse =
  | { valid: true; config: SpackleConfig; errors: [] }
  | { valid: false; errors: string[] };

/** Response from `validateSlotData()`. */
export type ValidationResponse = { valid: true } | { valid: false; errors: string[] };

/** Single output entry streamed from wasm during generate. Paths are
 * **relative to outDir**. Files carry their bytes; dirs are markers so
 * empty dirs (created by the Rust copy pass for directory entries that
 * had no files pass the ignore filter) survive the round-trip. */
export type GenerateStreamFileEvent = {
  kind: "file";
  path: string;
  bytes: Uint8Array;
};
export type GenerateStreamDirEvent = { kind: "dir"; path: string };
export type GenerateStreamEntry = GenerateStreamFileEvent | GenerateStreamDirEvent;

/** Public event union surfaced by the `generateStream` async generator.
 * Adds terminal `done` / `error` events to the streamed entries. */
export type GenerateStreamEvent =
  | GenerateStreamFileEvent
  | GenerateStreamDirEvent
  | { kind: "error"; error: string }
  | { kind: "done" };

/** Terminal envelope returned from a single `wasm.generate(...)` call.
 * Streamed file/dir entries arrive separately through the host callback —
 * this is just the success/error signal. Host-callback throws are
 * latched wasm-side and surfaced here with the original message. */
export type GenerateResult = { ok: true } | { ok: false; error: string };

/** Response from the buffered `generateBundle()` wrapper. Same shape
 * as the legacy bundle-output API: a flat list of files plus the
 * directory subtree (so empty dirs survive). Hosts that want to keep
 * the rendered output in memory (preview, in-process consumers) call
 * `generateBundle`. Hosts that want to write to disk call the
 * `generate(projectDir, outDir, ...)` wrapper, which uses
 * `DiskFs.writeEntry` per event and never materializes a `Bundle`. */
export type GenerateResponse =
  | { ok: true; files: Bundle; dirs: string[] }
  | { ok: false; error: string };

/** Response from the disk-streaming `generate()` wrapper. Returns
 * counts instead of a materialized bundle — that's the whole point of
 * streaming: bytes never accumulate host-side. */
export type GenerateDiskResponse =
  | { ok: true; files: number; dirs: number }
  | { ok: false; error: string };

/** One entry in a hook plan. Snake_case fields mirror Rust's
 * `HookPlanEntry` (`#[derive(Serialize)]` default casing). */
export interface HookPlanEntry {
  key: string;
  /** Templated command args — `{{ _project_name }}` etc. already resolved. */
  command: string[];
  /** `true` = runner should execute; `false` = skip (see `skip_reason`)
   * or abort (if `template_errors` is non-empty — treat as hard failure). */
  should_run: boolean;
  /** Present when `should_run = false` and `template_errors` is empty.
   * Values: `"user_disabled"`, `"false_conditional"`, `"unsatisfied_needs"`,
   * or `"conditional_error: ..."`. */
  skip_reason?: string;
  /** Non-empty = template rendering failed — hard error per native
   * `Error::ErrorRenderingTemplate`. Absent / empty = clean. */
  template_errors?: string[];
}

/** Response from `planHooks()`. Either the resolved plan or an error
 * (invalid bundle, TOML parse failure, bad JSON inputs, etc.). */
export type PlanHooksResponse = { ok: true; plan: HookPlanEntry[] } | { ok: false; error: string };
