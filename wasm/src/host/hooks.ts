// Hooks bridge — STUB.
//
// Hooks are deferred in this milestone. The Rust side currently refuses
// `generate(..., runHooks = true)` with an explicit unsupported error;
// this file reserves the JS-side shape for the future `JsHooks` bridge
// so callers can import symbols without them existing twice later.

import type { Hook } from "../wasm/types.ts";

/**
 * Interface a future host will implement to execute hook plans that
 * Rust emits. Not wired to anything yet — calling `generate()` with
 * `runHooks: true` surfaces "hooks are unsupported in this milestone".
 *
 * Expected future shape: Rust returns an evaluated hook plan (an
 * ordered list of commands with resolved templates) and the host
 * executes them via child_process / Bun.spawn. The bridge is
 * deliberately thin — Rust does planning, host does execution.
 */
export interface SpackleHooks {
    run(hook: Hook, env: Record<string, string>): Promise<HookResult>;
}

export interface HookResult {
    exitCode: number;
    stdout: string;
    stderr: string;
}

/** Helper for APIs that accept a `SpackleHooks` but aren't yet wired.
 * Throws with an explicit unsupported-operation error; keep the message
 * stable — tests match on it. */
export function throwUnsupportedHooks(): never {
    throw new Error("hooks are unsupported in this milestone");
}

export type { Hook } from "../wasm/types.ts";
