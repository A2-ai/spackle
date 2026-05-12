//! End-to-end tests for the structured-diagnostic surface (`spackle::check_project`
//! and `spackle::render`). Cover the cross-stage accumulation behavior
//! and the UI-level promises (collect-don't-abort, partial preview, etc.)
//! against scaffolded projects.

use std::collections::HashMap;

use spackle::fs::StdFs;
use spackle::{DiagnosticSource, NameOverrides, Severity};

mod common;
use common::{out_dir, scaffold};

fn data(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

#[test]
fn check_clean_project_has_no_diagnostics() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "name"
type = "String"
"#,
        ),
        ("greeting.j2", "Hello, {{ name }}!"),
    ]);
    let report = spackle::check_project(&StdFs::new(), &project.path());
    assert!(
        report.diagnostics.is_empty(),
        "diagnostics: {:?}",
        report.diagnostics
    );
    assert!(report.config.is_some());
}

#[test]
fn check_surfaces_toml_parse_error_with_span() {
    let project = scaffold(&[("spackle.toml", "[[broken\n")]);
    let report = spackle::check_project(&StdFs::new(), &project.path());

    assert!(report.config.is_none(), "config should not parse");
    assert_eq!(report.diagnostics.len(), 1);
    let d = &report.diagnostics[0];
    assert_eq!(d.source, DiagnosticSource::Config);
    assert_eq!(d.path.as_deref(), Some("spackle.toml"));
    // TOML's byte-offset span should give a 1-indexed line/col.
    let span = d.span.expect("toml parse span");
    assert_eq!(span.line, 1);
    assert!(span.column >= 1);
}

#[test]
fn check_collects_multiple_diagnostics_across_stages_at_once() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "age"
type = "Number"
default = "not-a-number"

[[hooks]]
key = "h1"
command = ["echo", "hi"]
needs = ["missing_hook"]
"#,
        ),
        ("template.j2", "{{ undefined_var }}"),
    ]);
    let report = spackle::check_project(&StdFs::new(), &project.path());
    assert!(report.config.is_some());

    let sources: Vec<DiagnosticSource> = report.diagnostics.iter().map(|d| d.source).collect();
    assert!(
        sources.contains(&DiagnosticSource::SlotConfig),
        "expected SlotConfig in {:?}",
        sources
    );
    assert!(
        sources.contains(&DiagnosticSource::HookConfig),
        "expected HookConfig in {:?}",
        sources
    );
    assert!(
        sources.contains(&DiagnosticSource::RenderBody),
        "expected RenderBody in {:?}",
        sources
    );
}

#[test]
fn check_surfaces_unknown_hook_needs_with_ref() {
    let project = scaffold(&[(
        "spackle.toml",
        r#"[[hooks]]
key = "child"
command = ["echo", "hi"]
needs = ["parent"]
"#,
    )]);
    let report = spackle::check_project(&StdFs::new(), &project.path());
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.source == DiagnosticSource::HookConfig)
        .expect("expected a hook_config diagnostic");
    assert_eq!(d.r#ref.as_deref(), Some("child"));
    assert!(
        d.message.contains("parent"),
        "expected mention of missing key 'parent', got: {}",
        d.message
    );
}

// ---------------------------------------------------------------------------
// render
// ---------------------------------------------------------------------------

#[test]
fn render_clean_project_returns_files_and_no_diagnostics() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "name"
type = "String"
"#,
        ),
        ("greeting.j2", "Hello, {{ name }}!"),
        ("static.txt", "plain content"),
    ]);
    let out = out_dir();
    let dst = out.path().join("out");

    let report = spackle::render(
        &StdFs::new(),
        &project.path(),
        &dst,
        &data(&[("name", "Ada")]),
        NameOverrides::NONE,
    );

    assert!(
        report.diagnostics.is_empty(),
        "diagnostics: {:?}",
        report.diagnostics
    );
    assert_eq!(report.files.len(), 1, "expected 1 rendered template");
    assert_eq!(report.files[0].contents, "Hello, Ada!");
    assert!(
        report.hook_plan.is_some(),
        "hook_plan should always be Some on success"
    );
}

#[test]
fn render_collects_multiple_per_file_errors_without_aborting() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "name"
type = "String"
"#,
        ),
        ("ok.j2", "Hello, {{ name }}!"),
        ("bad1.j2", "{{ undefined_var_1 }}"),
        ("bad2.j2", "{{ undefined_var_2 }}"),
    ]);
    let out = out_dir();
    let dst = out.path().join("out");

    let report = spackle::render(
        &StdFs::new(),
        &project.path(),
        &dst,
        &data(&[("name", "Ada")]),
        NameOverrides::NONE,
    );

    // Per-file diagnostics for both bad templates, plus check-stage
    // diagnostics for the same undefined references. Critically: NOT
    // a single fail-fast abort.
    let render_body_diags: Vec<_> = report
        .diagnostics
        .iter()
        .filter(|d| d.source == DiagnosticSource::RenderBody)
        .collect();
    assert!(
        render_body_diags.len() >= 2,
        "expected at least 2 render_body diagnostics, got {}: {:?}",
        render_body_diags.len(),
        report.diagnostics
    );

    // The good template still rendered (partial preview).
    let good_render = report
        .files
        .iter()
        .find(|f| f.contents.contains("Hello, Ada!"));
    assert!(good_render.is_some(), "good template should have rendered");
}

