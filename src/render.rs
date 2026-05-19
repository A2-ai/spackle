//! Dynamic render-with-data, diagnostics-first. Generic over `FileSystem`:
//! callers pass `MemoryFs` for in-memory preview or `StdFs` to write to disk.

use std::{collections::HashMap, path::Path};

use crate::{
    check, copy,
    diagnostic::{self, Diagnostic, DiagnosticSource, Severity},
    fs::FileSystem,
    get_output_name, hook, slot, template, NameOverrides,
};

pub struct RenderReport {
    pub files: Vec<template::RenderedFile>,
    pub dirs: Vec<std::path::PathBuf>,
    pub diagnostics: Vec<Diagnostic>,
    /// `None` when the config didn't parse; otherwise the resolved plan.
    /// Entries can still carry their own template errors via the
    /// `hook_config` diagnostics already in `diagnostics`.
    pub hook_plan: Option<Vec<hook::HookPlanEntry>>,
}

/// Never returns `Err` — empty `diagnostics` means a clean render.
///
/// `names` lets callers override the `_project_name` / `_output_name`
/// Tera vars independently of the bundle layout; pass
/// [`NameOverrides::NONE`] for the historical defaults.
pub fn render<F: FileSystem>(
    fs: &F,
    project_dir: &Path,
    out_dir: &Path,
    slot_data: &HashMap<String, String>,
    names: NameOverrides<'_>,
) -> RenderReport {
    let mut diagnostics = Vec::new();

    let check_report = check::check(fs, project_dir);
    diagnostics.extend(check_report.diagnostics);

    let config = match check_report.config {
        Some(c) => c,
        None => {
            // No config — the diagnostics already explain why.
            return RenderReport {
                files: Vec::new(),
                dirs: Vec::new(),
                diagnostics,
                hook_plan: None,
            };
        }
    };

    let project_name = names.project_name.map(str::to_owned).unwrap_or_else(|| {
        config.name.clone().unwrap_or_else(|| {
            project_dir
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        })
    });
    let output_name = names
        .output_name
        .map(str::to_owned)
        .unwrap_or_else(|| get_output_name(out_dir));

    // Slot data errors don't short-circuit — Tera will surface real
    // undefined-var hits per-file, which is more useful than aborting.
    if let Err(e) = slot::validate_data(slot_data, &config.slots) {
        diagnostics.push(diagnostic::from_slot_data_error(&e));
    }

    let mut data = slot_data.clone();
    data.insert("_project_name".to_string(), project_name);
    data.insert("_output_name".to_string(), output_name);

    // copy_collect's outer `Err` is reserved for non-recoverable
    // preconditions (no writable dest root, no readable src); per-entry
    // failures land in `report.errors` instead.
    let copy_report = match copy::copy_collect(fs, project_dir, out_dir, &config.ignore, &data) {
        Ok(r) => r,
        Err(fatal) => {
            diagnostics.push(diagnostic::from_copy_error(&fatal));
            return RenderReport {
                files: Vec::new(),
                dirs: Vec::new(),
                diagnostics,
                hook_plan: None,
            };
        }
    };
    for err in &copy_report.errors {
        diagnostics.push(diagnostic::from_copy_error(err));
    }

    let mut rendered: Vec<template::RenderedFile> = Vec::new();
    match template::fill(fs, project_dir, out_dir, &data) {
        Ok(results) => {
            for r in results {
                match r {
                    Ok(rf) => rendered.push(rf),
                    Err(file_err) => diagnostics.push(diagnostic::from_file_error(&file_err)),
                }
            }
        }
        Err(tera_err) => {
            // Global engine failure (couldn't load any templates) vs the
            // per-file errors that come through `Ok(Vec<Result<_>>)`.
            let mut d = Diagnostic::new(
                Severity::Error,
                DiagnosticSource::RenderBody,
                format!("template engine error: {}", tera_err),
            );
            if let Some(span) = diagnostic::extract_tera_span(&tera_err) {
                d = d.with_span(span);
            }
            diagnostics.push(d);
        }
    }

    let plan = hook::evaluate_hook_plan(&config.hooks, &config.slots, &data);
    for entry in &plan {
        diagnostics.extend(diagnostic::from_hook_template_errors(entry));
    }

    RenderReport {
        files: rendered,
        // `dirs` is populated wasm-side from MemoryFs after this call;
        // native callers can walk `out_dir` directly.
        dirs: Vec::new(),
        diagnostics,
        hook_plan: Some(plan),
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
    fn render_clean_project_has_no_diagnostics() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("out");
        let data = HashMap::from([
            ("greeting".to_string(), "hello".to_string()),
            ("target".to_string(), "world".to_string()),
            ("filename".to_string(), "dynamic".to_string()),
        ]);
        let report = render(
            &StdFs::new(),
            &fixture("basic_project"),
            &out,
            &data,
            NameOverrides::NONE,
        );
        assert!(
            report.diagnostics.is_empty(),
            "expected zero diagnostics, got {:?}",
            report.diagnostics
        );
        assert!(!report.files.is_empty(), "expected rendered files");
        assert!(report.hook_plan.is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn render_bad_template_surfaces_render_body_diagnostic_but_continues() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("out");
        let data = HashMap::from([("defined_slot".to_string(), "value".to_string())]);
        let report = render(
            &StdFs::new(),
            &fixture("bad_template"),
            &out,
            &data,
            NameOverrides::NONE,
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.source == DiagnosticSource::RenderBody),
            "expected a render_body diagnostic, got {:?}",
            report.diagnostics
        );
        // Hook plan was still computed.
        assert!(report.hook_plan.is_some());
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn render_missing_slot_data_emits_slot_data_diagnostic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = tmp.path().join("out");
        // basic_project requires `greeting`, `target`, `filename` slots;
        // supply only one.
        let data = HashMap::from([("greeting".to_string(), "hello".to_string())]);
        let report = render(
            &StdFs::new(),
            &fixture("basic_project"),
            &out,
            &data,
            NameOverrides::NONE,
        );
        assert!(
            report
                .diagnostics
                .iter()
                .any(|d| d.source == DiagnosticSource::SlotData),
            "expected a slot_data diagnostic, got {:?}",
            report.diagnostics
        );
    }
}
