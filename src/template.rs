use std::{
    collections::HashMap,
    fmt::{Debug, Display},
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tera::{Context, Tera};
use thiserror::Error;

use super::slot::Slot;
use crate::fs::{self as fsmod, FileSystem, FileType};

/// File extensions that mark a file as a template.
///
/// Files ending with any of these are routed through `template::fill`
/// instead of `copy::copy`; the extension is stripped from the output path.
pub const TEMPLATE_EXTS: &[&str] = &[".j2", ".tera"];

/// Returns `true` if `name` ends with any of the configured template extensions.
pub fn has_template_ext(name: &str) -> bool {
    TEMPLATE_EXTS.iter().any(|ext| name.ends_with(ext))
}

/// If `name` ends with a template extension, returns the name without it.
/// Only the trailing extension is stripped (e.g. `foo.j2.tera` → `foo.j2`).
pub fn strip_template_ext(name: &str) -> Option<&str> {
    TEMPLATE_EXTS.iter().find_map(|ext| name.strip_suffix(ext))
}

/// Add as many templates as possible to a fresh Tera instance using
/// multi-pass topological ordering. Each pass tries every still-
/// unaccepted template via `add_raw_template`; it succeeds when its
/// cross-template refs (`{% include %}` / `{% extends %}`) are already
/// in the registry, so a child that extends a base will accept on the
/// pass after its base does. The loop exits when no progress is made
/// — at that point any still-pending templates either have genuine
/// parse errors, missing dependencies, or transitively depend on a
/// broken sibling.
///
/// Returns `(Tera, last_errors)` where `last_errors` maps each
/// rejected template's name to the most recent `add_raw_template`
/// error for it. Templates not in `last_errors` were accepted.
///
/// This shape is what gives the orchestrators partial-preview
/// resilience: rendering a good `target_path` doesn't fail just
/// because an unrelated sibling has a missing include or a syntax
/// error — only `target_path`'s own ancestry has to resolve.
fn build_resilient_registry(
    templates: &HashMap<String, String>,
) -> (Tera, HashMap<String, tera::Error>) {
    let mut tera = Tera::default();
    let mut remaining: Vec<(String, String)> = templates
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let mut last_err: HashMap<String, tera::Error> = HashMap::new();

    loop {
        let mut progress = false;
        let mut still_remaining: Vec<(String, String)> = Vec::new();
        for (name, content) in remaining.drain(..) {
            match tera.add_raw_template(&name, &content) {
                Ok(()) => {
                    progress = true;
                    last_err.remove(&name);
                }
                Err(e) => {
                    last_err.insert(name.clone(), e);
                    still_remaining.push((name, content));
                }
            }
        }
        remaining = still_remaining;
        if !progress || remaining.is_empty() {
            break;
        }
    }

    (tera, last_err)
}

/// Validate templates in memory: check that bodies AND filename templates
/// parse cleanly and that all variable references resolve to known slot
/// keys (or the special `_project_name` / `_output_name` vars). Mirrors
/// what `validate()` does against disk, from pre-loaded content. Each
/// error carries a [`ValidateFileErrorKind`] so callers can distinguish
/// body vs filename failures and tag diagnostics with the right source.
pub fn validate_in_memory(
    templates: &HashMap<String, String>,
    slots: &[super::slot::Slot],
) -> Result<(), ValidateError> {
    let (tera, registry_errs) = build_resilient_registry(templates);
    let mut errors: Vec<ValidateFileError> = registry_errs
        .into_iter()
        .map(|(file, error)| ValidateFileError {
            file,
            kind: ValidateFileErrorKind::Body,
            error,
        })
        .collect();

    let mut context = Context::from_serialize(
        &slots
            .iter()
            .map(|s| (s.key.clone(), ""))
            .collect::<HashMap<_, _>>(),
    )
    .map_err(ValidateError::TeraError)?;
    context.insert("_project_name".to_string(), "");
    context.insert("_output_name".to_string(), "");

    // Body render against the empty context — catches undefined slot refs.
    let template_names: Vec<String> = tera.get_template_names().map(|s| s.to_string()).collect();
    for name in &template_names {
        if let Err(e) = tera.render(name, &context) {
            errors.push(ValidateFileError {
                file: name.clone(),
                kind: ValidateFileErrorKind::Body,
                error: e,
            });
        }
    }

    // Filename templating — `render_in_memory` runs `tera.render_str` on
    // every `.j2`/`.tera` filename; mirror that here so the static check
    // surfaces filename parse errors and undefined-var refs (e.g.
    // `{{ unknown }}.txt.j2`) without needing slot data.
    for path in templates.keys() {
        if let Err(e) = Tera::one_off(path, &context, false) {
            errors.push(ValidateFileError {
                file: path.clone(),
                kind: ValidateFileErrorKind::Filename,
                error: e,
            });
        }
    }

    if !errors.is_empty() {
        return Err(ValidateError::RenderError(errors));
    }

    Ok(())
}

/// Render a single template from an in-memory registry. Builds the
/// registry with [`build_resilient_registry`] so unrelated siblings
/// that fail to add (genuine parse errors, missing transitive deps)
/// don't poison the render of an unrelated `target_path`. Tera 2's
/// cross-template tags `{% include %}` and `{% extends %}` resolve;
/// `{% macro %}` / `{% import %}` are not supported by Tera 2.
///
/// Errors are attributed to the offending file: if `target_path`
/// itself failed to register, the returned `FileError.file` is
/// `target_path` and the kind is `ErrorParsingTemplate`. Render-time
/// failures (undefined slot refs in the target or its included
/// templates) come through `tera.render` and are attributed to
/// `target_path` since that's the entry point for this call.
pub fn render_one_from_memory(
    templates: &HashMap<String, String>,
    target_path: &str,
    data: &HashMap<String, String>,
) -> Result<String, FileError> {
    let (tera, mut registry_errs) = build_resilient_registry(templates);
    if let Some(e) = registry_errs.remove(target_path) {
        return Err(FileError {
            kind: FileErrorKind::ErrorParsingTemplate(e),
            file: target_path.to_string(),
        });
    }
    let context = Context::from_serialize(data).map_err(|e| FileError {
        kind: FileErrorKind::ErrorRenderingContents(e),
        file: target_path.to_string(),
    })?;
    tera.render(target_path, &context).map_err(|e| FileError {
        kind: FileErrorKind::ErrorRenderingContents(e),
        file: target_path.to_string(),
    })
}

/// Render templates in memory without touching the filesystem.
/// `templates` maps relative paths (e.g. "{{name}}.txt.j2") to content strings.
/// Returns a rendered file for each template, or per-file errors.
/// The caller (TypeScript) is responsible for reading and writing files.
pub fn render_in_memory(
    templates: &HashMap<String, String>,
    data: &HashMap<String, String>,
) -> Result<Vec<Result<RenderedFile, FileError>>, tera::Error> {
    let (tera, registry_errs) = build_resilient_registry(templates);
    let parse_failures: Vec<FileError> = registry_errs
        .into_iter()
        .map(|(file, error)| FileError {
            kind: FileErrorKind::ErrorParsingTemplate(error),
            file,
        })
        .collect();
    let context = Context::from_serialize(data)?;

    let template_names = tera.get_template_names();
    let rendered = template_names.into_iter().map(|template_name| {
        // std::time::Instant is not available on wasm32-unknown-unknown
        // (no OS clock). Use Duration::ZERO as a placeholder there.
        #[cfg(not(target_arch = "wasm32"))]
        let start_time = std::time::Instant::now();

        // Render the file contents
        let output = match tera.render(template_name, &context) {
            Ok(o) => o,
            Err(e) => {
                return Err(FileError {
                    kind: FileErrorKind::ErrorRenderingContents(e),
                    file: template_name.to_string(),
                });
            }
        };

        // Render the file name (allows {{ var }} in filenames). Only
        // template-extension files get their name templated — others
        // wouldn't reach `render_in_memory` in the first place, but
        // guard defensively.
        let mut rendered_name = template_name.to_string();
        if has_template_ext(&rendered_name) {
            let tera_clone = tera.clone();
            rendered_name = match tera_clone.render_str(&rendered_name, &context, false) {
                Ok(s) => s,
                Err(e) => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorRenderingName(e),
                        file: template_name.to_string(),
                    });
                }
            };
        }

        // Strip the trailing template extension (only one — so
        // `foo.j2.tera` becomes `foo.j2`).
        let rendered_name = strip_template_ext(&rendered_name)
            .map(|s| s.to_string())
            .unwrap_or(rendered_name);

        #[cfg(not(target_arch = "wasm32"))]
        let elapsed = start_time.elapsed();
        #[cfg(target_arch = "wasm32")]
        let elapsed = Duration::ZERO;

        Ok(RenderedFile {
            original_path: template_name.to_string().into(),
            path: rendered_name.into(),
            contents: output,
            elapsed,
        })
    });

    let mut out: Vec<Result<RenderedFile, FileError>> =
        parse_failures.into_iter().map(Err).collect();
    out.extend(rendered);
    Ok(out)
}

