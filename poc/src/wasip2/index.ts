// HOST-SIDE — loads the wasip2 component and issues request-scoped calls
// into it. Uses @bytecodealliance/preview2-shim for the WASI preview2
// imports, and Bun.spawnSync to back the component-imported run-command
// capability.
//
// Shape:
//   - Core wasm modules are compiled ONCE at module load (expensive step).
//   - Each request instantiates fresh with a workspace-parent preopen
//     mapped into the WASI filesystem view. This keeps request sandboxes
//     isolated and sidesteps the "outDir doesn't exist yet" problem that
//     makes per-outDir preopens infeasible.

import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { spawnSync as nodeSpawnSync } from "node:child_process";

// preview2-shim's io layer eagerly imports `process.binding("tcp_wrap")`
// on load (for TCP sockets we never touch). Bun doesn't implement that
// binding. Stub it before loading the shim so the module graph evaluates.
// Runs only on Bun — Node has the real binding and this is a no-op.
if (typeof (globalThis as any).Bun !== "undefined") {
    const stub = () => ({ TCP: class {}, constants: {} });
    const orig = process.binding.bind(process) as any;
    (process as any).binding = (name: string) => {
        try {
            return orig(name);
        } catch {
            return stub();
        }
    };
}

// @ts-expect-error — the transpiled module ships its own .d.ts; Bun
// resolves it via package.json exports at runtime.
import { instantiate } from "../../wasip2-pkg/spackle.js";

import { cli, clocks, filesystem, io, random } from "@bytecodealliance/preview2-shim";

const here = dirname(fileURLToPath(import.meta.url));
const pkgDir = join(here, "..", "..", "wasip2-pkg");

// Compile the core modules once. jco emits several (spackle.core.wasm
// plus adapter shims); resolve them by name from pkgDir.
//
// NOTE on `new Promise(...)`: Bun's `readFile(...).then(...)` produces
// an `InternalPromise` that fails `instanceof Promise`. jco's generator
// driver checks `value instanceof Promise` to decide whether to await,
// so an InternalPromise passes straight through to the generator as if
// it were a resolved value — leaving us trying to `WebAssembly.instantiate`
// a Promise. Wrapping in `new Promise(...)` forces a userland Promise
// that Bun + jco both recognize.
const coreCache = new Map<string, Promise<WebAssembly.Module>>();
function compileCore(path: string): Promise<WebAssembly.Module> {
    const cached = coreCache.get(path);
    if (cached) return cached;
    const p = new Promise<WebAssembly.Module>((resolve, reject) => {
        readFile(join(pkgDir, path))
            .then((bytes) => WebAssembly.compile(bytes))
            .then(resolve, reject);
    });
    coreCache.set(path, p);
    return p;
}

export type CheckResponse =
    | { valid: true; config: Config; errors: [] }
    | { valid: false; errors: string[] };

export type ValidationResponse =
    | { valid: true }
    | { valid: false; errors: string[] };

export type GenerateResponse =
    | {
          ok: true;
          rendered: Array<{ original_path: string; rendered_path: string }>;
          hook_results: WasipHookResult[];
      }
    | { ok: false; error: string };

export interface Config {
    name: string | null;
    ignore: string[];
    slots: Slot[];
    hooks: Hook[];
}

export interface Slot {
    key: string;
    type?: string;
    name?: string;
    description?: string;
    default?: string | null;
}

export interface Hook {
    key: string;
    command: string[];
    if?: string | null;
    needs?: string[];
    name?: string | null;
    description?: string | null;
    default?: boolean | null;
}

export type WasipHookResult = {
    hook_key: string;
} & (
    | { kind: "skipped"; reason: string; template_errors?: string[] }
    | { kind: "completed"; stdout: number[]; stderr: number[]; exit_code: number }
    | {
          kind: "failed";
          error: string;
          stdout?: number[];
          stderr?: number[];
          exit_code: number;
      }
);

export interface CheckRequest {
    workspaceRoot: string;
    projectDir: string;
}

export interface ValidateSlotDataRequest extends CheckRequest {
    slotData: Record<string, string>;
}

export interface GenerateRequest extends ValidateSlotDataRequest {
    outDir: string;
    runHooks: boolean;
}

/**
 * Build a fresh WASI import object for a single request.
 *
 * Preopens are scoped to `workspaceRoot` — the component can only open
 * files under that directory, and will see it at the same path it was
 * given (virtual path = host path). `runCommand` merges the hook-
 * configured env with the server process env so hooks inherit PATH and
 * other expected variables.
 */
