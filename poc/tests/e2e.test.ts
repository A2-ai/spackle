// E2E tests — drive the orchestration layer (`check`, `generate`) end-to-end
// against real fixtures in `tests/data/`. Proves the WASM+host composition
// produces spackle-equivalent output (including hook execution).

import { describe, expect, test, beforeAll, afterEach } from "bun:test";
import { mkdtemp, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { check, generate, TemplateRenderError } from "../src/spackle.ts";
import { loadSpackleWasm, type SpackleWasm } from "../src/wasm/index.ts";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const DATA = join(REPO_ROOT, "tests", "data");

let wasm: SpackleWasm;
let out: string;

beforeAll(async () => {
  wasm = await loadSpackleWasm();
  void wasm; // eager-load so the first test isn't slow
});

afterEach(async () => {
  if (out) await rm(out, { recursive: true, force: true });
});

async function mkOut(label: string): Promise<string> {
  out = await mkdtemp(join(tmpdir(), `spackle-e2e-${label}-`));
  await rm(out, { recursive: true, force: true });
  return out;
}

describe("check", () => {
  test("proj2: valid, returns config", async () => {
    const result = await check(join(DATA, "proj2"));
    expect(result.valid).toBe(true);
    expect(result.config).not.toBeNull();
    expect(result.config?.slots[0]?.key).toBe("defined_field");
  });

  test("bad_default_slot_val: invalid with slot-type error", async () => {
    const result = await check(join(DATA, "bad_default_slot_val"));
    expect(result.valid).toBe(false);
    expect(result.errors?.join(" ")).toMatch(/type mismatch/i);
  });
});

describe("generate: proj2 (clean happy path)", () => {
  test("renders good.j2 and copies subdir/file.txt", async () => {
    const outDir = await mkOut("proj2");
    const result = await generate(
      join(DATA, "proj2"),
      { defined_field: "hello" },
      outDir,
    );
    expect(result.written).toBe(1);
    expect(result.copied).toBe(1);
    expect(result.plan).toHaveLength(0);
    expect(await readFile(join(outDir, "good"), "utf-8")).toBe("hello");
    // subdir/file.txt should have been copied verbatim.
    const copied = await readFile(join(outDir, "subdir", "file.txt"), "utf-8");
    expect(copied.length).toBeGreaterThan(0);
  });
});

describe("generate: universal (filename-templated non-template)", () => {
  test("renders {{_output_name}}.j2 and {{_project_name}}.j2 to use computed specials", async () => {
    const outDir = await mkOut("universal");
    const result = await generate(join(DATA, "universal"), {}, outDir);
    // Both templates reference _output_name / _project_name — the WASM
    // layer renders the *contents*, the TS host renders the *filenames*
    // via renderString (identical behavior to copy::copy).
    expect(result.written).toBe(2);
    // The output directory is a mkdtemp(), so _output_name is the tempdir basename.
    const outputName = result.slotData._output_name!;
    const projectName = result.slotData._project_name!;
    expect(await readFile(join(outDir, outputName), "utf-8")).toBe(outputName);
    expect((await readFile(join(outDir, projectName), "utf-8")).length).toBeGreaterThan(0);
  });
});

describe("generate: hook fixture with runHooks", () => {
  test("spawns each should_run hook and captures stdout/stderr", async () => {
    const outDir = await mkOut("hook");
    const result = await generate(join(DATA, "hook"), {}, outDir, {
      runHooks: true,
    });
    const ran = result.hookOutcomes.filter((o) => o.kind === "ran");
    expect(ran.length).toBeGreaterThanOrEqual(2);
    // hook_1 and hook_2 both write to both stdout and stderr.
    const hook1 = ran.find(
      (o) => o.kind === "ran" && o.result.key === "hook_1",
    );
    expect(hook1?.kind).toBe("ran");
    if (hook1?.kind === "ran") {
      expect(hook1.result.exit_code).toBe(0);
      expect(hook1.result.stdout).toContain("stdout");
      expect(hook1.result.stderr).toContain("stderr");
    }
    // outDir should exist even though the hook fixture has no templates.
    const s = await stat(outDir);
    expect(s.isDirectory()).toBe(true);
  });
});

describe("generate: invalid slot data fails fast", () => {
  test("throws before touching disk when validateSlotData fails", async () => {
    const outDir = await mkOut("invalid");
    await expect(
      generate(
        join(DATA, "proj1"),
        { slot_1: "ok", slot_2: "not-a-number", slot_3: "true" },
        outDir,
      ),
    ).rejects.toThrow(/slot data invalid/);
    // Lock the "before touching disk" guarantee: outDir must not exist.
    await expect(stat(outDir)).rejects.toMatchObject({ code: "ENOENT" });
  });
});

describe("generate: outDir-exists protection (native parity)", () => {
  test("throws when outDir already exists (default: overwrite=false)", async () => {
    const outDir = await mkdtemp(join(tmpdir(), "spackle-exists-"));
    try {
      // Drop a sentinel file so we can verify the generate call truly
      // aborts before touching outDir's contents.
      await writeFile(join(outDir, "sentinel.txt"), "UNTOUCHED");
      await expect(
        generate(join(DATA, "proj2"), { defined_field: "x" }, outDir),
      ).rejects.toThrow(/already exists/);
      // No writes, no copies — sentinel is unchanged and no rendered
      // output appeared alongside it.
      expect(await readFile(join(outDir, "sentinel.txt"), "utf-8")).toBe("UNTOUCHED");
      await expect(stat(join(outDir, "good"))).rejects.toMatchObject({ code: "ENOENT" });
    } finally {
      await rm(outDir, { recursive: true, force: true });
    }
  });

  test("proceeds when overwrite: true", async () => {
    const outDir = await mkdtemp(join(tmpdir(), "spackle-overwrite-"));
    try {
      await writeFile(join(outDir, "pre-existing"), "keep-me");
      const result = await generate(
        join(DATA, "proj2"),
        { defined_field: "x" },
        outDir,
        { overwrite: true },
      );
      expect(result.written).toBe(1);
      // Pre-existing content should still be there (we don't wipe outDir).
      expect(await readFile(join(outDir, "pre-existing"), "utf-8")).toBe("keep-me");
    } finally {
      await rm(outDir, { recursive: true, force: true });
    }
  });
});

describe("generate: template-error fail-fast (native parity)", () => {
  test("throws on first error, references only that entry, attaches full batch", async () => {
    // Use an inline fixture with two failing templates whose names
    // cannot be substrings of each other — proj1 would give us
    // `bad.j2` + `subdir/bad.j2`, where `bad.j2` is a substring of the
    // longer path, making the "message doesn't mention the other" check
    // flaky depending on which one WASM iterates first.
    const projectDir = await mkdtemp(join(tmpdir(), "spackle-first-err-src-"));
    const outDir = join(
      await mkdtemp(join(tmpdir(), "spackle-first-err-parent-")),
      "out",
    );
    try {
      await writeFile(
        join(projectDir, "spackle.toml"),
        `[[slots]]\nkey = "ok"\n`,
      );
      await writeFile(join(projectDir, "alpha.j2"), "{{ missing_one }}");
      await writeFile(join(projectDir, "bravo.j2"), "{{ missing_two }}");

      let caught: unknown;
      try {
        await generate(projectDir, { ok: "yes" }, outDir);
      } catch (e) {
        caught = e;
      }
      expect(caught).toBeInstanceOf(TemplateRenderError);
      const err = caught as TemplateRenderError;

      // Both templates failed; the thrown error references exactly one of
      // them — whichever WASM iterated first. (Matches native
      // `Project::generate` which returns on the first Err encountered.)
      expect(["alpha.j2", "bravo.j2"]).toContain(err.original_path);
      expect(err.message).toContain(err.original_path);
      const other = err.original_path === "alpha.j2" ? "bravo.j2" : "alpha.j2";
      expect(err.message).not.toContain(other);

      // The full batch is attached so callers that want every failure can
      // reach them without rerunning — both alpha and bravo are present
      // in err.rendered with errors.
      const failed = err.rendered.filter((r) => r.error).map((r) => r.original_path);
      expect(failed.sort()).toEqual(["alpha.j2", "bravo.j2"]);
    } finally {
      await rm(projectDir, { recursive: true, force: true });
      await rm(outDir, { recursive: true, force: true });
    }
  });

  test("does not touch disk when failing fast", async () => {
    const outDir = await mkOut("render-fail-no-disk");
    await expect(
      generate(
        join(DATA, "proj1"),
        { slot_1: "a", slot_2: "1", slot_3: "true" },
        outDir,
      ),
    ).rejects.toBeInstanceOf(TemplateRenderError);
    // outDir must not exist — the error came before any mkdir/write/copy.
    await expect(stat(outDir)).rejects.toMatchObject({ code: "ENOENT" });
  });

  test("allowTemplateErrors: true writes only the successful entries", async () => {
    const outDir = await mkOut("render-skip");
    const result = await generate(
      join(DATA, "proj1"),
      { slot_1: "a", slot_2: "1", slot_3: "true" },
      outDir,
      { allowTemplateErrors: true },
    );
    // At least one template renders fine (a.j2 → a, etc.), bad.j2 errors.
    expect(result.written).toBeGreaterThan(0);
    const badEntry = result.rendered.find((r) => r.original_path.endsWith("bad.j2"));
    expect(badEntry?.error).toBeDefined();
  });
});

describe("generate: copy→render precedence (native parity)", () => {
  test("a template at path 'x.j2' overwrites a plain file at 'x'", async () => {
    // Build an inline fixture where a plain file and a template resolve
    // to the same destination path. Native behavior: template wins.
    const projectDir = await mkdtemp(join(tmpdir(), "spackle-prec-src-"));
    const outDir = join(
      await mkdtemp(join(tmpdir(), "spackle-prec-parent-")),
      "out",
    );
    try {
      await writeFile(
        join(projectDir, "spackle.toml"),
        `[[slots]]\nkey = "msg"\n`,
      );
      await writeFile(join(projectDir, "collide"), "FROM_PLAIN_COPY");
      await writeFile(join(projectDir, "collide.j2"), "FROM_TEMPLATE={{ msg }}");

      const result = await generate(projectDir, { msg: "hello" }, outDir);
      expect(result.copied).toBe(1);
      expect(result.written).toBe(1);

      // Template must win — if the order were reversed, the copy would clobber
      // the rendered file instead.
      const contents = await readFile(join(outDir, "collide"), "utf-8");
      expect(contents).toBe("FROM_TEMPLATE=hello");
    } finally {
      await rm(projectDir, { recursive: true, force: true });
      await rm(outDir, { recursive: true, force: true });
    }
  });
});
