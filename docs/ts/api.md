# API reference

`@a2-ai/spackle` exposes six primary operations — `check`, `validateSlotData`, `generate`, `render`, `planHooks`, `runHooksStream` — plus `…Bundle` variants that skip disk I/O for the first four. `generate` additionally has a `generateStream` async-generator sibling for progress UIs. Two host helpers (`DiskFs`, `MemoryFs`) handle moving data in and out; a `SpackleHooks` interface (with shipped `NodeHooks` / `BunHooks` impls + `defaultHooks()` auto-selector) handles subprocess execution for hooks.

## `check` vs `render` vs `generate` — when to use what

| Function | Slot data required? | Fail-fast? | Returns | Use case |
| --- | --- | --- | --- | --- |
| `check` | No | No (collects all) | `{ config, diagnostics[] }` | Live diagnostics while the publisher edits files; CLI `spackle check`. |
| `render` | Yes | No (collects all, partial preview) | `{ files, dirs, diagnostics[], hookPlan }` | Live preview as the publisher fills slot values; never throws. |
| `generate` | Yes | Yes (aborts on first error) | `{ ok: true; files: number; dirs: number } \| { ok: false; error }` | Production write-to-disk workflows. Streams each entry to disk through `DiskFs`; returns counts only. |

`check` and `render` share the same `Diagnostic` type — one UI rendering path covers both surfaces.

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

## `Diagnostic`

`check` and `render` return structured diagnostics with the same shape:

```ts
interface Diagnostic {
    severity: "error" | "warning";
    source:
        | "config"        // spackle.toml parse / structural error
        | "slot_config"   // slot config issue (duplicate key, bad default type, …)
        | "hook_config"   // hook config issue (unknown `needs`, bad command/if template)
        | "slot_data"     // user-supplied slot data is missing / wrong type
        | "copy"          // copy stage failure
        | "render_body"   // template body render failure
        | "render_name";  // template filename render failure
    message: string;
    path?: string;        // bundle-virtual path of offending file, or "spackle.toml"
    ref?: string;         // slot/hook key when the diagnostic targets a config item
    span?: { line: number; column: number };
    code?: string;        // stable id (e.g. "hook::unknown_needs")
}
```

Severity is `error` for everything in v1; `warning` is reserved for future use (deprecated patterns, dead slots).

## `check(projectDir, fs, opts?)`

Run every static project check — `spackle.toml` parse + structural validation, slot config, hook config (including `needs` reference resolution and command-template parsing), template syntax + slot reference resolution. **Does NOT need slot data** — call it continuously as the publisher edits files.

```ts
function check(
    projectDir: string,
    fs: DiskFs,
    opts?: { virtualProjectDir?: string },
): Promise<CheckResponse>;

interface CheckResponse {
    config: SpackleConfig | null;
    diagnostics: Diagnostic[];
}
```

`check` **never throws / never returns `valid: false`** — it always returns the response. Empty `diagnostics` ⇒ the project is structurally sound. `config` is `null` only when `spackle.toml` failed to parse (a `config`-source diagnostic explains why).

`checkBundle(bundle, virtualProjectDir?)` is the bundle-only equivalent.

## `render(projectDir, outDir, slotData, fs, opts?)`

Dynamic render — runs the full pipeline (`check` → slot data validation → copy → template fill → hook plan) in-memory under `fs` and produces a (possibly partial) bundle plus an exhaustive `diagnostics` array. **Never throws / never returns `ok: false`**.

```ts
function render(
    projectDir: string,
    outDir: string,
    slotData: SlotData,
    fs: DiskFs,
    opts?: {
        virtualProjectDir?: string;
        virtualOutDir?: string;
    },
): Promise<RenderResponse>;

interface RenderResponse {
    files: Bundle;
    dirs: string[];
    diagnostics: Diagnostic[];
    hookPlan: HookPlanEntry[] | null;
}
```

Partial preview semantics: `files` contains every template that rendered successfully even when other files failed. The UI shows the preview pane and the diagnostics chip side-by-side without branching on success/failure. `hookPlan` is `null` only when the config didn't load.

Use `render` for live UI previews; use `generate` (below) when you want fail-fast behavior for writing to disk.

`renderBundle(bundle, slotData, opts?)` is the bundle-only equivalent.

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

Run the full pipeline: copy non-template files, render `.j2` files, render path placeholders, **stream each entry to disk through `DiskFs.writeEntry`** as Rust produces it. Hooks are a separate call — see `runHooksStream()` below.

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

