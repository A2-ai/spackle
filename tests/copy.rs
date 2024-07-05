use std::fs;

use spackle::util::copy;
use tempdir::TempDir;
use tera::Context;

#[test]
fn ignore_one() {
    let src_dir = TempDir::new("spackle").unwrap().into_path();
    let dst_dir = TempDir::new("spackle").unwrap().into_path();

    for i in 0..3 {
        fs::write(
            src_dir.join(format!("file-{}.txt", i)),
            format!("file-{}.txt", i),
        )
        .unwrap();
    }

    let mut context = Context::new();
    context.insert("foo", &"bar");
    copy::copy(&src_dir, &dst_dir, &vec!["file-0.txt".to_string()], &context).unwrap();

    for i in 0..3 {
        if i == 0 {
            assert!(!dst_dir.join(format!("file-{}.txt", i)).exists());
        } else {
            assert!(dst_dir.join(format!("file-{}.txt", i)).exists());
        }
    }
}

#[test]
fn ignore_subdir() {
    let src_dir = TempDir::new("spackle").unwrap().into_path();
    let dst_dir = TempDir::new("spackle").unwrap().into_path();

    for i in 0..3 {
        fs::write(
            src_dir.join(format!("file-{}.txt", i)),
            format!("file-{}.txt", i),
        )
        .unwrap();
    }

    let subdir = src_dir.join("subdir");
    fs::create_dir(&subdir).unwrap();

    fs::write(subdir.join("file-0.txt"), "file-0.txt").unwrap();

    let mut context = Context::new();
    context.insert("foo", &"bar");
    copy::copy(&src_dir, &dst_dir, &vec!["file-0.txt".to_string()], &context).unwrap();

    assert!(!dst_dir.join("subdir").join("file-0.txt").exists());

    for i in 0..3 {
        if i == 0 {
            assert!(!dst_dir.join(format!("file-{}.txt", i)).exists());
        } else {
            assert!(dst_dir.join(format!("file-{}.txt", i)).exists());
        }
    }
}

#[test]
fn replace_file_name() {
    let src_dir = TempDir::new("spackle").unwrap().into_path();
    let dst_dir = TempDir::new("spackle").unwrap().into_path();

    // a file that has template structure in its name but does not end with .j2
    // should still be replaced, while leavings its contents untouched.
    // .j2 extensions should representing which files have _contents_ that need
    // replacing.
    fs::write(
        src_dir.join(format!("{}.tmpl", "{{template_name}}")),
        // copy will not do any replacement so contents should remain as is
        "{{project_name}}",
    )
    .unwrap();
    assert!(src_dir.join("{{template_name}}.tmpl").exists());

    let mut context = Context::new();
    context.insert("template_name", &"template");
    context.insert("project_name", &"foo");
    copy::copy(&src_dir, &dst_dir, &vec![], &context).unwrap();

    assert!(dst_dir.join("template.tmpl").exists(), "template.tmpl does not exist");
}
