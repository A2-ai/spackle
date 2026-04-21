# Hooks

**Status: unsupported in this milestone.**

Calling `generate(..., { runHooks: true })` currently returns:

```ts
{ ok: false, error: "hooks are unsupported in this milestone; call with run_hooks=false" }
```

This is an explicit refusal — not a silent no-op — so callers can observe the behavior and decide whether to degrade or fail.

## Why deferred

Hooks in spackle are arbitrary shell commands declared in `spackle.toml` and run after generation. The native CLI executes them via `std::process` (`async-process`). Under wasm, there's no process to spawn — subprocess creation has to cross the wasm boundary to a host-side executor.

That executor needs its own bridge (a future `JsHooks` analogue of the `SpackleFs` callback pattern from earlier design rounds). Designing and landing it is substantial work — threading plan evaluation, environment construction, exit-code routing, timeouts — and not critical for the initial `@a2-ai/spackle` release.

## What's in the repo today

- `spackle::hook::evaluate_hook_plan` — pure hook *planning* (expanding templates against slot data, resolving `if` conditionals, computing `needs`) lives in the core crate and is already callable from wasm if we expose it. No execution, no side effects.
- `ts/src/host/hooks.ts` — placeholder types (`SpackleHooks` interface, `HookResult`, `throwUnsupportedHooks()` helper). Reserved for when the bridge lands.

## What it will look like

Rough sketch:

```ts
// Future API (not yet implemented):
interface SpackleHooks {
    run(hook: Hook, env: Record<string, string>): Promise<HookResult>;
}

class NodeHooks implements SpackleHooks {
    async run(hook: Hook, env: Record<string, string>): Promise<HookResult> {
        // spawn + capture stdout/stderr/exit
    }
}

const result = await generate(projectDir, outDir, slotData, fs, {
    runHooks: true,
    hooks: new NodeHooks(),  // NEW
});
```

Rust will emit an evaluated hook plan (template-resolved commands, in need-order) and the host will iterate through calling `hooks.run(...)` for each. Bridge is thin by design: Rust does planning, host does execution.

Track progress in [`SUMMARY.md`](../../SUMMARY.md) at the repo root.