#[derive(Error, Debug)]
pub struct FileError {
    pub kind: FileErrorKind,
    pub file: String,
}

impl Display for FileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.file, self.kind)
    }
}

#[derive(Error, Debug)]
pub enum FileErrorKind {
    #[error("Template parse error: {0}")]
    ErrorParsingTemplate(tera::Error),
    #[error("Error rendering contents: {0}")]
    ErrorRenderingContents(tera::Error),
    #[error("Error rendering name: {0}")]
    ErrorRenderingName(tera::Error),
    #[error("Error creating destination: {0}")]
    ErrorCreatingDest(io::ErrorKind),
    #[error("Error writing to destination: {0}")]
    ErrorWritingToDest(io::Error),
}

#[derive(Debug, Clone)]
pub struct RenderedFile {
    /// The original template name as it appeared in the source (e.g. `{{slot_1}}.j2`).
    pub original_path: PathBuf,
    /// The rendered output path after variable substitution and template-extension stripping.
    pub path: PathBuf,
    pub contents: String,
    pub elapsed: Duration,
}

/// Collect all template files (ending in any `TEMPLATE_EXTS` suffix)
/// under `project_dir` into a map of `relative_path → content` using the
/// `fs` backend. Used by `fill` and `validate` to replace Tera's
/// built-in glob loader (which bypasses the abstraction and calls
/// `std::fs` directly).
fn collect_templates<F: FileSystem>(
    fs: &F,
    project_dir: &Path,
) -> io::Result<HashMap<String, String>> {
    let entries = fsmod::walk(fs, project_dir)?;
    let mut templates = HashMap::new();
    for (rel, stat) in entries {
        if stat.file_type != FileType::File {
            continue;
        }
        let name = rel.to_string_lossy().into_owned();
        if !has_template_ext(&name) {
            continue;
        }
        let bytes = fs.read_file(&project_dir.join(&rel))?;
        let content = String::from_utf8(bytes)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        templates.insert(name, content);
    }
    Ok(templates)
}

