// Smoke tests for the wasip2 component. Uses node:test (not bun test)
// because @bytecodealliance/preview2-shim's synchronous filesystem
// bridge depends on worker-thread modules (`process.binding("tcp_wrap")`,
// native TCP) that Bun doesn't implement. The wasip2 component itself
// is runtime-neutral — the shim is the Bun blocker.
//
// Run: `node --test poc/tests/wasip2/` (or via `just test-wasip2`).

import { test, before, after } from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, cp, rm, realpath, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const FIXTURES = resolve(here, "..", "..", "..", "tests", "data");

/** @type {typeof import("../../src/wasip2/index.ts")} */
let wasip2;

before(async () => {
    wasip2 = await import("../../src/wasip2/index.ts");
});

const workspaces = [];
after(async () => {
    for (const p of workspaces) {
        await rm(p, { recursive: true, force: true });
    }
});

async function makeWorkspace(fixture) {
    // realpath so preopens match what the component sees via canonicalize.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-wasip2-")));
    const projectDir = join(root, "project");
    await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
    const outDir = join(root, "output");
    workspaces.push(root);
    return { workspaceRoot: root, projectDir, outDir };
}

test("check: happy path returns parsed config", async () => {
    const ws = await makeWorkspace("proj2");
    const res = await wasip2.checkProject(ws);
    assert.equal(res.valid, true, `expected valid=true, got: ${JSON.stringify(res)}`);
    if (res.valid) {
        const keys = res.config.slots.map((s) => s.key);
        assert.ok(keys.includes("defined_field"), `missing defined_field in ${keys}`);
    }
});

test("check: template ref error surfaces in errors array", async () => {
    const ws = await makeWorkspace("bad_template");
    const res = await wasip2.checkProject(ws);
    assert.equal(res.valid, false);
    if (!res.valid) {
        assert.ok(res.errors.length > 0, "errors array should be non-empty");
        assert.ok(
            res.errors.join(" ").includes("invalid_slot"),
            `errors should mention 'invalid_slot', got: ${res.errors.join(" | ")}`,
        );
    }
});

test("validate_slot_data: accepts good data, rejects wrong type", async () => {
    const ws = await makeWorkspace("proj1");

    const ok = await wasip2.validateSlotData({
        ...ws,
        slotData: { slot_1: "hello", slot_2: "42", slot_3: "true" },
    });
    assert.equal(ok.valid, true, `valid data rejected: ${JSON.stringify(ok)}`);

    const bad = await wasip2.validateSlotData({
        ...ws,
        slotData: { slot_1: "hello", slot_2: "not-a-number", slot_3: "true" },
    });
    assert.equal(bad.valid, false, `bad type accepted: ${JSON.stringify(bad)}`);
});

test("generate: writes rendered + copied files to outDir", async () => {
    const ws = await makeWorkspace("proj2");

    const res = await wasip2.generateProject({
        ...ws,
        slotData: { defined_field: "hello" },
        runHooks: false,
    });

    assert.equal(res.ok, true, `generate failed: ${JSON.stringify(res)}`);
    if (res.ok) {
        const originals = res.rendered.map((r) => r.original_path);
        assert.ok(originals.includes("good.j2"), `good.j2 not rendered: ${originals}`);

        const rendered = await readFile(join(ws.outDir, "good"), "utf8");
        assert.equal(rendered, "hello");

        const copied = await readFile(join(ws.outDir, "subdir", "file.txt"), "utf8");
        assert.ok(
            copied.includes("{{ undefined_field }}"),
            `subdir/file.txt template not preserved verbatim: ${copied}`,
        );
    }
});

test("generate: runHooks=true invokes host runCommand and captures output", async () => {
    const ws = await makeWorkspace("hook");

    const res = await wasip2.generateProject({
        ...ws,
        slotData: {},
        runHooks: true,
    });

    assert.equal(res.ok, true, `generate failed: ${JSON.stringify(res)}`);
    if (res.ok) {
        const byKey = new Map(res.hook_results.map((r) => [r.hook_key, r]));

        // hook_1 and hook_2 use bash to emit stdout/stderr — exercise
        // the host-imported subprocess capability end-to-end.
        const h1 = byKey.get("hook_1");
        assert.ok(h1, "hook_1 missing from results");
        assert.equal(h1.kind, "completed");
        if (h1.kind === "completed") {
            const stdout = Buffer.from(h1.stdout).toString();
            const stderr = Buffer.from(h1.stderr).toString();
            assert.ok(
                stdout.includes("This is logged to stdout"),
                `hook_1 stdout unexpected: ${stdout}`,
            );
            assert.ok(
                stderr.includes("This is logged to stderr"),
                `hook_1 stderr unexpected: ${stderr}`,
            );
        }

        const h2 = byKey.get("hook_2");
        assert.equal(h2?.kind, "completed", "hook_2 should complete");

        // NB: `hook_3` uses the legacy `optional = { default = false }` TOML
        // shape which the current Hook deserializer ignores (Hook has a flat
        // `default` field, not nested `optional`). So hook_3 runs instead
        // of being skipped — a pre-existing quirk unrelated to this PR.
        const h3 = byKey.get("hook_3");
        assert.ok(h3, "hook_3 missing");
        assert.equal(h3.kind, "completed");
    }
});

test("generate: hook_ran conditional chain re-evaluates correctly", async () => {
    const ws = await makeWorkspace("hook_ran_cond");

    const res = await wasip2.generateProject({
        ...ws,
        slotData: {},
        runHooks: true,
    });

    assert.equal(res.ok, true, `generate failed: ${JSON.stringify(res)}`);
    if (res.ok) {
        const byKey = new Map(res.hook_results.map((r) => [r.hook_key, r]));

        // hook_1 runs (explicit default=true via legacy shape, ignored but
        // falls back to is_enabled()=true since no override disables it).
        assert.equal(byKey.get("hook_1")?.kind, "completed", "hook_1 should run");

        // dep_hook_should_run: if={{ hook_ran_hook_1 }} → true → runs.
        // This is the load-bearing assertion for conditional re-eval:
        // hook_ran_* state propagates across hooks inside a single WASI
        // component call.
        assert.equal(
            byKey.get("dep_hook_should_run")?.kind,
            "completed",
            "dep_hook_should_run (if={{hook_ran_hook_1}}=true) should run",
        );
    }
});