`result.files` / `result.dirs` are **counts**, not lists — the rendered tree is already on disk under `outDir` by the time the promise resolves. If you need the rendered output in memory (preview, snapshot, in-process consumer), call [`generateBundle`](#bundle-variants) instead.

Output-dir contract: throws if `outDir` already exists, matching native's `GenerateError::AlreadyExists`. The check happens **before** any wasm work, but `outDir` itself is created lazily on the first streamed entry — so a wasm-side failure (slot data type mismatch, bad bundle) leaves no empty directory on disk.

Empty-directory parity: native `spackle generate` calls `create_dir_all` for every directory walked during the copy pass, so a project whose `drafts/` dir is fully ignored still produces an empty `drafts/` on disk. `generate` mirrors this — directory entries arrive as `{ kind: "dir", path }` events, and `DiskFs.writeEntry` mkdirs them. Plus an `ensureOutDir` call at the end so empty projects produce an empty `outDir`.

**Memory caveat.** Output bytes are dropped per-event after `DiskFs.writeEntry` returns, so the host never holds a duplicate copy of the rendered bundle. However, spackle core's template stage (`template::fill`) renders all `.j2` files into a `Vec<RenderedFile>` before the per-file write loop — so peak heap during the template stage is the sum of all rendered template bytes plus the input bundle, not one entry. The copy stage *does* stream one entry at a time. Large template bodies remain a ceiling until core grows a streaming render pass.

### `generateStream(projectBundle, slotData, opts?)`

Async-generator variant for progress UIs / preview flows that want each entry as an event:

```ts
function generateStream(
    projectBundle: Bundle,
    slotData: SlotData,
    opts?: { virtualProjectDir?: string; virtualOutDir?: string },
): AsyncGenerator<GenerateStreamEvent>;

type GenerateStreamFileEvent = { kind: "file"; path: string; bytes: Uint8Array };
type GenerateStreamDirEvent  = { kind: "dir";  path: string };
type GenerateStreamEvent =
    | GenerateStreamFileEvent
    | GenerateStreamDirEvent
    | { kind: "error"; error: string }
    | { kind: "done" };
```

Paths are **relative to `outDir`**. Dirs arrive root-to-leaf and before any child file (parent-before-child). Iteration ends with `{ kind: "done" }` on success or `{ kind: "error", error }` on failure.

**Ergonomics, not memory.** The wasm call is synchronous, so the host callback runs to completion before the generator yields — entries pile up in an internal queue, then drain. Use this for progress UIs; use `generate(projectDir, outDir, …)` for true streaming-to-disk.

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

type GenerateResponse =
    | { ok: true; files: Bundle; dirs: string[] }
    | { ok: false; error: string };
function planHooksBundle(
    projectBundle: Bundle,
    virtualProjectDir: string,
    outDir: string,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
): Promise<PlanHooksResponse>;
```

`generateBundle` is internally driven by the same streaming wasm callback as `generate`, but collects file/dir events into the legacy `{ files, dirs }` shape. Overlapping copy + template paths (e.g. a project with `foo` and `foo.j2` both rendering to `foo`) are deduped — last-write wins, matching native disk-write semantics. Files and dirs are sorted by path so consumers see deterministic order.

Note: `generateBundle` materializes the full output in memory — peak host memory is the rendered bundle plus the same template-stage buffer that affects `generate`. Use `generate` if you're writing to disk anyway.

`generateBundle` has no streaming-hooks counterpart — bundle-only mode doesn't have a real `cwd` for subprocess execution. Use the disk-backed `runHooksStream()` above when you need hooks.

Pair with `MemoryFs.toBundle()` / `MemoryFs.fromBundle()` to inspect results in-memory.

## Host helpers

### `DiskFs`

```ts
class DiskFs {
    constructor(opts: { workspaceRoot: string });
    readProject(projectDir: string, opts?: { virtualRoot?: string }): Bundle;

    // Streaming-generate surface — `generate(...)` uses these under the hood,
    // but they're exported so custom drivers can stream entries from any source.
    assertOutDirAvailable(outDir: string): string;   // contains + AlreadyExists check, no mkdir
    prepareOutDir(outDir: string): string;           // assertOutDirAvailable + mkdir
    ensureOutDir(outDir: string): string;            // idempotent mkdir -p (no AlreadyExists throw)
    writeEntry(outDir: string, entry: GenerateStreamEntry): void;

    // Buffered-bundle surface — convenience for callers that already have a
    // full bundle in memory (e.g., generateBundle / render result).
    writeOutput(
        outDir: string,
        input: Bundle | { files: Bundle; dirs?: string[] },
    ): void;
}
```

**Streaming vs buffered**: `generate()` uses `assertOutDirAvailable` + a loop of `writeEntry` calls so failures before the first event don't leave an empty `outDir` on disk. `writeOutput` uses `prepareOutDir` and writes everything at once — fine for `generateBundle` / `render` outputs that are already in memory.

`writeOutput` accepts either a flat `Bundle` (files only — convenient for hand-rolled calls) or the `{ files, dirs }` shape returned by `generateBundle` / `render`. Pass `{ files, dirs }` to preserve empty directories. Ancestor dirs for file writes are created automatically either way.

**`outDir` must not already exist** for `assertOutDirAvailable` / `prepareOutDir` / `writeOutput` — same `already exists` error (parity with native `spackle generate`). `ensureOutDir` is the idempotent variant; `generate` calls it after a successful run so empty projects produce an empty `outDir`. Callers should pick a fresh `outDir` per run, or `rm -rf` the target before calling.

**Containment:** `workspaceRoot` is canonicalized once. Every path fed into `readProject` / `writeOutput` / `writeEntry` must resolve under it; anything else throws. Per-entry traversal is blocked using `path.resolve` + prefix comparison — rejects `../escape`, absolute overrides, and any OS-normalized escape.

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
- **Input bundle is buffered.** Project files are read into a single bundle before the wasm call. Fine for typical templates (KB–MB); very large fixtures should wait on an input-side streaming path.
- **Template render pass is buffered.** Spackle core's `template::fill` renders all `.j2` files into a `Vec<RenderedFile>` before writing. The `generate` streaming path drops bytes per entry on the *write* side, but the *render* side still spikes proportional to total rendered template bytes. Static copies stream cleanly.
- **Browser-side hooks require a custom `SpackleHooks`.** `defaultHooks()` throws in environments without Bun or Node. See [hooks.md](./hooks.md).
- **`run_as_user` / polyjuice not exposed.** Native CLI can spawn hooks as another user; wasm path can't. Wrap it in a custom `SpackleHooks.execute` if needed.
