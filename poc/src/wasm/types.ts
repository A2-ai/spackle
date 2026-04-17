// WASM-SIDE — pure computation. No I/O.
//
// Hand-written TypeScript interfaces that mirror the JSON shapes returned by
// the Rust `#[wasm_bindgen]` exports in `src/wasm.rs`. These are the *only*
// types that cross the WASM boundary; everything else is host-side.

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
  /** Optional conditional expression — rendered as a tera template and
   * parsed as a bool. */
  if?: string | null;
  /** Other hook/slot keys that must precede this one. */
  needs?: string[];
  name?: string | null;
  description?: string | null;
  /** Whether the hook runs by default when no per-hook run directive is
   * provided. Matches Rust `Hook.default: Option<bool>` exactly — a
   * top-level field, NOT nested under `optional`. (Some legacy test
   * fixtures use `optional = { default = X }` in TOML, but that field is
   * silently ignored by serde.) */
  default?: boolean | null;
}

export interface SpackleConfig {
  name: string | null;
  ignore: string[];
  slots: Slot[];
  hooks: Hook[];
}

export interface ValidationResult {
  valid: boolean;
  errors?: string[];
}

export interface TemplateInput {
  path: string;
  content: string;
}

export interface RenderedTemplate {
  original_path: string;
  rendered_path: string;
  content: string;
  error?: string;
}

export interface HookPlanEntry {
  key: string;
  command: string[];
  should_run: boolean;
  skip_reason?: string;
  template_errors?: string[];
}

/** Slot values the caller supplies to drive a generate. Always string-valued
 * (the WASM layer parses/coerces based on the slot type). */
export type SlotData = Record<string, string>;
