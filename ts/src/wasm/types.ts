// WASM-SIDE — types that cross the wasm-bindgen boundary, plus the
// host-orchestrator response shapes built on top of the per-file
// primitives.
//
// Hand-maintained to match the shapes emitted by Rust's wasm exports in
// `crates/spackle-wasm/src/lib.rs`. Keep in sync.

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

/** A single file entry. Paths are virtual — absolute from the
 * bundle's root, e.g. `/project/spackle.toml`. */
export interface BundleEntry {
  path: string;
  bytes: Uint8Array;
}

/** Project input bundle to `check` / `validateSlotData` / `planHooks`.
 * The host puts only the files those calls actually need into the
 * bundle (typically `spackle.toml` plus any `.j2` templates the caller
 * wants statically validated). */
export type Bundle = BundleEntry[];

/** Diagnostic severity. Only `error` is currently emitted; `warning` is
 * reserved for future use (deprecated patterns, dead slots, etc.). */
export type DiagnosticSeverity = "error" | "warning";

/** Which pipeline stage produced the diagnostic. */
export type DiagnosticSource =
  /** `spackle.toml` parse / structural error. */
  | "config"
  /** Slot config error (bad default value type, etc.). */
  | "slot_config"
  /** Hook config error (unknown `needs`, broken command/conditional template). */
  | "hook_config"
  /** User-supplied slot data error (missing required, wrong type, unknown key). */
  | "slot_data"
  /** Copy-stage filesystem failure (read / write / mkdir). Path-template
   * failures are classified as `render_name` instead, regardless of file
   * extension. */
  | "copy"
  /** Template body render failure. */
  | "render_body"
  /** Filename / path template parse or render failure. Fires for `.j2`
   * filename templating AND non-`.j2` path templating — anywhere Tera
   * is applied to a file path. */
  | "render_name";

/** One-based line and column into a source file. */
export interface DiagnosticSpan {
  line: number;
  column: number;
}

/** A single diagnostic surfaced by `check()` or `render()`. */
export interface Diagnostic {
  severity: DiagnosticSeverity;
  source: DiagnosticSource;
  message: string;
  /** Bundle-virtual or workspace-relative path of the offending file,
   * or `"spackle.toml"` for config-level diagnostics. Absent when no
   * file makes sense (slot data). */
  path?: string;
  /** Slot or hook key when the diagnostic targets a config item rather
   * than a file. */
  ref?: string;
  /** Best-effort line/column. Absent when Tera's error format didn't
   * carry position info. */
  span?: DiagnosticSpan;
  /** Stable identifier so UIs can group/filter without parsing messages. */
  code?: string;
}

/** Response from `check()`. Always carries a `diagnostics` array — empty
 * means the project is structurally sound. `config` is `null` only when
 * the TOML couldn't be parsed (a `config`-source diagnostic explains why). */
export interface CheckResponse {
  config: SpackleConfig | null;
  diagnostics: Diagnostic[];
}

/** Response from the host-orchestrated `render()`. Always returns this
 * shape — `files` carries every template that rendered successfully
 * (partial preview), and `diagnostics` enumerates every problem
 * across stages. Empty `diagnostics` ⇒ clean render. `hookPlan` is
 * `null` only when the config failed to load. */
export interface RenderResponse {
  files: Bundle;
  dirs: string[];
  diagnostics: Diagnostic[];
  hookPlan: HookPlanEntry[] | null;
}

/** Response from `validateSlotData()`. Legacy shape — superseded by
 * `render`'s `slot_data` diagnostics. Kept for granular standalone use. */
export type ValidationResponse = { valid: true } | { valid: false; errors: string[] };

/** Response from the per-file `render_file` wasm primitive. `bytes` is
 * the rendered output on success; on error it's empty and the failure
 * is described in `diagnostics`. Callers branch on diagnostics, not on
 * byte count.
 *
 * `render_file` builds a Tera instance per call from the supplied
 * template-source bundle and renders only the target template, so
 * Tera 2's cross-template tags (`{% include %}` and `{% extends %}`)
 * resolve across the project. Tera 2 dropped `{% macro %}` /
 * `{% import %}`. Static asset bytes never enter the bundle — the
 * host passes only `.j2` / `.tera` bodies. */
export interface RenderFileResponse {
  bytes: Uint8Array;
  diagnostics: Diagnostic[];
}

/** Response from the per-file `render_path` wasm primitive. `path` is
 * the rendered path on success; on error it falls back to the input so
 * UIs can attribute the diagnostic. */
export interface RenderPathResponse {
  path: string;
  diagnostics: Diagnostic[];
}

/** Response from the disk-direct `generate()` orchestrator. Returns
 * counts instead of a materialized bundle — the rendered tree is
 * already on disk under `outDir` by the time the promise resolves. */
export type GenerateResponse =
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
