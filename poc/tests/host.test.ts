// HOST-SIDE tests — cover the fs and subprocess helpers against real
// tempdir fixtures. These tests do not exercise the WASM layer beyond
// what `copyNonTemplates` and `executeHookPlan` need for happy-path
// orchestration.

import { describe, expect, test, beforeAll, afterEach } from "bun:test";
import { mkdir, mkdtemp, readFile, writeFile, rm, readdir } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { loadSpackleWasm, type SpackleWasm } from "../src/wasm/index.ts";
import {
  copyNonTemplates,
  readSpackleConfig,
  walkTemplates,
  writeRenderedFiles,
} from "../src/host/fs.ts";
import { executeHookPlan, type HookOutcome } from "../src/host/hooks.ts";

let wasm: SpackleWasm;
let scratch: string;

beforeAll(async () => {
  wasm = await loadSpackleWasm();
});

afterEach(async () => {
  if (scratch) await rm(scratch, { recursive: true, force: true });
});

async function makeScratch(): Promise<string> {
  scratch = await mkdtemp(join(tmpdir(), "spackle-poc-"));
  return scratch;
}

describe("readSpackleConfig", () => {
  test("reads spackle.toml content verbatim", async () => {
    const dir = await makeScratch();
    await writeFile(join(dir, "spackle.toml"), "name = \"example\"\n");
    const content = await readSpackleConfig(dir);
    expect(content).toBe('name = "example"\n');
  });
});

describe("walkTemplates", () => {
  test("collects .j2 files with relative paths; skips non-j2 and ignore", async () => {
    const dir = await makeScratch();
    await writeFile(join(dir, "a.j2"), "aaa");
    await writeFile(join(dir, "b.txt"), "bbb");
    await mkdir(join(dir, "sub"));
    await writeFile(join(dir, "sub", "c.j2"), "ccc");
    await mkdir(join(dir, "skip-me"));
    await writeFile(join(dir, "skip-me", "d.j2"), "ddd");

    const templates = await walkTemplates(dir, ["skip-me"]);
    const paths = templates.map((t) => t.path).sort();
    expect(paths).toEqual(["a.j2", join("sub", "c.j2")]);
    expect(templates.find((t) => t.path === "a.j2")?.content).toBe("aaa");
  });
});

describe("writeRenderedFiles", () => {
  test("writes each rendered file; skips entries with an error; creates parent dirs", async () => {
    const outDir = await makeScratch();
    const count = await writeRenderedFiles(outDir, [
      { original_path: "a.j2", rendered_path: "a", content: "alpha" },
      { original_path: "sub/b.j2", rendered_path: "sub/b", content: "beta" },
      {
        original_path: "bad.j2",
        rendered_path: "bad.j2",
        content: "",
        error: "synthetic render error",
      },
    ]);
    expect(count).toBe(2);
    expect(await readFile(join(outDir, "a"), "utf-8")).toBe("alpha");
    expect(await readFile(join(outDir, "sub", "b"), "utf-8")).toBe("beta");
    const entries = await readdir(outDir);
    expect(entries).not.toContain("bad.j2");
  });
});

describe("copyNonTemplates", () => {
  test("copies non-j2 files and templates the destination filename", async () => {
    // Use two sibling tempdirs so the walker can't descend into dst
    // mid-walk (which would double-count).
    const src = await mkdtemp(join(tmpdir(), "spackle-copy-src-"));
    const dst = await mkdtemp(join(tmpdir(), "spackle-copy-dst-"));
    try {
      await writeFile(join(src, "static.txt"), "STATIC");
      await writeFile(join(src, "{{_project_name}}"), "NAMED");
      await writeFile(join(src, "spackle.toml"), ""); // should be skipped
      await writeFile(join(src, "skip.j2"), "TEMPLATE"); // should be skipped

      const copied = await copyNonTemplates(
        src,
        dst,
        [],
        { _project_name: "my-proj", _output_name: "my-out" },
        wasm,
      );
      expect(copied).toBe(2);
      expect(await readFile(join(dst, "static.txt"), "utf-8")).toBe("STATIC");
      expect(await readFile(join(dst, "my-proj"), "utf-8")).toBe("NAMED");
      const entries = await readdir(dst);
      expect(entries).not.toContain("spackle.toml");
      expect(entries).not.toContain("skip.j2");
      expect(entries).not.toContain("{{_project_name}}");
    } finally {
      await rm(src, { recursive: true, force: true });
      await rm(dst, { recursive: true, force: true });
    }
  });
});

describe("executeHookPlan", () => {
  test("runs should_run=true entries, yields skipped for the rest", async () => {
    const dir = await makeScratch();
    const outcomes: HookOutcome[] = [];
    for await (const outcome of executeHookPlan(
      [
        { key: "runs", command: ["echo", "hello"], should_run: true },
        {
          key: "skips",
          command: ["echo", "never"],
          should_run: false,
          skip_reason: "false_conditional",
        },
      ],
      dir,
    )) {
      outcomes.push(outcome);
    }

    expect(outcomes).toHaveLength(2);

    const runs = outcomes[0];
    expect(runs.kind).toBe("ran");
    if (runs.kind !== "ran") throw new Error("unreachable");
    expect(runs.result.key).toBe("runs");
    expect(runs.result.exit_code).toBe(0);
    expect(runs.result.stdout.trim()).toBe("hello");

    const skips = outcomes[1];
    expect(skips.kind).toBe("skipped");
    if (skips.kind !== "skipped") throw new Error("unreachable");
    expect(skips.info.key).toBe("skips");
    expect(skips.info.skip_reason).toBe("false_conditional");
  });

  test("captures stderr separately from stdout", async () => {
    const dir = await makeScratch();
    const outcomes: HookOutcome[] = [];
    for await (const outcome of executeHookPlan(
      [
        {
          key: "mixed",
          command: ["bash", "-c", "echo out; echo err 1>&2"],
          should_run: true,
        },
      ],
      dir,
    )) {
      outcomes.push(outcome);
    }
    const ran = outcomes[0];
    if (ran.kind !== "ran") throw new Error("expected ran");
    expect(ran.result.stdout.trim()).toBe("out");
    expect(ran.result.stderr.trim()).toBe("err");
  });
});
