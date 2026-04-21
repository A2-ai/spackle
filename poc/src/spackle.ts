// Orchestration entry — calls the fs-backed wasm exports with a host-
// provided `SpackleFs` adapter. Nothing else happens on the host side:
// Rust reads config, walks templates, copies + writes outputs, all via
// the adapter.
//
// Two adapters ship with the reference: `DiskFs` (local disk under a
// workspace root) and `MemoryFs` (in-memory, useful for preview /
// testing). Consumers can implement their own (S3, git, virtual fs,
// etc.) as long as they match the `SpackleFs` shape in
// `./host/spackle-fs.ts`.

import { loadSpackleWasm } from "./wasm/index.ts";
import type { SpackleFs } from "./host/spackle-fs.ts";
import type {
    CheckResponse,
    GenerateResponse,
    SlotData,
    ValidationResponse,
} from "./wasm/types.ts";

/**
 * Validate a project: load config, check slot structure, validate
 * template references against slot keys. Returns the parsed config on
 * success so UIs can render slot forms without re-parsing TOML.
 */
export async function check(
    projectDir: string,
    fs: SpackleFs,
): Promise<CheckResponse> {
    const wasm = await loadSpackleWasm();
    return wasm.check(projectDir, fs);
}

/**
 * Validate slot data against a project's config. Rust loads the config
 * via the fs adapter — the host never touches `spackle.toml`.
 */
export async function validateSlotData(
    projectDir: string,
    slotData: SlotData,
    fs: SpackleFs,
): Promise<ValidationResponse> {
    const wasm = await loadSpackleWasm();
    return wasm.validateSlotData(projectDir, slotData, fs);
}

/**
 * Generate a filled project into `outDir`. Rust drives every copy /
 * render / write through the `fs` adapter. The host does not orchestrate
 * — it only decides which adapter backs the workspace.
 *
 * `runHooks = true` is unsupported in this milestone; calling with it
 * returns `{ ok: false, error: "hooks are unsupported in this milestone" }`.
 * Hooks will be added back via a separate `JsHooks`-style bridge.
 */
export async function generate(
    projectDir: string,
    outDir: string,
    slotData: SlotData,
    fs: SpackleFs,
    opts: { runHooks?: boolean } = {},
): Promise<GenerateResponse> {
    const wasm = await loadSpackleWasm();
    return wasm.generate(projectDir, outDir, slotData, opts.runHooks ?? false, fs);
}

export type { SpackleWasm } from "./wasm/index.ts";
export { loadSpackleWasm } from "./wasm/index.ts";
export type {
    SpackleFs,
    SpackleFsError,
    SpackleFsErrorKind,
    SpackleFileEntry,
    SpackleFileStat,
    SpackleFileType,
} from "./host/spackle-fs.ts";
export { fsError, isSpackleFsError } from "./host/spackle-fs.ts";
export { DiskFs, type DiskFsOptions } from "./host/disk-fs.ts";
export { MemoryFs, type MemoryFsSeed } from "./host/memory-fs.ts";
export type {
    CheckResponse,
    GenerateResponse,
    Hook,
    RenderedSummary,
    Slot,
    SlotData,
    SlotType,
    SpackleConfig,
    ValidationResponse,
} from "./wasm/types.ts";
