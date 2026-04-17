//! End-to-end templating tests.
//!
//! Goal: pin spackle's templating behavior with rendered-output assertions.
//! Tests are deliberately written against a conservative subset of Tera
//! syntax that exists in both v1 and v2 so the suite can double as a
//! cross-version compatibility contract.

use std::collections::HashMap;
use std::path::PathBuf;

use spackle::template::{self, FileErrorKind, ValidateError};
use spackle::{load_project, CheckError, GenerateError, LoadError};

mod common;
use common::{list_files, out_dir, scaffold};

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn data(pairs: &[(&str, &str)]) -> HashMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

/// Run `template::fill` over a scaffolded project and return the rendered
/// files collected into a sorted `Vec<(relative_path, contents)>`.
fn fill_all(
    project: &common::Scaffold,
    data: &HashMap<String, String>,
) -> Vec<(String, String)> {
    let out = out_dir();
    let results = template::fill(&project.path(), out.path(), data)
        .expect("fill should load templates")
        .into_iter()
        .map(|r| r.expect("render should succeed"))
        .collect::<Vec<_>>();

    let mut files: Vec<(String, String)> = results
        .into_iter()
        .map(|f| {
            let rel = f.path.to_string_lossy().replace('\\', "/");
            (rel, f.contents)
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

// ---------------------------------------------------------------------------
// content rendering
// ---------------------------------------------------------------------------

#[test]
fn renders_plain_string_slot() {
    let project = scaffold(&[("greeting.j2", "Hello, {{ name }}!")]);
    let out = fill_all(&project, &data(&[("name", "Ada")]));

    insta::assert_snapshot!(out[0].1, @"Hello, Ada!");
    assert_eq!(out[0].0, "greeting");
}

#[test]
fn renders_number_slot_as_string() {
    // Slot data is always a String at the library boundary; numbers pass
    // through as their string form.
    let project = scaffold(&[("count.j2", "count = {{ count }}")]);
    let out = fill_all(&project, &data(&[("count", "42")]));

    insta::assert_snapshot!(out[0].1, @"count = 42");
}

#[test]
fn renders_boolean_slot_as_string() {
    let project = scaffold(&[("flag.j2", "enabled = {{ enabled }}")]);
    let out = fill_all(&project, &data(&[("enabled", "true")]));

    insta::assert_snapshot!(out[0].1, @"enabled = true");
}

#[test]
fn renders_special_project_name() {
    // Special vars come from `Project::generate`, not raw `template::fill`.
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"name = "my-proj"
"#,
        ),
        ("readme.j2", "project: {{ _project_name }}"),
    ]);
    let proj = load_project(&project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("generated");

    let files = proj
        .generate(&project.path(), &dst, &HashMap::new())
        .unwrap();

    let readme = files.iter().find(|f| f.path.ends_with("readme")).unwrap();
    insta::assert_snapshot!(readme.contents, @"project: my-proj");
}

#[test]
fn renders_special_output_name() {
    let project = scaffold(&[
        ("spackle.toml", ""),
        ("where.j2", "output: {{ _output_name }}"),
    ]);
    let proj = load_project(&project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("chosen-output");

    let files = proj.generate(&project.path(), &dst, &HashMap::new()).unwrap();
    let where_file = files.iter().find(|f| f.path.ends_with("where")).unwrap();
    insta::assert_snapshot!(where_file.contents, @"output: chosen-output");
}

// ---------------------------------------------------------------------------
// Tera syntax surface (stable across v1/v2)
// ---------------------------------------------------------------------------

#[test]
fn tera_if_conditional() {
    let tmpl = "{% if mode == \"debug\" %}DEBUG{% else %}RELEASE{% endif %}";
    let project = scaffold(&[("mode.j2", tmpl)]);

    let debug = fill_all(&project, &data(&[("mode", "debug")]));
    let release = fill_all(&project, &data(&[("mode", "release")]));

    insta::assert_snapshot!(debug[0].1, @"DEBUG");
    insta::assert_snapshot!(release[0].1, @"RELEASE");
}

#[test]
fn tera_for_loop() {
    let tmpl = "{% for item in items | split(pat=\",\") %}- {{ item }}\n{% endfor %}";
    let project = scaffold(&[("list.j2", tmpl)]);

    let out = fill_all(&project, &data(&[("items", "a,b,c")]));
    insta::assert_snapshot!(out[0].1, @r"
    - a
    - b
    - c
    ");
}

#[test]
fn tera_filter_upper_lower() {
    let project = scaffold(&[(
        "case.j2",
        "up={{ word | upper }} down={{ word | lower }}",
    )]);
    let out = fill_all(&project, &data(&[("word", "Spackle")]));

    insta::assert_snapshot!(out[0].1, @"up=SPACKLE down=spackle");
}

#[test]
fn tera_whitespace_control() {
    // Note the leading/trailing whitespace around the tag — `{%- -%}` trims
    // adjacent whitespace/newlines on both sides.
    let tmpl = "before\n  {%- if yes -%}  MIDDLE  {%- endif -%}  \nafter";
    let project = scaffold(&[("ws.j2", tmpl)]);
    let out = fill_all(&project, &data(&[("yes", "true")]));

    // `{%- -%}` trims whitespace/newlines on both sides of each tag, so
    // the surrounding newlines collapse entirely.
    insta::assert_snapshot!(out[0].1, @"beforeMIDDLEafter");
}

#[test]
fn tera_default_filter() {
    let project = scaffold(&[("d.j2", "v={{ missing | default(value=\"fallback\") }}")]);
    let out = fill_all(&project, &data(&[]));

    insta::assert_snapshot!(out[0].1, @"v=fallback");
}

#[test]
fn tera_length_filter() {
    let project = scaffold(&[("len.j2", "len={{ word | length }}")]);
    let out = fill_all(&project, &data(&[("word", "spackle")]));

    insta::assert_snapshot!(out[0].1, @"len=7");
}

// ---------------------------------------------------------------------------
// file and path rendering
// ---------------------------------------------------------------------------

#[test]
fn renders_filename_from_slot() {
    let project = scaffold(&[("{{ slot_1 }}.j2", "hello")]);
    let out = fill_all(&project, &data(&[("slot_1", "generated")]));

    assert_eq!(out[0].0, "generated");
    insta::assert_snapshot!(out[0].1, @"hello");
}

#[test]
fn renders_double_j2_extension_preserves_one() {
    // `{{slot}}.j2.j2` — contents render, then only the trailing `.j2` is
    // stripped, leaving an output file that itself ends in `.j2`.
    let project = scaffold(&[("{{ slot_2 }}.j2.j2", "body")]);
    let out = fill_all(&project, &data(&[("slot_2", "keepme")]));

    assert_eq!(out[0].0, "keepme.j2");
    insta::assert_snapshot!(out[0].1, @"body");
}

#[test]
fn renders_nested_directory_templates() {
    let project = scaffold(&[("subdir/{{ slot }}.j2", "nested")]);
    let out = fill_all(&project, &data(&[("slot", "leaf")]));

    assert_eq!(out[0].0, "subdir/leaf");
    insta::assert_snapshot!(out[0].1, @"nested");
}

#[test]
fn copy_templates_directory_name() {
    // Non-`.j2` files get their *paths* templated but not their contents.
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"name = "pinned-proj"
"#,
        ),
        ("{{ _project_name }}/readme.txt", "plain body {{ var }}"),
    ]);
    let proj = load_project(&project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("out");

    proj.generate(&project.path(), &dst, &data(&[("var", "x")])).unwrap();

    let files = list_files(&dst);
    assert_eq!(files, vec!["pinned-proj/readme.txt".to_string()]);
    let contents = std::fs::read_to_string(dst.join("pinned-proj").join("readme.txt")).unwrap();
    assert_eq!(
        contents, "plain body {{ var }}",
        "contents of non-.j2 files are copied verbatim"
    );
}

#[test]
fn copy_skips_j2_files_from_copy_pass() {
    // `.j2` files are handled by template::fill, not copy::copy. A project
    // with only a `.j2` file should still produce exactly one output file
    // (the rendered one) — no stray source file ending in `.j2` gets copied.
    let project = scaffold(&[("spackle.toml", ""), ("only.j2", "rendered")]);
    let proj = load_project(&project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("out");

    proj.generate(&project.path(), &dst, &HashMap::new()).unwrap();

    let files = list_files(&dst);
    assert_eq!(files, vec!["only".to_string()]);
}

#[test]
fn copy_respects_ignore_patterns() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"ignore = ["secrets.txt"]
"#,
        ),
        ("keep.txt", "ok"),
        ("secrets.txt", "shh"),
    ]);
    let proj = load_project(&project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("out");

    proj.generate(&project.path(), &dst, &HashMap::new()).unwrap();

    let files = list_files(&dst);
    assert_eq!(files, vec!["keep.txt".to_string()]);
}

