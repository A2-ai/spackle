# API reference

`@a2-ai/spackle-wasm` exposes three primary operations — `check`, `validateSlotData`, `generate` — plus two `…Bundle` variants that skip disk I/O. Two host helpers (`DiskFs`, `MemoryFs`) handle moving data in and out.

## Core types

```ts
type SlotType = "String" | "Number" | "Boolean";
type SlotData = Record<string, string>;

interface BundleEntry {
    path: string;
    bytes: Uint8Array;
}
type Bundle = BundleEntry[];

interface SpackleConfig {
    name: string | null;
    ignore: string[];
    slots: Slot[];
    hooks: Hook[];
}
```

See `src/wasm/types.ts` in the package for the full shape (including `Slot`, `Hook`, error/response unions).

## `check(projectDir, fs, opts?)`

Validate a project: load its `spackle.toml`, check slot structure, verify template references match declared slots.

```ts
function check(
    projectDir: string,
    fs: DiskFs,
    opts?: { virtualProjectDir?: string },
): Promise<CheckResponse>;

type CheckResponse =
    | { valid: true; config: SpackleConfig; errors: [] }
    | { valid: false; errors: string[] };
```

`fs.readProject(projectDir)` runs; the resulting bundle goes to wasm's `check`. On success you get the parsed config back so UIs can render slot forms without re-parsing TOML.

## `validateSlotData(projectDir, slotData, fs, opts?)`

Check a data set against a project's slot types.

```ts
function validateSlotData(
    projectDir: string,
    slotData: SlotData,
    fs: DiskFs,
    opts?: { virtualProjectDir?: string },
): Promise<ValidationResponse>;

type ValidationResponse =
    | { valid: true }
    | { valid: false; errors: string[] };
```

Rules enforced: every declared slot is present, types coerce (`"42"` → `Number` ok; `"not-a-number"` → `Number` fails), no undeclared slots.

## `generate(projectDir, outDir, slotData, fs, opts?)`

Run the full pipeline: copy non-template files, render `.j2` files, render path placeholders, write everything under `outDir`.

```ts
function generate(
    projectDir: string,
    outDir: string,
    slotData: SlotData,
    fs: DiskFs,
    opts?: {
        virtualProjectDir?: string;
        virtualOutDir?: string;
        runHooks?: boolean;
    },
): Promise<GenerateResponse>;

type GenerateResponse =
    | { ok: true; files: Bundle; dirs: string[] }
    | { ok: false; error: string };
```

`result.files` carries the rendered bundle with paths **relative to `outDir`**. `result.dirs` carries directory paths (also relative) — present so **empty directories survive the round-trip**. Native `spackle generate` calls `create_dir_all` for every directory walked during the copy pass; without emitting them, a project whose `drafts/` directory is fully ignored (every file filtered out) would still have `drafts/` created on native but silently dropped under wasm. Hosts writing output manually MUST mkdir each entry in `dirs` to match native behavior.

`DiskFs.writeOutput` (built-in) handles both `files` and `dirs` for you. Output-dir contract: `writeOutput` throws if `outDir` already exists, matching native's `GenerateError::AlreadyExists`.

`runHooks: true` currently returns `{ ok: false, error: "hooks are unsupported in this milestone" }`. See [hooks.md](./hooks.md).

## Bundle variants

For preview flows with no disk I/O, skip `DiskFs` entirely and drive wasm directly:

```ts
function checkBundle(bundle: Bundle, virtualProjectDir?: string): Promise<CheckResponse>;
function validateSlotDataBundle(bundle: Bundle, slotData: SlotData, virtualProjectDir?: string): Promise<ValidationResponse>;
function generateBundle(
    bundle: Bundle,
    slotData: SlotData,
    opts?: { virtualProjectDir?: string; virtualOutDir?: string; runHooks?: boolean },
): Promise<GenerateResponse>;
```

Pair with `MemoryFs.toBundle()` / `MemoryFs.fromBundle()` to inspect results in-memory.

## Host helpers

### `DiskFs`

```ts
class DiskFs {
    constructor(opts: { workspaceRoot: string });
    readProject(projectDir: string, opts?: { virtualRoot?: string }): Bundle;
    writeOutput(
        outDir: string,
        input: Bundle | { files: Bundle; dirs?: string[] },
    ): void;
}
```

`writeOutput` accepts either a flat `Bundle` (files only — convenient for hand-rolled calls) or the `{ files, dirs }` shape returned by `generate`. Pass `{ files, dirs }` to preserve empty directories. Ancestor dirs for file writes are created automatically either way.

**`outDir` must not already exist.** `writeOutput` refuses a pre-existing `outDir` with an `already exists` error (parity with native `spackle generate`). Callers should pick a fresh path per run, or `rm -rf` the target before calling.

**Containment:** `workspaceRoot` is canonicalized once. Every path fed into `readProject` / `writeOutput` must resolve under it; anything else throws. Per-entry traversal is blocked using `path.resolve` + prefix comparison — rejects `../escape`, absolute overrides, and any OS-normalized escape.

### `MemoryFs`

```ts
class MemoryFs {
    constructor(seed?: { files?: Record<string, Uint8Array | string> });
    static fromBundle(bundle: Bundle, prefix?: string): MemoryFs;
    insertFile(path: string, content: Uint8Array | string): void;
    toBundle(): Bundle;
    get(path: string): Uint8Array | undefined;
    has(path: string): boolean;
    snapshot(): { files: Record<string, Uint8Array> };
}
```

Pure TS. No filesystem interaction. Useful for tests and preview flows.

## Known limitations

- **UTF-8 paths only.** The bundle boundary doesn't round-trip non-UTF-8 filenames.
- **Whole-project marshalling.** Input and output bundles materialize in memory. Fine for typical templates (KB–MB); very large fixtures should wait on a streaming path.
- **No hooks.** See [hooks.md](./hooks.md) for status.
