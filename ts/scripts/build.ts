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
import { join } from "node:path";

const TS_DIR = join(import.meta.dir, "..");
const CRATE = join(TS_DIR, "..", "crates", "spackle-wasm");
const PKG_ROOT = join(TS_DIR, "pkg");

const targets = ["nodejs", "web", "bundler"] as const;

for (const target of targets) {
    const outDir = join(PKG_ROOT, target);
    console.log(`\n=== wasm-pack build --target ${target} → ${outDir} ===`);
    const result = spawnSync(
        "wasm-pack",
        ["build", CRATE, "--target", target, "--out-dir", outDir],
        { stdio: "inherit" },
    );
    if (result.status !== 0) {
        console.error(`wasm-pack failed for target=${target}`);
        process.exit(result.status ?? 1);
    }
}

console.log("\nAll three wasm-pack targets built.");