// ---------------------------------------------------------------------------
// error surfaces
// ---------------------------------------------------------------------------

#[test]
fn error_undefined_variable_in_content() {
    let project = scaffold(&[("bad.j2", "{{ missing_slot }}")]);
    let out = out_dir();
    let results =
        template::fill(&project.path(), out.path(), &HashMap::new()).expect("load should succeed");

    assert_eq!(results.len(), 1);
    let err = results.into_iter().next().unwrap().unwrap_err();
    assert!(
        matches!(err.kind, FileErrorKind::ErrorRenderingContents(_)),
        "expected ErrorRenderingContents, got {:?}",
        err.kind
    );
}

#[test]
fn error_undefined_variable_in_filename() {
    // Contents render fine; the filename refers to a variable not in data,
    // so rendering the name should fail.
    let project = scaffold(&[("{{ missing_name }}.j2", "body")]);
    let out = out_dir();
    let results =
        template::fill(&project.path(), out.path(), &HashMap::new()).expect("load should succeed");

    let err = results.into_iter().next().unwrap().unwrap_err();
    assert!(
        matches!(err.kind, FileErrorKind::ErrorRenderingName(_)),
        "expected ErrorRenderingName, got {:?}",
        err.kind
    );
}

#[test]
fn error_unclosed_tag() {
    // Malformed syntax should surface as a Tera load error from `fill`.
    let project = scaffold(&[("busted.j2", "{% if foo %}oops")]);
    let err = template::fill(&project.path(), out_dir().path(), &HashMap::new()).unwrap_err();
    // We don't inspect the inner message — any tera::Error is correct here.
    assert!(!format!("{err}").is_empty());
}

