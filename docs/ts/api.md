# API reference

`@a2-ai/spackle` exposes six primary operations — `check`, `validateSlotData`, `generate`, `render`, `planHooks`, `runHooksStream`. The first three also have `…Bundle` pass-throughs (`checkBundle` / `validateSlotDataBundle` / `planHooksBundle`) for hosts that already have a bundle in memory and want to skip the disk read. `DiskFs` is the workspace boundary helper; `MemoryFs` is a pure-TS bundle holder for previews/tests; `SpackleHooks` (with shipped `NodeHooks` / `BunHooks` + `defaultHooks()`) executes hooks.

Internally `generate` and `render` are disk-walking orchestrators that call the **per-file wasm primitives** (`renderFile`, `renderPath`, `check`, `validateSlotData`, `planHooks`) as they go. Static asset bytes never enter wasm — `generate` streams them disk-direct via `pipeline(createReadStream, createWriteStream)`. There's only one `generate` path (disk-direct); browser hosts / custom-source hosts that need a bundle-input orchestrator compose the wasm primitives themselves via `loadSpackleWasm`.

## `check` vs `render` vs `generate` — when to use what

| Function | Slot data required? | Fail-fast? | Returns | Use case |
| --- | --- | --- | --- | --- |
| `check` | No | No (collects all) | `{ config, diagnostics[] }` | Live diagnostics while the publisher edits files; CLI `spackle check`. |
| `render` | Yes | No (collects all, partial preview) | `{ files, dirs, diagnostics[], hookPlan }` | Live preview as the publisher fills slot values; never throws. |
| `generate` | Yes | Yes (aborts on first error) | `{ ok: true; files: number; dirs: number } \| { ok: false; error }` | Production write-to-disk workflows. Streams each entry to disk; returns counts only. |

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
    path?: string;        // bundle-virtual or workspace-relative path
    ref?: string;         // slot/hook key when the diagnostic targets a config item
    span?: { line: number; column: number };
    code?: string;        // stable id (e.g. "hook::template_render_failed")
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

Under the hood, `check` builds a small bundle from disk containing `spackle.toml` (real bytes), every `.j2`/`.tera` template (real bytes for body validation), and a path-only placeholder (empty `bytes`) for every other file (for path-template validation). Static asset bytes never enter wasm.

`checkBundle(bundle, virtualProjectDir?)` is the bundle-only equivalent — useful when you've already built a bundle in memory (preview flows, browser hosts).

## `render(projectDir, outDir, slotData, fs, opts?)`

Dynamic render — walks the project on disk and produces a `(possibly partial) Bundle` plus an exhaustive `diagnostics` array. Per-file failures surface as diagnostics without aborting the walk; the host UI shows the preview pane and the diagnostics chip side-by-side. **Never throws / never returns `ok: false`**.

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

Partial preview semantics: `files` contains every template that rendered successfully even when other files failed. `hookPlan` is `null` only when the config didn't load.

Use `render` for live UI previews; use `generate` (below) when you want fail-fast behavior for writing to disk.

No bundle-input variant — `render` walks disk. Compose the wasm primitives directly if you need an in-memory preview.

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

Run the full pipeline: walk `projectDir`, render `.j2`/`.tera` template bodies via wasm, render path placeholders via wasm, stream-copy static files via `pipeline(createReadStream, createWriteStream)`, write everything under `outDir` as the walk progresses.

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
    | { ok: true; files: number; dirs: number }
    | { ok: false; error: string };
