use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};

use tera::{Context, Tera};

use crate::fs::{self as fsmod, FileSystem, FileType};
use crate::slot::Slot;
use crate::{config::CONFIG_FILE, template::has_template_ext};

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

/// Validate path templating for non-template files **without copying or
/// writing anything**. Walks `src` the same way `copy_collect` does,
/// runs `Tera::one_off` on each non-template entry's relative path
/// against an empty-context (slot keys → empty strings + `_project_name`
/// / `_output_name`), and returns one [`Error`] per failure.
///
/// This is the static-check companion to `copy_collect`: catches
/// path-template parse errors (e.g. `{{ unclosed`) and undefined-slot
/// references in filenames/directories without needing real slot data.
/// Mirrors what `template::validate_in_memory` does for body templates.
pub fn validate_paths<F: FileSystem>(
    fs: &F,
    src: &Path,
    skip: &Vec<String>,
    slots: &[Slot],
) -> Result<Vec<Error>, Error> {
    let mut context_data: HashMap<String, String> = slots
        .iter()
        .map(|s| (s.key.clone(), String::new()))
        .collect();
    context_data.insert("_project_name".to_string(), String::new());
    context_data.insert("_output_name".to_string(), String::new());
    let context = Context::from_serialize(&context_data).map_err(|e| Error {
        source: e.into(),
        path: src.to_path_buf(),
    })?;

    let entries = fsmod::walk(fs, src).map_err(|e| Error {
        source: Box::new(e),
        path: src.to_path_buf(),
    })?;

    let mut errors: Vec<Error> = Vec::new();
    let mut skipped_ancestors: Vec<PathBuf> = Vec::new();

    for (rel_path, _stat) in entries {
        if skipped_ancestors.iter().any(|a| rel_path.starts_with(a)) {
            continue;
        }
        let name = rel_path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if skip.iter().any(|s| s == &name) {
            skipped_ancestors.push(rel_path.clone());
            continue;
        }
        if name == CONFIG_FILE {
            skipped_ancestors.push(rel_path.clone());
            continue;
        }
        // `.j2`/`.tera` files are handled by `template::validate_in_memory`
        // (filename + body). Skip them here to avoid double-reporting.
        if has_template_ext(&name) {
            continue;
        }

        let path_str = rel_path.to_string_lossy();
        if let Err(e) = Tera::one_off(&path_str, &context, false) {
            errors.push(Error {
                source: e.into(),
                path: rel_path.clone(),
            });
        }
    }

    Ok(errors)
}

/// Collect-mode result: like [`CopyResult`] but with a per-entry error
/// list instead of bailing on the first failure. Used by the structured
/// `render` pipeline; legacy `copy` retains fail-fast semantics via
/// [`copy`] (which delegates here and surfaces the first error).
pub struct CopyReport {
    pub copied_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<Error>,
}

/// Fail-fast copy. Preserved as the existing entrypoint for
/// `Project::generate` and `Project::copy_files`; new code that wants
/// every per-entry error should call [`copy_collect`] instead.
pub fn copy<F: FileSystem>(
    fs: &F,
    src: &Path,
    dest: &Path,
    skip: &Vec<String>,
    data: &HashMap<String, String>,
) -> Result<CopyResult, Error> {
    let report = copy_collect(fs, src, dest, skip, data)?;
    if let Some(first) = report.errors.into_iter().next() {
        return Err(first);
    }
    Ok(CopyResult {
        copied_count: report.copied_count,
        skipped_count: report.skipped_count,
    })
}

/// Collect-don't-abort variant of `copy`. Returns every per-entry error
/// in [`CopyReport::errors`] instead of bailing. Reserves the outer
/// `Result::Err` for fatal preconditions (destination root unwritable,
/// source root unreadable) that genuinely can't recover.
pub fn copy_collect<F: FileSystem>(
    fs: &F,
    src: &Path,
    dest: &Path,
    skip: &Vec<String>,
    data: &HashMap<String, String>,
) -> Result<CopyReport, Error> {
    let mut copied_count = 0;
    let mut skipped_count = 0;
    let mut errors: Vec<Error> = Vec::new();

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

    // First pass: apply the skip/config-file/template-ext filter on a
    // per-entry basis. Unlike walkdir::filter_entry we can't prune whole
    // subtrees in a single pass — so we explicitly compute an "ancestor
    // skipped" set while iterating (entries are in DFS order).
    let mut skipped_ancestors: Vec<PathBuf> = Vec::new();

    for (rel_path, stat) in entries {
        // If any ancestor was skipped, skip everything under it.
        if skipped_ancestors.iter().any(|a| rel_path.starts_with(a)) {
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
        if has_template_ext(&name) {
            // Template files (.j2 / .tera) are skipped here;
            // template::fill handles them.
            continue;
        }

        let src_path = src.join(&rel_path);
        let dst_path_maybe_template = dest.join(&rel_path);

        let context = match Context::from_serialize(data) {
            Ok(c) => c,
            Err(e) => {
                errors.push(Error {
                    source: e.into(),
                    path: src_path.clone(),
                });
                continue;
            }
        };
        let dst_path: PathBuf =
            match Tera::one_off(&dst_path_maybe_template.to_string_lossy(), &context, false) {
                Ok(path) => path.into(),
                Err(e) => {
                    errors.push(Error {
                        source: e.into(),
                        path: dst_path_maybe_template.clone(),
                    });
                    continue;
                }
            };

        match stat.file_type {
            FileType::Directory => {
                if let Err(e) = fs.create_dir_all(&dst_path) {
                    errors.push(Error {
                        source: Box::new(e),
                        path: dst_path.clone(),
                    });
                }
            }
            FileType::File => {
                if let Some(parent) = dst_path.parent() {
                    if let Err(e) = fs.create_dir_all(parent) {
                        errors.push(Error {
                            source: Box::new(e),
                            path: parent.to_path_buf(),
                        });
                        continue;
                    }
                }
                match fs.copy_file(&src_path, &dst_path) {
                    Ok(()) => copied_count += 1,
                    Err(e) => errors.push(Error {
                        source: Box::new(e),
                        path: dst_path.clone(),
                    }),
                }
            }
            FileType::Symlink | FileType::Other => {
                // Symlinks and other special entries are not copied —
                // matches prior walkdir behavior (only is_dir / is_file
                // branches).
            }
        }
    }

    Ok(CopyReport {
        copied_count,
        skipped_count,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::{collections::HashMap, fs};
    use tempfile::TempDir;

    #[test]
    fn ignore_one() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let src_dir = src.path();
        let dst_dir = dst.path();

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
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let src_dir = src.path();
        let dst_dir = dst.path();

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
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let src_dir = src.path();
        let dst_dir = dst.path();

        // A file whose name contains template syntax but does not end with a
        // template extension (.j2 / .tera) should have its name replaced while
        // contents remain untouched. A template extension marks files whose
        // *contents* get rendered.
        fs::write(
            src_dir.join(format!("{}.tmpl", "{{template_name}}")),
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
        assert_eq!(
            fs::read_to_string(dst_dir.join("template.tmpl")).unwrap(),
            "{{_output_name}}",
            "contents should be copied verbatim (not templated)"
        );
    }
}
