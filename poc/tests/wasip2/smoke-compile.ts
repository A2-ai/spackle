// Minimal end-to-end smoke, designed for `bun build --compile`. Confirms
// the wasip2 component + custom Bun WASI shim survive bundling into a
// standalone binary (catches import-resolution drift that only shows
// under `bun build`, not `bun run`).
//
// Usage (see justfile):
//   bun build --compile tests/wasip2/smoke-compile.ts --outfile /tmp/spackle-bun-smoke
//   /tmp/spackle-bun-smoke <path-to-proj2-fixture>
//
// Takes the fixture path as argv[2] because `import.meta.dir` resolves
// to the binary's embed root after compile, not the source tree. That
// drift is exactly what this smoke catches: a `bun run`-only path
// resolution would silently stop working in the compiled binary.
// Exits 0 on success, 1 with an error message on failure.

import { mkdtemp, cp, rm, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { checkProject } from "../../src/wasip2/bun";

async function main() {
    const fixturePath = process.argv[2];
    if (!fixturePath) {
        console.error("Usage: spackle-bun-smoke <path-to-proj2-fixture>");
        process.exit(1);
    }

    const workspaceRoot = await realpath(
        await mkdtemp(join(tmpdir(), "spackle-compile-smoke-")),
    );
    const projectDir = join(workspaceRoot, "project");
    try {
        await cp(fixturePath, projectDir, { recursive: true });

        const res = await checkProject({ workspaceRoot, projectDir });

        if (!res.valid) {
            console.error("FAIL: check returned invalid:", JSON.stringify(res));
            process.exit(1);
        }
        if (!res.config.slots.some((s) => s.key === "defined_field")) {
            console.error("FAIL: expected `defined_field` slot");
            process.exit(1);
        }

        console.log("OK: compile-mode smoke passed — check() against proj2 returned valid");
    } finally {
        await rm(workspaceRoot, { recursive: true, force: true });
    }
}

main().catch((e) => {
    console.error("FAIL:", e);
    process.exit(1);
});