function buildImports(workspaceRoot: string) {
    filesystem._setPreopens({ [workspaceRoot]: workspaceRoot });
    cli._setCwd(workspaceRoot);
    cli._setArgs([]);
    cli._setEnv(process.env);

    return {
        "a2ai:spackle/host": {
            runCommand(
                cmd: string,
                args: string[],
                cwd: string,
                env: Array<[string, string]>,
            ) {
                // Bun and Node both offer sync spawn, with slightly
                // different APIs. Prefer Bun.spawnSync when available
                // (same runtime the server will use); fall back to
                // node:child_process.spawnSync for the test runner and
                // any Node-based consumer.
                const mergedEnv = { ...process.env, ...Object.fromEntries(env) };
                const bunSpawn = (globalThis as any).Bun?.spawnSync;
                if (bunSpawn) {
                    const proc = bunSpawn([cmd, ...args], { cwd, env: mergedEnv });
                    return {
                        stdout: proc.stdout,
                        stderr: proc.stderr,
                        exitCode: proc.exitCode ?? 1,
                    };
                }
                const proc = nodeSpawnSync(cmd, args, { cwd, env: mergedEnv });
                return {
                    stdout: proc.stdout ?? new Uint8Array(),
                    stderr: proc.stderr ?? new Uint8Array(),
                    exitCode: proc.status ?? 1,
                };
            },
        },
        "wasi:cli/environment": cli.environment,
        "wasi:cli/exit": cli.exit,
        "wasi:cli/stderr": cli.stderr,
        "wasi:cli/stdin": cli.stdin,
        "wasi:cli/stdout": cli.stdout,
        "wasi:cli/terminal-input": cli.terminalInput,
        "wasi:cli/terminal-output": cli.terminalOutput,
        "wasi:cli/terminal-stderr": cli.terminalStderr,
        "wasi:cli/terminal-stdin": cli.terminalStdin,
        "wasi:cli/terminal-stdout": cli.terminalStdout,
        "wasi:clocks/monotonic-clock": clocks.monotonicClock,
        "wasi:clocks/wall-clock": clocks.wallClock,
        "wasi:filesystem/preopens": filesystem.preopens,
        "wasi:filesystem/types": filesystem.types,
        "wasi:io/error": io.error,
        "wasi:io/streams": io.streams,
        "wasi:random/random": random.random,
    };
}

async function instantiateRequest(workspaceRoot: string) {
    const imports = buildImports(workspaceRoot);
    return await instantiate(
        (path: string) => compileCore(path),
        imports as any,
        // Bun's WebAssembly.instantiate(Module, undefined) throws "first
        // argument must be an ArrayBufferView or an ArrayBuffer" — it
        // seems to take the BufferSource path when imports is undefined.
        // Passing an empty object when jco's generator omits imports
        // dodges the quirk.
        // Bun's `WebAssembly.instantiate(Module, undefined)` throws;
        // passing {} dodges the quirk when jco omits imports.
        (mod: WebAssembly.Module, instImports?: Record<string, any>) =>
            new Promise<WebAssembly.Instance>((resolve, reject) => {
                WebAssembly.instantiate(mod, instImports ?? {}).then(
                    // `WebAssembly.instantiate(Module, ...)` returns Instance
                    // directly (unlike the BufferSource overload which
                    // returns {module, instance}).
                    (inst: any) => resolve(inst.instance ?? inst),
                    reject,
                );
            }),
    );
}

export async function checkProject(req: CheckRequest): Promise<CheckResponse> {
    const root = await instantiateRequest(req.workspaceRoot);
    const json = root.api.check(req.projectDir);
    return JSON.parse(json);
}

export async function validateSlotData(
    req: ValidateSlotDataRequest,
): Promise<ValidationResponse> {
    const root = await instantiateRequest(req.workspaceRoot);
    const json = root.api.validateSlotData(
        req.projectDir,
        JSON.stringify(req.slotData),
    );
    return JSON.parse(json);
}

export async function generateProject(
    req: GenerateRequest,
): Promise<GenerateResponse> {
    const root = await instantiateRequest(req.workspaceRoot);
    const json = root.api.generate(
        req.projectDir,
        req.outDir,
        JSON.stringify(req.slotData),
        req.runHooks,
    );
    return JSON.parse(json);
}