pub fn fill<F: FileSystem>(
    fs: &F,
    project_dir: &Path,
    out_dir: &Path,
    data: &HashMap<String, String>,
) -> Result<Vec<Result<RenderedFile, FileError>>, tera::Error> {
    // Collect templates via the fs trait, render in memory (per-file —
    // no accumulating the whole project's output bytes), then write each
    // result via the fs trait. No direct std::fs, no Tera::new(glob).
    let templates = collect_templates(fs, project_dir)
        .map_err(|e| tera::Error::message(format!("failed to walk project dir: {}", e)))?;

    let rendered = render_in_memory(&templates, data)?;

    let mut out = Vec::with_capacity(rendered.len());
    for result in rendered {
        match result {
            Ok(rf) => {
                let dest_path = out_dir.join(&rf.path);
                if let Some(parent) = dest_path.parent() {
                    if let Err(e) = fs.create_dir_all(parent) {
                        if e.kind() != io::ErrorKind::AlreadyExists {
                            out.push(Err(FileError {
                                kind: FileErrorKind::ErrorCreatingDest(e.kind()),
                                file: rf.path.to_string_lossy().into_owned(),
                            }));
                            continue;
                        }
                    }
                }
                if let Err(e) = fs.write_file(&dest_path, rf.contents.as_bytes()) {
                    out.push(Err(FileError {
                        kind: FileErrorKind::ErrorWritingToDest(e),
                        file: rf.path.to_string_lossy().into_owned(),
                    }));
                    continue;
                }
                out.push(Ok(rf));
            }
            Err(e) => out.push(Err(e)),
        }
    }
    Ok(out)
}

