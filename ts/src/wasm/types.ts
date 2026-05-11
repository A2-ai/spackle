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
  /** Bundle-virtual path of the offending file, or `"spackle.toml"` for
   * config-level diagnostics. Absent when no file makes sense (slot data). */
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

/** Response from `render()`. Always returns this shape — `files` carries
 * every template that rendered successfully (partial preview), and
 * `diagnostics` enumerates every problem across stages. Empty
 * `diagnostics` ⇒ clean render. `hookPlan` is `null` only when the
 * config failed to load. */
export interface RenderResponse {
  files: Bundle;
  dirs: string[];
  diagnostics: Diagnostic[];
  hookPlan: HookPlanEntry[] | null;
}

/** Response from `validateSlotData()`. Legacy shape — superseded by
 * `render`'s `slot_data` diagnostics. Kept for granular standalone use. */
export type ValidationResponse = { valid: true } | { valid: false; errors: string[] };

/** Response from `generate()`.
 *
 * `files` carries the rendered output subtree with paths **relative to
 * outDir**. `dirs` carries the directory subtree (also relative) so
 * empty dirs survive the bundle round-trip — the native `copy` pass
 * `create_dir_all`s every directory entry it walks, and we match that
 * behavior host-side by mkdir'ing each dir even if no files live under
 * it.
 *
 * Hooks are a separate step — call `planHooks` / `runHooksStream`
 * after `generate` (mirrors the native CLI's two-call shape). */
export type GenerateResponse =
  | { ok: true; files: Bundle; dirs: string[] }
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
