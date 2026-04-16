/**
 * Spackle WASM Proof of Concept
 *
 * Run: cd poc && bun ./index.ts
 *
 * Loads the spackle WASM module, reads test fixtures from disk, and
 * exercises every exported function: parse_config, validate_config,
 * validate_slot_data, render_templates, evaluate_hooks.
 *
 * Writes rendered output to poc/output/ so you can diff against
 * `spackle fill` for the same inputs.
 */

import { readFile, readdir, writeFile, mkdir, stat } from "node:fs/promises";
import { join, dirname, relative } from "node:path";
import initWasm, {
  parse_config,
  validate_config,
  validate_slot_data,
  render_templates,
  evaluate_hooks,
} from "./pkg/spackle.js";

// --- Config ---
const PROJECT_DIR = join(import.meta.dir, "..", "tests", "data", "proj1");
const OUT_DIR = join(import.meta.dir, "output");

// --- Init WASM ---
console.log("Loading WASM module...");
await initWasm();
console.log("WASM module loaded.\n");

// --- 1. Parse config ---
const tomlContent = await readFile(join(PROJECT_DIR, "spackle.toml"), "utf-8");
console.log("=== parse_config ===");
const configJson = parse_config(tomlContent);
const config = JSON.parse(configJson);
if (config.error) {
  console.error("PARSE ERROR:", config.error);
  process.exit(1);
}
console.log(
  `  Name: ${config.name ?? "(unnamed)"}`,
  `\n  Slots: ${config.slots.length}`,
  `\n  Hooks: ${config.hooks.length}`,
);
for (const s of config.slots) {
  console.log(`    slot: ${s.key} [${s.type}]${s.default ? ` = ${s.default}` : ""}`);
}
for (const h of config.hooks) {
  console.log(`    hook: ${h.key} → ${h.command.join(" ")}`);
}

// --- 2. Validate config ---
console.log("\n=== validate_config ===");
const validation = JSON.parse(validate_config(tomlContent));
if (validation.valid) {
  console.log("  Config is valid.");
} else {
  console.log("  ERRORS:", validation.errors.join(", "));
}

// --- 3. Validate slot data ---
console.log("\n=== validate_slot_data ===");
const slotData: Record<string, string> = {
  slot_1: "hello",
  slot_2: "42",
  slot_3: "true",
};
const slotValidation = JSON.parse(
  validate_slot_data(configJson, JSON.stringify(slotData)),
);
if (slotValidation.valid) {
  console.log("  Slot data is valid.");
} else {
  console.log("  ERRORS:", slotValidation.errors?.join(", ") ?? slotValidation.error);
}

// --- 4. Read template files from disk ---
console.log("\n=== Reading .j2 template files ===");
const templates: Array<{ path: string; content: string }> = [];
await walkTemplates(PROJECT_DIR, PROJECT_DIR, templates);
console.log(`  Found ${templates.length} template(s):`);
for (const t of templates) {
  console.log(`    ${t.path} (${t.content.length} bytes)`);
}

// --- 5. Render templates via WASM ---
console.log("\n=== render_templates ===");
const rendered = JSON.parse(
  render_templates(JSON.stringify(templates), JSON.stringify(slotData), configJson),
);
if (rendered.error) {
  console.error("RENDER ERROR:", rendered.error);
  process.exit(1);
}
for (const file of rendered) {
  if (file.error) {
    console.log(`  ERROR ${file.original_path}: ${file.error}`);
  } else {
    console.log(`  ${file.original_path} → ${file.rendered_path}`);
    console.log(`    ${file.content.substring(0, 100).replace(/\n/g, "\\n")}${file.content.length > 100 ? "..." : ""}`);
  }
}

// --- 6. Write rendered files to output ---
console.log(`\n=== Writing to ${relative(process.cwd(), OUT_DIR)} ===`);
await mkdir(OUT_DIR, { recursive: true });
let written = 0;
for (const file of rendered) {
  if (file.error) continue;
  const dest = join(OUT_DIR, file.rendered_path);
  await mkdir(dirname(dest), { recursive: true });
  await writeFile(dest, file.content);
  written++;
}
console.log(`  Wrote ${written} file(s).`);

// --- 7. Evaluate hook plan ---
console.log("\n=== evaluate_hooks ===");
const hookPlan = JSON.parse(evaluate_hooks(configJson, JSON.stringify(slotData)));
for (const h of hookPlan) {
  const status = h.should_run ? "WOULD RUN" : `SKIP (${h.skip_reason})`;
  console.log(`  ${h.key}: ${status}`);
  console.log(`    cmd: ${h.command.join(" ")}`);
}

console.log("\nDone.");

// --- Helpers ---

async function walkTemplates(
  base: string,
  dir: string,
  out: Array<{ path: string; content: string }>,
) {
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = join(dir, entry.name);
    if (entry.isDirectory()) {
      // Skip spackle internal dirs
      if (entry.name === ".git" || entry.name === "node_modules") continue;
      await walkTemplates(base, full, out);
    } else if (entry.name.endsWith(".j2")) {
      const rel = relative(base, full);
      const content = await readFile(full, "utf-8");
      out.push({ path: rel, content });
    }
  }
}
