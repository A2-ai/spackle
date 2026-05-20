# Custom bundle reader/writer

`DiskFs` is one path through the orchestrator. It's not the only way — the wasm primitives (`check`, `renderFile`, `renderPath`, `validateSlotData`, `planHooks`) only need bundles in and bytes out, so any source / sink that can shuffle those works.

## What you actually need to implement

For read:
- A function that returns a `Bundle` (`Array<{path: string, bytes: Uint8Array}>`) given whatever identifies a project in your storage (an S3 prefix, a git ref, a ZIP, a CMS entry).

For write:
- A function that accepts a destination identifier and writes one entry at a time. The orchestrator below renders each `.j2` template and emits each static file one by one, so the writer is called per-entry. The per-entry write path only creates parent directories lazily for the files it writes; truly empty source directories (no descendants, not in `ignore`) won't appear in the output unless your bundle reader emits explicit directory entries and your orchestrator handles them. The disk-backed `DiskFs` orchestrator preserves them by walking dir entries directly off the filesystem — the bundle shape (`Array<{path, bytes}>`) has no native concept of a directory, so custom hosts that care have to bring their own convention.

Also match native's **"outDir must not pre-exist"** contract: refuse the write if the target output location already exists, unless your host has a deliberate overwrite policy. The bundled `DiskFs.assertOutDirAvailable` enforces this on the disk path.

## Example: S3

```ts
import type { Bundle } from "@a2-ai/spackle";
import { S3Client, ListObjectsV2Command, GetObjectCommand, PutObjectCommand } from "@aws-sdk/client-s3";

const s3 = new S3Client({ region: "us-east-1" });

async function readProjectFromS3(bucket: string, prefix: string): Promise<Bundle> {
    const listed = await s3.send(new ListObjectsV2Command({ Bucket: bucket, Prefix: prefix }));
    const bundle: Bundle = [];
    for (const obj of listed.Contents ?? []) {
        if (!obj.Key) continue;
        const body = await s3.send(new GetObjectCommand({ Bucket: bucket, Key: obj.Key }));
        const bytes = new Uint8Array(await body.Body!.transformToByteArray());
        const relativePath = obj.Key.slice(prefix.length).replace(/^\/+/, "");
        bundle.push({ path: `/project/${relativePath}`, bytes });
    }
    return bundle;
}

interface OutputResult {
    files: { path: string; bytes: Uint8Array }[];
    dirs: string[];
}

async function writeOutputToS3(
    bucket: string,
    prefix: string,
    result: OutputResult,
): Promise<void> {
    // S3 has no notion of an empty directory — keys with a trailing
    // slash serve as markers for tools that care. Emit one per `dirs`
    // entry so browsers / sync tools see the structure.
    for (const dir of result.dirs) {
        await s3.send(new PutObjectCommand({
            Bucket: bucket,
            Key: `${prefix}/${dir}/`,
            Body: "",
        }));
    }
    for (const entry of result.files) {
        await s3.send(new PutObjectCommand({
            Bucket: bucket,
            Key: `${prefix}/${entry.path}`,
            Body: entry.bytes,
        }));
    }
}

// Compose the wasm primitives directly. No bundle-input `generate`
// ships — orchestrating a bundle-source walk is the custom-host's
// responsibility, because the orchestration shape varies per source.
import { loadSpackleWasm } from "@a2-ai/spackle";

const wasm = await loadSpackleWasm();
const bundle = await readProjectFromS3("my-bucket", "templates/my-template");

// 1. Validate.
const checkRes = wasm.check(bundle, "/project");
if (checkRes.diagnostics.some((d) => d.severity === "error")) {
    throw new Error("project check failed");
}
const slotRes = wasm.validateSlotData(bundle, "/project", { name: "hello" });
if (!slotRes.valid) throw new Error("invalid slot data");

// 2. Walk the bundle and render per file. Inject _project_name /
//    _output_name yourself (the disk orchestrator does this for you;
//    custom hosts handle it).
const data = {
    name: "hello",
    _project_name: checkRes.config?.name ?? "project",
    _output_name: "abc-123",
};
const ignore = new Set(checkRes.config?.ignore ?? []);
const segments = (rel: string) => rel.split("/");
const isIgnored = (rel: string) => segments(rel).some((s) => ignore.has(s));
// Skip the config file at any depth, plus any non-template under a
// directory literally named `spackle.toml` (native parity with
// `copy::copy_collect`'s skipped_ancestors).
const isConfigFile = (rel: string) => {
    const segs = segments(rel);
    return segs[segs.length - 1] === "spackle.toml";
};
const hasConfigAncestor = (rel: string) => {
    const segs = segments(rel);
    for (let i = 0; i < segs.length - 1; i++) {
        if (segs[i] === "spackle.toml") return true;
    }
    return false;
};

const outFiles: { path: string; bytes: Uint8Array }[] = [];
const outDirs = new Set<string>();
for (const entry of bundle) {
    if (!entry.path.startsWith("/project/")) continue;
    const rel = entry.path.slice("/project/".length);
    if (isConfigFile(rel)) continue;

    // Native `template::fill` walks the full tree; only the copy
    // stage applies ignore and the config-file ancestor skip.
    // Classify first, then filter non-templates.
    const isTemplate = /\.(j2|tera)$/.test(rel);
    if (!isTemplate) {
        if (isIgnored(rel)) continue;
        if (hasConfigAncestor(rel)) continue;
    }

    const pathRes = wasm.renderPath(rel, data);
    if (pathRes.diagnostics.length) throw new Error(pathRes.diagnostics[0].message);
    const renderedRel = pathRes.path;

    if (isTemplate) {
        const r = wasm.renderFile(entry.bytes, data, rel);
        if (r.diagnostics.length) throw new Error(r.diagnostics[0].message);
        outFiles.push({ path: renderedRel.replace(/\.(j2|tera)$/, ""), bytes: r.bytes });
    } else {
        outFiles.push({ path: renderedRel, bytes: entry.bytes });
    }
    // Accumulate the ancestor dirs of every written entry. Useful
    // for sinks that need explicit directory markers (e.g., S3
    // doesn't have real dirs — a `prefix/` key makes the structure
    // visible to browsers / sync tools). This does NOT preserve
    // truly empty source dirs — the bundle has no dir entries, so
    // a source dir with no descendants is invisible here.
    for (let p = renderedRel; p.includes("/"); p = p.slice(0, p.lastIndexOf("/"))) {
        outDirs.add(p.slice(0, p.lastIndexOf("/")));
    }
}

await writeOutputToS3("my-bucket", "outputs/abc-123", {
    files: outFiles,
    dirs: [...outDirs],
});
```

