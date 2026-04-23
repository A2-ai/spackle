use std::{
    collections::HashMap,
    fmt::Display,
    path::{Path, PathBuf},
};

use tera::{Context, Tera};

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

pub fn copy<F: FileSystem>(
    fs: &F,
    src: &Path,
    dest: &Path,
    skip: &Vec<String>,
    data: &HashMap<String, String>,
) -> Result<CopyResult, Error> {
    let mut copied_count = 0;
    let mut skipped_count = 0;

    let entries = WalkDir::new(src)
        .into_iter()
        .filter_entry(|entry| {
            // Skip those that match "skip"
            if skip
                .iter()
                .any(|s| entry.file_name().to_string_lossy() == *s)
            {
                skipped_count += 1;
                return false;
            }

            // TODO pull these out and pass as args if possible
            // Skip config file
            if entry.file_name() == CONFIG_FILE {
                return false;
            }

            // Skip template files (handled by template::fill)
            if has_template_ext(&entry.file_name().to_string_lossy()) {
                return false;
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