#[test]
fn validate_rejects_missing_slot_reference() {
    let project = scaffold(&[("file.j2", "{{ unlisted }}")]);
    let err = template::validate(&project.path(), &vec![]).unwrap_err();
    assert!(matches!(err, ValidateError::RenderError(_)));
}

#[test]
fn validate_accepts_fully_covered_template() {
    let project = scaffold(&[("file.j2", "{{ listed }}")]);
    let slots = vec![spackle::slot::Slot {
        key: "listed".to_string(),
        ..Default::default()
    }];
    assert!(template::validate(&project.path(), &slots).is_ok());
}

// ---------------------------------------------------------------------------
// project::check
// ---------------------------------------------------------------------------

#[test]
fn check_pass_on_fully_covered_project() {
    let project = scaffold(&[
        (
            "spackle.toml",
            r#"[[slots]]
key = "defined_field"
"#,
        ),
        ("good.j2", "{{ defined_field }}"),
    ]);
    let proj = load_project(&project.path()).unwrap();
    assert!(proj.check().is_ok());
}

#[test]
fn check_surfaces_slot_error_for_bad_default() {
    let project = scaffold(&[(
        "spackle.toml",
        r#"[[slots]]
key = "age"
type = "Number"
default = "not-a-number"
"#,
    )]);
    let proj = load_project(&project.path()).unwrap();
    let err = proj.check().unwrap_err();
    assert!(matches!(err, CheckError::SlotError(_)));
}

#[test]
fn check_surfaces_template_error_for_undefined_reference() {
    let project = scaffold(&[("spackle.toml", ""), ("bad.j2", "{{ invalid_slot }}")]);
    let proj = load_project(&project.path()).unwrap();
    let err = proj.check().unwrap_err();
    assert!(matches!(err, CheckError::TemplateError(_)));
}

#[test]
fn load_surfaces_config_error_for_missing_config() {
    let missing = PathBuf::from("tests/data/__does_not_exist__");
    match load_project(&missing) {
        Ok(_) => panic!("expected LoadError for missing config dir"),
        Err(e) => assert!(matches!(e, LoadError::ConfigError { .. })),
    }
}

// ---------------------------------------------------------------------------
// end-to-end via Project::generate
// ---------------------------------------------------------------------------

#[test]
fn generate_basic_project_fixture() {
    let fixture = PathBuf::from("tests/fixtures/basic_project");
    let proj = load_project(&fixture).unwrap();

    let out = out_dir();
    let dst = out.path().join("basic_project_out");

    let data = data(&[
        ("greeting", "hello"),
        ("target", "world"),
        ("filename", "dynamic"),
    ]);

    proj.generate(&fixture, &dst, &data).unwrap();

    // Structural snapshot — what files exist, relative to out dir.
    let tree = list_files(&dst);
    insta::assert_debug_snapshot!("basic_project_tree", tree);

    // Content snapshot — every file's contents, keyed by relative path.
    let mut contents: Vec<(String, String)> = tree
        .iter()
        .map(|rel| (rel.clone(), std::fs::read_to_string(dst.join(rel)).unwrap()))
        .collect();
    contents.sort_by(|a, b| a.0.cmp(&b.0));
    insta::assert_debug_snapshot!("basic_project_contents", contents);
}

#[test]
fn generate_fails_when_out_dir_exists() {
    let project = scaffold(&[("spackle.toml", "")]);
    let proj = load_project(&project.path()).unwrap();

    let out = out_dir();
    let dst = out.path().join("pre-existing");
    std::fs::create_dir_all(&dst).unwrap();

    let err = proj.generate(&project.path(), &dst, &HashMap::new()).unwrap_err();
    assert!(matches!(err, GenerateError::AlreadyExists(_)));
}