#[derive(Debug)]
pub enum ValidateError {
    TeraError(tera::Error),
    RenderError(Vec<ValidateFileError>),
}

#[derive(Debug)]
pub struct ValidateFileError {
    pub file: String,
    pub kind: ValidateFileErrorKind,
    pub error: tera::Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidateFileErrorKind {
    /// Template body — content failed to parse or referenced an undefined slot.
    Body,
    /// Filename template — the `.j2`/`.tera` file's name (e.g.
    /// `{{ slot }}.txt.j2`) failed to parse or referenced an undefined
    /// slot. Mirrors `render_in_memory`'s filename-render site.
    Filename,
}

impl Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidateError::TeraError(e) => write!(f, "Error validating template files: {}", e),
            ValidateError::RenderError(errors) => {
                // tera 2's `Error::Display` already prints a rich
                // diagnostic (via `ReportError::generate_report`) that
                // includes the offending variable name and span — no
                // need to unwrap `.source()` to get the useful text
                // (unlike tera 1, where the detail lived on the chained
                // source). Just format the error directly.
                writeln!(f, "Error rendering one or more templates:")?;
                for err in errors {
                    writeln!(f, "  {} ({:?}): {}", err.file, err.kind, err.error)?;
                }
                Ok(())
            }
        }
    }
}

