// MemoryFs tests — the post-pivot MemoryFs is a pure TS bundle holder,
// not a `SpackleFs` adapter. These tests pin the bundle-conversion
// semantics + seed helpers used by preview / in-memory flows.

import { describe, expect, test } from "bun:test";

import { MemoryFs } from "../src/spackle.ts";

describe("MemoryFs", () => {
  test("insertFile + get round-trip", () => {
    const fs = new MemoryFs();
    fs.insertFile("/ws/a.txt", "hello");
    expect(new TextDecoder().decode(fs.get("/ws/a.txt"))).toBe("hello");
  });

  test("rejects relative paths", () => {
    const fs = new MemoryFs();
    expect(() => fs.insertFile("relative", "x")).toThrow();
  });

  test("toBundle produces sorted bundle entries", () => {
    const fs = new MemoryFs({
      files: {
        "/b.txt": "B",
        "/a.txt": "A",
        "/sub/c.txt": "C",
      },
    });
    const bundle = fs.toBundle();
    expect(bundle.map((e) => e.path)).toEqual(["/a.txt", "/b.txt", "/sub/c.txt"]);
    expect(new TextDecoder().decode(bundle[0].bytes)).toBe("A");
  });

  test("fromBundle reconstructs a MemoryFs", () => {
    const bundle = [
      { path: "x.txt", bytes: new TextEncoder().encode("X") },
      { path: "sub/y.txt", bytes: new TextEncoder().encode("Y") },
    ];
    const fs = MemoryFs.fromBundle(bundle, "/output");
    expect(fs.has("/output/x.txt")).toBe(true);
    expect(new TextDecoder().decode(fs.get("/output/sub/y.txt"))).toBe("Y");
  });

  test("snapshot exposes current files map", () => {
    const fs = new MemoryFs({ files: { "/x/y.txt": "data" } });
    const snap = fs.snapshot();
    expect(Object.keys(snap.files)).toContain("/x/y.txt");
  });

  test("seed accepts string and Uint8Array content", () => {
    const fs = new MemoryFs({
      files: {
        "/str.txt": "hi",
        "/bin.bin": new Uint8Array([1, 2, 3]),
      },
    });
    expect(fs.get("/bin.bin")).toEqual(new Uint8Array([1, 2, 3]));
    expect(new TextDecoder().decode(fs.get("/str.txt"))).toBe("hi");
  });
});
