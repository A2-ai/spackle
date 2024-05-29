use spackle::core::copy;
use tempdir::TempDir;

#[test]
fn ignore_some() {
    let src_dir = TempDir::new("spackle").unwrap().into_path();
    let dst_dir = TempDir::new("spackle").unwrap().into_path();

    // add files to src_dir
    for i in 0..3 {
        std::fs::write(
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
