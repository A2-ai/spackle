# Custom bundle reader/writer

`DiskFs` is one way to get files in and out of wasm. It's not the only way — the bundle contract is just `Array<{path: string, bytes: Uint8Array}>`. Any source that can produce or consume that shape works.

## What you actually need to implement

For read:
- A function that returns a `Bundle` given whatever identifies a project in your storage (an S3 prefix, a git ref, a ZIP, a CMS entry).

For write:
- A function that accepts `(outIdentifier, { files, dirs })` and writes each entry under some output location. **Create each directory in `dirs` explicitly** — a successful `generate` response carries both `files` AND `dirs` precisely because native spackle's copy pass `create_dir_all`s every directory it walks, and the wasm output must match for behavioral parity. Emitting only files silently drops empty dirs (e.g. a `drafts/` folder whose every child is ignored).

That's it. There's no interface to implement. No callbacks passed to wasm. Just produce `Bundle` for input; consume `{ files, dirs }` for output.

Also match native's **"outDir must not pre-exist"** contract: refuse the write (or error out) if the target output location already exists. The bundled `DiskFs.writeOutput` does this; custom writers should too unless they have a deliberate overwrite policy.

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

// Use with generateBundle (bypass DiskFs entirely):
import { generateBundle } from "@a2-ai/spackle";

const bundle = await readProjectFromS3("my-bucket", "templates/my-template");
const result = await generateBundle(bundle, { name: "hello" });
if (result.ok) {
    await writeOutputToS3("my-bucket", "outputs/abc-123", {
        files: result.files,
        dirs: result.dirs,
    });
}
```

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

Bundle paths are virtual — they only need to be absolute and consistent within one `check` / `generate` call. The conventions used by `DiskFs`:

- Input project: `/project/<relative>` (configurable via `opts.virtualRoot`).
- Output bundle: paths relative to `outDir` (fixed by Rust; can't be changed).

You can use any prefix you like in a custom reader — just pass the same string as `virtualProjectDir` when you call `checkBundle` / `generateBundle`.

## Containment

`DiskFs` enforces a `workspaceRoot` boundary because "user-supplied project path" on disk could resolve anywhere via symlinks. Your bundle reader doesn't face that problem for storage it fully controls (S3, a vetted git repo) — the bundle only contains what you put in it.

If your source CAN contain hostile paths (e.g., a user-uploaded tarball), apply your own path filtering before building the bundle. wasm treats every entry as trusted.
