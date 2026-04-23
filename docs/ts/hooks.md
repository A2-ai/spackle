# Hooks

Hooks are arbitrary shell commands declared in `spackle.toml` and run after generation. The TS package supports them end-to-end under the **plan-in-wasm, execute-host-side** model: a native-parity planner runs inside the wasm module (pure — no subprocess, no fs), TS spawns the resolved commands via a `SpackleHooks` executor. The package ships Node and Bun executors out of the box — iterate `runHooksStream()` and events flow as each hook progresses.

## Why the split

wasm has no subprocess APIs. In browsers, spawning a process is impossible; in Node/Bun it would require marshalling callbacks across the wasm boundary on every hook (the JsFs pattern we removed from earlier milestones). Keeping Rust pure and letting the host execute is simpler, faster, and portable to every JS runtime.

## Two-call shape (mirrors the native CLI)

The native CLI calls `project.generate(...)` then separately `project.run_hooks_stream(...)`. The TS package mirrors that:

```ts
import { DiskFs, generate, runHooksStream } from "@a2-ai/spackle";

const fs = new DiskFs({ workspaceRoot });

await generate(projectDir, outDir, slotData, fs);    // writes files
for await (const event of runHooksStream(projectDir, outDir, data, fs)) {
  // event is a HookEvent — see below. Drive your UI / SSE frames here.
}
```

`data` is a single map, matching native's `Project::run_hooks_stream`. It carries slot values *and* hook toggles (keyed by the hook's own `key`, e.g. `data["format_code"] = "false"` disables the `format_code` hook). `_project_name` and `_output_name` are injected wasm-side — do not pre-inject them.

## Event shape

```ts
type HookEvent =
  | { type: "run_start"; plan: HookPlanEntry[] }
  | { type: "hook_start"; key; command: string[]; startedAt: number }
  | { type: "hook_end"; key; result: HookRunResult;
      startedAt?; finishedAt?; durationMs? }
  | { type: "replan"; afterKey: string; plan: HookPlanEntry[] }
  | { type: "template_errors"; error: string; templateErrors: TemplateErrorDetail[] }
  | { type: "plan_error"; error: string };

type HookRunResult =
  | { key; kind: "completed"; exitCode: 0; stdout; stderr }
  | { key; kind: "failed";    exitCode;     stdout; stderr; error? }
  | { key; kind: "skipped";   skipReason };
```

Guarantees:

- `run_start` is the first event whenever the initial plan is clean. It carries the full initial plan so UIs can paint skeleton rows.
- Each runnable hook emits `hook_start` then exactly one `hook_end`. Skipped hooks (and conditional-error hooks, which surface as `kind: "failed"` for native parity) emit only `hook_end` — no subprocess ran, so no start event.
- `hook_end.durationMs` (and `startedAt` / `finishedAt`) are present for hooks that actually ran; absent for skipped / conditional-error hooks.
- `replan` fires after a failed hook whenever the remaining plan was re-evaluated against mutated `hook_ran` state. Consumers can reconcile their row list (e.g. demote a dependent from pending → skipped) without waiting for that dependent's own `hook_end`.
- `template_errors` and `plan_error` are **terminal** — the iterator ends immediately after yielding them. No throws for expected failure modes.

Per-hook `result` kinds:

- **`completed`** — process exited 0.
- **`failed`** — non-zero exit, the process couldn't launch, or the `if` conditional failed to evaluate. Subsequent hooks still run (see "Failure semantics" below).
- **`skipped`** — user-disabled, unmet `needs`, or the `if` conditional evaluated false. `skipReason` distinguishes.

## Bridging to SSE

The async generator maps one-to-one onto a server-sent event stream. Bun / Node example:

```ts
export function handleHookRun(req: Request): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream({
    async start(controller) {
      for await (const event of runHooksStream(projectDir, outDir, data, fs)) {
        controller.enqueue(encoder.encode(`data: ${serializeHookEvent(event)}\n\n`));
      }
      controller.close();
    },
  });
  return new Response(body, {
    headers: { "content-type": "text/event-stream", "cache-control": "no-cache" },
  });
}
```

