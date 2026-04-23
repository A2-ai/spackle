// End-to-end tests for the hooks pipeline — plan_hooks (wasm) +
// runHooksStream / runHookPlanStream (host-side executor). Exercises
// the default runner (BunHooks under Bun) against the hooks_fixture,
// plus mock-executor cases for the native parity semantics:
// continue-on-failure, chained-conditional re-plan, template-error
// hard abort. Additionally covers the streaming event protocol:
// run_start ordering, hook_start/hook_end pairing, replan emission,
// timing fields.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  BunHooks,
  DiskFs,
  NodeHooks,
  defaultHooks,
  formatArgv,
  parseShellLine,
  planHooks,
  runHookPlanStream,
  runHooksStream,
  type HookEvent,
  type HookExecuteResult,
  type HookRunResult,
  type PlanHooksResponse,
  type SpackleHooks,
} from "../src/spackle.ts";

const FIXTURES = resolve(import.meta.dir, "..", "..", "tests", "fixtures");

async function workspace(fixture: string) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-hooks-")));
  const { cp } = await import("node:fs/promises");
  const projectDir = join(root, "project");
  await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
  const outDir = join(root, "output");
  return { root, projectDir, outDir };
}

/** Drain an AsyncIterable<HookEvent> into an array. Used across tests
 * that only care about the final event set, not real-time ordering. */
async function drain(stream: AsyncIterable<HookEvent>): Promise<HookEvent[]> {
  const events: HookEvent[] = [];
  for await (const e of stream) events.push(e);
  return events;
}

/** Pull the per-hook outcomes out of a drained event list. Preserves
 * emission order. */
function resultsOf(events: HookEvent[]): HookRunResult[] {
  const out: HookRunResult[] = [];
  for (const e of events) if (e.type === "hook_end") out.push(e.result);
  return out;
}

/** Record each execute() call, return a scripted outcome per invocation.
 * Scripts are matched by hook command[0] — e.g. `{ sh: () => ok() }`
 * returns a 0 exit for every call whose first arg is "sh". Missing
 * scripts default to exit 0 with empty buffers. */
class MockHooks implements SpackleHooks {
  readonly calls: Array<{ command: string[]; cwd: string }> = [];
  constructor(
    private readonly script: (cmd: string[]) => Partial<HookExecuteResult> = () => ({}),
  ) {}
  async execute(command: string[] | string, cwd: string): Promise<HookExecuteResult> {
    const argv = typeof command === "string" ? parseShellLine(command) : command;
    this.calls.push({ command: argv, cwd });
    const override = this.script(argv);
    return {
      ok: override.ok ?? (override.exitCode ?? 0) === 0,
      exitCode: override.exitCode ?? 0,
      stdout: override.stdout ?? new Uint8Array(),
      stderr: override.stderr ?? new Uint8Array(),
    };
  }
}

describe("planHooks", () => {
  const cleanup: string[] = [];
  beforeEach(() => void (cleanup.length = 0));
  afterEach(async () => {
    await Promise.all(cleanup.map((p) => rm(p, { recursive: true, force: true })));
  });

  test("best-case plan returns all hooks runnable", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    const res = await planHooks(ws.projectDir, ws.outDir, {}, fs);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.plan.map((e) => e.key)).toEqual(["hook_a", "hook_b", "hook_names"]);
      for (const e of res.plan) expect(e.should_run).toBe(true);
    }
  });

  test("hookRan override demotes chained hook", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    const res = await planHooks(ws.projectDir, ws.outDir, {}, fs, {
      hookRan: { hook_a: false },
    });
    expect(res.ok).toBe(true);
    if (res.ok) {
      // hook_a is filtered out (already "executed"); hook_b demoted.
      const hookB = res.plan.find((e) => e.key === "hook_b");
      expect(hookB).toBeDefined();
      expect(hookB?.should_run).toBe(false);
      expect(hookB?.skip_reason).toBe("false_conditional");
    }
  });

  test("raw-key hook toggle disables a hook (Hook::is_enabled parity)", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    const res = await planHooks(ws.projectDir, ws.outDir, { hook_a: "false" }, fs);
    expect(res.ok).toBe(true);
    if (res.ok) {
      const hookA = res.plan.find((e) => e.key === "hook_a");
      expect(hookA?.should_run).toBe(false);
      expect(hookA?.skip_reason).toBe("user_disabled");
    }
  });

  test("_project_name / _output_name injected into templated command", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    // Pass a specific outDir so we can assert against its basename.
    const outDir = join(ws.root, "my-output-dir");
    const res = await planHooks(ws.projectDir, outDir, {}, fs);
    expect(res.ok).toBe(true);
    if (res.ok) {
      const names = res.plan.find((e) => e.key === "hook_names");
      expect(names).toBeDefined();
      const body = names?.command[2];
      expect(body).toBeDefined();
      // _project_name comes from spackle.toml `name = "hooks-demo"`.
      expect(body).toContain("hooks-demo");
      // _output_name is the outDir file_name.
      expect(body).toContain("my-output-dir");
    }
  });
});

