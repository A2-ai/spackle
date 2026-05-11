// End-to-end tests — exercise check/validateSlotData/generate through
// the bundle-in / bundle-out API. DiskFs covers the disk-backed flow,
// checkBundle / generateBundle covers the in-memory flow.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { cp, mkdtemp, realpath, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  DiskFs,
  MemoryFs,
  check,
  checkBundle,
  configureSpackleWasm,
  generate,
  generateBundle,
  renderBundle,
  validateSlotData,
} from "../src/spackle.ts";

const FIXTURES = resolve(import.meta.dir, "..", "..", "tests", "fixtures");
const WASM = resolve(import.meta.dir, "..", "pkg", "spackle_wasm_bg.wasm");

try {
  configureSpackleWasm({ moduleOrPath: readFile(WASM) });
} catch (err) {
  if (!(err instanceof Error) || !err.message.includes("before loadSpackleWasm")) throw err;
}

async function workspace(fixture: string) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-")));
  const projectDir = join(root, "project");
  await cp(join(FIXTURES, fixture), projectDir, { recursive: true });
  const outDir = join(root, "output");
  return { root, projectDir, outDir };
}

async function bundleFromDisk(fixtureSubpaths: string[], fixtureRoot: string, virtualRoot: string) {
  return Promise.all(
    fixtureSubpaths.map(async (sub) => {
      const content = await readFile(join(fixtureRoot, sub));
      return { path: `${virtualRoot}/${sub}`, bytes: new Uint8Array(content) };
    }),
  );
}

describe("spackle (DiskFs)", () => {
  const cleanup: string[] = [];
  beforeEach(() => void (cleanup.length = 0));
  afterEach(async () => {
    await Promise.all(cleanup.map((p) => rm(p, { recursive: true, force: true })));
  });

  test("check: happy path returns parsed config and no diagnostics", async () => {
    const ws = await workspace("basic_project");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const res = await check(ws.projectDir, fs);

    expect(res.diagnostics).toEqual([]);
    expect(res.config).not.toBeNull();
    if (res.config) {
      const keys = res.config.slots.map((s) => s.key);
      expect(keys).toContain("greeting");
      expect(keys).toContain("target");
      expect(keys).toContain("filename");
    }
  });

  test("check: bad_template surfaces template diagnostic with path", async () => {
    const ws = await workspace("bad_template");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const res = await check(ws.projectDir, fs);

    expect(res.diagnostics.length).toBeGreaterThan(0);
    const renderDiag = res.diagnostics.find((d) => d.source === "render_body");
    expect(renderDiag).toBeDefined();
    expect(renderDiag?.message).toContain("invalid_slot");
    expect(renderDiag?.path).toBe("bad.j2");
    // Config still parsed — slot/hook lists are inspectable.
    expect(res.config).not.toBeNull();
  });

  test("validateSlotData: accepts good data, rejects wrong type", async () => {
    const ws = await workspace("typed_slots");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    const ok = await validateSlotData(
      ws.projectDir,
      { name: "hello", count: "42", enabled: "true" },
      fs,
    );
    expect(ok.valid).toBe(true);

    const bad = await validateSlotData(
      ws.projectDir,
      { name: "hello", count: "not-a-number", enabled: "true" },
      fs,
    );
    expect(bad.valid).toBe(false);
  });

  test("generate: writes rendered + copied files to outDir", async () => {
    const ws = await workspace("basic_project");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    const res = await generate(
      ws.projectDir,
      ws.outDir,
      { greeting: "hi", target: "world", filename: "notes" },
      fs,
    );
    expect(res.ok).toBe(true);
    if (res.ok) {
      const paths = res.files.map((f) => f.path).toSorted();
      expect(paths).toContain("README.md");

      const readme = await readFile(join(ws.outDir, "README.md"), "utf8");
      expect(readme).toContain("HI, world!");

      // Static file copied verbatim (tokens not interpolated).
      const copied = await readFile(join(ws.outDir, "docs", "static.md"), "utf8");
      expect(copied).toContain("{{ greeting }}");

      // `drafts/` is in the ignore list and must not be copied.
      let draftThrew = false;
      try {
        await readFile(join(ws.outDir, "drafts", "ignored.md"));
      } catch {
        draftThrew = true;
      }
      expect(draftThrew).toBe(true);
    }
  });

  test("generate: refuses a pre-existing outDir (native AlreadyExists parity)", async () => {
    const ws = await workspace("basic_project");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    // Pre-create the out dir so writeOutput sees it already present.
    await import("node:fs/promises").then((mod) => mod.mkdir(ws.outDir, { recursive: true }));

    let err: unknown = null;
    try {
      await generate(
        ws.projectDir,
        ws.outDir,
        { greeting: "hi", target: "world", filename: "notes" },
        fs,
      );
    } catch (e) {
      err = e;
    }
    expect(err).not.toBeNull();
    expect(String(err)).toMatch(/already exists/);
  });
});

