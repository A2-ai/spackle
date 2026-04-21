use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};

use tera::{Context, Tera};

use crate::fs::{self as fsmod, FileSystem, FileType};
use crate::{config::CONFIG_FILE, template::TEMPLATE_EXT};

#[derive(Debug)]
pub struct Error {
    source: Box<dyn std::error::Error>,
    pub path: PathBuf,
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&*self.source)
    }
}

pub struct CopyResult {
    pub copied_count: usize,
    pub skipped_count: usize,
}

pub fn copy<F: FileSystem>(
    fs: &F,
    src: &Path,
    dest: &Path,
    skip: &Vec<String>,
    data: &HashMap<String, String>,
) -> Result<CopyResult, Error> {
    let mut copied_count = 0;
    let mut skipped_count = 0;

    // Ensure the destination root exists. The old walkdir-based flow
    // yielded the source root as its first entry, which resulted in a
    // `create_dir_all(dest)` call before any children were processed.
    // Our `fsmod::walk` only yields descendants — so do this eagerly to
    // preserve behavior downstream (notably: hooks that cwd into
    // `dest` need it to exist even when the source tree was empty).
    fs.create_dir_all(dest).map_err(|e| Error {
        source: Box::new(e),
        path: dest.to_path_buf(),
    })?;

    // Recursive walk via the fs trait. Yields each descendant as
    // (path_relative_to_src, stat). We filter + re-root + template the
    // destination path, then either mkdir or copy.
    let entries = fsmod::walk(fs, src).map_err(|e| Error {
        source: Box::new(e),
        path: src.to_path_buf(),
    })?;

    // First pass: apply the skip/config-file/.j2 filter on a per-entry
    // basis. Unlike walkdir::filter_entry we can't prune whole subtrees
    // in a single pass — so we explicitly compute an "ancestor skipped"
    // set while iterating (entries are in DFS order).
    let mut skipped_ancestors: Vec<PathBuf> = Vec::new();

    for (rel_path, stat) in entries {
        // If any ancestor was skipped, skip everything under it.
        if skipped_ancestors
            .iter()
            .any(|a| rel_path.starts_with(a))
        {
            continue;
        }

        let name = rel_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();

        if skip.iter().any(|s| s == &name) {
            skipped_count += 1;
            skipped_ancestors.push(rel_path.clone());
            continue;
        }
        if name == CONFIG_FILE {
            skipped_ancestors.push(rel_path.clone());
            continue;
        }
        if name.ends_with(TEMPLATE_EXT) {
            // .j2 files are skipped here; template::fill handles them.
            continue;
        }

        let src_path = src.join(&rel_path);
        let dst_path_maybe_template = dest.join(&rel_path);

        let context = Context::from_serialize(data).map_err(|e| Error {
            source: e.into(),
            path: src_path.clone(),
        })?;
        let dst_path: PathBuf =
            match Tera::one_off(&dst_path_maybe_template.to_string_lossy(), &context, false) {
                Ok(path) => path.into(),
                Err(e) => {
                    return Err(Error {
                        source: e.into(),
                        path: dst_path_maybe_template.clone(),
                    });
                }
            };

        match stat.file_type {
            FileType::Directory => {
                fs.create_dir_all(&dst_path).map_err(|e| Error {
                    source: Box::new(e),
                    path: dst_path.clone(),
                })?;
            }
            FileType::File => {
                if let Some(parent) = dst_path.parent() {
                    fs.create_dir_all(parent).map_err(|e| Error {
                        source: Box::new(e),
                        path: parent.to_path_buf(),
                    })?;
                }
                fs.copy_file(&src_path, &dst_path).map_err(|e| Error {
                    source: Box::new(e),
                    path: dst_path.clone(),
                })?;
                copied_count += 1;
            }
            FileType::Symlink | FileType::Other => {
                // Symlinks and other special entries are not copied —
                // matches prior walkdir behavior (only is_dir / is_file
                // branches).
            }
        }
    }

    Ok(CopyResult {
        copied_count,
        skipped_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{collections::HashMap, fs};
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

        copy(
            &crate::fs::StdFs::new(),
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

        copy(
            &crate::fs::StdFs::new(),
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
            "{{_output_name}}",
        )
        .unwrap();
        assert!(src_dir.join("{{template_name}}.tmpl").exists());

        copy(
            &crate::fs::StdFs::new(),
            &src_dir,
            &dst_dir,
            &vec![],
            &HashMap::from([
                ("template_name".to_string(), "template".to_string()),
                ("_output_name".to_string(), "foo".to_string()),
            ]),
        )
        .unwrap();

        assert!(
            dst_dir.join("template.tmpl").exists(),
            "template.tmpl does not exist"
        );
    }
}
