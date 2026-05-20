// DiskFs tests — post-shrink DiskFs is a workspace-root boundary plus
// a handful of per-file disk helpers (`readFile`, `writeFile`,
// `streamCopy`) that the TS orchestrator (`generate` / `render` / etc.)
// calls into. These tests pin:
//   - workspaceRoot canonicalization + containment
//   - `assertOutDirAvailable` / `ensureOutDir` semantics
//   - `containedJoin` rejects path-traversal under outDir
//   - `streamCopy` round-trips bytes through `pipeline()`

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, realpath, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { DiskFs } from "../src/spackle.ts";

describe("DiskFs", () => {
  let root: string;

  beforeEach(async () => {
    root = await realpath(await mkdtemp(join(tmpdir(), "spackle-disk-")));
  });
  afterEach(async () => {
    await rm(root, { recursive: true, force: true });
  });

  test("constructor canonicalizes and exposes workspaceRoot", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(fs.workspaceRoot).toBe(root);
  });

  test("containProject accepts paths under workspaceRoot", async () => {
    await mkdir(join(root, "project"), { recursive: true });
    const fs = new DiskFs({ workspaceRoot: root });
    expect(fs.containProject(join(root, "project"))).toBe(join(root, "project"));
  });

  test("containProject rejects paths outside workspaceRoot", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() => fs.containProject("/etc")).toThrow();
  });

  test("assertOutDirAvailable returns canonical path when outDir is absent", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const absOut = fs.assertOutDirAvailable(join(root, "output"));
    expect(absOut).toBe(join(root, "output"));
    // No directory created — `generate` mkdirs lazily on first write.
    expect(existsSync(absOut)).toBe(false);
  });

  test("assertOutDirAvailable throws when outDir already exists", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    await mkdir(join(root, "out"), { recursive: true });
    expect(() => fs.assertOutDirAvailable(join(root, "out"))).toThrow(/already exists/);
  });

  test("assertOutDirAvailable rejects outDir outside workspaceRoot", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() => fs.assertOutDirAvailable("/etc/escape")).toThrow();
  });

  test("ensureOutDir is idempotent and creates the dir", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "out");
    fs.ensureOutDir(outDir);
    expect(existsSync(outDir)).toBe(true);
    // Second call must NOT throw — idempotent is the whole point.
    expect(() => fs.ensureOutDir(outDir)).not.toThrow();
  });

  test("containedJoin rejects `../` traversal", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const out = join(root, "out");
    expect(() => fs.containedJoin(out, "../escape.txt")).toThrow(/escapes outDir/);
  });

  test("containedJoin rejects `../../` traversal", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const out = join(root, "out");
    expect(() => fs.containedJoin(out, "../../escape.txt")).toThrow(/escapes outDir/);
  });

  test("containedJoin rejects absolute entry paths", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const out = join(root, "out");
    expect(() => fs.containedJoin(out, "/etc/pwned")).toThrow(/escapes outDir/);
  });

  test("containedJoin resolves a clean relative path under outDir", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const out = join(root, "out");
    expect(fs.containedJoin(out, "sub/file.txt")).toBe(join(out, "sub", "file.txt"));
  });

  // On Unix, `\\` is NOT a path separator — `path.resolve` treats
  // `"..\\escape.txt"` as a single literal filename (inside outDir).
  test.skipIf(process.platform === "win32")(
    "containedJoin: on non-Windows, backslash is a literal char",
    () => {
      const fs = new DiskFs({ workspaceRoot: root });
      const out = join(root, "out");
      expect(() => fs.containedJoin(out, "..\\escape.txt")).not.toThrow();
    },
  );

  test.skipIf(process.platform !== "win32")(
    "containedJoin: on Windows, backslash traversal is rejected",
    () => {
      const fs = new DiskFs({ workspaceRoot: root });
      const out = join(root, "out");
      expect(() => fs.containedJoin(out, "..\\escape.txt")).toThrow(/escapes outDir/);
    },
  );

  test("writeFile creates parent dirs and writes bytes", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const target = join(root, "out", "sub", "f.txt");
    fs.writeFile(target, new TextEncoder().encode("hello"));
    expect(existsSync(target)).toBe(true);
  });

  test("streamCopy pipes bytes through createReadStream/createWriteStream", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const srcPath = join(root, "src.bin");
    const dstPath = join(root, "out", "dst.bin");
    // 256 KiB payload — comfortably bigger than Node's default
    // highWaterMark (~64 KiB), so multiple chunks traverse the pipe.
    // The point of this test isn't perf, it's correctness: the bytes
    // round-trip through `pipeline()` exactly.
    const payload = new Uint8Array(256 * 1024);
    for (let i = 0; i < payload.length; i++) payload[i] = i & 0xff;
    await writeFile(srcPath, payload);

    await fs.streamCopy(srcPath, dstPath);

    const back = await readFile(dstPath);
    expect(back.byteLength).toBe(payload.byteLength);
    expect(new Uint8Array(back)).toEqual(payload);
  });

  test("readFile returns the file's bytes", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const p = join(root, "x.txt");
    await writeFile(p, "abc");
    const bytes = fs.readFile(p);
    expect(new TextDecoder().decode(bytes)).toBe("abc");
  });

  test("exists reflects on-disk state", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(fs.exists(join(root, "nope"))).toBe(false);
    await writeFile(join(root, "real"), "x");
    expect(fs.exists(join(root, "real"))).toBe(true);
  });
});