describe("spackle render (diagnostics-first)", () => {
  test("renderBundle clean project: empty diagnostics, files present", async () => {
    const bundle = await bundleFromDisk(
      ["spackle.toml", "README.md.j2", "docs/static.md", "src/{{ filename }}.txt.j2"],
      join(FIXTURES, "basic_project"),
      "/project",
    );
    const res = await renderBundle(
      bundle,
      { greeting: "hi", target: "world", filename: "notes" },
      { virtualProjectDir: "/project", virtualOutDir: "/output" },
    );
    expect(res.diagnostics).toEqual([]);
    expect(res.files.length).toBeGreaterThan(0);
    expect(res.hookPlan).not.toBeNull();
  });

  test("renderBundle bad_template: surfaces per-file diagnostic with path", async () => {
    const bundle = await bundleFromDisk(
      ["spackle.toml", "bad.j2"],
      join(FIXTURES, "bad_template"),
      "/project",
    );
    const res = await renderBundle(
      bundle,
      { defined_slot: "value" },
      { virtualProjectDir: "/project", virtualOutDir: "/output" },
    );
    const renderDiag = res.diagnostics.find((d) => d.source === "render_body");
    expect(renderDiag).toBeDefined();
    expect(renderDiag?.message).toContain("invalid_slot");
    expect(renderDiag?.path).toBeDefined();
    // hook plan still computed (no hooks defined, but plan should be []).
    expect(res.hookPlan).toEqual([]);
  });

  test("renderBundle missing slot data: surfaces slot_data diagnostic", async () => {
    const bundle = await bundleFromDisk(
      ["spackle.toml", "README.md.j2", "docs/static.md", "src/{{ filename }}.txt.j2"],
      join(FIXTURES, "basic_project"),
      "/project",
    );
    const res = await renderBundle(
      bundle,
      // missing `target` and `filename`
      { greeting: "hi" },
      { virtualProjectDir: "/project", virtualOutDir: "/output" },
    );
    const slotDataDiag = res.diagnostics.find((d) => d.source === "slot_data");
    expect(slotDataDiag).toBeDefined();
  });
});

describe("spackle (bundle-only / MemoryFs)", () => {
  test("checkBundle + generateBundle end-to-end without touching disk", async () => {
    const bundle = await bundleFromDisk(
      ["spackle.toml", "README.md.j2", "docs/static.md", "src/{{ filename }}.txt.j2"],
      join(FIXTURES, "basic_project"),
      "/project",
    );

    const checkRes = await checkBundle(bundle, "/project");
    expect(checkRes.diagnostics).toEqual([]);

    const genRes = await generateBundle(
      bundle,
      { greeting: "hey", target: "mem", filename: "notes" },
      { virtualProjectDir: "/project", virtualOutDir: "/output" },
    );
    expect(genRes.ok).toBe(true);
    if (genRes.ok) {
      // Output bundle paths are RELATIVE to out_dir.
      const snap = MemoryFs.fromBundle(genRes.files, "/output").snapshot();
      const readme = snap.files["/output/README.md"];
      expect(readme).toBeDefined();
      expect(new TextDecoder().decode(readme)).toContain("HEY, mem!");

      const copied = snap.files["/output/docs/static.md"];
      expect(copied).toBeDefined();
      expect(new TextDecoder().decode(copied)).toContain("{{ greeting }}");
    }
  });
});
