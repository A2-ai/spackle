// DiskFs — host-side bundle reader/writer for local-disk projects.
//
// No longer a `SpackleFs` callback adapter (that model is gone). Under
// the bundle-in / bundle-out design, the host serializes project files
// into a bundle before calling wasm, and writes the returned output
// bundle back to disk after. DiskFs is the reference impl of that
// serialize / deserialize pair, rooted at a workspace directory with
// containment enforced.

import {
  copyFileSync,
  existsSync,
  lstatSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import {
  dirname as pathDirname,
  isAbsolute,
  join as pathJoin,
  resolve as pathResolve,
  sep as pathSep,
} from "node:path";

import type { Bundle, GenerateStreamEntry } from "../wasm/types.ts";

/** Shape of a successful `generate` response — kept here (rather than
 * in wasm/types.ts) so DiskFs's signature doesn't import the full
 * response union. Matches the `{ ok: true, files, dirs }` shape. */
export interface WriteOutputInput {
  files: Bundle;
  dirs?: string[];
}

export interface DiskFsOptions {
  /**
   * Absolute path to the workspace root. The adapter refuses any path
   * that canonicalizes outside this root. Must exist on disk at
   * construction time so it can be canonicalized once.
   */
  workspaceRoot: string;
}

export interface ReadProjectOptions {
  /**
   * Virtual path prefix the bundle will use. Defaults to `/project`.
   * Every file read from `projectDir` is emitted as `{virtualRoot}/...`
   * in the bundle, relative to `projectDir`. Pass the same string as
   * `projectDir` in subsequent wasm calls (`check`, `generate`).
   */
  virtualRoot?: string;
}

export class DiskFs {
  private readonly root: string;

  constructor(opts: DiskFsOptions) {
    if (!isAbsolute(opts.workspaceRoot)) {
      throw new Error(`DiskFs workspaceRoot must be absolute: ${opts.workspaceRoot}`);
    }
    try {
      this.root = realpathSync(opts.workspaceRoot);
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e);
      throw new Error(`DiskFs: workspaceRoot not accessible: ${opts.workspaceRoot}: ${msg}`, {
        cause: e,
      });
    }
  }

  /**
   * Walk `projectDir` recursively and emit a bundle with virtualized
   * paths. Containment: `projectDir` must resolve under `workspaceRoot`;
   * symlinks during the walk are skipped (not followed, not emitted) to
   * prevent escape via planted links.
   */
  readProject(projectDir: string, opts: ReadProjectOptions = {}): Bundle {
    const virtualRoot = opts.virtualRoot ?? "/project";
    const absRoot = this.containDisk(projectDir);

    const out: Bundle = [];
    const walk = (absDir: string, virtDir: string) => {
      for (const entry of readdirSync(absDir, { withFileTypes: true })) {
        const abs = pathJoin(absDir, entry.name);
        const virt = `${virtDir}/${entry.name}`;
        if (entry.isSymbolicLink()) {
          // Skip symlinks — don't follow, don't emit. Matches
          // the walker behavior in `spackle::fs::walk`.
          continue;
        }
        if (entry.isDirectory()) {
          walk(abs, virt);
        } else if (entry.isFile()) {
          out.push({ path: virt, bytes: readFileSync(abs) });
        }
      }
    };
    walk(absRoot, virtualRoot);
    return out;
  }

  /**
   * Verify `outDir` is contained under `workspaceRoot` and does not
   * already exist; return its canonical absolute path. Does NOT
   * create the directory.
   *
   * Use this in streaming-generate flows where you want to fail fast
   * on AlreadyExists / containment without leaving an empty `outDir`
   * on disk if a downstream step (e.g., wasm validation) errors out
   * before any entry has streamed. `writeEntry` recursively mkdirs the
   * parent of each entry it writes, so `outDir` gets created lazily on
   * the first write — matching native `Project::generate`, which only
   * creates the destination as part of `copy::copy`.
   *
   * Contract (matches native `GenerateError::AlreadyExists`):
   *   `outDir` must NOT already exist on disk. If it does, throws —
   *   same as `spackle generate` on native.
   */
  assertOutDirAvailable(outDir: string): string {
    const absOut = this.containDiskForCreate(outDir);
    if (existsSync(absOut)) {
      throw new Error(`assertOutDirAvailable: output directory already exists: ${absOut}`);
    }
    return absOut;
  }

  /**
   * `assertOutDirAvailable` + create the directory. Use this when
   * you've already buffered the full output (e.g., `generateBundle` →
   * `writeOutput`) — eager creation matches the buffered model.
   * Streaming callers should use `assertOutDirAvailable` and let
   * `writeEntry` create the directory lazily.
   */
  prepareOutDir(outDir: string): string {
    const absOut = this.assertOutDirAvailable(outDir);
    mkdirSync(absOut, { recursive: true });
    return absOut;
  }

  /**
   * Idempotent `mkdir -p` for `outDir`, with workspaceRoot
   * containment. Unlike `prepareOutDir`, this does NOT throw if the
   * directory already exists — used by streaming generate to preserve
   * native parity for empty projects, where no streamed events fire
   * and `writeEntry`'s parent-mkdir never runs.
   */
  ensureOutDir(outDir: string): string {
    const absOut = this.containDiskForCreate(outDir);
    mkdirSync(absOut, { recursive: true });
    return absOut;
  }

  /**
   * Write a single streamed entry to disk under `outDir`.
   *
   * Sync sibling of `writeOutput` for the streaming-generate path:
   * `wasm.generate(...)` invokes a host callback per file/dir entry,
   * and that callback ends up here, dropping bytes to disk before the
   * next event arrives. Peak memory is bounded by the size of one
   * entry, not by the rendered output.
   *
   * Re-validates that `outDir` is under `workspaceRoot` on every call
   * (via `containDiskForCreate`) so external streaming consumers can't
   * accidentally write outside the DiskFs root by passing an
   * arbitrary path. After the first write, `outDir` exists and the
   * canonicalization hits the existing-path branch — single
   * `realpathSync`, microseconds per call.
   *
   * Containment for the entry's relative path uses `containedJoin` so
   * traversal escapes (`../`, absolute paths, platform-specific
   * separators) are rejected before any write. Parent dirs are
   * mkdir'd recursively (idempotent) — that's also what creates
   * `outDir` itself on the first write when `assertOutDirAvailable`
   * was used in lieu of `prepareOutDir`.
   */
  writeEntry(outDir: string, entry: GenerateStreamEntry): void {
    const absOut = this.containDiskForCreate(outDir);
    const absPath = this.containedJoin(absOut, entry.path);
    if (entry.kind === "dir") {
      mkdirSync(absPath, { recursive: true });
      return;
    }
    mkdirSync(pathDirname(absPath), { recursive: true });
    writeFileSync(absPath, entry.bytes);
  }

  /**
   * Write a rendered output bundle to `outDir` (buffered shape).
   *
   * Accepts either a flat `Bundle` (just files) or an object with
   * `files` + `dirs` — when `dirs` is present, each listed directory
   * is mkdir'd so empty dirs created by the Rust copy pass survive
   * the round-trip.
   *
   * Same contract as `prepareOutDir` + a loop of `writeEntry` calls;
   * use this when you already have the full bundle in memory (e.g.,
   * from `generateBundle`). Streaming callers should drive
   * `writeEntry` directly off the wasm callback to avoid the
   * intermediate buffer.
   */
  writeOutput(outDir: string, input: Bundle | WriteOutputInput): void {
    const { files, dirs } = Array.isArray(input)
      ? { files: input, dirs: undefined as string[] | undefined }
      : input;

    const absOut = this.prepareOutDir(outDir);
    for (const rel of dirs ?? []) {
      this.writeEntry(absOut, { kind: "dir", path: rel });
    }
    for (const entry of files) {
      this.writeEntry(absOut, { kind: "file", path: entry.path, bytes: entry.bytes });
    }
  }

  /**
   * Join `rel` under `absBase` and verify the result stays under
   * `absBase`. Catches both `../escape` (Unix) and `..\escape`
   * (Windows) because `path.resolve` normalizes separators before the
   * prefix check. Also catches absolute `rel` values that would
   * `path.join` on top of `absBase`.
   */
  private containedJoin(absBase: string, rel: string): string {
    const resolved = pathResolve(absBase, rel);
    if (resolved !== absBase && !resolved.startsWith(absBase + pathSep)) {
      throw new Error(`entry path escapes outDir: ${rel}`);
    }
    return resolved;
  }

  /**
   * Canonicalize an already-existing path and enforce that it lives
   * under `workspaceRoot`. Throws if not.
   */
  private containDisk(path: string): string {
    if (!isAbsolute(path)) {
      throw new Error(`path must be absolute: ${path}`);
    }
    const canonical = realpathSync(pathResolve(path));
    if (canonical !== this.root && !canonical.startsWith(this.root + pathSep)) {
      throw new Error(`path escapes workspaceRoot: ${canonical} not under ${this.root}`);
    }
    return canonical;
  }

  /**
   * Canonicalize a path that may not yet exist (write / mkdir flows).
   * Walks up to the nearest existing ancestor, canonicalizes that, and
   * rejoins remaining segments lexically. See the git history of the
   * previous `resolve(..., forCreate: true)` method for the rationale:
   * we catch symlink escapes through existing directories but accept
   * a TOCTOU gap for symlinks planted mid-operation. OS-level
   * sandboxing (Landlock etc.) is the real fix.
   */
  private containDiskForCreate(path: string): string {
    if (!isAbsolute(path)) {
      throw new Error(`path must be absolute: ${path}`);
    }
    const lexical = pathResolve(path);
    let existing = lexical;
    const tail: string[] = [];
    while (!existsSync(existing)) {
      const parent = pathDirname(existing);
      if (parent === existing) {
        throw new Error(`containDiskForCreate(${path}): no existing ancestor`);
      }
      tail.unshift(existing.slice(parent.length + 1));
      existing = parent;
    }
    const canonicalExisting = realpathSync(existing);
    const canonical = tail.length
      ? `${canonicalExisting}${pathSep}${tail.join(pathSep)}`
      : canonicalExisting;
    if (canonical !== this.root && !canonical.startsWith(this.root + pathSep)) {
      throw new Error(`path escapes workspaceRoot: ${canonical} not under ${this.root}`);
    }
    return canonical;
  }
}

// Re-export Bundle type for callers that don't want to reach into wasm/.
export type { Bundle } from "../wasm/types.ts";

// Convenience: surface utilities callers might want for custom flows.
export { copyFileSync, readFileSync, writeFileSync, lstatSync };
