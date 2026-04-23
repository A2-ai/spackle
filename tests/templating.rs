//! End-to-end templating tests.
//!
//! Goal: pin spackle's templating behavior with rendered-output assertions.
//! Tests are deliberately written against a conservative subset of Tera
//! syntax that exists in both v1 and v2 so the suite can double as a
//! cross-version compatibility contract.

use std::collections::HashMap;
use std::path::PathBuf;

use spackle::fs::StdFs;
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
fn fill_all(project: &common::Scaffold, data: &HashMap<String, String>) -> Vec<(String, String)> {
    let out = out_dir();
    let fs = StdFs::new();
    let results = template::fill(&fs, &project.path(), out.path(), data)
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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("generated");

    let files = proj
        .generate(&StdFs::new(), &project.path(), &dst, &HashMap::new())
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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("chosen-output");

    let files = proj
        .generate(&StdFs::new(), &project.path(), &dst, &HashMap::new())
        .unwrap();
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
    let project = scaffold(&[("case.j2", "up={{ word | upper }} down={{ word | lower }}")]);
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
fn renders_nested_directory_templates() {
    let project = scaffold(&[("subdir/{{ slot }}.j2", "nested")]);
    let out = fill_all(&project, &data(&[("slot", "leaf")]));

    assert_eq!(out[0].0, "subdir/leaf");
    insta::assert_snapshot!(out[0].1, @"nested");
}

#[test]
fn template_extensions_trigger_render_and_strip_trailing() {
    // Files ending with a template extension (`.j2` or `.tera`) are sent
    // through `fill`; only the *trailing* extension is stripped from the
    // output path, so double-extension filenames keep the inner one.
    struct Case {
        src_name: &'static str,
        src_body: &'static str,
        expected_out_name: &'static str,
        expected_out_body: &'static str,
    }
    let cases = &[
        Case {
            src_name: "greeting.j2",
            src_body: "Hello, {{ name }}!",
            expected_out_name: "greeting",
            expected_out_body: "Hello, Ada!",
        },
        Case {
            src_name: "greeting.tera",
            src_body: "Hello, {{ name }}!",
            expected_out_name: "greeting",
            expected_out_body: "Hello, Ada!",
        },
        // Double-extension: only the trailing one is stripped.
        Case {
            src_name: "keep.j2.j2",
            src_body: "body",
            expected_out_name: "keep.j2",
            expected_out_body: "body",
        },
        Case {
            src_name: "keep.tera.tera",
            src_body: "body",
            expected_out_name: "keep.tera",
            expected_out_body: "body",
        },
        Case {
            src_name: "keep.j2.tera",
            src_body: "body",
            expected_out_name: "keep.j2",
            expected_out_body: "body",
        },
        Case {
            src_name: "keep.tera.j2",
            src_body: "body",
            expected_out_name: "keep.tera",
            expected_out_body: "body",
        },
    ];

    for case in cases {
        let project = scaffold(&[(case.src_name, case.src_body)]);
        let out = fill_all(&project, &data(&[("name", "Ada")]));

        assert_eq!(
            out.len(),
            1,
            "case {}: expected one rendered file",
            case.src_name
        );
        assert_eq!(
            out[0].0, case.expected_out_name,
            "case {}: output name",
            case.src_name
        );
        assert_eq!(
            out[0].1, case.expected_out_body,
            "case {}: rendered contents",
            case.src_name
        );
    }
}

#[test]
fn renders_mixed_j2_and_tera_extensions() {
    // A project can mix both extensions; both get rendered and both have
    // their trailing extension stripped from the output path. Also verifies
    // that a single `fill` pass picks up both extensions (glob alternation).
    let project = scaffold(&[("a.j2", "j2:{{ who }}"), ("b.tera", "tera:{{ who }}")]);
    let out = fill_all(&project, &data(&[("who", "spackle")]));

    let by_name: HashMap<_, _> = out.iter().cloned().collect();
    assert_eq!(by_name.get("a").map(String::as_str), Some("j2:spackle"));
    assert_eq!(by_name.get("b").map(String::as_str), Some("tera:spackle"));
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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("out");

    proj.generate(&StdFs::new(), &project.path(), &dst, &data(&[("var", "x")]))
        .unwrap();

    let files = list_files(&dst);
    assert_eq!(files, vec!["pinned-proj/readme.txt".to_string()]);
    let contents = std::fs::read_to_string(dst.join("pinned-proj").join("readme.txt")).unwrap();
    assert_eq!(
        contents, "plain body {{ var }}",
        "contents of non-.j2 files are copied verbatim"
    );
}

#[test]
fn copy_skips_template_extensions() {
    // Template-extension files are handled by `template::fill`, not
    // `copy::copy`. A project containing only one such file should produce
    // exactly one output file (the rendered one) — no stray source file
    // with the template extension gets copied.
    for src_name in &["only.j2", "only.tera"] {
        let project = scaffold(&[("spackle.toml", ""), (src_name, "rendered")]);
        let proj = load_project(&StdFs::new(), &project.path()).unwrap();
        let out = out_dir();
        let dst = out.path().join("out");

        proj.generate(&StdFs::new(), &project.path(), &dst, &HashMap::new())
            .unwrap();

        let files = list_files(&dst);
        assert_eq!(files, vec!["only".to_string()], "case {}", src_name);
    }
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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    let out = out_dir();
    let dst = out.path().join("out");

    proj.generate(&StdFs::new(), &project.path(), &dst, &HashMap::new())
        .unwrap();

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
    let results = template::fill(&StdFs::new(), &project.path(), out.path(), &HashMap::new())
        .expect("load should succeed");

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
    let results = template::fill(&StdFs::new(), &project.path(), out.path(), &HashMap::new())
        .expect("load should succeed");

    let err = results.into_iter().next().unwrap().unwrap_err();
    assert!(
        matches!(err.kind, FileErrorKind::ErrorRenderingName(_)),
        "expected ErrorRenderingName, got {:?}",
        err.kind
    );
}

#[test]
#[ignore = "blocked on tera 2.0.0-alpha.2: load_from_glob panics on parse errors \
(see tera.rs:74 `expect(\"to have a source\")`). Re-enable when upstream \
returns Err instead of panicking."]
fn error_malformed_syntax() {
    // Malformed syntax should surface as a Tera load error from `fill`.
    let project = scaffold(&[("busted.j2", "{% if foo %}oops")]);
    let err = template::fill(
        &StdFs::new(),
        &project.path(),
        out_dir().path(),
        &HashMap::new(),
    )
    .unwrap_err();
    // We don't inspect the inner message — any tera::Error is correct here.
    assert!(!format!("{err}").is_empty());
}

#[test]
fn validate_rejects_missing_slot_reference() {
    let project = scaffold(&[("file.j2", "{{ unlisted }}")]);
    let err = template::validate(&StdFs::new(), &project.path(), &vec![]).unwrap_err();
    assert!(matches!(err, ValidateError::RenderError(_)));
}

#[test]
fn validate_accepts_fully_covered_template() {
    let project = scaffold(&[("file.j2", "{{ listed }}")]);
    let slots = vec![spackle::slot::Slot {
        key: "listed".to_string(),
        ..Default::default()
    }];
    assert!(template::validate(&StdFs::new(), &project.path(), &slots).is_ok());
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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    assert!(proj.check(&StdFs::new()).is_ok());
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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    let err = proj.check(&StdFs::new()).unwrap_err();
    assert!(matches!(err, CheckError::SlotError(_)));
}

#[test]
fn check_surfaces_template_error_for_undefined_reference() {
    let project = scaffold(&[("spackle.toml", ""), ("bad.j2", "{{ invalid_slot }}")]);
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();
    let err = proj.check(&StdFs::new()).unwrap_err();
    assert!(matches!(err, CheckError::TemplateError(_)));
}

#[test]
fn load_surfaces_config_error_for_missing_config() {
    let missing = PathBuf::from("tests/data/__does_not_exist__");
    match load_project(&StdFs::new(), &missing) {
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
    let fs = StdFs::new();
    let proj = load_project(&fs, &fixture).unwrap();

    let out = out_dir();
    let dst = out.path().join("basic_project_out");

    let data = data(&[
        ("greeting", "hello"),
        ("target", "world"),
        ("filename", "dynamic"),
    ]);

    proj.generate(&fs, &fixture, &dst, &data).unwrap();

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
    let proj = load_project(&StdFs::new(), &project.path()).unwrap();

    let out = out_dir();
    let dst = out.path().join("pre-existing");
    std::fs::create_dir_all(&dst).unwrap();

    let err = proj
        .generate(&StdFs::new(), &project.path(), &dst, &HashMap::new())
        .unwrap_err();
    assert!(matches!(err, GenerateError::AlreadyExists(_)));
}
