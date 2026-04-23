// Host-side hook runner — executes the plan that wasm `plan_hooks` emits.
//
// The package ships working execution out of the box. Node and Bun each
// get a concrete `SpackleHooks` impl (`NodeHooks` / `BunHooks`);
// `defaultHooks()` auto-selects between them. Browser-like environments
// (no Bun, no child_process) throw with a clear message — consumers
// there can plug a custom `SpackleHooks` that e.g. posts commands to a
// backend for execution.
//
// Semantics mirror native `run_hooks_stream` (src/hook.rs:383-540):
//   - continues on non-zero exit (collects a Failed result, moves on)
//   - template_errors → hard abort before any execution (matches
//     Error::ErrorRenderingTemplate in native)
//   - chained conditionals: re-plan after any hook whose outcome
//     diverges from the best-case assumption, so downstream
//     `if = "{{ hook_ran_X }}"` evaluates against actual state.

import type { Bundle, HookPlanEntry, PlanHooksResponse } from "../wasm/types.ts";

// --- public types ---

export interface HookExecuteResult {
  /** Convenience flag: `exitCode === 0`. */
  ok: boolean;
  exitCode: number;
  stdout: Uint8Array;
  stderr: Uint8Array;
}

export interface SpackleHooks {
  execute(
    command: string[] | string,
    cwd: string,
    env?: Record<string, string>,
  ): Promise<HookExecuteResult>;
}

/** Per-hook outcome carried inside `hook_end` events. */
export type HookRunResult =
  | {
      key: string;
      kind: "completed";
      exitCode: 0;
      stdout: Uint8Array;
      stderr: Uint8Array;
    }
  | {
      key: string;
      kind: "failed";
      exitCode: number;
      stdout: Uint8Array;
      stderr: Uint8Array;
      /** Set for launch errors (process couldn't even start); empty
       * string when the process ran but exited non-zero. */
      error?: string;
    }
  | { key: string; kind: "skipped"; skipReason: string };

export interface TemplateErrorDetail {
  key: string;
  errors: string[];
}

/** Events yielded by `runHookPlanStream` / `runHooksStream`.
 *
 * Ordering guarantees:
 * - `run_start` is always the first event if the initial plan is clean.
 * - Each runnable hook emits `hook_start` then exactly one `hook_end`.
 * - Skipped hooks (including conditional-error hooks, which surface as
 *   `kind: "failed"` for native parity) emit only `hook_end`.
 * - `replan` appears after a `hook_end` with `kind: "failed"` whenever
 *   the remaining plan was re-evaluated against mutated `hook_ran` state.
 * - `template_errors` and `plan_error` are terminal — the iterator ends
 *   immediately after emitting them.
 */
export type HookEvent =
  | { type: "run_start"; plan: HookPlanEntry[] }
  | {
      type: "hook_start";
      key: string;
      command: string[];
      /** ms since epoch (Date.now()) when the subprocess was spawned. */
      startedAt: number;
    }
  | {
      type: "hook_end";
      key: string;
      result: HookRunResult;
      /** Present for completed/failed (the hook actually ran); absent
       * for skipped and conditional-error hooks (no subprocess). */
      startedAt?: number;
      finishedAt?: number;
      durationMs?: number;
    }
  | { type: "replan"; afterKey: string; plan: HookPlanEntry[] }
  | { type: "template_errors"; error: string; templateErrors: TemplateErrorDetail[] }
  | { type: "plan_error"; error: string };

export interface RunHookPlanOptions {
  bundle: Bundle;
  projectDir: string;
  /** Used for `_output_name` injection during re-plans, AND as the
   * default `cwd` for spawned processes. */
  outDir: string;
  /** Full data map: slot values + hook toggles (keyed by raw hook key,
   * e.g. `data["format_code"] = "false"`). Matches native shape. */
  data: Record<string, string>;
  /** Optional injected executor. Defaults to `defaultHooks()`. */
  hooks?: SpackleHooks;
  /** Working dir for spawned processes. Defaults to `outDir`. */
  cwd?: string;
  env?: Record<string, string>;
}