describe("runHooksStream (default runner — actually spawns)", () => {
  const cleanup: string[] = [];
  beforeEach(() => void (cleanup.length = 0));
  afterEach(async () => {
    await Promise.all(cleanup.map((p) => rm(p, { recursive: true, force: true })));
  });

  test("spawns processes and hook side effects land on disk", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    // Hooks cwd defaults to outDir, which must exist before spawn.
    const { mkdir } = await import("node:fs/promises");
    await mkdir(ws.outDir, { recursive: true });

    const events = await drain(runHooksStream(ws.projectDir, ws.outDir, {}, fs));
    const results = resultsOf(events);
    expect(results.map((r) => r.kind)).toEqual(["completed", "completed", "completed"]);

    // hook_a wrote "hook_a" to hook_a.out inside cwd (=outDir).
    const aOut = await readFile(join(ws.outDir, "hook_a.out"), "utf8");
    expect(aOut).toBe("hook_a");
    const bOut = await readFile(join(ws.outDir, "hook_b.out"), "utf8");
    expect(bOut).toBe("hook_b");
    // hook_names wrote "<project>/<output>" to names.out.
    const names = await readFile(join(ws.outDir, "names.out"), "utf8");
    expect(names).toContain("hooks-demo");
    expect(names).toContain("output"); // basename of ws.outDir
  });

  test("emits run_start, hook_start/hook_end pairs with timing fields", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    const { mkdir } = await import("node:fs/promises");
    await mkdir(ws.outDir, { recursive: true });

    const events = await drain(runHooksStream(ws.projectDir, ws.outDir, {}, fs));

    // First event must be run_start with the initial plan.
    expect(events[0]?.type).toBe("run_start");
    if (events[0]?.type === "run_start") {
      expect(events[0].plan.map((e) => e.key)).toEqual(["hook_a", "hook_b", "hook_names"]);
    }

    // Every runnable hook emits hook_start immediately before hook_end.
    // All three hooks in this fixture run successfully, so we expect
    // three matched pairs following run_start.
    const body = events.slice(1);
    expect(body.length).toBe(6);
    for (let i = 0; i < body.length; i += 2) {
      const start = body[i];
      const end = body[i + 1];
      expect(start?.type).toBe("hook_start");
      expect(end?.type).toBe("hook_end");
      if (start?.type === "hook_start" && end?.type === "hook_end") {
        expect(start.key).toBe(end.key);
        expect(typeof start.startedAt).toBe("number");
        expect(end.startedAt).toBe(start.startedAt);
        expect(typeof end.finishedAt).toBe("number");
        expect(end.durationMs).toBeGreaterThanOrEqual(0);
        expect(end.finishedAt ?? 0).toBeGreaterThanOrEqual(start.startedAt);
      }
    }
  });
});

describe("defaultHooks()", () => {
  test("returns BunHooks under Bun", () => {
    // Tests run under bun, so defaultHooks() picks BunHooks with no
    // env override.
    expect(defaultHooks()).toBeInstanceOf(BunHooks);
  });

  test("falls back to NodeHooks when Bun is absent but Node is present", () => {
    // Simulate a non-Bun Node environment by passing an explicit env.
    expect(defaultHooks({ hasBun: false, hasNode: true })).toBeInstanceOf(NodeHooks);
  });

  test("throws in browser-like environments (no Bun, no Node)", () => {
    expect(() => defaultHooks({ hasBun: false, hasNode: false })).toThrow(
      /no subprocess available/,
    );
  });

  test("NodeHooks is instantiable independently", () => {
    // Smoke: we can construct it even under Bun. Used by non-Bun
    // runtimes and the explicit-runner code path.
    expect(new NodeHooks()).toBeInstanceOf(NodeHooks);
  });
});

