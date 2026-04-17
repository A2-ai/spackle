// HOST-SIDE — requires Node/Bun filesystem I/O. Not available in WASM.
//
// Everything in this file replaces a native-only piece of spackle:
//   readSpackleConfig  <- config::load_dir
//   walkTemplates      <- part of template::fill (the directory walk)
//   writeRenderedFiles <- part of template::fill (the fs::write loop)
//   copyNonTemplates   <- copy::copy (recursive copy, filename-templated)
//
// The WASM layer handles the actual parsing/rendering; these helpers just
// move bytes between disk and the WASM layer.

import { readFile, readdir, writeFile, mkdir, stat, copyFile } from "node:fs/promises";
import { join, dirname, relative } from "node:path";
import type { RenderedTemplate, SlotData, TemplateInput } from "../wasm/types.ts";
import type { SpackleWasm } from "../wasm/index.ts";

const CONFIG_FILE = "spackle.toml";
const TEMPLATE_EXT = ".j2";

/** HOST: Read `spackle.toml` from a project directory and return its raw
 * text. The WASM layer parses it. */
export async function readSpackleConfig(projectDir: string): Promise<string> {
  return readFile(join(projectDir, CONFIG_FILE), "utf-8");
}

/** HOST: Walk a project directory and collect every `.j2` template's
 * relative path + content, suitable for passing into `renderTemplates`.
 *
 * Skips: `spackle.toml`, any top-level name in `ignore`, and files whose
 * path contains an `ignore` name as a segment. */
export async function walkTemplates(
  projectDir: string,
  ignore: string[],
): Promise<TemplateInput[]> {
  const templates: TemplateInput[] = [];
  await walk(projectDir, projectDir, ignore, async (full, rel) => {
    if (!rel.endsWith(TEMPLATE_EXT)) return;
    const content = await readFile(full, "utf-8");
    templates.push({ path: rel, content });
  });
  return templates;
}

/** HOST: Write each rendered template to `<outDir>/<rendered_path>`.
 * Creates parent directories as needed. Entries with an `error` field are
 * skipped (the caller already surfaced them). Returns the number written. */
export async function writeRenderedFiles(
  outDir: string,
  rendered: RenderedTemplate[],
): Promise<number> {
  let count = 0;
  for (const file of rendered) {
    if (file.error) continue;
    const dest = join(outDir, file.rendered_path);
    await mkdir(dirname(dest), { recursive: true });
    await writeFile(dest, file.content);
    count++;
  }
  return count;
}

/** HOST: Copy all non-`.j2` files from `projectDir` to `outDir`, rendering
 * filenames through WASM (mirrors `copy::copy`: path templated, contents
 * left alone). Skips `spackle.toml`, any `ignore` entry, and `.j2` files.
 *
 * `data` should already include the special `_project_name` and
 * `_output_name` vars so that filename templates can reference them. */
export async function copyNonTemplates(
  projectDir: string,
  outDir: string,
  ignore: string[],
  data: SlotData,
  wasm: SpackleWasm,
): Promise<number> {
  let count = 0;
  await walk(projectDir, projectDir, ignore, async (full, rel) => {
    if (rel.endsWith(TEMPLATE_EXT)) return;
    if (rel === CONFIG_FILE) return;
    const renderedRel = wasm.renderString(rel, data);
    const dest = join(outDir, renderedRel);
    await mkdir(dirname(dest), { recursive: true });
    await copyFile(full, dest);
    count++;
  });
  return count;
}

/** HOST: Shared directory walker. Calls `visit(fullPath, relPath)` for each
 * file; skips any directory or file whose basename is in `ignore`. */
async function walk(
  base: string,
  dir: string,
  ignore: string[],
  visit: (full: string, rel: string) => Promise<void>,
): Promise<void> {
  const entries = await readdir(dir, { withFileTypes: true });
  for (const entry of entries) {
    if (ignore.includes(entry.name)) continue;
    const full = join(dir, entry.name);
    const rel = relative(base, full);
    if (entry.isDirectory()) {
      await walk(base, full, ignore, visit);
    } else if (entry.isFile()) {
      await visit(full, rel);
    }
  }
}

/** HOST: `stat(path).isDirectory()` — tiny convenience wrapper used by
 * orchestration to decide where to read config from. */
export async function isDirectory(path: string): Promise<boolean> {
  try {
    const s = await stat(path);
    return s.isDirectory();
  } catch {
    return false;
  }
}
