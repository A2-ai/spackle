use std::{
    collections::HashMap,
    fmt::Display,
    fs,
    path::{Path, PathBuf},
};

use tera::{Context, Tera};
use walkdir::WalkDir;

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

pub fn copy(
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

            // Skip .j2 files
            if entry.file_name().to_string_lossy().ends_with(TEMPLATE_EXT) {
                return false;
            }

            true
        })
        .collect::<Vec<_>>();

    for entry in entries {
        let entry = entry.map_err(|e| Error {
            source: e.into(),
            path: src.to_path_buf(),
        })?;

        let src_path = entry.path();
        let relative_path = src_path.strip_prefix(src).map_err(|e| Error {
            source: e.into(),
            path: src_path.to_path_buf(),
        })?;
        let dst_path_maybe_template = dest.join(relative_path);

        let context = Context::from_serialize(data).map_err(|e| Error {
            source: e.into(),
            path: src_path.to_path_buf(),
        })?;
        let dst_path: PathBuf =
            match Tera::one_off(&dst_path_maybe_template.to_string_lossy(), &context, false) {
                Ok(path) => path.into(),
                Err(e) => {
                    return Err(Error {
                        source: e.into(),
                        path: dst_path_maybe_template.to_path_buf(),
                    });
                }
            };

        if entry.file_type().is_dir() {
            fs::create_dir_all(&dst_path).map_err(|e| Error {
                source: e.into(),
                path: dst_path.clone(),
            })?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = dst_path.parent() {
                fs::create_dir_all(parent).map_err(|e| Error {
                    source: e.into(),
                    path: parent.to_path_buf(),
                })?;
            }
            fs::copy(src_path, &dst_path).map_err(|e| Error {
                source: e.into(),
                path: dst_path.clone(),
            })?;

            copied_count += 1;
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