#[test]
fn render_missing_slot_data_surfaces_slot_data_diagnostic_and_continues() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "name"
type = "String"
"#,
        ),
        ("greeting.j2", "Hello, {{ name }}!"),
    ]);
    let out = out_dir();
    let dst = out.path().join("out");

    // Provide NOTHING — `name` is missing.
    let report = spackle::render(
        &StdFs::new(),
        &project.path(),
        &dst,
        &HashMap::new(),
        NameOverrides::NONE,
    );

    let slot_data_diag = report
        .diagnostics
        .iter()
        .find(|d| d.source == DiagnosticSource::SlotData);
    assert!(slot_data_diag.is_some(), "expected a slot_data diagnostic");
    // Hook plan still computed (no hooks => empty plan).
    assert!(report.hook_plan.is_some());
}

#[test]
fn check_surfaces_filename_template_parse_error_for_j2_files() {
    // `.j2` file whose FILENAME (not body) has a malformed template. The
    // body is fine. Static check must still catch this (mirrors the
    // dynamic `render_in_memory` filename-render site).
    let project = scaffold(&[
        ("spackle.toml", ""),
        // Unclosed `{{` in the filename, regardless of body content.
        ("{{ unclosed.txt.j2", "static body"),
    ]);
    let report = spackle::check_project(&StdFs::new(), &project.path());
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.source == spackle::DiagnosticSource::RenderName)
        .expect("expected a render_name diagnostic");
    assert!(
        d.path.as_deref() == Some("{{ unclosed.txt.j2"),
        "expected path to match the user-written filename, got: {:?}",
        d.path
    );
    assert!(
        !d.message.contains("failed to add template"),
        "diagnostic must not carry a wrapping prefix: {}",
        d.message
    );
}

#[test]
fn check_classifies_path_template_parse_error_as_render_name_not_copy() {
    // Non-template file whose PATH contains malformed Tera syntax. The
    // underlying error path goes through `copy::validate_paths`, but the
    // *user-meaningful* class is "filename template error" — same as a
    // `.j2` filename failure. The `Copy` source is reserved for actual
    // fs read/write failures; Tera-sourced failures are `RenderName`
    // regardless of file extension.
    let project = scaffold(&[
        ("spackle.toml", ""),
        ("{{_output_name_2}.Rproj", "irrelevant content"),
    ]);
    let report = spackle::check_project(&StdFs::new(), &project.path());
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.path.as_deref() == Some("{{_output_name_2}.Rproj"))
        .expect("expected a diagnostic for {{_output_name_2}.Rproj");
    assert_eq!(
        d.source,
        spackle::DiagnosticSource::RenderName,
        "Tera failure during path templating should be RenderName, not Copy"
    );
}

#[test]
fn check_surfaces_undefined_slot_in_filename_template() {
    // No slot defined named `nope`. Static check renders against
    // empty-string slot values, so referencing an undefined slot in a
    // filename is caught the same way it is for bodies.
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "defined"
"#,
        ),
        ("{{ nope }}.txt.j2", "{{ defined }}"),
    ]);
    let report = spackle::check_project(&StdFs::new(), &project.path());
    let d = report
        .diagnostics
        .iter()
        .find(|d| d.source == spackle::DiagnosticSource::RenderName && d.message.contains("nope"))
        .expect("expected a render_name diagnostic mentioning the undefined slot");
    assert_eq!(d.path.as_deref(), Some("{{ nope }}.txt.j2"));
}

#[test]
fn render_template_parse_error_attaches_path_and_drops_wrapping_prefix() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "name"
type = "String"
"#,
        ),
        ("ok.j2", "Hello, {{ name }}!"),
        ("bad.j2", r#"name = "{{ unclosed"#),
    ]);
    let out = out_dir();
    let dst = out.path().join("out");
    let report = spackle::render(
        &StdFs::new(),
        &project.path(),
        &dst,
        &data(&[("name", "Ada")]),
        NameOverrides::NONE,
    );

    let parse_diag = report
        .diagnostics
        .iter()
        .find(|d| d.path.as_deref() == Some("bad.j2"))
        .expect("expected a diagnostic with path=bad.j2");
    // Path is on the diagnostic; the message must NOT carry the leaky
    // "failed to add template <path>:" prefix the inner Tera-message
    // wrapping used to bake in.
    assert!(
        !parse_diag.message.contains("failed to add template"),
        "message should not carry the wrapping prefix, got: {}",
        parse_diag.message
    );

    // Other templates still rendered (parse failure of `bad.j2` does
    // not abort the rest of the run).
    let good_render = report
        .files
        .iter()
        .find(|f| f.contents.contains("Hello, Ada!"));
    assert!(good_render.is_some(), "ok.j2 should still have rendered");
}

#[test]
fn render_severity_is_error_for_all_v1_diagnostics() {
    let project = scaffold(&[("spackle.toml", ""), ("bad.j2", "{{ undefined }}")]);
    let out = out_dir();
    let dst = out.path().join("out");
    let report = spackle::render(
        &StdFs::new(),
        &project.path(),
        &dst,
        &HashMap::new(),
        NameOverrides::NONE,
    );
    for d in &report.diagnostics {
        assert!(
            matches!(d.severity, Severity::Error),
            "v1 emits only Error severity, got {:?}",
            d.severity
        );
    }
}
