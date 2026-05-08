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

Run the full pipeline: copy non-template files, render `.j2` files, render path placeholders, write everything under `outDir`. Each rendered entry is **streamed straight to disk** as Rust produces it — peak memory is bounded by one file, not by the whole rendered output. Hooks are a separate call — see `runHooksStream()` below.

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
): Promise<GenerateDiskResponse>;

type GenerateDiskResponse =
    | { ok: true; files: number; dirs: number }
    | { ok: false; error: string };
```

The success shape is **counts**, not a materialized bundle. The rendered tree lands directly under `outDir`; if you also need the bytes in memory (preview, in-process consumers), call `generateBundle` instead.

Output-dir contract: `generate` (via `DiskFs.prepareOutDir`) throws if `outDir` already exists, matching native's `GenerateError::AlreadyExists`. The check runs **before** the wasm call, so a pre-existing target fails fast with no Rust-side work.

Errors mid-stream are not rolled back: any files already written before the failure remain on disk (matches native CLI behavior). Pick a fresh `outDir` per run.

The input read (`fs.readProject(projectDir)`) is still eager — the project bundle is materialized in memory before the stream starts. Templates are typically small on input and large on output, so the streaming win is on the output side; lazy input reads are deferred to a later change.

> **Breaking change (vs. 0.5.0-rc3):** `generate` previously returned `{ ok: true; files: Bundle; dirs: string[] }` and host code consumed `result.files` / `result.dirs` to render UI. Migration: read the rendered tree from disk, or switch to `generateBundle` if you specifically need the bundle shape in memory.

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
function generateStream(
    bundle: Bundle,
    slotData: SlotData,
    opts?: { virtualProjectDir?: string; virtualOutDir?: string },
): AsyncGenerator<GenerateStreamEvent>;
function planHooksBundle(
    projectBundle: Bundle,
    virtualProjectDir: string,
    outDir: string,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
): Promise<PlanHooksResponse>;

type GenerateResponse =
    | { ok: true; files: Bundle; dirs: string[] }
    | { ok: false; error: string };

type GenerateStreamEvent =
    | { kind: "file"; path: string; bytes: Uint8Array }
    | { kind: "dir"; path: string }
    | { kind: "error"; error: string }
    | { kind: "done" };
```

Three modes for in-memory consumers:

- **`generateBundle`** — buffers every streamed entry into a `{ files, dirs }` bundle. Same shape and semantics as the pre-streaming API; suited to in-process preview where you want the whole rendered tree at once. **Does not reduce peak memory** — the buffer holds everything. Output is **deduped by path** (later writes replace earlier ones, matching disk-streaming's `writeFileSync` overwrite semantics — relevant when both `copy::copy` and `template::fill` produce the same output path) and **sorted by path** for deterministic ordering across runs.
- **`generateStream`** — yields each entry as an `AsyncGenerator` event with a terminal `done` / `error`. Use for progress UIs or anything driven off an async iterator. **Also does not reduce peak memory**: the wasm call is synchronous, so events accumulate in a queue while Rust runs and flush out after. The win here is API ergonomics, not memory.
- **`generate(projectDir, outDir, …)`** — the only path that genuinely bounds peak memory at one entry, because writes happen synchronously inside the host callback (see above).

Output bundle paths are **relative to `virtualOutDir`** (default `/output`). `dirs` exists so empty directories survive the round-trip — native `spackle generate` calls `create_dir_all` for every directory walked during the copy pass, including ones whose contents are fully ignored. Without emitting dir entries, a project whose `drafts/` directory is fully ignored would still have `drafts/` created on native but silently dropped under wasm.

`generateBundle` / `generateStream` have no streaming-hooks counterpart — bundle-only mode doesn't have a real `cwd` for subprocess execution. Use the disk-backed `runHooksStream()` above when you need hooks.

Pair with `MemoryFs.toBundle()` / `MemoryFs.fromBundle()` to inspect results in-memory.

## Host helpers

### `DiskFs`

```ts
class DiskFs {
    constructor(opts: { workspaceRoot: string });
    readProject(projectDir: string, opts?: { virtualRoot?: string }): Bundle;
    assertOutDirAvailable(outDir: string): string;
    prepareOutDir(outDir: string): string;
    ensureOutDir(outDir: string): string;
    writeEntry(outDir: string, entry: GenerateStreamEntry): void;
    writeOutput(
        outDir: string,
        input: Bundle | { files: Bundle; dirs?: string[] },
    ): void;
}

type GenerateStreamEntry =
    | { kind: "file"; path: string; bytes: Uint8Array }
    | { kind: "dir"; path: string };
```

Three write APIs at three layers:

- **`assertOutDirAvailable(outDir)`** — AlreadyExists + workspaceRoot containment check; returns the canonical path **without creating the directory**. Streaming `generate(...)` uses this so wasm validation failures (bad config, type-mismatched slot data, malformed bundle) leave no empty `outDir` on disk — `writeEntry`'s recursive parent-mkdir creates `outDir` lazily on the first event, matching native `Project::generate` which only creates the destination as part of `copy::copy`.
- **`prepareOutDir(outDir)`** — `assertOutDirAvailable` + eagerly creates the directory. Use this when you've already buffered the full output (`generateBundle` → `writeOutput`).
- **`ensureOutDir(outDir)`** — idempotent `mkdir -p` with workspaceRoot containment. Used by streaming `generate(...)` after a successful wasm call to handle the empty-project case (no events fire, but native still creates an empty `outDir`).
- **`writeEntry(outDir, entry)`** — sync sibling for the streaming-generate path: each `wasm.generate` callback synchronously routes a file or dir entry to disk. Re-validates `outDir` containment under `workspaceRoot` per call so external streaming consumers can't accidentally write outside the DiskFs root. Parent dirs for files are mkdir'd recursively (idempotent), which is also what creates `outDir` itself when `assertOutDirAvailable` was used in lieu of `prepareOutDir`.
- **`writeOutput(outDir, input)`** — convenience for callers that already buffered. Accepts either a flat `Bundle` (files only) or the `{ files, dirs }` shape returned by `generateBundle`. Internally calls `prepareOutDir` then loops `writeEntry`.

**`outDir` must not already exist.** Both `prepareOutDir` and `writeOutput` refuse a pre-existing `outDir` with an `already exists` error (parity with native `spackle generate`). Callers should pick a fresh path per run, or `rm -rf` the target before calling.

**Containment:** `workspaceRoot` is canonicalized once. Every path fed into `readProject` / `prepareOutDir` / `writeEntry` / `writeOutput` must resolve under it; anything else throws. Per-entry traversal is blocked using `path.resolve` + prefix comparison — rejects `../escape`, absolute overrides, and any OS-normalized escape.

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
- **Input bundle still materialized in memory.** `DiskFs.readProject` reads the whole project before generation begins. The streaming path bounds **output** memory at one entry, but a very large project tree still occupies its full size on the input side. A lazy-input change is deferred to a later PR; templates are typically small on input and large on output, so the output streaming wins back the dominant peak today.
- **`generateStream` does not reduce peak memory.** The wasm call is synchronous, so streamed entries accumulate in a queue and flush after the call returns. Use `generate(projectDir, outDir, ...)` for true streaming-to-disk; `generateStream` is for ergonomics only.
- **Browser-side hooks require a custom `SpackleHooks`.** `defaultHooks()` throws in environments without Bun or Node. See [hooks.md](./hooks.md).
- **`run_as_user` / polyjuice not exposed.** Native CLI can spawn hooks as another user; wasm path can't. Wrap it in a custom `SpackleHooks.execute` if needed.
