// Build wasm-pack output for all three target profiles.
//
// - nodejs   — CommonJS, eager init at import time. Consumed by the TS
//              wrapper layer in `src/wasm/index.ts` for Bun / Node.
// - web      — ESM, requires explicit async `init()` before exports work.
//              For browsers that fetch the `.wasm` file directly.
// - bundler  — ESM, delegates wasm loading to the bundler (webpack, vite).
//
// Output lands in `ts/pkg/<target>/` (gitignored). Shipping all three
// lets consumers import `@a2-ai/spackle/pkg/<t>` matching their runtime.
//
// Run: `cd ts && bun run scripts/build.ts` (or `just build-wasm`).

import { spawnSync } from "node:child_process";
import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const TS_DIR = join(import.meta.dir, "..");
const CRATE = join(TS_DIR, "..", "crates", "spackle-wasm");
const PKG_ROOT = join(TS_DIR, "pkg");

const targets = ["nodejs", "web", "bundler"] as const;

for (const target of targets) {
  const outDir = join(PKG_ROOT, target);
  console.log(`\n=== wasm-pack build --target ${target} → ${outDir} ===`);
  const result = spawnSync("wasm-pack", ["build", CRATE, "--target", target, "--out-dir", outDir], {
    stdio: "inherit",
  });
  if (result.status !== 0) {
    console.error(`wasm-pack failed for target=${target}`);
    process.exit(result.status ?? 1);
  }
  if (target === "nodejs") {
    // wasm-pack's --target nodejs emits CommonJS (`exports.foo = ...`) but
    // doesn't mark its package.json as such. When the parent @a2-ai/spackle
    // package is `"type": "module"`, Node / Vite SSR treat the CJS file as
    // ESM and crash with "exports is not defined". Mark the nested package
    // as `"type": "commonjs"` so module resolution picks the right loader.
    markNodejsPackageAsCjs(join(outDir, "package.json"));
  }
}

console.log("\nAll three wasm-pack targets built.");

function markNodejsPackageAsCjs(pkgJsonPath: string): void {
  const raw = readFileSync(pkgJsonPath, "utf8");
  // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
  const pkg = JSON.parse(raw) as Record<string, unknown>;
  if (pkg.type === "commonjs") return;
  pkg.type = "commonjs";
  writeFileSync(pkgJsonPath, `${JSON.stringify(pkg, null, 2)}\n`, "utf8");
  console.log(`  + marked ${pkgJsonPath} as "type": "commonjs"`);
}
