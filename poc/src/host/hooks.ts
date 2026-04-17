// HOST-SIDE — requires subprocess spawning (Bun.spawn). Not available in
// WASM: `wasm32-unknown-unknown` has no process APIs, and there's no WASI
// analog for fork/exec either. This file is the ONLY place spackle shells
// out — the WASM layer (`evaluate_hooks`) plans what to run, this layer
// actually runs it.
//
// Mirrors `hook::run_hooks_stream` semantically: iterate the plan, run
// each `should_run: true` entry in order, capture stdout/stderr.

import type { HookPlanEntry } from "../wasm/types.ts";

export interface HookResult {
  /** Hook key from the plan. */
  key: string;
  /** Command that was spawned (pre-templated by WASM). */
  command: string[];
  /** Process exit code (null if the process was killed before exiting). */
  exit_code: number | null;
  stdout: string;
  stderr: string;
  duration_ms: number;
}

export interface HookSkipped {
  key: string;
  skip_reason: string;
  template_errors?: string[];
}

export type HookOutcome =
  | { kind: "ran"; result: HookResult }
  | { kind: "skipped"; info: HookSkipped };

/** HOST: Execute the hook plan against `cwd`. Yields one outcome per plan
 * entry in order. `should_run: false` entries are yielded as `skipped`
 * without spawning. */
export async function* executeHookPlan(
  plan: HookPlanEntry[],
  cwd: string,
): AsyncGenerator<HookOutcome> {
  for (const entry of plan) {
    if (!entry.should_run) {
      yield {
        kind: "skipped",
        info: {
          key: entry.key,
          skip_reason: entry.skip_reason ?? "unknown",
          template_errors: entry.template_errors,
        },
      };
      continue;
    }
    yield { kind: "ran", result: await runOne(entry, cwd) };
  }
}

async function runOne(entry: HookPlanEntry, cwd: string): Promise<HookResult> {
  const start = performance.now();
  const proc = Bun.spawn(entry.command, {
    cwd,
    stdout: "pipe",
    stderr: "pipe",
  });
  const [stdout, stderr, exit_code] = await Promise.all([
    new Response(proc.stdout).text(),
    new Response(proc.stderr).text(),
    proc.exited,
  ]);
  return {
    key: entry.key,
    command: entry.command,
    exit_code,
    stdout,
    stderr,
    duration_ms: Math.round(performance.now() - start),
  };
}