const SAFE_ARG_PATTERN = /^[\w@%+=:,./-]+$/;

/** Parse a shell-like command line into argv.
 *
 * Supports:
 * - whitespace splitting
 * - single and double quotes
 * - backslash escapes (outside and inside double quotes)
 * - adjacent quoted/unquoted segments in one arg (`a"b"c` -> `abc`)
 *
 * Throws when quotes are unmatched or a trailing backslash is left dangling.
 */
export function parseShellLine(text: string): string[] {
  const argv: string[] = [];
  let current = "";
  let hasCurrent = false;
  let quote: "'" | '"' | null = null;
  let escaped = false;

  for (let i = 0; i < text.length; i += 1) {
    const ch = text[i];
    if (ch === undefined) break;

    if (escaped) {
      current += ch;
      hasCurrent = true;
      escaped = false;
      continue;
    }

    if (quote === "'") {
      if (ch === "'") {
        quote = null;
      } else {
        current += ch;
        hasCurrent = true;
      }
      continue;
    }

    if (quote === '"') {
      if (ch === '"') {
        quote = null;
      } else if (ch === "\\") {
        escaped = true;
      } else {
        current += ch;
        hasCurrent = true;
      }
      continue;
    }

    if (/\s/.test(ch)) {
      if (hasCurrent) {
        argv.push(current);
        current = "";
        hasCurrent = false;
      }
      continue;
    }

    if (ch === "'" || ch === '"') {
      quote = ch;
      hasCurrent = true;
      continue;
    }

    if (ch === "\\") {
      escaped = true;
      hasCurrent = true;
      continue;
    }

    current += ch;
    hasCurrent = true;
  }

  if (escaped) {
    throw new Error("parseShellLine: unterminated escape sequence");
  }
  if (quote !== null) {
    throw new Error("parseShellLine: unterminated quoted string");
  }
  if (hasCurrent) {
    argv.push(current);
  }
  return argv;
}

/** Render argv as a shell-safe command line.
 *
 * Uses single-quote wrapping for values that need quoting, with
 * POSIX-compatible escaping for embedded single quotes.
 */
export function formatArgv(argv: readonly string[]): string {
  return argv
    .map((arg) => {
      if (arg.length === 0) return "''";
      if (SAFE_ARG_PATTERN.test(arg)) return arg;
      return `'${arg.replaceAll("'", `'"'"'`)}'`;
    })
    .join(" ");
}

function normalizeCommandArgv(command: string[] | string): string[] {
  const argv = typeof command === "string" ? parseShellLine(command) : command;
  if (argv.length === 0) {
    throw new Error("empty command");
  }
  return argv;
}

// --- shipped runner impls ---

type NodeSpawn = (
  command: string,
  args: string[],
  options: { cwd: string; env: NodeJS.ProcessEnv },
) => {
  stdout: NodeJS.ReadableStream | null;
  stderr: NodeJS.ReadableStream | null;
  on(event: "error", cb: (e: Error) => void): void;
  on(event: "close", cb: (code: number | null) => void): void;
};

async function collectStream(s: NodeJS.ReadableStream | null): Promise<Uint8Array> {
  if (!s) return new Uint8Array();
  const chunks: Uint8Array[] = [];
  for await (const chunk of s) {
    chunks.push(chunk instanceof Uint8Array ? chunk : new Uint8Array(Buffer.from(chunk)));
  }
  // Concat into one Uint8Array.
  const total = chunks.reduce((n, c) => n + c.byteLength, 0);
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.byteLength;
  }
  return out;
}

