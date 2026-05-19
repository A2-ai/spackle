// End-to-end tests — exercise check/validateSlotData/generate through
// the bundle-in / bundle-out API. DiskFs covers the disk-backed flow,
// checkBundle / generateBundle covers the in-memory flow.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { cp, mkdtemp, realpath, rm, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import {
  type Bundle,
  DiskFs,
  MemoryFs,
  check,
  checkBundle,
  configureSpackleWasm,
  generate,
  generateBundle,
  generateStream,
  loadSpackleWasm,
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
      // Streaming generate returns counts, not a materialized bundle —
      // verify the rendered tree is actually on disk.
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

describe("spackle (streaming generate)", () => {
  async function basicBundle() {
    return bundleFromDisk(
      ["spackle.toml", "README.md.j2", "docs/static.md", "src/{{ filename }}.txt.j2"],
      join(FIXTURES, "basic_project"),
      "/project",
    );
  }

  test("generateStream yields entries before a terminal done event", async () => {
    const bundle = await basicBundle();

    const events: Array<{ kind: string; path?: string }> = [];
    let bytesSeen = 0;
    for await (const e of generateStream(bundle, {
      greeting: "hi",
      target: "stream",
      filename: "notes",
    })) {
      if (e.kind === "file") {
        events.push({ kind: e.kind, path: e.path });
        bytesSeen += e.bytes.length;
      } else if (e.kind === "dir") {
        events.push({ kind: e.kind, path: e.path });
      } else {
        events.push({ kind: e.kind });
      }
    }
    // Must terminate with `done`, not `error`.
    expect(events[events.length - 1]).toEqual({ kind: "done" });

    // README rendered through the stream — bytes flowed.
    const readme = events.find((e) => e.kind === "file" && e.path === "README.md");
    expect(readme).toBeDefined();
    expect(bytesSeen).toBeGreaterThan(0);
  });

  test("generateStream emits parent dirs before any child file", async () => {
    const bundle = await basicBundle();

    let firstDocFile = -1;
    let docDirIdx = -1;
    let i = 0;
    for await (const e of generateStream(bundle, {
      greeting: "hi",
      target: "stream",
      filename: "notes",
    })) {
      if (e.kind === "dir" && e.path === "docs") docDirIdx = i;
      if (e.kind === "file" && e.path.startsWith("docs/") && firstDocFile === -1) firstDocFile = i;
      i++;
    }
    expect(docDirIdx).toBeGreaterThanOrEqual(0);
    expect(firstDocFile).toBeGreaterThanOrEqual(0);
    expect(docDirIdx).toBeLessThan(firstDocFile);
  });

  test("disk-streaming generate aborts when a host write throws (no rollback)", async () => {
    // The disk-streaming `generate(...fs)` writes synchronously inside
    // the wasm callback. If a write throws (e.g., disk full), the
    // CallbackFs latches the error and Rust short-circuits the rest of
    // the pipeline — the wasm export returns ok:false and any files
    // already written stay on disk (no rollback). This test simulates
    // a failure by wrapping DiskFs.writeEntry to throw on the 2nd call.
    const ws = await workspace("basic_project");
    try {
      const fs = new DiskFs({ workspaceRoot: ws.root });
      const realWriteEntry = fs.writeEntry.bind(fs);
      let callCount = 0;
      // oxlint-disable-next-line typescript-eslint/no-unsafe-type-assertion
      (fs as unknown as { writeEntry: typeof fs.writeEntry }).writeEntry = (outDir, entry) => {
        callCount++;
        if (callCount === 2) {
          throw new Error("simulated disk failure");
        }
        realWriteEntry(outDir, entry);
      };

      const res = await generate(
        ws.projectDir,
        ws.outDir,
        { greeting: "hi", target: "x", filename: "notes" },
        fs,
      );
      expect(res.ok).toBe(false);
      if (!res.ok) {
        expect(res.error).toMatch(/simulated disk failure/);
      }
    } finally {
      await rm(ws.root, { recursive: true, force: true });
    }
  });

  test("generate does not create outDir on slot validation failure", async () => {
    // Native parity: Project::generate validates config + slot data
    // BEFORE copy::copy creates the destination. Our streaming wrapper
    // must defer outDir creation until the first event so wasm-side
    // validation failures don't leave an empty directory on disk.
    const ws = await workspace("typed_slots");
    try {
      const fs = new DiskFs({ workspaceRoot: ws.root });
      const res = await generate(
        ws.projectDir,
        ws.outDir,
        // count is declared as Number — passing a non-numeric string
        // is a slot validation failure, surfaced before any walk
        // happens in Rust.
        { name: "demo", count: "not-a-number", enabled: "true" },
        fs,
      );
      expect(res.ok).toBe(false);
      // existsSync is imported from node:fs at top of test file.
      const { existsSync } = await import("node:fs");
      expect(existsSync(ws.outDir)).toBe(false);
    } finally {
      await rm(ws.root, { recursive: true, force: true });
    }
  });

  test("generate creates outDir on success even for empty projects", async () => {
    // Native `copy::copy` unconditionally calls create_dir_all(dest),
    // so even an empty project produces an empty outDir. Wasm streams
    // skip the out_root event, so the disk wrapper must mkdir on
    // success to preserve parity.
    const root = await realpath(await mkdtemp(join(tmpdir(), "spackle-empty-")));
    try {
      const projectDir = join(root, "project");
      await import("node:fs/promises").then((m) => m.mkdir(projectDir, { recursive: true }));
      // Minimal valid spackle.toml — no slots, no files to render.
      await import("node:fs/promises").then((m) =>
        m.writeFile(join(projectDir, "spackle.toml"), 'name = "empty"\n'),
      );
      const outDir = join(root, "output");
      const fs = new DiskFs({ workspaceRoot: root });
      const res = await generate(projectDir, outDir, {}, fs);
      expect(res.ok).toBe(true);
      const { existsSync } = await import("node:fs");
      expect(existsSync(outDir)).toBe(true);
    } finally {
      await rm(root, { recursive: true, force: true });
    }
  });

  test("generateBundle dedupes overlapping copy + template paths (template wins)", async () => {
    // Project::generate runs copy::copy first then template::fill.
    // When both write to the same output path (e.g., a project with
    // `foo` and `foo.j2` rendering to `foo`), the streaming events
    // include two file entries for `foo`. The buffered generateBundle
    // wrapper must collapse them — last-write wins, matching
    // disk-streaming's writeFileSync overwrite semantics.
    const bundle: Bundle = [
      {
        path: "/project/spackle.toml",
        bytes: new TextEncoder().encode(
          'name = "overlap"\n[[slots]]\nkey = "x"\ntype = "String"\n',
        ),
      },
      {
        path: "/project/foo",
        bytes: new TextEncoder().encode("static-foo"),
      },
      {
        path: "/project/foo.j2",
        bytes: new TextEncoder().encode("rendered-{{ x }}"),
      },
    ];
    const res = await generateBundle(bundle, { x: "value" });
    expect(res.ok).toBe(true);
    if (res.ok) {
      const fooEntries = res.files.filter((f) => f.path === "foo");
      expect(fooEntries.length).toBe(1);
      // template's render runs second in core, so it wins.
      expect(new TextDecoder().decode(fooEntries[0].bytes)).toBe("rendered-value");
    }
  });

  test("generateBundle returns files and dirs sorted by path", async () => {
    // Streaming order depends on HashMap iteration in MemoryFs's
    // list_dir, which Rust does not guarantee to be stable. The old
    // drain_subtree path explicitly sorted; the buffered wrapper must
    // do the same so snapshots / downstream consumers see deterministic
    // output.
    const bundle = await bundleFromDisk(
      ["spackle.toml", "README.md.j2", "docs/static.md", "src/{{ filename }}.txt.j2"],
      join(FIXTURES, "basic_project"),
      "/project",
    );
    const res = await generateBundle(bundle, {
      greeting: "hi",
      target: "sort",
      filename: "notes",
    });
    expect(res.ok).toBe(true);
    if (res.ok) {
      const filePaths = res.files.map((f) => f.path);
      const sortedFiles = filePaths.toSorted();
      expect(filePaths).toEqual(sortedFiles);

      const sortedDirs = res.dirs.toSorted();
      expect(res.dirs).toEqual(sortedDirs);
    }
  });

  test("wasm.generate surfaces a thrown host callback as the response error", async () => {
    // Direct wasm-level test: a callback that throws should latch
    // wasm-side and come back as { ok: false, error }. Subsequent
    // entries do not trigger the callback.
    const bundle = await basicBundle();
    const wasm = await loadSpackleWasm();
    let calls = 0;
    const result = wasm.generate(
      bundle,
      "/project",
      "/output",
      { greeting: "hi", target: "x", filename: "notes" },
      () => {
        calls++;
        throw new Error("host boom");
      },
    );
    expect(result.ok).toBe(false);
    if (!result.ok) {
      expect(result.error).toMatch(/host boom/);
    }
    expect(calls).toBe(1);
  });
});
