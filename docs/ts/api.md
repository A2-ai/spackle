# API reference

`@a2-ai/spackle` exposes five primary operations — `check`, `validateSlotData`, `generate`, `planHooks`, `runHooksStream` — plus `…Bundle` variants that skip disk I/O for the first three. Two host helpers (`DiskFs`, `MemoryFs`) handle moving data in and out; a `SpackleHooks` interface (with shipped `NodeHooks` / `BunHooks` impls + `defaultHooks()` auto-selector) handles subprocess execution for hooks.

## Core types

```ts
type SlotType = "String" | "Number" | "Boolean";
type SlotData = Record<string, string>;

interface BundleEntry {
    path: string;
    bytes: Uint8Array;
}
type Bundle = BundleEntry[];

interface SpackleConfig {
    name: string | null;
    ignore: string[];
    slots: Slot[];
    hooks: Hook[];
}
```

See `src/wasm/types.ts` in the package for the full shape (including `Slot`, `Hook`, error/response unions).

## `check(projectDir, fs, opts?)`

Validate a project: load its `spackle.toml`, check slot structure, verify template references match declared slots.

```ts
function check(
    projectDir: string,
    fs: DiskFs,
    opts?: { virtualProjectDir?: string },
): Promise<CheckResponse>;

type CheckResponse =
    | { valid: true; config: SpackleConfig; errors: [] }
    | { valid: false; errors: string[] };
```

`fs.readProject(projectDir)` runs; the resulting bundle goes to wasm's `check`. On success you get the parsed config back so UIs can render slot forms without re-parsing TOML.

## `validateSlotData(projectDir, slotData, fs, opts?)`

Check a data set against a project's slot types.

```ts
function validateSlotData(
    projectDir: string,
    slotData: SlotData,
    fs: DiskFs,
    opts?: { virtualProjectDir?: string },
): Promise<ValidationResponse>;

type ValidationResponse =
    | { valid: true }
    | { valid: false; errors: string[] };
```

Rules enforced: every declared slot is present, types coerce (`"42"` → `Number` ok; `"not-a-number"` → `Number` fails), no undeclared slots.

## `generate(projectDir, outDir, slotData, fs, opts?)`

Run the full pipeline: copy non-template files, render `.j2` files, render path placeholders, write everything under `outDir`. Hooks are a separate call — see `runHooksStream()` below.

```ts
function generate(
    projectDir: string,
    outDir: string,
    slotData: SlotData,
    fs: DiskFs,
    opts?: {
        virtualProjectDir?: string;
        virtualOutDir?: string;
    },
): Promise<GenerateResponse>;

type GenerateResponse =
    | { ok: true; files: Bundle; dirs: string[] }
    | { ok: false; error: string };
```

`result.files` carries the rendered bundle with paths **relative to `outDir`**. `result.dirs` carries directory paths (also relative) — present so **empty directories survive the round-trip**. Native `spackle generate` calls `create_dir_all` for every directory walked during the copy pass; without emitting them, a project whose `drafts/` directory is fully ignored (every file filtered out) would still have `drafts/` created on native but silently dropped under wasm. Hosts writing output manually MUST mkdir each entry in `dirs` to match native behavior.

`DiskFs.writeOutput` (built-in) handles both `files` and `dirs` for you. Output-dir contract: `writeOutput` throws if `outDir` already exists, matching native's `GenerateError::AlreadyExists`.

## `planHooks(projectDir, outDir, data, fs, opts?)`

Inspect the hook plan without executing — resolves `needs`, templates command args, evaluates conditionals. Useful for UIs that want to preview what would run.

```ts
function planHooks(
    projectDir: string,
    outDir: string,
    data: Record<string, string>,
    fs: DiskFs,
    opts?: {
        virtualProjectDir?: string;
        hookRan?: Record<string, boolean>;
    },
): Promise<PlanHooksResponse>;

type PlanHooksResponse =
    | { ok: true; plan: HookPlanEntry[] }
    | { ok: false; error: string };

interface HookPlanEntry {
    key: string;
    command: string[];            // templated args
    should_run: boolean;
    skip_reason?: string;         // "user_disabled" | "unsatisfied_needs"
                                  // | "false_conditional" | "template_error"
                                  // | "conditional_error: ..."
    template_errors?: string[];   // non-empty = hard error (native parity)
}
```