// Validates the templates in the directory against the slots
// Returns an error if any of the templates reference a slot that doesn't exist
pub fn validate<F: FileSystem>(fs: &F, dir: &Path, slots: &Vec<Slot>) -> Result<(), ValidateError> {
    let templates = collect_templates(fs, dir).map_err(|e| {
        ValidateError::TeraError(tera::Error::message(format!(
            "failed to walk project dir: {}",
            e
        )))
    })?;
    validate_in_memory(&templates, slots)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Disk-backed end-to-end coverage for `fill` + `validate` lives in
    // `tests/templating.rs` against `tests/fixtures/basic_project` — it's
    // thorough enough that duplicating it here (against private mirror
    // fixtures) just adds drift risk.

    // --- Table-driven tests for in-memory functions ---

    #[test]
    fn render_in_memory_table() {
        struct Case {
            name: &'static str,
            templates: Vec<(&'static str, &'static str)>,
            data: Vec<(&'static str, &'static str)>,
            expect_ok_count: usize,
            expect_err_count: usize,
            // (original_path, rendered_path, content_contains)
            check_files: Vec<(&'static str, &'static str, &'static str)>,
        }

        let cases = vec![
            Case {
                name: "simple variable substitution",
                templates: vec![("hello.txt.j2", "Hello {{ name }}!")],
                data: vec![("name", "world")],
                expect_ok_count: 1,
                expect_err_count: 0,
                check_files: vec![("hello.txt.j2", "hello.txt", "Hello world!")],
            },
            Case {
                name: "templated filename",
                templates: vec![("{{ name }}.txt.j2", "content")],
                data: vec![("name", "output")],
                expect_ok_count: 1,
                expect_err_count: 0,
                check_files: vec![("{{ name }}.txt.j2", "output.txt", "content")],
            },
            Case {
                name: "undefined variable causes per-file error",
                templates: vec![("good.j2", "{{ x }}"), ("bad.j2", "{{ undefined_var }}")],
                data: vec![("x", "ok")],
                expect_ok_count: 1,
                expect_err_count: 1,
                check_files: vec![("good.j2", "good", "ok")],
            },
            Case {
                name: "empty template map",
                templates: vec![],
                data: vec![("x", "1")],
                expect_ok_count: 0,
                expect_err_count: 0,
                check_files: vec![],
            },
            Case {
                name: "multiple variables",
                templates: vec![("t.j2", "{{ a }} + {{ b }} = {{ c }}")],
                data: vec![("a", "1"), ("b", "2"), ("c", "3")],
                expect_ok_count: 1,
                expect_err_count: 0,
                check_files: vec![("t.j2", "t", "1 + 2 = 3")],
            },
        ];

        for c in cases {
            let templates: HashMap<String, String> = c
                .templates
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let data: HashMap<String, String> = c
                .data
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();

            let results = render_in_memory(&templates, &data).expect(&format!(
                "case {}: render_in_memory should not return Err",
                c.name
            ));

            let ok_count = results.iter().filter(|r| r.is_ok()).count();
            let err_count = results.iter().filter(|r| r.is_err()).count();
            assert_eq!(ok_count, c.expect_ok_count, "case {}: ok count", c.name);
            assert_eq!(err_count, c.expect_err_count, "case {}: err count", c.name);

            for (orig, rendered, content_needle) in &c.check_files {
                let file = results
                    .iter()
                    .filter_map(|r| r.as_ref().ok())
                    .find(|f| f.original_path.to_string_lossy() == *orig)
                    .unwrap_or_else(|| {
                        panic!("case {}: missing file with original_path={}", c.name, orig)
                    });
                assert_eq!(
                    file.path.to_string_lossy(),
                    *rendered,
                    "case {}: rendered_path",
                    c.name
                );
                assert!(
                    file.contents.contains(content_needle),
                    "case {}: content should contain {:?}, got {:?}",
                    c.name,
                    content_needle,
                    file.contents,
                );
            }
        }
    }

    #[test]
    fn render_one_from_memory_resolves_include() {
        let mut templates = HashMap::new();
        templates.insert(
            "main.j2".to_string(),
            r#"hello {% include "partial.j2" %}"#.to_string(),
        );
        templates.insert("partial.j2".to_string(), "world".to_string());
        let data: HashMap<String, String> = HashMap::new();
        let out = render_one_from_memory(&templates, "main.j2", &data).expect("render ok");
        assert_eq!(out, "hello world");
    }

    #[test]
    fn render_one_from_memory_resolves_extends() {
        let mut templates = HashMap::new();
        templates.insert(
            "base.j2".to_string(),
            "BEGIN {% block body %}default{% endblock body %} END".to_string(),
        );
        templates.insert(
            "child.j2".to_string(),
            r#"{% extends "base.j2" %}{% block body %}hi {{ who }}{% endblock body %}"#.to_string(),
        );
        let mut data: HashMap<String, String> = HashMap::new();
        data.insert("who".to_string(), "world".to_string());
        let out = render_one_from_memory(&templates, "child.j2", &data).expect("render ok");
        assert_eq!(out, "BEGIN hi world END");
    }

    #[test]
    fn render_one_from_memory_renders_only_target() {
        // A non-target template with a bad reference does NOT fail the
        // render call unless the target transitively references it.
        let mut templates = HashMap::new();
        templates.insert("target.j2".to_string(), "ok".to_string());
        templates.insert("unrelated.j2".to_string(), "{{ undefined_var }}".to_string());
        let data: HashMap<String, String> = HashMap::new();
        let out = render_one_from_memory(&templates, "target.j2", &data).expect("render ok");
        assert_eq!(out, "ok");
    }

    #[test]
    fn validate_in_memory_allows_cross_template_tags() {
        // include / extends should validate cleanly when the
        // referenced template is in the registry, regardless of
        // HashMap iteration order. (Tera 2 dropped macros/import, so
        // those aren't exercised here.)
        let mut templates = HashMap::new();
        templates.insert(
            "main.j2".to_string(),
            r#"{% include "partial.j2" %}"#.to_string(),
        );
        templates.insert("partial.j2".to_string(), "static".to_string());
        templates.insert(
            "base.j2".to_string(),
            "{% block body %}default{% endblock body %}".to_string(),
        );
        templates.insert(
            "child.j2".to_string(),
            r#"{% extends "base.j2" %}{% block body %}hi{% endblock body %}"#.to_string(),
        );
        let slots: Vec<Slot> = Vec::new();
        validate_in_memory(&templates, &slots).expect("validation should pass");
    }

    #[test]
    fn validate_in_memory_attributes_missing_include_to_offender_not_siblings() {
        // `good.j2` is fine. `bad.j2` references a missing template.
        // Validation should flag `bad.j2`, not `good.j2`.
        let mut templates = HashMap::new();
        templates.insert("good.j2".to_string(), "{{ x }}".to_string());
        templates.insert(
            "bad.j2".to_string(),
            r#"{% include "nope.j2" %}"#.to_string(),
        );
        let slots = vec![Slot {
            key: "x".to_string(),
            ..Default::default()
        }];
        let err = validate_in_memory(&templates, &slots).expect_err("should flag bad.j2");
        match err {
            ValidateError::RenderError(errs) => {
                let files: Vec<&str> = errs.iter().map(|e| e.file.as_str()).collect();
                assert!(files.contains(&"bad.j2"), "expected bad.j2 in errs: {:?}", files);
                assert!(!files.contains(&"good.j2"), "good.j2 wrongly flagged: {:?}", files);
            }
            _ => panic!("expected RenderError"),
        }
    }

    #[test]
    fn render_in_memory_partial_preview_skips_only_unresolvable() {
        // One template with missing include, two that are fine. The
        // good ones should render; the bad one should surface as a
        // per-file FileError. No global Err out — `render_in_memory`'s
        // top-level Err is reserved for `Context::from_serialize` /
        // global tera setup failures.
        let mut templates = HashMap::new();
        templates.insert("a.j2".to_string(), "A".to_string());
        templates.insert("b.j2".to_string(), "B".to_string());
        templates.insert(
            "bad.j2".to_string(),
            r#"{% include "nope.j2" %}"#.to_string(),
        );
        let data: HashMap<String, String> = HashMap::new();
        let results = render_in_memory(&templates, &data).expect("global render ok");
        let oks: Vec<String> = results
            .iter()
            .filter_map(|r| r.as_ref().ok())
            .map(|f| f.original_path.to_string_lossy().into_owned())
            .collect();
        let errs: Vec<String> = results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .map(|e| e.file.clone())
            .collect();
        assert!(oks.contains(&"a.j2".to_string()), "a.j2 missing from oks: {:?}", oks);
        assert!(oks.contains(&"b.j2".to_string()), "b.j2 missing from oks: {:?}", oks);
        assert_eq!(errs, vec!["bad.j2".to_string()]);
    }

    #[test]
    fn validate_in_memory_flags_missing_include_target() {
        let mut templates = HashMap::new();
        templates.insert(
            "main.j2".to_string(),
            r#"{% include "missing.j2" %}"#.to_string(),
        );
        let slots: Vec<Slot> = Vec::new();
        let err = validate_in_memory(&templates, &slots).expect_err("should flag missing include");
        match err {
            ValidateError::RenderError(errs) => {
                assert!(!errs.is_empty());
            }
            _ => panic!("expected RenderError"),
        }
    }

    #[test]
    fn render_one_from_memory_unrelated_bad_template_does_not_poison_target() {
        // Multi-pass registry construction: `target.j2` has no
        // cross-refs and parses cleanly, so it lands in the registry
        // on pass 1 regardless of `bad.j2`. `bad.j2` references a
        // non-existent template and never registers, but rendering
        // `target.j2` succeeds because its own ancestry is intact.
        let mut templates = HashMap::new();
        templates.insert("target.j2".to_string(), "ok".to_string());
        templates.insert(
            "bad.j2".to_string(),
            r#"{% include "nope.j2" %}"#.to_string(),
        );
        let data: HashMap<String, String> = HashMap::new();
        let out =
            render_one_from_memory(&templates, "target.j2", &data).expect("target should render");
        assert_eq!(out, "ok");
    }

    #[test]
    fn render_one_from_memory_target_with_missing_include_attributes_to_target() {
        // Target itself has a broken include — error is attributed to
        // the target's file, not the missing one.
        let mut templates = HashMap::new();
        templates.insert(
            "target.j2".to_string(),
            r#"{% include "nope.j2" %}"#.to_string(),
        );
        let data: HashMap<String, String> = HashMap::new();
        let err = render_one_from_memory(&templates, "target.j2", &data)
            .expect_err("missing include should error");
        assert_eq!(err.file, "target.j2");
        assert!(matches!(err.kind, FileErrorKind::ErrorParsingTemplate(_)));
    }

    #[test]
    fn render_one_from_memory_target_parse_error_carries_path() {
        let mut templates = HashMap::new();
        templates.insert("target.j2".to_string(), "{% if %}".to_string());
        let data: HashMap<String, String> = HashMap::new();
        let err = render_one_from_memory(&templates, "target.j2", &data).expect_err("parse err");
        assert_eq!(err.file, "target.j2");
        assert!(matches!(err.kind, FileErrorKind::ErrorParsingTemplate(_)));
    }

    #[test]
    fn validate_in_memory_table() {
        struct Case {
            name: &'static str,
            templates: Vec<(&'static str, &'static str)>,
            slots: Vec<&'static str>,
            expect_valid: bool,
        }

        let cases = vec![
            Case {
                name: "all vars defined",
                templates: vec![("t.j2", "{{ x }}")],
                slots: vec!["x"],
                expect_valid: true,
            },
            Case {
                name: "undefined var",
                templates: vec![("t.j2", "{{ missing }}")],
                slots: vec!["x"],
                expect_valid: false,
            },
            Case {
                name: "special vars always available",
                templates: vec![("t.j2", "{{ _project_name }} {{ _output_name }}")],
                slots: vec![],
                expect_valid: true,
            },
            Case {
                name: "empty templates = valid",
                templates: vec![],
                slots: vec![],
                expect_valid: true,
            },
            Case {
                name: "mix of valid and invalid",
                templates: vec![("good.j2", "{{ x }}"), ("bad.j2", "{{ nope }}")],
                slots: vec!["x"],
                expect_valid: false,
            },
        ];

        for c in cases {
            let templates: HashMap<String, String> = c
                .templates
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            let slots: Vec<Slot> = c
                .slots
                .into_iter()
                .map(|k| Slot {
                    key: k.to_string(),
                    ..Default::default()
                })
                .collect();

            let result = validate_in_memory(&templates, &slots);
            assert_eq!(
                result.is_ok(),
                c.expect_valid,
                "case {}: expected valid={}, got {:?}",
                c.name,
                c.expect_valid,
                result,
            );
        }
    }

}
