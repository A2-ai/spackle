//! Shared helpers for integration tests.
//!
//! The `scaffold` helper lets a test write a small inline spackle project
//! (`spackle.toml` plus template/regular files) to a temp directory so each
//! test scenario is self-contained and readable top-to-bottom.

use std::{fs, path::PathBuf};

use tempfile::TempDir;

/// A scaffolded temp project.
///
/// Holds the [`TempDir`] so files stay on disk for the duration of the test
/// and get cleaned up on drop.
pub struct Scaffold {
    pub dir: TempDir,
}

impl Scaffold {
    pub fn path(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }
}

/// Build a project directory from a list of `(relative_path, contents)`.
///
/// Intermediate directories are created as needed. Use `"spackle.toml"` as
/// the config path. Template files use the `.j2` suffix.
pub fn scaffold(files: &[(&str, &str)]) -> Scaffold {
    let dir = TempDir::new().expect("create tempdir");
    for (rel, contents) in files {
        let full = dir.path().join(rel);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&full, contents).expect("write scaffold file");
    }
    Scaffold { dir }
}

/// A throwaway destination for rendered output. The caller should pass the
/// returned path into e.g. `template::fill` or `Project::generate`.
pub fn out_dir() -> TempDir {
    TempDir::new().expect("create out tempdir")
}

/// Walk `dir` and return a sorted list of relative paths (forward slashes)
/// for every file. Directories are omitted. Useful as a structural snapshot
/// target.
pub fn list_files(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    for entry in walkdir::WalkDir::new(dir)
        .into_iter()
        .filter_map(Result::ok)
    {
        if entry.file_type().is_file() {
            let rel = entry
                .path()
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    out.sort();
    out
}