**Serialization caveat:** `stdout` / `stderr` on `hook_end.result` are `Uint8Array`. Plain `JSON.stringify` turns them into `{"0": 72, …}` which is wasteful and hard to decode client-side. Write a small `serializeHookEvent(ev)` that base64-encodes (or hex-encodes) any byte arrays before stringifying, and a matching decoder on the client. The plan events (`HookPlanEntry[]` in `run_start` / `replan`) are already pure JSON — no special handling needed.

## Executors (`SpackleHooks`)

```ts
interface SpackleHooks {
  execute(
    command: string[] | string,
    cwd: string,
    env?: Record<string, string>,
  ): Promise<HookExecuteResult>;
}
```

If you accept text commands from users, use the exported helpers to stay
consistent with the runner's argv contract:

```ts
import { formatArgv, parseShellLine } from "@a2-ai/spackle";

const argv = parseShellLine(`echo "hello world"`); // ["echo", "hello world"]
const shellText = formatArgv(argv); // `echo 'hello world'`
```

Shipped impls:

- **`BunHooks`** — wraps `Bun.spawn`.
- **`NodeHooks`** — wraps `child_process.spawn`.
- **`defaultHooks()`** — auto-selects `BunHooks` under Bun, `NodeHooks` under Node, throws otherwise.

`runHooksStream()` calls `defaultHooks()` when no `hooks` option is passed. Inject a custom executor for mocking, containerized execution, or browser-side scenarios:

```ts
for await (const event of runHooksStream(projectDir, outDir, data, fs, { hooks: myExecutor })) {
  // ...
}
```

## `planHooks()` — inspect without executing

For UIs that want to show "here's what will run":

```ts
import { planHooks } from "@a2-ai/spackle";

const res = await planHooks(projectDir, outDir, data, fs);
if (res.ok) {
  for (const entry of res.plan) {
    console.log(entry.key, entry.should_run, entry.skip_reason);
  }
}
```

Returns the templated commands, `should_run` flags, and skip reasons. Pure — no side effects.

## Failure semantics (native parity)

Matches `run_hooks_stream` in the core crate:

- **Non-zero exit ≠ abort.** A failed hook emits `hook_end` with `kind: "failed"`; subsequent hooks still run. `hook_ran_<key>` stays `false`, so downstream `if = "{{ hook_ran_X }}"` conditionals naturally demote dependent hooks to `skipped`.
- **Template errors = hard abort.** An unresolved `{{ ... }}` reference is a terminal `template_errors` event — no executor call is ever made, and the iterator ends immediately.
- **Chained conditionals re-plan after divergence.** When a hook's actual outcome differs from the best-case assumption, the remaining plan is re-evaluated against the updated `hook_ran` state and a `replan` event is yielded so consumers can reconcile. Preserves chained-conditional fidelity with native execution.

## Browser hosts

`defaultHooks()` throws in environments without Bun or Node (`no subprocess available in this environment — provide a custom SpackleHooks to runHooksStream()`). If a browser consumer needs hook-like behavior, they can ship a custom `SpackleHooks` that posts the command to a backend:

```ts
class RemoteHooks implements SpackleHooks {
  async execute(cmd, cwd) {
    const r = await fetch("/api/run-hook", { method: "POST", body: JSON.stringify({ cmd, cwd }) });
    const { exitCode, stdout, stderr } = await r.json();
    return { ok: exitCode === 0, exitCode, stdout: new Uint8Array(stdout), stderr: new Uint8Array(stderr) };
  }
}
```

## Future work — stateful session (deferred)

Each `planHooks()` / `runHooksStream()` call re-parses the bundle and rebuilds `MemoryFs`. At current scale (typical `spackle.toml` + small bundles) parse is sub-millisecond and dwarfed by subprocess spawn time, so the extra roundtrips are free in practice.

If profiles ever show per-call parse dominating — for interactive workflows or multi-generation-per-process hosts — a stateful session API becomes worthwhile:

```ts
// Sketch (not implemented):
const session = await openSession(bundle, projectDir);
const plan = session.planHooks(data);
// ... run plan ...
session.close();
```

Until that signal shows up, the pure-function model is the right default.

## Not in scope

- **`run_as_user`.** The native CLI can spawn hooks as a different user via `polyjuice`. The wasm path doesn't expose this — wrap your own `SpackleHooks.execute` if you need it.
- **`generateBundle` + hooks.** Bundle-only (MemoryFs) generation has no real `cwd`; hooks are disk-scoped by design.
