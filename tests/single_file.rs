//! Tests for single-file `.j2t` spackle projects.
//!
//! A single-file project is one `.j2t` file containing TOML frontmatter
//! (delimited by `---`) followed by a Tera template body. `config::load`
//! dispatches file paths to `load_file`, and `Project::render_single_file`
//! renders the body into a `String`.
//!
//! Written against Tera v1 behavior; suite is intended to also pass on
//! Tera v2 (see `tests/templating.rs` for the version-compatibility
//! principle).

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use spackle::fs::StdFs;
use spackle::{config, load_project, LoadError, SingleFileError};

mod common;
use common::scaffold;

fn data(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Scaffold a single `.j2t` file inside a tempdir and return its absolute
/// path along with the owning scaffold (kept alive for the test).
fn scaffold_j2t(contents: &str) -> (common::Scaffold, PathBuf) {
    let project = scaffold(&[("template.j2t", contents)]);
    let path = project.path().join("template.j2t");
    (project, path)
}

// ---------------------------------------------------------------------------
// config loading
// ---------------------------------------------------------------------------

#[test]
fn load_file_parses_frontmatter_slots() {
    let contents = r#"---
[[slots]]
key = "cmd"
type = "String"

[[slots]]
key = "description"
type = "String"
---
{{ cmd }} {{ description }}
"#;
    let (_project, path) = scaffold_j2t(contents);

    let cfg = config::load_file(&StdFs::new(), &path).expect("load_file should parse frontmatter");
    let keys: Vec<_> = cfg.slots.iter().map(|s| s.key.clone()).collect();
    assert_eq!(keys, vec!["cmd".to_string(), "description".to_string()]);
}

#[test]
fn config_load_dispatches_file_path_to_load_file() {
    let contents = r#"---
[[slots]]
key = "x"
---
body
"#;
    let (_project, path) = scaffold_j2t(contents);

    let cfg = config::load(&StdFs::new(), &path).expect("load should dispatch to load_file");
    assert_eq!(cfg.slots.len(), 1);
    assert_eq!(cfg.slots[0].key, "x");
}

#[test]
fn config_load_dispatches_dir_path_to_load_dir() {
    let project = scaffold(&[(
        "spackle.toml",
        r#"[[slots]]
key = "y"
"#,
    )]);

    let cfg =
        config::load(&StdFs::new(), project.path()).expect("load should dispatch to load_dir");
    assert_eq!(cfg.slots.len(), 1);
    assert_eq!(cfg.slots[0].key, "y");
}

#[test]
fn load_project_accepts_single_file() {
    let contents = r#"---
[[slots]]
key = "slot_a"
---
{{ slot_a }}
"#;
    let (_project, path) = scaffold_j2t(contents);

    let proj = load_project(&StdFs::new(), &path).expect("load_project should accept a .j2t file");
    assert_eq!(proj.path, path);
    assert_eq!(proj.config.slots.len(), 1);
}

// ---------------------------------------------------------------------------
// rendering
// ---------------------------------------------------------------------------

#[test]
fn renders_body_with_slot_substitution() {
    let contents = r#"---
[[slots]]
key = "cmd"
[[slots]]
key = "description"
---
{{ cmd }}: {{ description }}
"#;
    let (_project, path) = scaffold_j2t(contents);
    let proj = load_project(&StdFs::new(), &path).unwrap();

    let rendered = proj
//         .render_single_file(&data(&[
//             ("cmd", "build"),
//             ("description", "compile sources"),
//         ]))
        .render_single_file(
            &StdFs::new(),
            &data(&[("cmd", "build"), ("description", "compile sources")]),
        )
        .unwrap();

    insta::assert_snapshot!(rendered.trim_end(), @"build: compile sources");
}

#[test]
fn renders_body_with_tera_syntax() {
    // Confirm the same engine powers single-file templates — if/filter should
    // behave identically to directory-mode templates.
    let contents = r#"---
[[slots]]
key = "mode"
---
{% if mode == "debug" %}DEBUG {{ mode | upper }}{% else %}RELEASE{% endif %}
"#;
    let (_project, path) = scaffold_j2t(contents);
    let proj = load_project(&StdFs::new(), &path).unwrap();

    let rendered = proj
        .render_single_file(&StdFs::new(), &data(&[("mode", "debug")]))
        .unwrap();
    insta::assert_snapshot!(rendered.trim_end(), @"DEBUG DEBUG");
}

#[test]
fn render_does_not_inject_special_vars() {
    // Document current library behavior: single-file mode has no concept of
    // an output directory and no `_project_name`/`_output_name` gets injected.
    // A body referencing them surfaces as a Render error.
    let contents = r#"---
[[slots]]
key = "x"
---
{{ _project_name }}
"#;
    let (_project, path) = scaffold_j2t(contents);
    let proj = load_project(&StdFs::new(), &path).unwrap();

    let err = proj
        .render_single_file(&StdFs::new(), &data(&[("x", "v")]))
        .unwrap_err();
    assert!(matches!(err, SingleFileError::Render(_)));
}

// ---------------------------------------------------------------------------
// error surfaces
// ---------------------------------------------------------------------------

#[test]
fn missing_frontmatter_delimiters_errors() {
    // A template body with no `---` delimiters at all.
    let (_project, path) = scaffold_j2t("{{ just_a_body }}\n");
    let proj_result = load_project(&StdFs::new(), &path);
    match proj_result {
        Ok(_) => panic!("expected frontmatter parse error"),
        Err(LoadError::ConfigError { error, .. }) => {
            assert!(
                matches!(error, config::Error::FronmaError(_)),
                "expected FronmaError, got {:?}",
                error
            );
        }
    }
}

#[test]
fn invalid_toml_in_frontmatter_errors() {
    // Valid delimiters, malformed TOML.
    let contents = r#"---
this is = not valid = toml
[[bad
---
body
"#;
    let (_project, path) = scaffold_j2t(contents);
    match load_project(&StdFs::new(), &path) {
        Ok(_) => panic!("expected frontmatter parse error"),
        Err(LoadError::ConfigError { error, .. }) => {
            assert!(
                matches!(error, config::Error::FronmaError(_)),
                "expected FronmaError, got {:?}",
                error
            );
        }
    }
}

#[test]
fn undefined_variable_in_body_surfaces_as_render_error() {
    let contents = r#"---
[[slots]]
key = "known"
---
{{ unknown_slot }}
"#;
    let (_project, path) = scaffold_j2t(contents);
    let proj = load_project(&StdFs::new(), &path).unwrap();

    let err = proj
        .render_single_file(&StdFs::new(), &data(&[("known", "v")]))
        .unwrap_err();
    assert!(matches!(err, SingleFileError::Render(_)));
}

#[test]
fn load_missing_file_surfaces_as_config_error() {
    let missing = PathBuf::from("tests/data/__missing__.j2t");
    match load_project(&StdFs::new(), &missing) {
        Ok(_) => panic!("expected LoadError for missing file"),
        Err(LoadError::ConfigError { error, .. }) => {
            assert!(
                matches!(error, config::Error::ReadError { .. }),
                "expected ReadError, got {:?}",
                error
            );
        }
    }
}

#[test]
fn render_single_file_handles_io_error_when_path_vanishes() {
    // Load the project, then delete the file before render. render_single_file
    // re-reads self.path so this surfaces as SingleFileError::Read.
    let contents = r#"---
[[slots]]
key = "x"
---
{{ x }}
"#;
    let (_project, path) = scaffold_j2t(contents);
    let proj = load_project(&StdFs::new(), &path).unwrap();
    fs::remove_file(&path).unwrap();

    let err = proj
        .render_single_file(&StdFs::new(), &data(&[("x", "v")]))
        .unwrap_err();
    assert!(matches!(err, SingleFileError::Read(_)));
}