export class NodeHooks implements SpackleHooks {
  async execute(
    command: string[] | string,
    cwd: string,
    env?: Record<string, string>,
  ): Promise<HookExecuteResult> {
    const [cmd, ...args] = normalizeCommandArgv(command);
    if (cmd === undefined) throw new Error("NodeHooks.execute: empty command");
    // Lazy-load child_process so this module still imports cleanly in
    // environments that lack it (browsers) — `defaultHooks()` catches
    // and reports those hosts before we get here.
    // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
    const { spawn } = (await import("node:child_process")) as unknown as {
      spawn: NodeSpawn;
    };
    const mergedEnv: NodeJS.ProcessEnv = env ? { ...process.env, ...env } : process.env;

    const child = spawn(cmd, args, { cwd, env: mergedEnv });
    const stdoutPromise = collectStream(child.stdout);
    const stderrPromise = collectStream(child.stderr);

    return new Promise<HookExecuteResult>((resolve, reject) => {
      let settled = false;
      child.on("error", (e) => {
        if (settled) return;
        settled = true;
        reject(e);
      });
      child.on("close", (code) => {
        if (settled) return;
        settled = true;
        Promise.all([stdoutPromise, stderrPromise])
          .then(([stdout, stderr]) => {
            const exitCode = code ?? -1;
            resolve({ ok: exitCode === 0, exitCode, stdout, stderr });
            return undefined;
          })
          .catch(reject);
      });
    });
  }
}

// Minimal structural typing of the Bun APIs we touch, to avoid pulling
// in @types/bun or asserting across the full surface.
interface BunSpawnSubprocess {
  stdout: ReadableStream<Uint8Array>;
  stderr: ReadableStream<Uint8Array>;
  exited: Promise<number>;
  exitCode: number | null;
}
interface BunLike {
  spawn(opts: {
    cmd: string[];
    cwd?: string;
    env?: Record<string, string>;
    stdout: "pipe";
    stderr: "pipe";
  }): BunSpawnSubprocess;
}

async function readAll(stream: ReadableStream<Uint8Array>): Promise<Uint8Array> {
  const reader = stream.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    // Sequential reads are the API; each chunk depends on the prior read.
    // oxlint-disable-next-line eslint/no-await-in-loop
    const { value, done } = await reader.read();
    if (done) break;
    if (value) {
      chunks.push(value);
      total += value.byteLength;
    }
  }
  const out = new Uint8Array(total);
  let off = 0;
  for (const c of chunks) {
    out.set(c, off);
    off += c.byteLength;
  }
  return out;
}

export class BunHooks implements SpackleHooks {
  async execute(
    command: string[] | string,
    cwd: string,
    env?: Record<string, string>,
  ): Promise<HookExecuteResult> {
    const argv = normalizeCommandArgv(command);
    // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
    const bun = (globalThis as unknown as { Bun: BunLike }).Bun;
    // `process.env` types are `ProcessEnv` (keys may be undefined). We
    // control the merge shape here; asserting to Record is the boundary.
    // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
    const baseEnv = process.env as Record<string, string>;
    const proc = bun.spawn({
      cmd: argv,
      cwd,
      env: env ? { ...baseEnv, ...env } : undefined,
      stdout: "pipe",
      stderr: "pipe",
    });
    const [stdout, stderr, exitCode] = await Promise.all([
      readAll(proc.stdout),
      readAll(proc.stderr),
      proc.exited,
    ]);
    return { ok: exitCode === 0, exitCode, stdout, stderr };
  }
}

/** Runtime capability flags fed into `defaultHooks`. Split out so tests
 * can simulate Node-only / browser environments without mutating
 * globals. */
export interface HooksEnv {
  hasBun: boolean;
  hasNode: boolean;
}

/** Inspect the current runtime. Used by `defaultHooks()` to pick an
 * impl; exported so callers can invoke directly if they want to fork
 * on the result. */
export function detectHooksEnv(): HooksEnv {
  // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
  const hasBun = typeof (globalThis as { Bun?: unknown }).Bun !== "undefined";
  const hasNode =
    typeof process !== "undefined" &&
    typeof process.versions !== "undefined" &&
    typeof process.versions.node === "string";
  return { hasBun, hasNode };
}

