//! In-memory `FileSystem` impl for the wasm bundle-in / bundle-out path.
//!
//! Rust runs generation entirely against this in-process VFS. The host
//! serializes a project into a [`Bundle`] (JS `Array<{path, bytes}>`), we
//! hydrate a `MemoryFs`, call `Project::generate` through it, then drain
//! the resulting files back into a bundle for return.
//!
//! This is shaped like `spackle::fs::MockFs` but is a separate impl —
//! `MockFs` is a test utility that lives in the core crate; `MemoryFs`
//! is a production runtime component and carries bundle helpers.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spackle::fs::{FileEntry, FileStat, FileSystem, FileType};

/// One file in a project or output bundle. Paths are absolute from the
/// bundle's root (typically `/project` or `/output`). Bytes are raw —
/// JS side supplies `Uint8Array`, Rust side receives `Vec<u8>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleEntry {
    pub path: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

pub struct MemoryFs {
    files: RefCell<HashMap<PathBuf, Vec<u8>>>,
    dirs: RefCell<HashSet<PathBuf>>,
}

impl MemoryFs {
    pub fn new() -> Self {
        let mut dirs = HashSet::new();
        dirs.insert(PathBuf::from("/"));
        Self {
            files: RefCell::new(HashMap::new()),
            dirs: RefCell::new(dirs),
        }
    }

    /// Build a `MemoryFs` from a decoded bundle. Ancestor directories
    /// are implicit — adding `/project/src/a.txt` auto-creates `/project`
    /// and `/project/src`.
    pub fn from_bundle(entries: Vec<BundleEntry>) -> Self {
        let fs = Self::new();
        for entry in entries {
            let path = PathBuf::from(&entry.path);
            if let Some(parent) = path.parent() {
                fs.insert_dir_recursive(parent.to_path_buf());
            }
            fs.files.borrow_mut().insert(path, entry.bytes);
        }
        fs
    }

    /// Drain every file into a flat bundle. Directories are implicit —
    /// the host can rebuild the tree from paths alone.
    pub fn into_bundle(self) -> Vec<BundleEntry> {
        let files = self.files.into_inner();
        let mut entries: Vec<BundleEntry> = files
            .into_iter()
            .map(|(path, bytes)| BundleEntry {
                path: path.to_string_lossy().into_owned(),
                bytes,
            })
            .collect();
        // Deterministic ordering — makes snapshots and test assertions
        // stable across runs.
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        entries
    }

    /// Drain both files and directories under `prefix`. Paths in the
    /// returned lists are **relative** to `prefix` — the host knows its
    /// own output root and prepends it. Used by `generate` to return the
    /// rendered subtree without leaking the input-project files.
    ///
    /// Directories are emitted so empty dirs (created by the Rust copy
    /// pass for directory entries that have no files passing the ignore
    /// filter) survive the bundle round-trip. `spackle::copy::copy` calls
    /// `create_dir_all` for every `FileType::Directory` it walks; without
    /// this, those dirs would be silently dropped on the JS side.
    ///
    /// The empty relative path `""` — meaning `prefix` itself — is
    /// filtered out. Host code creates `out_dir` separately.
    pub fn drain_subtree(self, prefix: &Path) -> (Vec<BundleEntry>, Vec<String>) {
        let files = self.files.into_inner();
        let dirs = self.dirs.into_inner();

        let mut file_entries: Vec<BundleEntry> = files
            .into_iter()
            .filter_map(|(path, bytes)| {
                path.strip_prefix(prefix).ok().map(|rel| BundleEntry {
                    path: rel.to_string_lossy().into_owned(),
                    bytes,
                })
            })
            .collect();
        file_entries.sort_by(|a, b| a.path.cmp(&b.path));

        let mut dir_entries: Vec<String> = dirs
            .into_iter()
            .filter_map(|path| {
                path.strip_prefix(prefix).ok().map(|rel| {
                    rel.to_string_lossy().into_owned()
                })
            })
            .filter(|rel| !rel.is_empty())
            .collect();
        dir_entries.sort();

        (file_entries, dir_entries)
    }

    fn insert_dir_recursive(&self, path: PathBuf) {
        let mut to_add = Vec::new();
        let mut current = path.as_path();
        loop {
            to_add.push(current.to_path_buf());
            match current.parent() {
                Some(parent) if parent != current => current = parent,
                _ => break,
            }
        }
        let mut dirs = self.dirs.borrow_mut();
        for p in to_add {
            dirs.insert(p);
        }
    }
}

