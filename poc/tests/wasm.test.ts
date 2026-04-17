// WASM-SIDE tests — exercise each `#[wasm_bindgen]` export through the
// typed wrapper. Proves the WASM artifact is loadable by Bun and that
// every export's JSON contract matches the TypeScript interface.
//
// These tests DO NOT touch the filesystem except to read the fixture
// spackle.toml and template files — `walkTemplates` is tested separately
// in host.test.ts.

import { describe, expect, test, beforeAll } from "bun:test";
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { loadSpackleWasm, type SpackleWasm } from "../src/wasm/index.ts";
import { walkTemplates } from "../src/host/fs.ts";

const REPO_ROOT = join(import.meta.dir, "..", "..");
const PROJ1 = join(REPO_ROOT, "tests", "data", "proj1");
const PROJ2 = join(REPO_ROOT, "tests", "data", "proj2");
const HOOK = join(REPO_ROOT, "tests", "data", "hook");
const HOOK_RAN_COND = join(REPO_ROOT, "tests", "data", "hook_ran_cond");

let wasm: SpackleWasm;

beforeAll(async () => {
  wasm = await loadSpackleWasm();
});

describe("parseConfig", () => {
  test("parses proj1 toml into structured config", async () => {
    const toml = await readFile(join(PROJ1, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    expect(cfg.slots).toHaveLength(3);
    expect(cfg.slots.map((s) => s.key).sort()).toEqual(["slot_1", "slot_2", "slot_3"]);
    expect(cfg.hooks).toHaveLength(2);
  });

  test("throws for invalid toml", () => {
    expect(() => wasm.parseConfig("[[[ broken")).toThrow(/parseConfig/);
  });
});

describe("validateConfig", () => {
  test("valid proj1", async () => {
    const toml = await readFile(join(PROJ1, "spackle.toml"), "utf-8");
    expect(wasm.validateConfig(toml).valid).toBe(true);
  });

  test("duplicate keys returns valid=false with errors array", () => {
    const bad = `[[slots]]\nkey = "x"\n[[hooks]]\nkey = "x"\ncommand = ["true"]\n`;
    const res = wasm.validateConfig(bad);
    expect(res.valid).toBe(false);
    expect(res.errors?.length).toBeGreaterThan(0);
  });
});

describe("checkProject", () => {
  test("proj2 is fully valid", async () => {
    const toml = await readFile(join(PROJ2, "spackle.toml"), "utf-8");
    const templates = await walkTemplates(PROJ2, []);
    expect(wasm.checkProject(toml, templates).valid).toBe(true);
  });

  test("always returns {valid, errors} shape even on malformed input", () => {
    const res = wasm.checkProject("[[[ broken", []);
    expect(res.valid).toBe(false);
    expect(Array.isArray(res.errors)).toBe(true);
  });
});

describe("validateSlotData", () => {
  test("accepts valid slot_1/slot_2/slot_3 values", async () => {
    const toml = await readFile(join(PROJ1, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    const res = wasm.validateSlotData(JSON.stringify(cfg), {
      slot_1: "hi",
      slot_2: "42",
      slot_3: "true",
    });
    expect(res.valid).toBe(true);
  });

  test("rejects wrong type for Number slot", async () => {
    const toml = await readFile(join(PROJ1, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    const res = wasm.validateSlotData(JSON.stringify(cfg), {
      slot_1: "hi",
      slot_2: "not-a-number",
      slot_3: "true",
    });
    expect(res.valid).toBe(false);
  });
});

describe("renderTemplates", () => {
  test("renders proj2 template with filename untouched", async () => {
    const toml = await readFile(join(PROJ2, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    const templates = await walkTemplates(PROJ2, []);
    const rendered = wasm.renderTemplates(
      templates,
      { defined_field: "howdy" },
      JSON.stringify(cfg),
    );
    const good = rendered.find((r) => r.original_path === "good.j2");
    expect(good).toBeDefined();
    expect(good!.rendered_path).toBe("good");
    expect(good!.content).toBe("howdy");
    expect(good!.error).toBeUndefined();
  });

  test("surfaces per-file errors for undefined vars without failing the batch", async () => {
    const toml = await readFile(join(PROJ1, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    const templates = await walkTemplates(PROJ1, []);
    const rendered = wasm.renderTemplates(
      templates,
      { slot_1: "a", slot_2: "1", slot_3: "true" },
      JSON.stringify(cfg),
    );
    // bad.j2 references {{ undefined_field }} — should show up as an error
    // entry but NOT blow up the whole call.
    const bad = rendered.find((r) => r.original_path.endsWith("bad.j2"));
    expect(bad).toBeDefined();
    expect(bad!.error).toBeDefined();
    // slot_1-named file should have rendered fine.
    const good = rendered.find((r) => r.rendered_path === "a");
    expect(good).toBeDefined();
    expect(good!.error).toBeUndefined();
  });
});

describe("evaluateHooks", () => {
  test("plans all hooks for the 'hook' fixture", async () => {
    const toml = await readFile(join(HOOK, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    const plan = wasm.evaluateHooks(JSON.stringify(cfg), {});
    expect(plan).toHaveLength(3);
    expect(plan.map((h) => h.key).sort()).toEqual(["hook_1", "hook_2", "hook_3"]);
    expect(plan.every((h) => Array.isArray(h.command))).toBe(true);
  });

  test("conditional hooks honor the hook_ran_<key> injection", async () => {
    const toml = await readFile(join(HOOK_RAN_COND, "spackle.toml"), "utf-8");
    const cfg = wasm.parseConfig(toml);
    const plan = wasm.evaluateHooks(JSON.stringify(cfg), {});
    // dep_hook_should_run has `if = "{{ hook_ran_hook_1 }}"` and hook_1
    // defaults to running — so the dependent should_run should also be true.
    const depRun = plan.find((h) => h.key === "dep_hook_should_run");
    expect(depRun).toBeDefined();
    expect(depRun!.should_run).toBe(true);
  });

  test("template errors mark should_run=false with skip_reason", () => {
    const cfgJson = JSON.stringify({
      name: null,
      ignore: [],
      slots: [],
      hooks: [
        { key: "bad", command: ["echo", "{{ undefined }}"], default: true, needs: [] },
      ],
    });
    const plan = wasm.evaluateHooks(cfgJson, {});
    expect(plan[0]!.should_run).toBe(false);
    expect(plan[0]!.skip_reason).toBe("template_error");
    expect(plan[0]!.template_errors?.length).toBeGreaterThan(0);
  });
});

describe("renderString (one-off template)", () => {
  test("substitutes variables in a path-like string", () => {
    expect(wasm.renderString("{{ _project_name }}/file", { _project_name: "p" }))
      .toBe("p/file");
  });

  test("throws on undefined variable", () => {
    expect(() => wasm.renderString("{{ nope }}", {})).toThrow(/renderString/);
  });
});

describe("getOutputName + getProjectName", () => {
  test("getOutputName returns the last path segment", () => {
    expect(wasm.getOutputName("/tmp/my-output")).toBe("my-output");
    expect(wasm.getOutputName("/")).toBe("project");
  });

  test("getProjectName: config.name wins", () => {
    const cfg = JSON.stringify({
      name: "from-config",
      ignore: [],
      slots: [],
      hooks: [],
    });
    expect(wasm.getProjectName(cfg, "/ignored")).toBe("from-config");
  });

  test("getProjectName: falls back to project_dir file_stem", () => {
    const cfg = JSON.stringify({ name: null, ignore: [], slots: [], hooks: [] });
    expect(wasm.getProjectName(cfg, "/tmp/my-project")).toBe("my-project");
  });
});