/** Returns a `SpackleHooks` appropriate for the current runtime (or a
 * passed-in `env`, for tests):
 *
 * - Bun → `BunHooks`
 * - Node → `NodeHooks`
 * - Anything else (browsers) → throws a clear "no subprocess
 *   available" error so the caller knows they must provide a custom
 *   `SpackleHooks`.
 */
export function defaultHooks(env: HooksEnv = detectHooksEnv()): SpackleHooks {
  if (env.hasBun) return new BunHooks();
  if (env.hasNode) return new NodeHooks();
  throw new Error(
    "no subprocess available in this environment — provide a custom SpackleHooks to runHooksStream()",
  );
}

// --- orchestrator ---

// Wrapper that can be dependency-injected in tests / bundle-only flows.
// Kept private to this module so the public API stays the same.
type Planner = (
  bundle: Bundle,
  projectDir: string,
  outDir: string,
  data: Record<string, string>,
  hookRan: Record<string, boolean> | undefined,
) => PlanHooksResponse;

/**
 * Internal worker that takes a pre-bound planner (via `loadSpackleWasm()`
 * in `spackle.ts`) and iterates the plan, yielding events in real time.
 * Matches native `run_hooks_stream` semantics exactly. Exported for
 * direct use by the `spackle.ts` layer — most callers should use
 * `runHooksStream()` there.
 *
 * Event protocol: see `HookEvent`. The generator emits `run_start` once,
 * then per-hook `hook_start` (runnable hooks only) + `hook_end`, with
 * optional `replan` events between failures and their follow-ups.
 * Terminal `template_errors` / `plan_error` events end the iterator.
 */
