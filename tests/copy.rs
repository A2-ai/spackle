use std::{collections::HashMap, fs};

use spackle::util::copy;
use tempdir::TempDir;

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

    copy::copy(
        &src_dir,
        &dst_dir,
        &vec!["file-0.txt".to_string()],
        &HashMap::from([("foo".to_string(), "bar".to_string())]),
    )
    .unwrap();

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

    copy::copy(
        &src_dir,
        &dst_dir,
        &vec!["file-0.txt".to_string()],
        &HashMap::from([("foo".to_string(), "bar".to_string())]),
    )
    .unwrap();

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

    copy::copy(
        &src_dir,
        &dst_dir,
        &vec![],
        &HashMap::from([
            ("template_name".to_string(), "template".to_string()),
            ("project_name".to_string(), "foo".to_string()),
        ]),
    )
    .unwrap();

    assert!(
        dst_dir.join("template.tmpl").exists(),
        "template.tmpl does not exist"
    );
}