`data` matches native's `Project::run_hooks_stream` input: slot values PLUS hook toggles keyed by the hook's own `key` (so `data["format_code"] = "false"` disables that hook). `_project_name` / `_output_name` are injected wasm-side — don't pre-inject.

`hookRan` (optional): map of `{ hookKey: actualRanOutcome }`. Hooks present in this map are filtered from the returned plan (host already has their results) while `hook_ran_<key>` is pre-seeded so chained conditionals in downstream hooks evaluate against the real state.

## `runHooksStream(projectDir, outDir, data, fs, opts?)`

Run the project's hooks, yielding `HookEvent`s as each hook progresses. Mirrors the native CLI's two-call shape: call after `generate()`. Reads the bundle, plans via wasm, executes each hook host-side via the injected or auto-selected `SpackleHooks`, and re-plans internally after any non-zero exit so chained conditionals re-evaluate against actual outcomes.

```ts
function runHooksStream(
    projectDir: string,
    outDir: string,
    data: Record<string, string>,
    fs: DiskFs,
    opts?: {
        virtualProjectDir?: string;
        hooks?: SpackleHooks;     // defaults to defaultHooks()
        cwd?: string;              // defaults to outDir
        env?: Record<string, string>;
    },
): AsyncGenerator<HookEvent>;

type HookEvent =
    | { type: "run_start"; plan: HookPlanEntry[] }
    | { type: "hook_start"; key: string; command: string[]; startedAt: number }
    | { type: "hook_end"; key: string; result: HookRunResult;
        startedAt?: number; finishedAt?: number; durationMs?: number }
    | { type: "replan"; afterKey: string; plan: HookPlanEntry[] }
    | { type: "template_errors"; error: string; templateErrors: { key: string; errors: string[] }[] }
    | { type: "plan_error"; error: string };

type HookRunResult =
    | { key; kind: "completed"; exitCode: 0; stdout; stderr }
    | { key; kind: "failed";    exitCode;     stdout; stderr; error? }
    | { key; kind: "skipped";   skipReason };
```

Usage:

```ts
for await (const event of runHooksStream(projectDir, outDir, data, fs)) {
    // drive UI transitions / SSE frames off each event
}
```

Event protocol (full detail in [`docs/ts/hooks.md`](./hooks.md)):

- `run_start` is first when the initial plan is clean.
- Runnable hooks emit `hook_start` + `hook_end`; skipped / conditional-error hooks emit only `hook_end`.
- `hook_end.durationMs` is present when the hook actually ran.
- `replan` fires after a failed hook whenever chained conditionals need re-evaluation.
- `template_errors` / `plan_error` are **terminal** — the iterator ends after yielding them.

Failure semantics match native `run_hooks_stream`:

