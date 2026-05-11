//! Static project-level validation — no slot data required.

use std::path::Path;

use crate::{
    config, copy,
    diagnostic::{self, Diagnostic, DiagnosticSource, Severity},
    fs::FileSystem,
    hook, slot, template,
};

const SPACKLE_TOML_FILE: &str = "spackle.toml";

pub struct CheckReport {
    /// `Some` once `spackle.toml` parses cleanly. Re-exposed so UIs can
    /// render slot/hook forms without parsing the TOML again.
    pub config: Option<config::Config>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Never returns `Err` — empty `diagnostics` means a clean project.
pub fn check<F: FileSystem>(fs: &F, project_dir: &Path) -> CheckReport {
    let mut diagnostics = Vec::new();

    // Read bytes ourselves rather than going through `config::load_dir`
    // so we keep the source string around for byte-offset → line/col
    // span resolution on TOML parse errors.
    let toml_path = project_dir.join(SPACKLE_TOML_FILE);
    let toml_source: Option<String> = match fs.read_file(&toml_path) {
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(s) => Some(s),
            Err(e) => {
                diagnostics.push(
                    Diagnostic::new(
                        Severity::Error,
                        DiagnosticSource::Config,
                        format!("spackle.toml is not valid UTF-8: {}", e),
                    )
                    .with_path(SPACKLE_TOML_FILE),
                );
                return CheckReport {
                    config: None,
                    diagnostics,
                };
            }
        },
        Err(e) => {
            diagnostics.push(
                Diagnostic::new(
                    Severity::Error,
                    DiagnosticSource::Config,
                    format!("could not read spackle.toml: {}", e),
                )
                .with_path(SPACKLE_TOML_FILE),
            );
            return CheckReport {
                config: None,
                diagnostics,
            };
        }
    };

    let config = match config::parse(toml_source.as_deref().unwrap_or("")) {
        Ok(c) => c,
        Err(e) => {
            diagnostics.push(diagnostic::from_config_error(&e, toml_source.as_deref()));
            return CheckReport {
                config: None,
                diagnostics,
            };
        }
    };

    if let Err(e) = config.validate() {
        diagnostics.push(diagnostic::from_config_error(&e, toml_source.as_deref()));
    }

    if let Err(e) = slot::validate(&config.slots) {
        diagnostics.push(diagnostic::from_slot_config_error(&e));
    }

    for err in hook::validate_config(&config.hooks, &config.slots) {
        diagnostics.push(diagnostic::from_hook_config_error(&err));
    }

    // `template::validate` renders bodies AND filenames against
    // empty-string slot values, so it catches both syntax errors and
    // undefined-slot refs in either spot without needing real data.
    match template::validate(fs, project_dir, &config.slots) {
        Ok(()) => {}
        Err(template::ValidateError::TeraError(e)) => {
            let mut d = Diagnostic::new(
                Severity::Error,
                DiagnosticSource::RenderBody,
                format!("template engine error: {}", e),
            );
            if let Some(span) = diagnostic::extract_tera_span(&e) {
                d = d.with_span(span);
            }
            diagnostics.push(d);
        }
        Err(template::ValidateError::RenderError(items)) => {
            for item in items {
                let source = match item.kind {
                    template::ValidateFileErrorKind::Body => DiagnosticSource::RenderBody,
                    template::ValidateFileErrorKind::Filename => DiagnosticSource::RenderName,
                };
                let mut d = Diagnostic::new(Severity::Error, source, item.error.to_string())
                    .with_path(item.file);
                if let Some(span) = diagnostic::extract_tera_span(&item.error) {
                    d = d.with_span(span);
                }
                diagnostics.push(d);
            }
        }
    }

    // Non-template file paths get templated by `copy::copy_collect` at
    // render time; this is the static counterpart that catches their
    // parse errors / undefined-var refs without slot data.
    match copy::validate_paths(fs, project_dir, &config.ignore, &config.slots) {
        Ok(errs) => {
            for err in &errs {
                diagnostics.push(diagnostic::from_copy_error(err));
            }
        }
        Err(fatal) => {
            // Walk failed or context serialization failed — surface as
            // one copy diagnostic.
            diagnostics.push(diagnostic::from_copy_error(&fatal));
        }
    }

    CheckReport {
        config: Some(config),
        diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fs::StdFs;
    use std::path::PathBuf;

    #[cfg(not(target_arch = "wasm32"))]
    fn fixture(name: &str) -> PathBuf {
        PathBuf::from("tests/fixtures").join(name)
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn check_clean_project_has_no_diagnostics() {
        let report = check(&StdFs::new(), &fixture("basic_project"));
        assert!(
            report.diagnostics.is_empty(),
            "expected zero diagnostics, got {:?}",
            report.diagnostics
        );
        assert!(report.config.is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn check_bad_template_surfaces_render_body_diagnostic() {
        let report = check(&StdFs::new(), &fixture("bad_template"));
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.source == DiagnosticSource::RenderBody),
            "expected a render_body diagnostic, got {:?}",
            report.diagnostics
        );
        // Bad template's path appears in the diagnostic.
        assert!(report
            .diagnostics
            .iter()
            .any(|d| d.path.as_deref() == Some("bad.j2")));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn check_missing_config_surfaces_config_diagnostic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let report = check(&StdFs::new(), tmp.path());
        assert_eq!(report.diagnostics.len(), 1);
        assert_eq!(report.diagnostics[0].source, DiagnosticSource::Config);
        assert!(report.config.is_none());
    }
}