The custom orchestrator looks repetitive only because it's spelling out exactly what `generate` does for disk sources. The reason this isn't bundled into a `generateBundle` wrapper: each custom source has different idioms for reading bundles, allocating an output identifier, and writing back. Pulling that into one helper hides those decisions; spelling them out keeps the contract honest.

## Example: git ref

```ts
import { execSync } from "node:child_process";
import type { Bundle } from "@a2-ai/spackle";

function readProjectFromGit(repo: string, ref: string, subtree: string): Bundle {
    const tree = execSync(`git -C ${repo} ls-tree -r ${ref} ${subtree}`).toString();
    const bundle: Bundle = [];
    for (const line of tree.split("\n").filter(Boolean)) {
        const [, , hash, path] = line.match(/^(\S+)\s+(\S+)\s+(\S+)\s+(.+)$/)!;
        const bytes = execSync(`git -C ${repo} cat-file blob ${hash}`);
        bundle.push({
            path: `/project/${path.slice(subtree.length + 1)}`,
            bytes: new Uint8Array(bytes),
        });
    }
    return bundle;
}
```

## Virtual path conventions

Bundle paths are virtual — they only need to be absolute and consistent across all wasm calls for a single project. The convention used by the orchestrator is `/project/<relative>`; any prefix works in a custom reader as long as you pass the same string as `virtualProjectDir` to `checkBundle` (and to `wasm.renderPath` / `wasm.renderFile` in your own composition).

Output paths are entirely the host's choice — wasm only renders bytes; it has no opinion on where they live.

## Containment

`DiskFs` enforces a `workspaceRoot` boundary because "user-supplied project path" on disk could resolve anywhere via symlinks. Your bundle reader doesn't face that problem for storage it fully controls (S3, a vetted git repo) — the bundle only contains what you put in it.

If your source CAN contain hostile paths (e.g., a user-uploaded tarball), apply your own path filtering before building the bundle. wasm treats every entry as trusted.