- **Non-zero exit continues the run** — emits `hook_end` with `kind: "failed"` but subsequent hooks still execute. `hook_ran_<key>` stays `false`; chained `if = "{{ hook_ran_X }}"` conditionals naturally demote dependents (visible via a `replan` event).
- **Template errors hard-abort** — an unresolved `{{ ... }}` in any hook's command yields a terminal `template_errors` event before any execution (parity with `Error::ErrorRenderingTemplate` native). Checked on the initial plan AND after every re-plan.
- **Re-plan failures surface as a terminal `plan_error` event, not throws** — if the planner errors mid-run (shouldn't happen in practice), yields `{ type: "plan_error", error: "re-plan failed after hook X: ..." }`. `runHooksStream` never throws for expected outcomes.

## `SpackleHooks` / `defaultHooks` / `detectHooksEnv`

```ts
interface HookExecuteResult {
    ok: boolean;       // exitCode === 0
    exitCode: number;
    stdout: Uint8Array;
    stderr: Uint8Array;
}

interface SpackleHooks {
    execute(command: string[] | string, cwd: string, env?: Record<string, string>):
        Promise<HookExecuteResult>;
}

class NodeHooks implements SpackleHooks { /* child_process.spawn */ }
class BunHooks implements SpackleHooks { /* Bun.spawn */ }

interface HooksEnv {
    hasBun: boolean;
    hasNode: boolean;
}
function detectHooksEnv(): HooksEnv;
function defaultHooks(env?: HooksEnv): SpackleHooks;
function parseShellLine(text: string): string[];
function formatArgv(argv: readonly string[]): string;
```

`defaultHooks()` picks `BunHooks` or `NodeHooks` based on `detectHooksEnv()`. In environments without either (browsers), it throws with a clear "no subprocess available" message — supply a custom `SpackleHooks` (e.g. one that POSTs to a backend) if you need browser-side hook behavior. Pass an explicit `env` to force a particular impl (useful in tests).

`parseShellLine` and `formatArgv` are exported from the same module as the hook runner and use the same argv semantics. Use them when converting user-entered command text (e.g. `echo "hello world"`) to argv and back.

## Bundle variants

For preview flows with no disk I/O, skip `DiskFs` entirely and drive wasm directly:

```ts
function checkBundle(bundle: Bundle, virtualProjectDir?: string): Promise<CheckResponse>;
function validateSlotDataBundle(bundle: Bundle, slotData: SlotData, virtualProjectDir?: string): Promise<ValidationResponse>;
function generateBundle(
    bundle: Bundle,
    slotData: SlotData,
    opts?: { virtualProjectDir?: string; virtualOutDir?: string },
): Promise<GenerateResponse>;
function planHooksBundle(
    projectBundle: Bundle,
    virtualProjectDir: string,
    outDir: string,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
): Promise<PlanHooksResponse>;
```

`generateBundle` has no streaming-hooks counterpart — bundle-only mode doesn't have a real `cwd` for subprocess execution. Use the disk-backed `runHooksStream()` above when you need hooks.

Pair with `MemoryFs.toBundle()` / `MemoryFs.fromBundle()` to inspect results in-memory.

## Host helpers

### `DiskFs`

```ts
class DiskFs {
    constructor(opts: { workspaceRoot: string });
    readProject(projectDir: string, opts?: { virtualRoot?: string }): Bundle;
    writeOutput(
        outDir: string,
        input: Bundle | { files: Bundle; dirs?: string[] },
    ): void;
}
```

`writeOutput` accepts either a flat `Bundle` (files only — convenient for hand-rolled calls) or the `{ files, dirs }` shape returned by `generate`. Pass `{ files, dirs }` to preserve empty directories. Ancestor dirs for file writes are created automatically either way.

**`outDir` must not already exist.** `writeOutput` refuses a pre-existing `outDir` with an `already exists` error (parity with native `spackle generate`). Callers should pick a fresh path per run, or `rm -rf` the target before calling.

**Containment:** `workspaceRoot` is canonicalized once. Every path fed into `readProject` / `writeOutput` must resolve under it; anything else throws. Per-entry traversal is blocked using `path.resolve` + prefix comparison — rejects `../escape`, absolute overrides, and any OS-normalized escape.

### `MemoryFs`

```ts
class MemoryFs {
    constructor(seed?: { files?: Record<string, Uint8Array | string> });
    static fromBundle(bundle: Bundle, prefix?: string): MemoryFs;
    insertFile(path: string, content: Uint8Array | string): void;
    toBundle(): Bundle;
    get(path: string): Uint8Array | undefined;
    has(path: string): boolean;
    snapshot(): { files: Record<string, Uint8Array> };
}
```

Pure TS. No filesystem interaction. Useful for tests and preview flows.

## Known limitations

- **UTF-8 paths only.** The bundle boundary doesn't round-trip non-UTF-8 filenames.
- **Whole-project marshalling.** Input and output bundles materialize in memory. Fine for typical templates (KB–MB); very large fixtures should wait on a streaming path.
- **Browser-side hooks require a custom `SpackleHooks`.** `defaultHooks()` throws in environments without Bun or Node. See [hooks.md](./hooks.md).
- **`run_as_user` / polyjuice not exposed.** Native CLI can spawn hooks as another user; wasm path can't. Wrap it in a custom `SpackleHooks.execute` if needed.