```

`result.files` / `result.dirs` are **counts**, not lists — the rendered tree is already on disk under `outDir` by the time the promise resolves. If you need the rendered output in memory, call `render` (diagnostics-first preview that returns a `Bundle`) or read the output back from disk after the call.

Output-dir contract: throws if `outDir` already exists, matching native's `GenerateError::AlreadyExists`. The check happens **before** any wasm work, but `outDir` itself is created lazily on the first per-entry write — so a wasm-side failure (slot data type mismatch, bad bundle) leaves no empty directory on disk.

Empty-directory parity: native `spackle generate` calls `create_dir_all` for every non-ignored directory it walks during the copy pass, so empty source directories (no children, but not in `ignore`) survive into the output. `generate` mirrors this — directories are mkdir'd as the walk yields them. Ignored directories are skipped entirely and do **not** appear in the output (unless a template inside them renders, in which case the directory is created as a side effect of writing that template — native parity with `template::fill`, which walks the full tree regardless of `ignore`). An `ensureOutDir` call at the end makes empty projects produce an empty `outDir`.

**Memory.** Static files stream-copy through `pipeline(createReadStream, createWriteStream)` — peak memory per static is one ~64 KiB Node `highWaterMark` chunk regardless of file size. GB-scale assets are fine. Templated bodies still buffer fully in memory (Tera produces a `String`), but typical templates are KB-scale.

**Template semantics.** `renderFile` builds a Tera instance per call from a template-source registry — every `.j2` / `.tera` body the host walked — and renders only the requested target. Tera 2's cross-template tags (`{% include %}` and `{% extends %}`) resolve across the project. Tera 2 does not support `{% macro %}` / `{% import %}`. Static assets never enter the registry; only template bodies cross the wasm boundary.

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

Run the project's hooks, yielding `HookEvent`s as each hook progresses. Mirrors the native CLI's two-call shape: call after `generate()`. Reads `spackle.toml` into a tiny bundle, plans via wasm, executes each hook host-side via the injected or auto-selected `SpackleHooks`, and re-plans internally after any non-zero exit so chained conditionals re-evaluate against actual outcomes.

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

## Bundle pass-throughs

For hosts that already have a bundle in memory (browsers, S3/git-backed sources, tests), three thin pass-throughs hand the bundle directly to the wasm exports without touching disk:

```ts
function checkBundle(bundle: Bundle, virtualProjectDir?: string): Promise<CheckResponse>;
function validateSlotDataBundle(bundle: Bundle, slotData: SlotData, virtualProjectDir?: string): Promise<ValidationResponse>;
function planHooksBundle(
    projectBundle: Bundle,
    virtualProjectDir: string,
    outDir: string,
    data: Record<string, string>,
    hookRan?: Record<string, boolean>,
): Promise<PlanHooksResponse>;
```

No bundle-input variant of `generate` / `render` ships — those are disk-walking orchestrators. Browser / custom-source hosts that need bundle-input generation compose the per-file primitives themselves: read the bundle, call `wasm.renderFile` / `wasm.renderPath` per entry via `loadSpackleWasm()`, and assemble results in whatever shape suits them.

Pair with `MemoryFs.toBundle()` / `MemoryFs.fromBundle()` to inspect inputs / outputs in-memory.

## Host helpers

### `DiskFs`

```ts
class DiskFs {
    constructor(opts: { workspaceRoot: string });
    readonly workspaceRoot: string;

    // Containment: every `projectDir` / `outDir` argument passed to the
    // orchestrator goes through one of these.
    containProject(projectDir: string): string;
    assertOutDirAvailable(outDir: string): string;  // contains + AlreadyExists, no mkdir
    ensureOutDir(outDir: string): string;           // idempotent mkdir -p
    containedJoin(absBase: string, rel: string): string;

    // Per-file I/O the orchestrator drives directly.
    readFile(absPath: string): Uint8Array;
    writeFile(absPath: string, bytes: Uint8Array): void;
    streamCopy(srcAbs: string, dstAbs: string): Promise<void>;  // via pipeline()
    exists(absPath: string): boolean;
}
```

**Containment.** `workspaceRoot` is canonicalized once. Every path the orchestrator hands back to `DiskFs` resolves under it; anything else throws. Per-entry write paths are joined with `containedJoin` so a rendered path can't escape `outDir` even if a template produced `../`-flavored output. The block uses `path.resolve` + prefix comparison so OS-normalized escapes (Windows `..\`) are caught transparently.

**Streaming.** `streamCopy` uses `pipeline(createReadStream, createWriteStream)`. GB-scale static assets cross the pipe in Node's ~64 KiB chunks; never sit fully in memory. `readFileSync(src) + writeFileSync(dst)` would slurp the whole file into a `Buffer` per copy — never use that pattern in custom orchestrators.

**`outDir` must not already exist** for `assertOutDirAvailable` — same `already exists` error as native `spackle generate`. `ensureOutDir` is the idempotent variant; `generate` calls it after a successful run so empty projects still produce an empty `outDir`.

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

Pure TS. No filesystem interaction. Useful for tests, custom-source preview flows, and snapshotting `render` output.

## Known limitations

- **UTF-8 paths only.** Path strings cross the wasm boundary as UTF-8; non-UTF-8 filenames don't round-trip.
- **Template render is buffered.** Spackle core's `render_in_memory` produces a `String` per template. Static copies stream cleanly via `pipeline()`; templated bodies remain a ceiling proportional to the template size. Typical templates are KB-scale, so this rarely binds in practice.
- **Browser-side hooks require a custom `SpackleHooks`.** `defaultHooks()` throws in environments without Bun or Node. See [hooks.md](./hooks.md).
- **`run_as_user` / polyjuice not exposed.** Native CLI can spawn hooks as another user; wasm path can't. Wrap it in a custom `SpackleHooks.execute` if needed.
