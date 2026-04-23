// Walk through the public API in `../src/spackle.ts` so you can eyeball
// the output after a wasm-pack build.
//
// Run: `just wasm-demo` or `cd ts && bun run scripts/demo.ts`

import { cp, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";

import {
  DiskFs,
  MemoryFs,
  check,
  generate,
  generateBundle,
  planHooks,
  runHooksStream,
  validateSlotData,
} from "../src/spackle.ts";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const FIXTURES = join(REPO_ROOT, "tests", "fixtures");

/** Throwaway workspace seeded with a fixture. */
async function workspace(fixture: string) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-demo-")));
  const projectDir = join(root, "project");
  await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
  const outDir = join(root, "output");
  return { root, projectDir, outDir };
}

// --- check: basic_project (clean) + bad_template (template error) ---
//
// Awaits in the loops below are intentionally sequential: parallelizing
// would interleave the per-fixture console output and make the demo
// hard to read. Correctness > throughput for a demo script.

for (const fixture of ["basic_project", "bad_template"]) {
  // oxlint-disable-next-line eslint/no-await-in-loop
  const ws = await workspace(fixture);
  try {
    console.log(`=== check(${fixture}) — DiskFs ===`);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    // oxlint-disable-next-line eslint/no-await-in-loop
    const result = await check(ws.projectDir, fs);
    console.log(
      `  valid=${result.valid}`,
      !result.valid ? `errors=${JSON.stringify(result.errors)}` : "",
    );
    if (result.valid) {
      console.log(
        `  name=${result.config.name ?? "(unnamed)"}`,
        `slots=${result.config.slots.length}`,
        `hooks=${result.config.hooks.length}`,
      );
    }
    console.log();
  } finally {
    // oxlint-disable-next-line eslint/no-await-in-loop
    await rm(ws.root, { recursive: true, force: true });
  }
}

// --- validateSlotData: typed_slots happy + bad-type ---

{
  const ws = await workspace("typed_slots");
  try {
    const fs = new DiskFs({ workspaceRoot: ws.root });
    console.log("=== validateSlotData(typed_slots, good) ===");
    const ok = await validateSlotData(
      ws.projectDir,
      { name: "hello", count: "42", enabled: "true" },
      fs,
    );
    console.log(`  valid=${ok.valid}`);

    console.log("=== validateSlotData(typed_slots, wrong-type) ===");
    const bad = await validateSlotData(
      ws.projectDir,
      { name: "hello", count: "not-a-number", enabled: "true" },
      fs,
    );
    console.log(`  valid=${bad.valid}`, !bad.valid ? `errors=${JSON.stringify(bad.errors)}` : "");
    console.log();
  } finally {
    await rm(ws.root, { recursive: true, force: true });
  }
}

// --- generate: basic_project with DiskFs (writes to disk) ---

{
  const ws = await workspace("basic_project");
  try {
    const fs = new DiskFs({ workspaceRoot: ws.root });
    console.log(`=== generate(basic_project, DiskFs) → ${relative(process.cwd(), ws.outDir)} ===`);
    const result = await generate(
      ws.projectDir,
      ws.outDir,
      { greeting: "hi", target: "demo", filename: "notes" },
      fs,
    );
    if (result.ok) {
      for (const f of result.files) console.log(`  ${f.path}  (${f.bytes.length} bytes)`);
    } else {
      console.log(`  FAILED: ${result.error}`);
    }
    console.log();
  } finally {
    await rm(ws.root, { recursive: true, force: true });
  }
}

// --- generateBundle: basic_project in-memory (no disk for input or output) ---

{
  const projectBundle = new MemoryFs({
    files: {
      "/project/spackle.toml": await Bun.file(
        join(FIXTURES, "basic_project", "spackle.toml"),
      ).text(),
      "/project/README.md.j2": await Bun.file(
        join(FIXTURES, "basic_project", "README.md.j2"),
      ).text(),
      "/project/docs/static.md": await Bun.file(
        join(FIXTURES, "basic_project", "docs", "static.md"),
      ).text(),
    },
  }).toBundle();

  console.log("=== generateBundle(basic_project) — in-memory preview ===");
  const result = await generateBundle(projectBundle, {
    greeting: "hey",
    target: "mem",
    filename: "notes",
  });
  if (result.ok) {
    console.log(`  rendered ${result.files.length} file(s):`);
    for (const f of result.files) {
      const preview = new TextDecoder().decode(f.bytes).slice(0, 40);
      console.log(`    ${f.path}  ${JSON.stringify(preview)}`);
    }
  } else {
    console.log(`  FAILED: ${result.error}`);
  }
  console.log();
}

// --- planHooks + runHooksStream: two-call flow against hooks_fixture ---
//
// planHooks returns the resolved plan (templated commands, should-run,
// skip reasons) without executing. runHooksStream then actually spawns
// the commands via defaultHooks() (BunHooks under Bun, NodeHooks on
// Node) and yields HookEvents as each hook progresses — this is the
// shape you'd bridge directly to an SSE endpoint.

{
  const ws = await workspace("hooks_fixture");
  try {
    const fs = new DiskFs({ workspaceRoot: ws.root });

    console.log("=== planHooks(hooks_fixture) ===");
    const plan = await planHooks(ws.projectDir, ws.outDir, {}, fs);
    if (plan.ok) {
      for (const e of plan.plan) {
        console.log(`  ${e.key}  should_run=${e.should_run}`, e.skip_reason ?? "");
      }
    } else {
      console.log(`  FAILED: ${plan.error}`);
    }

    console.log(`\n=== runHooksStream(hooks_fixture) → ${relative(process.cwd(), ws.outDir)} ===`);
    // runHooksStream needs outDir to exist (cwd for spawned processes).
    await (await import("node:fs/promises")).mkdir(ws.outDir, { recursive: true });
    for await (const event of runHooksStream(ws.projectDir, ws.outDir, {}, fs)) {
      switch (event.type) {
        case "run_start":
          console.log(`  [run_start] ${event.plan.length} hook(s) planned`);
          break;
        case "hook_start":
          console.log(`  [hook_start] ${event.key}`);
          break;
        case "hook_end": {
          const timing = event.durationMs !== undefined ? ` (${event.durationMs}ms)` : "";
          console.log(`  [hook_end]   ${event.key} → ${event.result.kind}${timing}`);
          break;
        }
        case "replan":
          console.log(`  [replan]     after ${event.afterKey}; ${event.plan.length} remaining`);
          break;
        case "template_errors":
          console.log(`  [template_errors] ${event.error}`);
          break;
        case "plan_error":
          console.log(`  [plan_error] ${event.error}`);
          break;
      }
    }
  } finally {
    await rm(ws.root, { recursive: true, force: true });
  }
}

console.log("\nDone.");
