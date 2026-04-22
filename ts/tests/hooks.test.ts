// End-to-end tests for the hooks pipeline — plan_hooks (wasm) + runHooks
// (host-side executor). Exercises the default runner (BunHooks under Bun)
// against the hooks_fixture, plus mock-executor cases for the native
// parity semantics: continue-on-failure, chained-conditional re-plan,
// template-error hard abort.

import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { mkdtemp, readFile, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  BunHooks,
  DiskFs,
  NodeHooks,
  defaultHooks,
  planHooks,
  runHookPlan,
  runHooks,
  type HookExecuteResult,
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

/** Record each execute() call, return a scripted outcome per invocation.
 * Scripts are matched by hook command[0] — e.g. `{ sh: () => ok() }`
 * returns a 0 exit for every call whose first arg is "sh". Missing
 * scripts default to exit 0 with empty buffers. */
class MockHooks implements SpackleHooks {
  readonly calls: Array<{ command: string[]; cwd: string }> = [];
  constructor(
    private readonly script: (cmd: string[]) => Partial<HookExecuteResult> = () => ({}),
  ) {}
  async execute(command: string[], cwd: string): Promise<HookExecuteResult> {
    this.calls.push({ command, cwd });
    const override = this.script(command);
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

describe("runHooks (default runner — actually spawns)", () => {
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

    const res = await runHooks(ws.projectDir, ws.outDir, {}, fs);
    expect(res.ok).toBe(true);
    if (res.ok) {
      expect(res.results.map((r) => r.kind)).toEqual(["completed", "completed", "completed"]);
    }

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

async function fixtureBundle(ws: { projectDir: string }, fs: DiskFs) {
  return fs.readProject(ws.projectDir, { virtualRoot: "/project" });
}

describe("runHookPlan — injected mock executor (native parity cases)", () => {
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
    const res = await runHookPlan((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
      bundle,
      projectDir: "/project",
      outDir: ws.outDir,
      data: {},
      hooks: mock,
      cwd: ws.root, // fixture root exists; outDir may not
    });
    expect(res.ok).toBe(true);
    if (res.ok) {
      const byKey = Object.fromEntries(res.results.map((r) => [r.key, r]));
      expect(byKey.hook_a?.kind).toBe("failed");
      expect(byKey.hook_b?.kind).toBe("skipped");
      if (byKey.hook_b?.kind === "skipped") {
        expect(byKey.hook_b.skipReason).toBe("false_conditional");
      }
      // hook_names is independent; runs regardless of hook_a's failure.
      expect(byKey.hook_names?.kind).toBe("completed");
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
    const res = await runHookPlan((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
      bundle,
      projectDir: "/project",
      outDir: ws.outDir,
      data: {},
      hooks: mock,
      cwd: ws.root,
    });

    expect(res.ok).toBe(true);
    if (res.ok) {
      // Mock was called for hook_a (which failed) and hook_names
      // (which runs after re-plan). hook_b was demoted — mock NOT
      // called for it.
      const commandsCalled = mock.calls.map((c) => c.command.join(" "));
      expect(commandsCalled.some((c) => c.includes("hook_a"))).toBe(true);
      expect(commandsCalled.some((c) => c.includes("hook_b"))).toBe(false);
      expect(commandsCalled.some((c) => c.includes("_project_name"))).toBe(false);
    }
  });

  test("re-plan failure mid-run returns ok=false (does not throw)", async () => {
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

    const res = await runHookPlan(planner, {
      bundle,
      projectDir: "/project",
      outDir: ws.outDir,
      data: {},
      hooks: mock,
      cwd: ws.root,
    });
    expect(res.ok).toBe(false);
    if (!res.ok) {
      expect(res.error).toContain("re-plan failed");
      expect(res.error).toContain("synthetic re-plan failure");
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
    const res = await runHookPlan((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
      bundle,
      projectDir: "/project",
      outDir: "/tmp",
      data: {},
      hooks: mock,
      cwd: "/tmp",
    });
    expect(res.ok).toBe(true);
    if (res.ok) {
      const byKey = Object.fromEntries(res.results.map((r) => [r.key, r]));
      expect(byKey.hook_ok?.kind).toBe("completed");
      expect(byKey.hook_fail?.kind).toBe("failed");
      // hook_dep must still run — executed hook_ok remains in items set
      // for needs resolution, and hook_dep's needs = [hook_ok] is met.
      expect(byKey.hook_dep?.kind).toBe("completed");
    }
  });

  test("conditional evaluation error surfaces as failed, not skipped (native parity)", async () => {
    // Native run_hooks_stream at src/hook.rs:485 yields
    // HookResultKind::Failed(HookError::ConditionalFailed) when the
    // `if` expression fails to evaluate to a boolean (e.g. a string
    // that isn't "true"/"false"). Our planner surfaces this as
    // skip_reason="conditional_error: ...", and the runner must
    // re-categorize it to { kind: "failed" } — not "skipped".
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
    const res = await runHookPlan((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
      bundle,
      projectDir: "/project",
      outDir: "/tmp",
      data: {},
      hooks: mock,
      cwd: "/tmp",
    });

    expect(res.ok).toBe(true);
    if (res.ok) {
      const bad = res.results.find((r) => r.key === "bad_cond");
      expect(bad).toBeDefined();
      expect(bad?.kind).toBe("failed");
      if (bad?.kind === "failed") {
        expect(bad.error).toContain("conditional_error");
      }
    }
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
    const res = await runHookPlan((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
      bundle,
      projectDir: "/project",
      outDir: "/tmp",
      data: {},
      hooks: mock,
      cwd: "/tmp",
    });

    expect(res.ok).toBe(false);
    if (!res.ok) {
      expect(res.error).toContain("template error");
      expect(res.templateErrors?.[0]?.key).toBe("masked");
    }
    expect(mock.calls.length).toBe(0);
  });

  test("template-error hard abort: no execute() calls, ok=false", async () => {
    // Build an inline bundle with a hook whose command references an
    // undefined variable. Tera's one_off returns an error for
    // undefined variables → template_errors is non-empty → runHookPlan
    // must abort before calling execute().
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
    const res = await runHookPlan((b, pdir, odir, d, hr) => wasm.planHooks(b, pdir, odir, d, hr), {
      bundle,
      projectDir: "/project",
      outDir: "/tmp",
      data: {},
      hooks: mock,
      cwd: "/tmp",
    });

    expect(res.ok).toBe(false);
    if (!res.ok) {
      expect(res.error).toContain("template error");
      expect(res.templateErrors).toBeDefined();
      expect(res.templateErrors?.[0]?.key).toBe("broken");
    }
    expect(mock.calls.length).toBe(0);
  });
});
