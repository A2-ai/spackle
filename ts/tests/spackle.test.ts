// End-to-end tests for the TS orchestrator. Exercise the public
// surface (`check`, `validateSlotData`, `generate`, `render`) against
// the fixture projects in `tests/fixtures/`.
//
// The orchestrator owns project walking + disk I/O; wasm handles
// config validation, per-file render, and hook planning. These tests
// catch:
//   - check / validateSlotData flow through wasm cleanly
//   - disk-direct `generate` writes the rendered tree with the right
//     contents, applies the ignore filter (basename at any depth),
//     classifies templates by source name, and streams static files
//   - render (diagnostics-first) collects diagnostics without aborting
//   - failure modes (pre-existing outDir, slot validation failure)
//     match native semantics

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { cp, mkdtemp, realpath, rm, readFile, writeFile, mkdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  DiskFs,
  check,
  checkBundle,
  configureSpackleWasm,
  generate,
  render,
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

  test("check: rejects templates that use {% include %} (multi-template tag follow-up)", async () => {
    // Per-file Tera::one_off has no template registry, so cross-
    // template tags can't resolve. `check` flags them up front so the
    // limitation surfaces here rather than as a confusing render-time
    // error. See docs/design/wasm.md for the planned follow-up.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-tag-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(projectDir, { recursive: true });
    await writeFile(join(projectDir, "spackle.toml"), 'name = "incl"\n');
    await writeFile(join(projectDir, "main.j2"), '{% include "partial.j2" %}\n');
    await writeFile(join(projectDir, "partial.j2"), "static partial\n");
    const fs = new DiskFs({ workspaceRoot: root });
    const res = await check(projectDir, fs);
    const tagDiag = res.diagnostics.find((d) => d.message.includes("{% include %}"));
    expect(tagDiag).toBeDefined();
    expect(tagDiag?.source).toBe("render_body");
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
      // generate returns counts; rendered tree lives on disk.
      expect(res.files).toBeGreaterThan(0);

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

  test("generate: ignore matches by basename at any depth, not just first segment", async () => {
    // Native `copy::copy_collect` matches the ignore list against
    // each entry's basename as the walker descends, then prunes the
    // subtree. So `ignore = ["secret"]` should skip BOTH `secret/...`
    // and `sub/secret/...`. A first-segment-only check would miss
    // the nested case.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-ignore-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(join(projectDir, "secret"), { recursive: true });
    await mkdir(join(projectDir, "sub", "secret"), { recursive: true });
    await mkdir(join(projectDir, "keep"), { recursive: true });
    await writeFile(join(projectDir, "spackle.toml"), 'name = "ig"\nignore = ["secret"]\n');
    await writeFile(join(projectDir, "secret", "a.txt"), "top");
    await writeFile(join(projectDir, "sub", "secret", "b.txt"), "nested");
    await writeFile(join(projectDir, "keep", "c.txt"), "keepme");

    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    const res = await generate(projectDir, outDir, {}, fs);
    expect(res.ok).toBe(true);

    const { existsSync } = await import("node:fs");
    expect(existsSync(join(outDir, "secret"))).toBe(false);
    expect(existsSync(join(outDir, "sub", "secret"))).toBe(false);
    expect(existsSync(join(outDir, "keep", "c.txt"))).toBe(true);
  });

  test("generate: a template inside an ignored subtree still renders (native template::fill parity)", async () => {
    // Native `template::fill` walks the full project regardless of
    // the ignore list; only `copy::copy` applies the filter. So a
    // `.j2` in an ignored dir still renders, and its parent dir
    // appears in the output as a side effect of writing the template.
    // Static siblings in the same ignored dir do NOT get copied.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-ignore-tmpl-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(join(projectDir, "drafts"), { recursive: true });
    await writeFile(
      join(projectDir, "spackle.toml"),
      'name = "ig-tmpl"\nignore = ["drafts"]\n[[slots]]\nkey = "who"\ntype = "String"\n',
    );
    await writeFile(join(projectDir, "drafts", "greet.j2"), "hi {{ who }}");
    await writeFile(join(projectDir, "drafts", "static.txt"), "should not copy");

    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    const res = await generate(projectDir, outDir, { who: "world" }, fs);
    expect(res.ok).toBe(true);

    const { existsSync } = await import("node:fs");
    const rendered = await readFile(join(outDir, "drafts", "greet"), "utf8");
    expect(rendered).toBe("hi world");
    expect(existsSync(join(outDir, "drafts", "static.txt"))).toBe(false);
  });

  test("generate: a template under a directory literally named spackle.toml still renders", async () => {
    // Exotic edge case: a directory whose basename happens to be
    // `spackle.toml`. Native `copy::copy_collect` skips that basename
    // for copy, but `template::fill` walks the full tree regardless
    // and renders any `.j2` inside. Non-template siblings get
    // dropped by copy's ancestor-skip logic.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-toml-dir-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(join(projectDir, "sub", "spackle.toml"), { recursive: true });
    await writeFile(
      join(projectDir, "spackle.toml"),
      'name = "x"\n[[slots]]\nkey = "who"\ntype = "String"\n',
    );
    await writeFile(join(projectDir, "sub", "spackle.toml", "greet.j2"), "hi {{ who }}");
    await writeFile(join(projectDir, "sub", "spackle.toml", "static.txt"), "should not copy");

    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    const res = await generate(projectDir, outDir, { who: "world" }, fs);
    expect(res.ok).toBe(true);

    const { existsSync } = await import("node:fs");
    // Template under the spackle.toml-named dir renders; parent dir
    // is created as a side effect of writing the rendered template.
    const rendered = await readFile(join(outDir, "sub", "spackle.toml", "greet"), "utf8");
    expect(rendered).toBe("hi world");
    // Static sibling skipped (config-file ancestor).
    expect(existsSync(join(outDir, "sub", "spackle.toml", "static.txt"))).toBe(false);
  });

  test("generate: a static file whose rendered name ends in .j2 is still copied verbatim", async () => {
    // Native classifies templates by **source** basename: `{{ name }}`
    // is not a template because its source name doesn't end in `.j2`,
    // even if it renders to `foo.j2`. The orchestrator must match —
    // a render-time classification would wrongly route the static
    // file through render_file.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-classify-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(projectDir, { recursive: true });
    await writeFile(
      join(projectDir, "spackle.toml"),
      'name = "classify"\n[[slots]]\nkey = "name"\ntype = "String"\n',
    );
    // Source has no template ext; body has tokens that should NOT be
    // interpolated (it's a copy, not a template).
    await writeFile(join(projectDir, "{{ name }}"), "raw {{ unrelated }} body");

    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    const res = await generate(projectDir, outDir, { name: "foo.j2" }, fs);
    expect(res.ok).toBe(true);

    // Filename was path-templated to `foo.j2` — but the body must be
    // copied verbatim with `{{ unrelated }}` intact.
    const body = await readFile(join(outDir, "foo.j2"), "utf8");
    expect(body).toBe("raw {{ unrelated }} body");
  });

  test("generate: refuses a pre-existing outDir (native AlreadyExists parity)", async () => {
    const ws = await workspace("basic_project");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });

    await mkdir(ws.outDir, { recursive: true });

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

  test("generate streams a large static file through pipeline() without loading it all in memory", async () => {
    // Build a project on the fly containing a large (1 MiB) static
    // asset. The orchestrator should route it through
    // `DiskFs.streamCopy` (which uses `pipeline(createReadStream,
    // createWriteStream)`) rather than slurping it into a Uint8Array
    // and round-tripping through wasm. We assert correctness by
    // byte-comparing the copy; the streaming win is structural
    // (`generate` never sees the file's bytes).
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-large-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(projectDir, { recursive: true });
    await writeFile(join(projectDir, "spackle.toml"), 'name = "stream-test"\n');

    const payload = new Uint8Array(1024 * 1024);
    for (let i = 0; i < payload.length; i++) payload[i] = (i * 7) & 0xff;
    await writeFile(join(projectDir, "asset.bin"), payload);

    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    const res = await generate(projectDir, outDir, {}, fs);
    expect(res.ok).toBe(true);

    const back = await readFile(join(outDir, "asset.bin"));
    expect(back.byteLength).toBe(payload.byteLength);
    expect(new Uint8Array(back)).toEqual(payload);
  });

  test("generate does not create outDir on slot validation failure", async () => {
    const ws = await workspace("typed_slots");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const res = await generate(
      ws.projectDir,
      ws.outDir,
      { name: "demo", count: "not-a-number", enabled: "true" },
      fs,
    );
    expect(res.ok).toBe(false);
    const { existsSync } = await import("node:fs");
    expect(existsSync(ws.outDir)).toBe(false);
  });

  test("generate creates outDir on success even for empty projects", async () => {
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-empty-")));
    cleanup.push(root);
    const projectDir = join(root, "project");
    await mkdir(projectDir, { recursive: true });
    await writeFile(join(projectDir, "spackle.toml"), 'name = "empty"\n');
    const outDir = join(root, "output");
    const fs = new DiskFs({ workspaceRoot: root });
    const res = await generate(projectDir, outDir, {}, fs);
    expect(res.ok).toBe(true);
    const { existsSync } = await import("node:fs");
    expect(existsSync(outDir)).toBe(true);
  });
});

describe("spackle render (diagnostics-first)", () => {
  const cleanup: string[] = [];
  beforeEach(() => void (cleanup.length = 0));
  afterEach(async () => {
    await Promise.all(cleanup.map((p) => rm(p, { recursive: true, force: true })));
  });

  test("clean project: empty diagnostics, files present", async () => {
    const ws = await workspace("basic_project");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const res = await render(
      ws.projectDir,
      ws.outDir,
      { greeting: "hi", target: "world", filename: "notes" },
      fs,
    );
    expect(res.diagnostics).toEqual([]);
    expect(res.files.length).toBeGreaterThan(0);
    expect(res.hookPlan).not.toBeNull();
  });

  test("bad_template: surfaces per-file diagnostic with path", async () => {
    const ws = await workspace("bad_template");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const res = await render(ws.projectDir, ws.outDir, { defined_slot: "value" }, fs);
    const renderDiag = res.diagnostics.find((d) => d.source === "render_body");
    expect(renderDiag).toBeDefined();
    expect(renderDiag?.message).toContain("invalid_slot");
    expect(renderDiag?.path).toBeDefined();
    // No hooks defined in this fixture → empty plan, not null.
    expect(res.hookPlan).toEqual([]);
  });

  test("missing slot data: surfaces slot_data diagnostic", async () => {
    const ws = await workspace("basic_project");
    cleanup.push(ws.root);
    const fs = new DiskFs({ workspaceRoot: ws.root });
    const res = await render(ws.projectDir, ws.outDir, { greeting: "hi" }, fs);
    const slotDataDiag = res.diagnostics.find((d) => d.source === "slot_data");
    expect(slotDataDiag).toBeDefined();
  });
});

describe("checkBundle (in-memory)", () => {
  test("clean project bundle has no diagnostics", async () => {
    // checkBundle is a 1:1 pass-through over wasm.check, useful for
    // browser hosts that already have bundles in memory.
    const tomlBytes = new TextEncoder().encode(
      'name = "x"\n[[slots]]\nkey = "name"\ntype = "String"\n',
    );
    const bundle = [{ path: "/project/spackle.toml", bytes: tomlBytes }];
    const res = await checkBundle(bundle, "/project");
    expect(res.diagnostics).toEqual([]);
    expect(res.config?.name).toBe("x");
  });
});