impl Default for MemoryFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for MemoryFs {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files.borrow().get(path).cloned().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("no such file: {}", path.display()),
            )
        })
    }

    fn write_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            if !self.dirs.borrow().contains(parent) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("parent directory does not exist: {}", parent.display()),
                ));
            }
        }
        self.files
            .borrow_mut()
            .insert(path.to_path_buf(), content.to_vec());
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.insert_dir_recursive(path.to_path_buf());
        Ok(())
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<FileEntry>> {
        if !self.dirs.borrow().contains(path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no such directory: {}", path.display()),
            ));
        }
        let mut out = Vec::new();
        let files = self.files.borrow();
        let dirs = self.dirs.borrow();

        for (p, _) in files.iter() {
            if p.parent() == Some(path) {
                if let Some(name) = p.file_name() {
                    out.push(FileEntry {
                        name: name.to_string_lossy().into_owned(),
                        file_type: FileType::File,
                    });
                }
            }
        }
        for d in dirs.iter() {
            if d.parent() == Some(path) {
                if let Some(name) = d.file_name() {
                    out.push(FileEntry {
                        name: name.to_string_lossy().into_owned(),
                        file_type: FileType::Directory,
                    });
                }
            }
        }
        Ok(out)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let content = self.read_file(src)?;
        self.write_file(dst, &content)
    }

    fn exists(&self, path: &Path) -> bool {
        self.files.borrow().contains_key(path) || self.dirs.borrow().contains(path)
    }

    fn stat(&self, path: &Path) -> io::Result<FileStat> {
        if let Some(bytes) = self.files.borrow().get(path) {
            return Ok(FileStat {
                file_type: FileType::File,
                size: bytes.len() as u64,
            });
        }
        if self.dirs.borrow().contains(path) {
            return Ok(FileStat {
                file_type: FileType::Directory,
                size: 0,
            });
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("no such path: {}", path.display()),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_roundtrip_preserves_files() {
        let entries = vec![
            BundleEntry {
                path: "/project/spackle.toml".into(),
                bytes: b"name = \"demo\"".to_vec(),
            },
            BundleEntry {
                path: "/project/src/main.rs.j2".into(),
                bytes: b"fn main() {}".to_vec(),
            },
        ];
        let fs = MemoryFs::from_bundle(entries);

        assert_eq!(
            fs.read_file(Path::new("/project/spackle.toml")).unwrap(),
            b"name = \"demo\""
        );
        assert!(fs.exists(Path::new("/project/src")));
        assert!(fs.exists(Path::new("/project")));

        let back = fs.into_bundle();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].path, "/project/spackle.toml");
        assert_eq!(back[1].path, "/project/src/main.rs.j2");
    }

    #[test]
    fn write_to_missing_parent_errors() {
        let fs = MemoryFs::new();
        let err = fs.write_file(Path::new("/nope/a.txt"), b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn create_dir_all_is_idempotent_and_creates_ancestors() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/a/b/c")).unwrap();
        fs.create_dir_all(Path::new("/a/b/c")).unwrap();
        assert!(fs.exists(Path::new("/a")));
        assert!(fs.exists(Path::new("/a/b")));
        assert!(fs.exists(Path::new("/a/b/c")));
    }

    #[test]
    fn list_dir_surfaces_files_and_dirs() {
        let fs = MemoryFs::from_bundle(vec![
            BundleEntry {
                path: "/p/a".into(),
                bytes: b"a".to_vec(),
            },
            BundleEntry {
                path: "/p/sub/b".into(),
                bytes: b"b".to_vec(),
            },
        ]);
        let mut entries = fs.list_dir(Path::new("/p")).unwrap();
        entries.sort_by(|x, y| x.name.cmp(&y.name));
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "a");
        assert_eq!(entries[0].file_type, FileType::File);
        assert_eq!(entries[1].name, "sub");
        assert_eq!(entries[1].file_type, FileType::Directory);
    }

    #[test]
    fn stat_missing_errors() {
        let fs = MemoryFs::new();
        let err = fs.stat(Path::new("/absent")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn drain_subtree_strips_prefix_and_filters() {
        let fs = MemoryFs::from_bundle(vec![
            BundleEntry {
                path: "/project/spackle.toml".into(),
                bytes: b"toml".to_vec(),
            },
            BundleEntry {
                path: "/output/a.txt".into(),
                bytes: b"a".to_vec(),
            },
            BundleEntry {
                path: "/output/sub/b.txt".into(),
                bytes: b"b".to_vec(),
            },
        ]);
        let (files, _dirs) = fs.drain_subtree(Path::new("/output"));
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "a.txt");
        assert_eq!(files[1].path, "sub/b.txt");
    }

    #[test]
    fn drain_subtree_emits_dirs_for_empty_dir_preservation() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/output/empty-dir")).unwrap();
        fs.create_dir_all(Path::new("/output/sub/also-empty")).unwrap();
        // And a file under /output so files path also exercises.
        fs.write_file(Path::new("/output/a.txt"), b"a").unwrap();

        let (files, dirs) = fs.drain_subtree(Path::new("/output"));
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.txt");
        // /output itself is filtered out; its descendants remain.
        assert!(!dirs.iter().any(|d| d.is_empty()));
        assert!(dirs.contains(&"empty-dir".to_string()));
        assert!(dirs.contains(&"sub".to_string()));
        assert!(dirs.contains(&"sub/also-empty".to_string()));
    }
}
