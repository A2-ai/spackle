use std::fs;

use spackle::core::copy;
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

    copy::copy(&src_dir, &dst_dir, &vec!["file-0.txt".to_string()]).unwrap();

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

    copy::copy(&src_dir, &dst_dir, &vec!["file-0.txt".to_string()]).unwrap();

    assert!(!dst_dir.join("subdir").join("file-0.txt").exists());

    for i in 0..3 {
        if i == 0 {
            assert!(!dst_dir.join(format!("file-{}.txt", i)).exists());
        } else {
            assert!(dst_dir.join(format!("file-{}.txt", i)).exists());
        }
    }
}
