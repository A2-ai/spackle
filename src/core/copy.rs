use std::{fmt::Display, fs, path::PathBuf};

use walkdir::WalkDir;

use super::{config::CONFIG_FILE, template::TEMPLATE_EXT};

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

pub fn copy(src: &PathBuf, dest: &PathBuf, skip: &Vec<String>) -> Result<CopyResult, Error> {
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
        let dst_path = dest.join(relative_path);

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
            fs::copy(&src_path, &dst_path).map_err(|e| Error {
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
