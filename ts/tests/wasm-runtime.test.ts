// Focused unit tests for `createSpackleWasmLoader`. Exercise the loader
// state machine in isolation with a mock `RawWasmExports` so we can assert
// the default-path, override-path, throw-after-load, and unconfigured-throw
// branches independently of the package-level singletons that the
// end-to-end suites in `spackle.test.ts` / `hooks.test.ts` configure at
// module load time.

import { describe, expect, test } from "bun:test";

import { createSpackleWasmLoader, type RawWasmExports } from "../src/wasm/runtime.ts";

interface MockRaw extends RawWasmExports {
  initWasmCalls: unknown[];
}

function mockRaw(): MockRaw {
  const initWasmCalls: unknown[] = [];
  return {
    initWasmCalls,
    initWasm: (arg) => {
      initWasmCalls.push(arg);
      return Promise.resolve(undefined);
    },
    check: () =>
      `{"valid":true,"config":{"name":null,"ignore":[],"slots":[],"hooks":[]},"errors":[]}`,
    validateSlotData: () => `{"valid":true}`,
    generate: () => ({ ok: true, files: [], dirs: [] }),
    planHooks: () => `{"ok":true,"plan":[]}`,
  };
}

describe("createSpackleWasmLoader", () => {
  test("loadSpackleWasm passes the default moduleOrPath to initWasm when no override is set", async () => {
    const raw = mockRaw();
    const defaultUrl = new URL("file:///fake/default.wasm");
    const { loadSpackleWasm } = createSpackleWasmLoader(raw, defaultUrl);

    await loadSpackleWasm();

    expect(raw.initWasmCalls).toEqual([{ module_or_path: defaultUrl }]);
  });

  test("configureSpackleWasm overrides the default", async () => {
    const raw = mockRaw();
    const { loadSpackleWasm, configureSpackleWasm } = createSpackleWasmLoader(
      raw,
      new URL("file:///fake/default.wasm"),
    );

    const overrideBytes = new Uint8Array([0, 1, 2]);
    configureSpackleWasm({ moduleOrPath: overrideBytes });
    await loadSpackleWasm();

    expect(raw.initWasmCalls).toEqual([{ module_or_path: overrideBytes }]);
  });

  test("configureSpackleWasm throws once loadSpackleWasm has been called", async () => {
    const raw = mockRaw();
    const { loadSpackleWasm, configureSpackleWasm } = createSpackleWasmLoader(
      raw,
      new URL("file:///fake/default.wasm"),
    );

    await loadSpackleWasm();

    expect(() => configureSpackleWasm({ moduleOrPath: new Uint8Array() })).toThrow(
      /must be called before loadSpackleWasm/,
    );
  });

  test("loadSpackleWasm throws when neither a default nor a configured source is present", () => {
    const raw = mockRaw();
    const { loadSpackleWasm } = createSpackleWasmLoader(raw);

    expect(() => loadSpackleWasm()).toThrow(/not configured/);
    expect(raw.initWasmCalls).toEqual([]);
  });

  test("loadSpackleWasm succeeds after configureSpackleWasm when there is no default", async () => {
    const raw = mockRaw();
    const { loadSpackleWasm, configureSpackleWasm } = createSpackleWasmLoader(raw);
    const bytes = new Uint8Array([42]);

    configureSpackleWasm({ moduleOrPath: bytes });
    await loadSpackleWasm();

    expect(raw.initWasmCalls).toEqual([{ module_or_path: bytes }]);
  });

  test("loadSpackleWasm caches: concurrent and sequential callers share one initWasm call", async () => {
    const raw = mockRaw();
    const { loadSpackleWasm } = createSpackleWasmLoader(raw, new URL("file:///fake/default.wasm"));

    const a = loadSpackleWasm();
    const b = loadSpackleWasm();
    expect(a).toBe(b);

    await Promise.all([a, b, loadSpackleWasm()]);
    expect(raw.initWasmCalls).toHaveLength(1);
  });
});
