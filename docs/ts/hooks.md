# Hooks

Hooks are arbitrary shell commands declared in `spackle.toml` and run after generation. The TS package supports them end-to-end under the **plan-in-wasm, execute-host-side** model: a native-parity planner runs inside the wasm module (pure — no subprocess, no fs), TS spawns the resolved commands via a `SpackleHooks` executor. The package ships Node and Bun executors out of the box — `runHooks()` just works.

## Why the split

wasm has no subprocess APIs. In browsers, spawning a process is impossible; in Node/Bun it would require marshalling callbacks across the wasm boundary on every hook (the JsFs pattern we removed from earlier milestones). Keeping Rust pure and letting the host execute is simpler, faster, and portable to every JS runtime.

## Two-call shape (mirrors the native CLI)

The native CLI calls `project.generate(...)` then separately `project.run_hooks_stream(...)`. The TS package mirrors that:

```ts
import { DiskFs, generate, runHooks } from "@a2-ai/spackle";

const fs = new DiskFs({ workspaceRoot });

await generate(projectDir, outDir, slotData, fs);        // writes files
const hookResult = await runHooks(projectDir, outDir, data, fs);
//                   ^^^^^^^^                     ^^^^
//                   reads spackle.toml           full data map:
//                   planner + executor           slots + hook toggles
```

`data` is a single map, matching native's `Project::run_hooks_stream`. It carries slot values *and* hook toggles (keyed by the hook's own `key`, e.g. `data["format_code"] = "false"` disables the `format_code` hook). `_project_name` and `_output_name` are injected wasm-side — do not pre-inject them.

## Output shape

```ts
type RunHooksResponse =
  | { ok: true; results: HookRunResult[] }
  | { ok: false; error: string; templateErrors?: TemplateErrorDetail[] };

type HookRunResult =
  | { key; kind: "completed"; exitCode: 0; stdout; stderr }
  | { key; kind: "failed";    exitCode;     stdout; stderr; error? }
  | { key; kind: "skipped";   skipReason };
```

- **`completed`** — process exited 0.
- **`failed`** — non-zero exit, or the process couldn't launch. Subsequent hooks still run (see "Failure semantics" below).
- **`skipped`** — user-disabled, unmet `needs`, or the `if` conditional evaluated false. `skipReason` distinguishes.

`ok: false` at the top level means a template-error hard abort: one of the hook commands had an unresolvable `{{ ... }}` reference. No hooks were executed.

## Executors (`SpackleHooks`)

```ts
interface SpackleHooks {
  execute(
    command: string[],
    cwd: string,
    env?: Record<string, string>,
  ): Promise<HookExecuteResult>;
}
```

Shipped impls:

- **`BunHooks`** — wraps `Bun.spawn`.
- **`NodeHooks`** — wraps `child_process.spawn`.
- **`defaultHooks()`** — auto-selects `BunHooks` under Bun, `NodeHooks` under Node, throws otherwise.

`runHooks()` calls `defaultHooks()` when no `hooks` option is passed. Inject a custom executor for mocking, containerized execution, or browser-side scenarios:

```ts
await runHooks(projectDir, outDir, data, fs, { hooks: myExecutor });
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

- **Non-zero exit ≠ abort.** A failed hook yields a `failed` result; subsequent hooks still run. `hook_ran_<key>` stays `false`, so downstream `if = "{{ hook_ran_X }}"` conditionals naturally demote dependent hooks to `skipped`.
- **Template errors = hard abort.** An unresolved `{{ ... }}` reference is a `RunHooksResponse` `ok: false` — no executor call is ever made.
- **Chained conditionals re-plan after divergence.** When a hook's actual outcome differs from the best-case assumption, the remaining plan is re-evaluated against the updated `hook_ran` state. This keeps chained-conditional fidelity with native execution.

## Browser hosts

`defaultHooks()` throws in environments without Bun or Node (`no subprocess available in this environment — provide a custom SpackleHooks to runHooks()`). If a browser consumer needs hook-like behavior, they can ship a custom `SpackleHooks` that posts the command to a backend:

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

Each `planHooks()` / `runHooks()` call re-parses the bundle and rebuilds `MemoryFs`. At current scale (typical `spackle.toml` + small bundles) parse is sub-millisecond and dwarfed by subprocess spawn time, so the extra roundtrips are free in practice.

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
