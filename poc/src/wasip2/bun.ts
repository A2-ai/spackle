// HOST-SIDE — Bun-specific loader for the wasip2 component.
//
// Uses the jco `--no-wasi-shim` artifact at `poc/wasip2-pkg-no-shim/`
// combined with the minimal Bun-native WASI implementation in
// `./bun-wasi.ts`. No `@bytecodealliance/preview2-shim`, no worker
// threads, no `process.binding` calls — so nothing depends on Bun
// implementing Node's internal bindings.
//
// Public API matches `./index.ts` (the Node reference loader) so test
// code can swap between runtimes.

import { createWasiImports } from "./bun-wasi";

// @ts-expect-error — the transpiled module ships its own .d.ts
import { instantiate } from "../../wasip2-pkg-no-shim/spackle.js";

// Import the core wasm modules as file assets — Bun resolves these
// to a filesystem path in dev mode and bundles them into the output
// in `bun build --compile` mode. Keeps the loader agnostic to runtime
// mode; avoids the "ENOENT on /wasip2-pkg-no-shim/spackle.core.wasm"
// drift that plain `readFile` hits inside a compiled binary.
// @ts-expect-error — type: "file" asset imports
import coreWasm from "../../wasip2-pkg-no-shim/spackle.core.wasm" with { type: "file" };
// @ts-expect-error
import core2Wasm from "../../wasip2-pkg-no-shim/spackle.core2.wasm" with { type: "file" };
// @ts-expect-error
import core3Wasm from "../../wasip2-pkg-no-shim/spackle.core3.wasm" with { type: "file" };
// @ts-expect-error
import core4Wasm from "../../wasip2-pkg-no-shim/spackle.core4.wasm" with { type: "file" };

const coreByName: Record<string, string> = {
    "spackle.core.wasm": coreWasm,
    "spackle.core2.wasm": core2Wasm,
    "spackle.core3.wasm": core3Wasm,
    "spackle.core4.wasm": core4Wasm,
};

// Cache compiled modules. Wrapped in a real `new Promise(...)` because
// Bun's `Bun.file().bytes()` returns an `InternalPromise` that fails
// `instanceof Promise`, and jco's generator driver uses that check.
const coreCache = new Map<string, Promise<WebAssembly.Module>>();
function compileCore(name: string): Promise<WebAssembly.Module> {
    const cached = coreCache.get(name);
    if (cached) return cached;
    const p = new Promise<WebAssembly.Module>((resolve, reject) => {
        const path = coreByName[name];
        if (!path) {
            reject(new Error(`unknown core wasm: ${name}`));
            return;
        }
        Bun.file(path)
            .bytes()
            .then((bytes) => WebAssembly.compile(bytes))
            .then(resolve, reject);
    });
    coreCache.set(name, p);
    return p;
}

export type CheckResponse =
    | { valid: true; config: Config; errors: [] }
    | { valid: false; errors: string[] };

export type ValidationResponse = { valid: true } | { valid: false; errors: string[] };

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

function buildImports(workspaceRoot: string) {
    const wasi = createWasiImports({
        preopens: { [workspaceRoot]: workspaceRoot },
    });

    return {
        ...wasi,
        "a2ai:spackle/host": {
            runCommand(
                cmd: string,
                args: string[],
                cwd: string,
                env: Array<[string, string]>,
            ) {
                const mergedEnv = { ...process.env, ...Object.fromEntries(env) };
                const proc = Bun.spawnSync([cmd, ...args], { cwd, env: mergedEnv });
                return {
                    stdout: proc.stdout,
                    stderr: proc.stderr,
                    exitCode: proc.exitCode ?? 1,
                };
            },
        },
    };
}

async function instantiateRequest(workspaceRoot: string) {
    const imports = buildImports(workspaceRoot);
    return await instantiate(
        (path: string) => compileCore(path),
        imports as any,
        // Bun's WebAssembly.instantiate(Module, undefined) throws; passing
        // {} dodges the quirk when jco's generator omits imports.
        (mod: WebAssembly.Module, instImports?: Record<string, any>) =>
            new Promise<WebAssembly.Instance>((resolve, reject) => {
                WebAssembly.instantiate(mod, instImports ?? {}).then(
                    (inst: any) => resolve(inst.instance ?? inst),
                    reject,
                );
            }),
    );
}

export async function checkProject(req: CheckRequest): Promise<CheckResponse> {
    const root = await instantiateRequest(req.workspaceRoot);
    return JSON.parse(root.api.check(req.projectDir));
}

export async function validateSlotData(
    req: ValidateSlotDataRequest,
): Promise<ValidationResponse> {
    const root = await instantiateRequest(req.workspaceRoot);
    return JSON.parse(
        root.api.validateSlotData(req.projectDir, JSON.stringify(req.slotData)),
    );
}

export async function generateProject(
    req: GenerateRequest,
): Promise<GenerateResponse> {
    const root = await instantiateRequest(req.workspaceRoot);
    return JSON.parse(
        root.api.generate(
            req.projectDir,
            req.outDir,
            JSON.stringify(req.slotData),
            req.runHooks,
        ),
    );
}
