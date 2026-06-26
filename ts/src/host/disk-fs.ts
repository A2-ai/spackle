// DiskFs — workspace boundary + per-file disk I/O helpers used by the
// TS orchestrator. Canonicalizes paths under `workspaceRoot`,
// stream-copies static files via `pipeline()`, writes rendered
// template bytes via `writeFile`.

import {
  chmodSync,
  createReadStream,
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  statSync,
  writeFileSync,
} from "node:fs";
import {
  basename as pathBasename,
  dirname as pathDirname,
  isAbsolute,
  join as pathJoin,
  resolve as pathResolve,
  sep as pathSep,
} from "node:path";
import { pipeline } from "node:stream/promises";

export interface DiskFsOptions {
  /**
   * Absolute path to the workspace root. The adapter refuses any path
   * that canonicalizes outside this root. Must exist on disk at
   * construction time so it can be canonicalized once. POSIX `"/"` is
   * supported.
   */
  workspaceRoot: string;
}

/**
 * True iff `absPath` lives under `absRoot`. Handles `absRoot === "/"`
 * correctly: appending `pathSep` would produce `"//"` and reject every
 * real path, so we detect a root that already ends in the separator
 * and skip the append.
 */
function isContainedUnder(absRoot: string, absPath: string): boolean {
  if (absPath === absRoot) return true;
  const prefix = absRoot.endsWith(pathSep) ? absRoot : absRoot + pathSep;
  return absPath.startsWith(prefix);
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

  /** Canonicalize an existing path under `workspaceRoot`. Throws if
   * the path isn't absolute, doesn't exist, or escapes the root. */
  containProject(projectDir: string): string {
    return this.containDisk(projectDir);
  }

  /** Contain `outDir` and verify it does not already exist. Doesn't
   * create the directory — the caller mkdirs lazily so validation
   * failures leave no empty dir on disk. Matches native's
   * `GenerateError::AlreadyExists`. */
  assertOutDirAvailable(outDir: string): string {
    const absOut = this.containDiskForCreate(outDir);
    if (existsSync(absOut)) {
      throw new Error(`assertOutDirAvailable: output directory already exists: ${absOut}`);
    }
    return absOut;
  }

  /** Idempotent `mkdir -p` for `outDir` under `workspaceRoot`. Always
   * creates the dir writable; preserved directory modes are applied
   * later via [[chmod]], once descendants are populated. */
  ensureOutDir(outDir: string): string {
    const absOut = this.containDiskForCreate(outDir);
    mkdirSync(absOut, { recursive: true });
    return absOut;
  }

  /** Join `rel` under `absBase` and verify the result stays under it.
   * Catches `../escape`, absolute `rel`, and (on Windows) backslash
   * traversal because `path.resolve` normalizes separators before the
   * prefix check. POSIX `absBase === "/"` works correctly — see
   * [[isContainedUnder]]. */
  containedJoin(absBase: string, rel: string): string {
    const resolved = pathResolve(absBase, rel);
    if (!isContainedUnder(absBase, resolved)) {
      throw new Error(`entry path escapes outDir: ${rel}`);
    }
    return resolved;
  }

  /** Write `bytes` at `absPath`, creating parent dirs as needed. When
   * `mode` is supplied, chmod after writing so the bits survive umask. */
  writeFile(absPath: string, bytes: Uint8Array, mode?: number): void {
    mkdirSync(pathDirname(absPath), { recursive: true });
    writeFileSync(absPath, bytes);
    if (mode !== undefined) chmodSync(absPath, mode);
  }

  /** Stream-copy `srcAbs` → `dstAbs` via `pipeline()`. Bytes traverse
   * Node's default ~64 KiB highWaterMark chunks; large files never
   * sit fully in memory. When `mode` is supplied, chmod the
   * destination after the copy completes so the bits survive umask. */
  async streamCopy(srcAbs: string, dstAbs: string, mode?: number): Promise<void> {
    mkdirSync(pathDirname(dstAbs), { recursive: true });
    await pipeline(createReadStream(srcAbs), createWriteStream(dstAbs));
    if (mode !== undefined) chmodSync(dstAbs, mode);
  }

  /** Permission bits (0o777-masked) of an existing path. */
  modeOf(absPath: string): number {
    return statSync(absPath).mode & 0o777;
  }

  /** Apply permission bits to an existing path. Best-effort on Windows
   * (only the read-only bit is honored there). */
  chmod(absPath: string, mode: number): void {
    chmodSync(absPath, mode);
  }

  /** Read a single file's bytes from disk. */
  readFile(absPath: string): Uint8Array {
    return new Uint8Array(readFileSync(absPath));
  }

  /** True iff `path` exists on disk. */
  exists(absPath: string): boolean {
    return existsSync(absPath);
  }

  /** The canonicalized workspaceRoot. */
  get workspaceRoot(): string {
    return this.root;
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
    if (!isContainedUnder(this.root, canonical)) {
      throw new Error(`path escapes workspaceRoot: ${canonical} not under ${this.root}`);
    }
    return canonical;
  }

  /**
   * Canonicalize a path that may not yet exist. Walks up to the
   * nearest existing ancestor, canonicalizes that, rejoins the
   * remaining segments lexically. Catches symlink escapes through
   * existing dirs; TOCTOU on symlinks planted mid-op is out of scope
   * (OS-level sandboxing is the real fix).
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
      // `pathBasename` + `pathJoin` instead of `existing.slice(parent.length + 1)`
      // and string concat: when `parent === "/"`, the slice form drops an
      // extra char (existing="/foo" → "oo") and the concat form doubles
      // the separator (canonicalExisting="/" + sep + tail = "//foo").
      tail.unshift(pathBasename(existing));
      existing = parent;
    }
    const canonicalExisting = realpathSync(existing);
    const canonical = tail.length ? pathJoin(canonicalExisting, ...tail) : canonicalExisting;
    if (!isContainedUnder(this.root, canonical)) {
      throw new Error(`path escapes workspaceRoot: ${canonical} not under ${this.root}`);
    }
    return canonical;
  }
}
