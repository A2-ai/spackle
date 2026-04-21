use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display},
    io,
    path::{Path, PathBuf},
    time::Duration,
};
use tera::{Context, Tera};
use thiserror::Error;

use super::slot::Slot;
use crate::fs::{self as fsmod, FileSystem, FileType};

pub const TEMPLATE_EXT: &str = ".j2";

/// Validate templates in memory: check that all variable references in the
/// templates resolve to known slot keys (or the special _project_name /
/// _output_name vars). Mirrors what `validate()` does against disk, but
/// from pre-loaded content.
pub fn validate_in_memory(
    templates: &HashMap<String, String>,
    slots: &[super::slot::Slot],
) -> Result<(), ValidateError> {
    let mut tera = Tera::default();
    for (path, content) in templates {
        tera.add_raw_template(path, content)
            .map_err(|e| ValidateError::TeraError(e))?;
    }

    let mut context = Context::from_serialize(
        slots
            .iter()
            .map(|s| (s.key.clone(), ""))
            .collect::<HashMap<_, _>>(),
    )
    .map_err(ValidateError::TeraError)?;
    context.insert("_project_name".to_string(), "");
    context.insert("_output_name".to_string(), "");

    let errors: Vec<(String, tera::Error)> = tera
        .get_template_names()
        .filter_map(|name| match tera.render(name, &context) {
            Ok(_) => None,
            Err(e) => Some((name.to_string(), e)),
        })
        .collect();

    if !errors.is_empty() {
        return Err(ValidateError::RenderError(errors));
    }

    Ok(())
}

/// Render templates in memory without touching the filesystem.
/// `templates` maps relative paths (e.g. "{{name}}.txt.j2") to content strings.
/// Returns a rendered file for each template, or per-file errors.
/// The caller (TypeScript) is responsible for reading and writing files.
pub fn render_in_memory(
    templates: &HashMap<String, String>,
    data: &HashMap<String, String>,
) -> Result<Vec<Result<RenderedFile, FileError>>, tera::Error> {
    let mut tera = Tera::default();
    for (path, content) in templates {
        tera.add_raw_template(path, content)
            .map_err(|e| tera::Error::msg(format!("failed to add template {}: {}", path, e)))?;
    }
    let context = Context::from_serialize(data)?;

    let template_names: Vec<String> = tera.get_template_names().map(|s| s.to_string()).collect();
    let rendered = template_names.iter().map(|template_name| {
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

        // Render the file name (allows {{ var }} in filenames)
        let mut rendered_name = template_name.to_string();
        if rendered_name.ends_with(TEMPLATE_EXT) {
            let mut tera_clone = tera.clone();
            rendered_name = match tera_clone.render_str(&rendered_name, &context) {
                Ok(s) => s,
                Err(e) => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorRenderingName(e),
                        file: template_name.to_string(),
                    });
                }
            };
        }

        // Strip .j2 suffix
        let rendered_name = match rendered_name.strip_suffix(TEMPLATE_EXT) {
            Some(name) => name.to_string(),
            None => rendered_name,
        };

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

    Ok(rendered.collect())
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
    /// The rendered output path after variable substitution and .j2 stripping.
    pub path: PathBuf,
    pub contents: String,
    pub elapsed: Duration,
}

/// Collect all `.j2` templates under `project_dir` into a map of
/// `relative_path → content` using the `fs` backend. Used by `fill` and
/// `validate` to replace Tera's built-in glob loader (which bypasses
/// the abstraction and calls `std::fs` directly).
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
        if !name.ends_with(TEMPLATE_EXT) {
            continue;
        }
        let bytes = fs.read_file(&project_dir.join(&rel))?;
        let content = String::from_utf8(bytes).map_err(|e| {
            io::Error::new(io::ErrorKind::InvalidData, e.to_string())
        })?;
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
    let templates = collect_templates(fs, project_dir).map_err(|e| {
        tera::Error::msg(format!("failed to walk project dir: {}", e))
    })?;

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
    RenderError(Vec<(String, tera::Error)>),
}

// Add Display implementation for ValidateError
impl Display for ValidateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ValidateError::TeraError(e) => write!(f, "Error validating template files: {}", e),
            ValidateError::RenderError(errors) => {
                writeln!(f, "Error rendering one or more templates:")?;
                for (template, error) in errors {
                    writeln!(
                        f,
                        "  {}: {}",
                        template,
                        error.source().map(|e| e.to_string()).unwrap_or_default()
                    )?;
                }
                Ok(())
            }
        }
    }
}

// Validates the templates in the directory against the slots
// Returns an error if any of the templates reference a slot that doesn't exist
pub fn validate<F: FileSystem>(
    fs: &F,
    dir: &Path,
    slots: &Vec<Slot>,
) -> Result<(), ValidateError> {
    let templates = collect_templates(fs, dir).map_err(|e| {
        ValidateError::TeraError(tera::Error::msg(format!(
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
                templates: vec![
                    ("good.j2", "{{ x }}"),
                    ("bad.j2", "{{ undefined_var }}"),
                ],
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
                templates: vec![
                    ("good.j2", "{{ x }}"),
                    ("bad.j2", "{{ nope }}"),
                ],
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
