// ORCHESTRATION DEMO — replaces the old `poc/index.ts`. Walks through every
// orchestration entry point so you can eyeball the output after a build.
//
// Run: `just poc` or `bun run scripts/demo.ts`

import { join, relative } from "node:path";
import { rm } from "node:fs/promises";
import { check, generate, loadSpackleWasm } from "../src/spackle.ts";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const OUT_ROOT = join(import.meta.dir, "..", "output");

console.log("Loading WASM module...");
const wasm = await loadSpackleWasm();
console.log("WASM module loaded.\n");

// --- check: proj2 (clean) vs bad_default_slot_val ---

for (const fixture of ["proj2", "bad_default_slot_val"]) {
  const dir = join(REPO_ROOT, "tests", "data", fixture);
  console.log(`=== check(${fixture}) ===`);
  const result = await check(dir);
  console.log(
    `  valid=${result.valid}`,
    result.errors ? `errors=${JSON.stringify(result.errors)}` : "",
  );
  if (result.config) {
    console.log(
      `  name=${result.config.name ?? "(unnamed)"}`,
      `slots=${result.config.slots.length}`,
      `hooks=${result.config.hooks.length}`,
    );
  }
  console.log();
}

// --- generate: proj2 (clean, no hooks) ---

const proj2Dir = join(REPO_ROOT, "tests", "data", "proj2");
const proj2Out = join(OUT_ROOT, "proj2");
await rm(proj2Out, { recursive: true, force: true });
console.log(`=== generate(proj2) → ${relative(process.cwd(), proj2Out)} ===`);
const proj2Result = await generate(
  proj2Dir,
  { defined_field: "hello world" },
  proj2Out,
);
console.log(
  `  wrote=${proj2Result.written} copied=${proj2Result.copied} hooks_planned=${proj2Result.plan.length}`,
);
for (const r of proj2Result.rendered) {
  const status = r.error ? `ERROR: ${r.error}` : r.content.substring(0, 60).replace(/\n/g, "\\n");
  console.log(`  ${r.original_path} → ${r.rendered_path}  ${status}`);
}
console.log();

// --- generate: hook fixture with runHooks enabled ---

const hookDir = join(REPO_ROOT, "tests", "data", "hook");
const hookOut = join(OUT_ROOT, "hook");
await rm(hookOut, { recursive: true, force: true });
console.log(`=== generate(hook, runHooks=true) → ${relative(process.cwd(), hookOut)} ===`);
const hookResult = await generate(hookDir, {}, hookOut, { runHooks: true });
for (const outcome of hookResult.hookOutcomes) {
  if (outcome.kind === "ran") {
    console.log(
      `  RAN ${outcome.result.key} exit=${outcome.result.exit_code}`,
      `stdout=${JSON.stringify(outcome.result.stdout.trim())}`,
      `stderr=${JSON.stringify(outcome.result.stderr.trim())}`,
    );
  } else {
    console.log(`  SKIP ${outcome.info.key} reason=${outcome.info.skip_reason}`);
  }
}
console.log();

// --- evaluate_hooks: conditional-hook fixture (no generate) ---

const condDir = join(REPO_ROOT, "tests", "data", "hook_ran_cond");
console.log("=== evaluate_hooks(hook_ran_cond) ===");
const condToml = await Bun.file(join(condDir, "spackle.toml")).text();
const condConfig = wasm.parseConfig(condToml);
const condPlan = wasm.evaluateHooks(JSON.stringify(condConfig), {});
for (const h of condPlan) {
  const status = h.should_run ? "WOULD RUN" : `SKIP (${h.skip_reason})`;
  console.log(`  ${h.key}: ${status}  cmd=${JSON.stringify(h.command)}`);
}

console.log("\nDone.");
