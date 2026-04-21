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

import type { Bundle } from "../wasm/types.ts";

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
   * Write a rendered output bundle to `outDir`.
   *
   * Accepts either a flat `Bundle` (just files) or an object with
   * `files` + `dirs` — when `dirs` is present, each listed directory
   * is mkdir'd so empty dirs created by the Rust copy pass survive
   * the round-trip (native spackle calls `create_dir_all` for every
   * Directory entry during copy).
   *
   * Contract (matches native `GenerateError::AlreadyExists`):
   *   `outDir` must NOT already exist on disk. If it does, throws —
   *   same as `spackle generate` on native.
   *
   * Containment: `outDir` must resolve under `workspaceRoot` (we walk
   * up to the nearest existing ancestor and canonicalize that).
   * Per-entry traversal guard uses `path.resolve` to normalize
   * platform-specific separators; `../x.txt`, `..\x.txt`, and any
   * normalized escape are all rejected.
   */
  writeOutput(outDir: string, input: Bundle | WriteOutputInput): void {
    const { files, dirs } = Array.isArray(input)
      ? { files: input, dirs: undefined as string[] | undefined }
      : input;

    const absOut = this.containDiskForCreate(outDir);
    if (existsSync(absOut)) {
      throw new Error(`writeOutput: output directory already exists: ${absOut}`);
    }
    mkdirSync(absOut, { recursive: true });

    // Create empty dirs first so the final tree matches native
    // generation even when a directory has no files under it.
    for (const rel of dirs ?? []) {
      const absDir = this.containedJoin(absOut, rel);
      mkdirSync(absDir, { recursive: true });
    }

    for (const entry of files) {
      const absFile = this.containedJoin(absOut, entry.path);
      mkdirSync(pathDirname(absFile), { recursive: true });
      writeFileSync(absFile, entry.bytes);
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
      throw new Error(`writeOutput: entry path escapes outDir: ${rel}`);
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
