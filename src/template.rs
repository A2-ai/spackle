use std::{
    collections::HashMap,
    error::Error,
    fmt::{Debug, Display},
    fs, io,
    path::{Path, PathBuf},
    time::Duration,
};
use tera::{Context, Tera};
use thiserror::Error;

use super::slot::Slot;

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

/// Glob suffix matching any template extension, for use with Tera's
/// `load_from_glob` (globwalk supports brace alternation).
fn template_glob_suffix() -> String {
    let alts = TEMPLATE_EXTS
        .iter()
        .map(|ext| ext.trim_start_matches('.'))
        .collect::<Vec<_>>()
        .join(",");
    format!("*.{{{}}}", alts)
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
    pub path: PathBuf,
    pub contents: String,
    pub elapsed: Duration,
}

pub fn fill(
    project_dir: &Path,
    out_dir: &Path,
    data: &HashMap<String, String>,
) -> Result<Vec<Result<RenderedFile, FileError>>, tera::Error> {
    let glob = project_dir.join("**").join(template_glob_suffix());

    let mut tera = Tera::new();
    tera.load_from_glob(&glob.to_string_lossy())?;

    let context = Context::from_serialize(data)?;

    let rendered_templates = tera.templates.iter().map(|(template_name, _)| {
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

        // Render the file name
        let mut template_name = template_name.to_string();
        if has_template_ext(&template_name) {
            let tera = tera.clone();
            template_name = match tera.render_str(&template_name, &context, false) {
                Ok(s) => s,
                Err(e) => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorRenderingName(e),
                        file: template_name.to_string(),
                    });
                }
            };
        }

        let template_name = strip_template_ext(&template_name).unwrap_or(template_name.as_str());

        // Write the output
        let output_dir = out_dir.join(template_name);

        match fs::create_dir_all(output_dir.parent().unwrap()) {
            Ok(_) => (),
            Err(e) => match e.kind() {
                std::io::ErrorKind::AlreadyExists => (),
                e => {
                    return Err(FileError {
                        kind: FileErrorKind::ErrorCreatingDest(e),
                        file: template_name.to_string(),
                    })
                }
            },
        }

        fs::write(&output_dir, output.clone()).map_err(|e| FileError {
            kind: FileErrorKind::ErrorWritingToDest(e),
            file: template_name.to_string(),
        })?;

        Ok(RenderedFile {
            path: template_name.into(),
            contents: output,
            elapsed: start_time.elapsed(),
        })
    });

    Ok(rendered_templates.collect::<Vec<_>>())
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
pub fn validate(dir: &PathBuf, slots: &Vec<Slot>) -> Result<(), ValidateError> {
    let glob = dir.join("**").join(template_glob_suffix());

    let mut tera = Tera::new();
    tera.load_from_glob(&glob.to_string_lossy())
        .map_err(ValidateError::TeraError)?;
    let mut context = Context::from_serialize(
        &slots
            .iter()
            .map(|s| (s.key.clone(), ""))
            .collect::<HashMap<_, _>>(),
    )
    .map_err(ValidateError::TeraError)?;
    context.insert("_project_name".to_string(), "");
    context.insert("_output_name".to_string(), "");

    let errors = tera
        .templates
        .iter()
        .filter_map(
            |(template_name, _)| match tera.render(template_name, &context) {
                Ok(_) => None,
                Err(e) => Some((template_name.to_string(), e)),
            },
        )
        .collect::<Vec<_>>();

    if !errors.is_empty() {
        return Err(ValidateError::RenderError(errors));
    }

    Ok(())
}