describe("argv helpers", () => {
  test("parseShellLine handles quoted arguments", () => {
    expect(parseShellLine(`echo "hello world"`)).toEqual(["echo", "hello world"]);
  });

  test("formatArgv round-trips through parseShellLine", () => {
    const argv = ["echo", "it's", "hello world", ""];
    expect(parseShellLine(formatArgv(argv))).toEqual(argv);
  });

  test("parseShellLine errors on unterminated quote", () => {
    expect(() => parseShellLine(`echo "hello`)).toThrow(/unterminated quoted string/);
  });

  test("BunHooks accepts string commands", async () => {
    const hooks = new BunHooks();
    const res = await hooks.execute(`printf "hello world"`, "/tmp");
    expect(res.ok).toBe(true);
    expect(new TextDecoder().decode(res.stdout)).toBe("hello world");
  });
});

async function fixtureBundle(ws: { projectDir: string }, fs: DiskFs) {
  return fs.readProject(ws.projectDir, { virtualRoot: "/project" });
}

describe("runHookPlanStream — injected mock executor (native parity cases)", () => {
  const cleanup: string[] = [];
  beforeEach(() => void (cleanup.length = 0));
  afterEach(async () => {
    await Promise.all(cleanup.map((p) => rm(p, { recursive: true, force: true })));
  });

  test("continue-on-failure: hook_a fails, unrelated hooks still run", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const bundle = await fixtureBundle(ws, fs);

    // hook_b depends on hook_ran_hook_a — it demotes to skipped after
    // re-plan. hook_names is independent of hook_a and still runs.
    const mock = new MockHooks((cmd) => {
      // hook_a: fail. Everything else: succeed.
      if (cmd.some((a) => a.includes("hook_a"))) return { exitCode: 1, ok: false };
      return { exitCode: 0, ok: true };
    });

    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    const events = await drain(
      runHookPlanStream((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
        bundle,
        projectDir: "/project",
        outDir: ws.outDir,
        data: {},
        hooks: mock,
        cwd: ws.root, // fixture root exists; outDir may not
      }),
    );

    // No terminal error events — run completed cleanly.
    expect(events.some((e) => e.type === "template_errors")).toBe(false);
    expect(events.some((e) => e.type === "plan_error")).toBe(false);

    const byKey = Object.fromEntries(resultsOf(events).map((r) => [r.key, r]));
    expect(byKey.hook_a?.kind).toBe("failed");
    expect(byKey.hook_b?.kind).toBe("skipped");
    if (byKey.hook_b?.kind === "skipped") {
      expect(byKey.hook_b.skipReason).toBe("false_conditional");
    }
    // hook_names is independent; runs regardless of hook_a's failure.
    expect(byKey.hook_names?.kind).toBe("completed");

    // Failure should have triggered a replan event naming hook_a.
    const replan = events.find((e) => e.type === "replan");
    expect(replan).toBeDefined();
    if (replan?.type === "replan") {
      expect(replan.afterKey).toBe("hook_a");
    }
  });

  test("chained conditional demotion after failure is driven by re-plan", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const bundle = await fixtureBundle(ws, fs);

    const mock = new MockHooks((cmd) => {
      if (cmd.some((a) => a.includes("hook_a"))) return { exitCode: 2, ok: false };
      return { exitCode: 0, ok: true };
    });

    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    await drain(
      runHookPlanStream((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
        bundle,
        projectDir: "/project",
        outDir: ws.outDir,
        data: {},
        hooks: mock,
        cwd: ws.root,
      }),
    );

    // Mock was called for hook_a (which failed) and hook_names (which
    // runs after re-plan). hook_b was demoted — mock NOT called for it.
    const commandsCalled = mock.calls.map((c) => c.command.join(" "));
    expect(commandsCalled.some((c) => c.includes("hook_a"))).toBe(true);
    expect(commandsCalled.some((c) => c.includes("hook_b"))).toBe(false);
    expect(commandsCalled.some((c) => c.includes("_project_name"))).toBe(false);
  });

  test("re-plan failure mid-run emits terminal plan_error (does not throw)", async () => {
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const bundle = await fixtureBundle(ws, fs);

    // Mock: hook_a fails (triggers a re-plan). The planner we inject
    // succeeds on the initial call, then returns a synthetic planner
    // error on the subsequent re-plan.
    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    let plannerCalls = 0;
    const planner = (
      b: Parameters<typeof wasm.planHooks>[0],
      pdir: string,
      odir: string,
      d: Record<string, string>,
      hr: Record<string, boolean> | undefined,
    ): PlanHooksResponse => {
      plannerCalls += 1;
      if (plannerCalls === 1) return wasm.planHooks(b, pdir, odir, d, hr);
      return { ok: false, error: "synthetic re-plan failure" };
    };

    const mock = new MockHooks((cmd) => {
      if (cmd.some((a) => a.includes("hook_a"))) return { exitCode: 1, ok: false };
      return {};
    });

    const events = await drain(
      runHookPlanStream(planner, {
        bundle,
        projectDir: "/project",
        outDir: ws.outDir,
        data: {},
        hooks: mock,
        cwd: ws.root,
      }),
    );

    const terminal = events[events.length - 1];
    expect(terminal?.type).toBe("plan_error");
    if (terminal?.type === "plan_error") {
      expect(terminal.error).toContain("re-plan failed");
      expect(terminal.error).toContain("synthetic re-plan failure");
    }
  });

  test("executed hook with dependents stays satisfied through re-plan", async () => {
    // Build an inline bundle where hook_ok succeeds and hook_dep needs
    // hook_ok. Then fail an unrelated hook_fail (which comes between
    // them) to trigger a re-plan — hook_dep must still be satisfied
    // because plan_hooks keeps executed hooks in the items set.
    const toml = `
[[hooks]]
key = "hook_ok"
command = ["echo", "ok"]
default = true

[[hooks]]
key = "hook_fail"
command = ["echo", "fail"]
default = true

[[hooks]]
key = "hook_dep"
command = ["echo", "dep"]
needs = ["hook_ok"]
default = true
`;
    const bundle = [{ path: "/project/spackle.toml", bytes: new TextEncoder().encode(toml) }];

    const mock = new MockHooks((cmd) => {
      if (cmd.includes("fail")) return { exitCode: 1, ok: false };
      return {};
    });

    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    const events = await drain(
      runHookPlanStream((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
        bundle,
        projectDir: "/project",
        outDir: "/tmp",
        data: {},
        hooks: mock,
        cwd: "/tmp",
      }),
    );

    expect(events.some((e) => e.type === "plan_error")).toBe(false);
    const byKey = Object.fromEntries(resultsOf(events).map((r) => [r.key, r]));
    expect(byKey.hook_ok?.kind).toBe("completed");
    expect(byKey.hook_fail?.kind).toBe("failed");
    // hook_dep must still run — executed hook_ok remains in items set
    // for needs resolution, and hook_dep's needs = [hook_ok] is met.
    expect(byKey.hook_dep?.kind).toBe("completed");
  });

  test("conditional evaluation error surfaces as failed, not skipped (native parity)", async () => {
    // Native run_hooks_stream at src/hook.rs:485 yields
    // HookResultKind::Failed(HookError::ConditionalFailed) when the
    // `if` expression fails to evaluate to a boolean (e.g. a string
    // that isn't "true"/"false"). Our planner surfaces this as
    // skip_reason="conditional_error: ...", and the runner must
    // re-categorize it to { kind: "failed" } — not "skipped".
    // Because the hook never ran, hook_start is NOT emitted for it —
    // only hook_end with kind: "failed".
    const toml = `
[[hooks]]
key = "bad_cond"
command = ["echo", "unreachable"]
"if" = "not-a-boolean-at-all"
default = true
`;
    const bundle = [{ path: "/project/spackle.toml", bytes: new TextEncoder().encode(toml) }];

    const mock = new MockHooks();
    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    const events = await drain(
      runHookPlanStream((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
        bundle,
        projectDir: "/project",
        outDir: "/tmp",
        data: {},
        hooks: mock,
        cwd: "/tmp",
      }),
    );

    const bad = resultsOf(events).find((r) => r.key === "bad_cond");
    expect(bad).toBeDefined();
    expect(bad?.kind).toBe("failed");
    if (bad?.kind === "failed") {
      expect(bad.error).toContain("conditional_error");
    }
    // No hook_start for bad_cond — the runner skipped execute entirely.
    expect(events.some((e) => e.type === "hook_start" && e.key === "bad_cond")).toBe(false);
    // Command never executed — planner short-circuited before execute.
    expect(mock.calls.length).toBe(0);
  });

  test("template-error behind false conditional is a hard abort (native parity)", async () => {
    // Hook whose `if` evaluates false AND has a broken template in its
    // command. Native templates all queued_hooks before evaluating
    // the conditional, so this is a hard error natively. Our planner
    // must match — not silently skip as "false_conditional".
    const toml = `
[[hooks]]
key = "masked"
command = ["echo", "{{ definitely_undefined_variable }}"]
"if" = "false"
default = true
`;
    const bundle = [{ path: "/project/spackle.toml", bytes: new TextEncoder().encode(toml) }];

    const mock = new MockHooks();
    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    const events = await drain(
      runHookPlanStream((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
        bundle,
        projectDir: "/project",
        outDir: "/tmp",
        data: {},
        hooks: mock,
        cwd: "/tmp",
      }),
    );

    // Terminal event is template_errors; no run_start emitted because
    // the hard abort happens before we enter the execution loop.
    expect(events.length).toBe(1);
    const terminal = events[0];
    expect(terminal?.type).toBe("template_errors");
    if (terminal?.type === "template_errors") {
      expect(terminal.error).toContain("template error");
      expect(terminal.templateErrors[0]?.key).toBe("masked");
    }
    expect(mock.calls.length).toBe(0);
  });

  test("consumer mutations of yielded payloads do not leak into runner state", async () => {
    // Regression guard: run_start.plan and hook_start.command used to
    // share references with the runner's own iteration state. A
    // consumer (e.g. a UI reducer) mutating those objects could
    // reorder hooks or alter what actually gets spawned. The
    // orchestrator now clones at the yield boundary; this test
    // verifies that by aggressively mutating every received payload
    // and asserting the actual execution was unaffected.
    const ws = await workspace("hooks_fixture");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const bundle = await fixtureBundle(ws, fs);

    const mock = new MockHooks();
    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();

    for await (const event of runHookPlanStream(
      (b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr),
      {
        bundle,
        projectDir: "/project",
        outDir: ws.outDir,
        data: {},
        hooks: mock,
        cwd: ws.root,
      },
    )) {
      if (event.type === "run_start") {
        // Try to derail iteration: truncate the plan, and corrupt each
        // surviving entry's command.
        event.plan.length = 0;
      } else if (event.type === "hook_start") {
        // Try to change what the runner is about to spawn.
        event.command.length = 0;
        event.command.push("not-a-real-binary", "--danger");
      }
    }

    // All three hooks must have run with their original commands,
    // despite the consumer's mutations.
    expect(mock.calls.length).toBe(3);
    for (const call of mock.calls) {
      expect(call.command[0]).toBe("sh");
      expect(call.command.includes("not-a-real-binary")).toBe(false);
    }
  });

  test("template-error hard abort: no execute() calls, terminal template_errors", async () => {
    // Build an inline bundle with a hook whose command references an
    // undefined variable. Tera's one_off returns an error for
    // undefined variables → template_errors is non-empty →
    // runHookPlanStream must abort before calling execute().
    const toml = `
[[hooks]]
key = "broken"
command = ["echo", "{{ nope_undefined_var }}"]
default = true
`;
    const bundle = [{ path: "/project/spackle.toml", bytes: new TextEncoder().encode(toml) }];

    const mock = new MockHooks();
    const { loadSpackleWasm } = await import("../src/wasm/index.ts");
    const wasm = await loadSpackleWasm();
    const events = await drain(
      runHookPlanStream((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
        bundle,
        projectDir: "/project",
        outDir: "/tmp",
        data: {},
        hooks: mock,
        cwd: "/tmp",
      }),
    );

    expect(events.length).toBe(1);
    const terminal = events[0];
    expect(terminal?.type).toBe("template_errors");
    if (terminal?.type === "template_errors") {
      expect(terminal.error).toContain("template error");
      expect(terminal.templateErrors[0]?.key).toBe("broken");
    }
    expect(mock.calls.length).toBe(0);
  });
});
