// DiskFs tests — the post-pivot DiskFs is a bundle reader/writer, not
// a `SpackleFs` adapter. These tests cover:
//   - `readProject`: walks a disk dir into a bundle with virtualized paths;
//     rejects workspaceRoot escapes; skips symlinks.
//   - `writeOutput`: writes a bundle back to disk under a contained out dir;
//     rejects `..`-traversal in entry paths.

import { describe, expect, test, beforeEach, afterEach } from "bun:test";
import { existsSync } from "node:fs";
import { mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
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

  test("readProject walks a directory into a virtualized bundle", async () => {
    await mkdir(join(root, "project", "sub"), { recursive: true });
    await writeFile(join(root, "project", "a.txt"), "A");
    await writeFile(join(root, "project", "sub", "b.txt"), "B");

    const fs = new DiskFs({ workspaceRoot: root });
    const bundle = fs
      .readProject(join(root, "project"))
      .toSorted((a, b) => a.path.localeCompare(b.path));

    expect(bundle.map((e) => e.path)).toEqual(["/project/a.txt", "/project/sub/b.txt"]);
    expect(new TextDecoder().decode(bundle[0].bytes)).toBe("A");
  });

  test("readProject honors a custom virtualRoot", async () => {
    await writeFile(join(root, "x.txt"), "X");
    const fs = new DiskFs({ workspaceRoot: root });
    const bundle = fs.readProject(root, { virtualRoot: "/proj" });
    expect(bundle[0].path).toBe("/proj/x.txt");
  });

  test("readProject skips symlinks (doesn't follow or emit)", async () => {
    // `target/` is a real directory; `link` is a symlink pointing at
    // it. The walker should emit real.txt and target/secret (both
    // live on real disk paths) but NOT emit anything under `link/`
    // (would be a symlink follow).
    await mkdir(join(root, "target"), { recursive: true });
    await writeFile(join(root, "target", "secret"), "s");
    await symlink(join(root, "target"), join(root, "link"));
    await writeFile(join(root, "real.txt"), "R");

    const fs = new DiskFs({ workspaceRoot: root });
    const bundle = fs.readProject(root);
    const paths = bundle.map((e) => e.path).toSorted();

    expect(paths).toContain("/project/real.txt");
    expect(paths).toContain("/project/target/secret");
    // No entry should have been produced by traversing `link` —
    // that would mean we followed a symlink. (Paths reached via
    // the real `target/` dir are fine.)
    expect(paths.some((p) => p.startsWith("/project/link/"))).toBe(false);
  });

  test("readProject rejects a projectDir that escapes workspaceRoot", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() => fs.readProject("/etc")).toThrow();
  });

  test("writeOutput creates ancestor dirs and writes files", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    fs.writeOutput(outDir, [
      { path: "a.txt", bytes: new TextEncoder().encode("A") },
      { path: "sub/b.txt", bytes: new TextEncoder().encode("B") },
    ]);
    expect(await readFile(join(outDir, "a.txt"), "utf8")).toBe("A");
    expect(await readFile(join(outDir, "sub", "b.txt"), "utf8")).toBe("B");
  });

  test("writeOutput creates empty dirs from the dirs list", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "output");
    fs.writeOutput(outDir, {
      files: [{ path: "a.txt", bytes: new TextEncoder().encode("A") }],
      dirs: ["empty-dir", "sub/also-empty"],
    });
    expect(existsSync(join(outDir, "empty-dir"))).toBe(true);
    expect(existsSync(join(outDir, "sub", "also-empty"))).toBe(true);
  });

  test("writeOutput refuses a pre-existing outDir (parity with native AlreadyExists)", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "out");
    await mkdir(outDir, { recursive: true });
    expect(() =>
      fs.writeOutput(outDir, [{ path: "a.txt", bytes: new TextEncoder().encode("hi") }]),
    ).toThrow(/already exists/);
  });

  test("writeOutput rejects entry paths that escape via /", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() =>
      fs.writeOutput(join(root, "out"), [{ path: "../escape.txt", bytes: new Uint8Array() }]),
    ).toThrow(/escapes outDir/);
  });

  test("writeOutput rejects deeper `..` traversal (../../)", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() =>
      fs.writeOutput(join(root, "out"), [{ path: "../../escape.txt", bytes: new Uint8Array() }]),
    ).toThrow(/escapes outDir/);
  });

  // On Unix, `\\` is NOT a path separator — `path.resolve` treats
  // `"..\\escape.txt"` as a single literal filename (weird, but inside
  // outDir). Pinning the Unix semantics here; Windows backslash-escape
  // handling is exercised by the same `containedJoin` helper via
  // `path.resolve` on that platform but cannot be directly asserted
  // from a Unix CI runner without forging a `path.win32` call.
  test.skipIf(process.platform === "win32")(
    "writeOutput: on non-Windows, backslash is a literal char (no escape)",
    () => {
      const fs = new DiskFs({ workspaceRoot: root });
      // Accepted: resolves to a file whose name contains a
      // backslash, still inside outDir. Must NOT throw.
      expect(() =>
        fs.writeOutput(join(root, "out"), [
          {
            path: "..\\escape.txt",
            bytes: new TextEncoder().encode("benign-on-unix"),
          },
        ]),
      ).not.toThrow();
    },
  );

  test.skipIf(process.platform !== "win32")(
    "writeOutput: on Windows, backslash traversal is rejected",
    () => {
      const fs = new DiskFs({ workspaceRoot: root });
      expect(() =>
        fs.writeOutput(join(root, "out"), [{ path: "..\\escape.txt", bytes: new Uint8Array() }]),
      ).toThrow(/escapes outDir/);
    },
  );

  test("writeOutput rejects absolute entry paths (would override outDir)", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() =>
      fs.writeOutput(join(root, "out"), [{ path: "/etc/pwned", bytes: new Uint8Array() }]),
    ).toThrow(/escapes outDir/);
  });

  test("writeOutput rejects an outDir that escapes workspaceRoot", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() => fs.writeOutput("/etc/spackle-out", [])).toThrow();
  });

  test("prepareOutDir creates the outDir and refuses pre-existing", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "out");
    const abs = fs.prepareOutDir(outDir);
    expect(existsSync(outDir)).toBe(true);
    expect(abs).toBe(outDir);

    expect(() => fs.prepareOutDir(outDir)).toThrow(/already exists/);
  });

  test("writeEntry writes file and dir entries under outDir", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "out");
    fs.prepareOutDir(outDir);

    fs.writeEntry(outDir, { kind: "dir", path: "sub" });
    fs.writeEntry(outDir, {
      kind: "file",
      path: "sub/a.txt",
      bytes: new TextEncoder().encode("A"),
    });
    expect(existsSync(join(outDir, "sub"))).toBe(true);
    expect(await readFile(join(outDir, "sub", "a.txt"), "utf8")).toBe("A");
  });

  test("writeEntry creates missing parent dirs on file events defensively", async () => {
    // Streaming events arrive parent-before-child so the parent dir
    // typically already exists, but a templated path can introduce a
    // dir that wasn't its own dir event. writeEntry mkdir's the parent
    // recursively to be safe.
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "out");
    fs.prepareOutDir(outDir);

    fs.writeEntry(outDir, {
      kind: "file",
      path: "deep/nested/x.txt",
      bytes: new TextEncoder().encode("x"),
    });
    expect(await readFile(join(outDir, "deep", "nested", "x.txt"), "utf8")).toBe("x");
  });

  test("writeEntry rejects entry paths that escape outDir", () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "out");
    fs.prepareOutDir(outDir);
    expect(() =>
      fs.writeEntry(outDir, {
        kind: "file",
        path: "../escape.txt",
        bytes: new Uint8Array(),
      }),
    ).toThrow(/escapes outDir/);
    expect(() => fs.writeEntry(outDir, { kind: "dir", path: "../escape" })).toThrow(
      /escapes outDir/,
    );
  });

  test("writeEntry rejects an outDir outside workspaceRoot", () => {
    // Public API safety: a custom streaming consumer that calls
    // writeEntry directly (bypassing prepareOutDir / generate()) must
    // not be able to write outside the DiskFs's workspaceRoot. Older
    // implementations only enforced entry-path containment relative to
    // outDir; this regression test pins the per-call workspaceRoot
    // check too.
    const fs = new DiskFs({ workspaceRoot: root });
    expect(() =>
      fs.writeEntry("/etc/spackle-pwn", {
        kind: "file",
        path: "a.txt",
        bytes: new TextEncoder().encode("x"),
      }),
    ).toThrow(/escapes workspaceRoot/);
    expect(() => fs.writeEntry("/etc/spackle-pwn", { kind: "dir", path: "sub" })).toThrow(
      /escapes workspaceRoot/,
    );
  });

  test("assertOutDirAvailable returns canonical path without creating outDir", async () => {
    const fs = new DiskFs({ workspaceRoot: root });
    const outDir = join(root, "deferred");
    const abs = fs.assertOutDirAvailable(outDir);
    expect(abs).toBe(outDir);
    expect(existsSync(outDir)).toBe(false);
    // AlreadyExists check still fires even though it doesn't create.
    await mkdir(outDir, { recursive: true });
    expect(() => fs.assertOutDirAvailable(outDir)).toThrow(/already exists/);
  });
});