export async function* runHookPlanStream(
  planner: Planner,
  opts: RunHookPlanOptions,
): AsyncGenerator<HookEvent> {
  const hooks = opts.hooks ?? defaultHooks();
  const cwd = opts.cwd ?? opts.outDir;

  /** Walk a plan for template_errors. Returns a terminal event if any
   * are present (hard abort, matching native Error::ErrorRenderingTemplate),
   * or `null` if the plan is clean. Applied to the initial plan AND every
   * re-plan — a re-plan's data context differs from the initial's
   * (hook_ran_* state has been updated), so a previously-clean hook
   * could surface errors on re-plan if its command depends on mutated
   * state. */
  const scanTemplateErrors = (
    p: HookPlanEntry[],
  ): Extract<HookEvent, { type: "template_errors" }> | null => {
    const errs: TemplateErrorDetail[] = [];
    for (const e of p) {
      if (e.template_errors && e.template_errors.length > 0) {
        errs.push({ key: e.key, errors: e.template_errors });
      }
    }
    const first = errs[0];
    if (first === undefined) return null;
    return {
      type: "template_errors",
      error: `template error in hook ${first.key}: ${first.errors.join("; ")}`,
      templateErrors: errs,
    };
  };

  const initial = planner(opts.bundle, opts.projectDir, opts.outDir, opts.data, undefined);
  if (!initial.ok) {
    yield { type: "plan_error", error: initial.error };
    return;
  }

  const initialErrs = scanTemplateErrors(initial.plan);
  if (initialErrs !== null) {
    yield initialErrs;
    return;
  }

  // Yielded plans / commands are cloned so consumer mutations (e.g.
  // a UI reducer accidentally splicing a row) can never reach back
  // into the runner's own iteration state.
  yield { type: "run_start", plan: structuredClone(initial.plan) };

  const hookRan: Record<string, boolean> = {};
  let plan: HookPlanEntry[] = initial.plan;
  let idx = 0;

  while (idx < plan.length) {
    const entry = plan[idx];
    if (entry === undefined) break;

    if (!entry.should_run) {
      // Native run_hooks_stream at src/hook.rs:485 treats a conditional
      // evaluation error as Failed(HookError::ConditionalFailed), not
      // Skipped. Our planner surfaces these as skip_reason starting
      // with "conditional_error:" — re-categorize to "failed" here so
      // the event stream matches native outcome kinds. hook_ran is
      // already false (planner never flipped it); no re-plan needed.
      const reason = entry.skip_reason ?? "unknown";
      if (reason.startsWith("conditional_error:")) {
        yield {
          type: "hook_end",
          key: entry.key,
          result: {
            key: entry.key,
            kind: "failed",
            exitCode: -1,
            stdout: new Uint8Array(),
            stderr: new Uint8Array(),
            error: reason,
          },
        };
      } else {
        yield {
          type: "hook_end",
          key: entry.key,
          result: { key: entry.key, kind: "skipped", skipReason: reason },
        };
      }
      idx += 1;
      continue;
    }

    // Execute. A launch error (process can't start) becomes a failed
    // result rather than a thrown exception, to match the native
    // CommandLaunchFailed shape. Hooks run sequentially (native parity
    // per run_hooks_stream) so the await-in-loop is load-bearing.
    const startedAt = Date.now();
    // Clone `command` — we're about to pass the same array to
    // `hooks.execute` below; a consumer mutating the yielded payload
    // between the yield and the await would otherwise alter what gets
    // spawned.
    yield { type: "hook_start", key: entry.key, command: [...entry.command], startedAt };

    let outcome: HookExecuteResult;
    let launchError: string | undefined;
    try {
      // oxlint-disable-next-line eslint/no-await-in-loop
      outcome = await hooks.execute(entry.command, cwd, opts.env);
    } catch (e) {
      launchError = e instanceof Error ? e.message : String(e);
      outcome = {
        ok: false,
        exitCode: -1,
        stdout: new Uint8Array(),
        stderr: new Uint8Array(),
      };
    }
    const finishedAt = Date.now();
    const durationMs = finishedAt - startedAt;

    if (outcome.ok) {
      yield {
        type: "hook_end",
        key: entry.key,
        result: {
          key: entry.key,
          kind: "completed",
          exitCode: 0,
          stdout: outcome.stdout,
          stderr: outcome.stderr,
        },
        startedAt,
        finishedAt,
        durationMs,
      };
      hookRan[entry.key] = true;
      idx += 1;
      continue;
    }

    // Non-zero exit (or launch error). Continue — native parity per
    // src/hook.rs:527. Flip hookRan=false (already default, but be
    // explicit so re-plan includes it), then re-plan remaining hooks.
    yield {
      type: "hook_end",
      key: entry.key,
      result: {
        key: entry.key,
        kind: "failed",
        exitCode: outcome.exitCode,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        ...(launchError ? { error: launchError } : {}),
      },
      startedAt,
      finishedAt,
      durationMs,
    };
    hookRan[entry.key] = false;

    // Re-plan the remainder with actual hookRan state so chained
    // conditionals (`if = "{{ hook_ran_X }}"`) evaluate against reality.
    // Our plan_hooks filters out hooks already in hookRan, so the
    // returned plan is strictly the still-unexecuted tail.
    const replan = planner(opts.bundle, opts.projectDir, opts.outDir, opts.data, hookRan);
    if (!replan.ok) {
      // Re-plan error mid-run: surface as a terminal event, not an
      // exception. Consumers that care about partial results have
      // already seen every prior hook_end event.
      yield {
        type: "plan_error",
        error: `re-plan failed after hook ${entry.key}: ${replan.error}`,
      };
      return;
    }
    const replanErrs = scanTemplateErrors(replan.plan);
    if (replanErrs !== null) {
      yield replanErrs;
      return;
    }
    plan = replan.plan;
    idx = 0;
    yield { type: "replan", afterKey: entry.key, plan: structuredClone(plan) };
  }
}
