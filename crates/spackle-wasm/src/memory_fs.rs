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
                path.strip_prefix(prefix)
                    .ok()
                    .map(|rel| rel.to_string_lossy().into_owned())
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
        fs.create_dir_all(Path::new("/output/sub/also-empty"))
            .unwrap();
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

    // --- soundness of individual FileSystem methods ---

    #[test]
    fn read_file_missing_errors() {
        let fs = MemoryFs::new();
        let err = fs.read_file(Path::new("/nope")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn write_file_overwrites_existing() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/d")).unwrap();
        fs.write_file(Path::new("/d/x"), b"first").unwrap();
        fs.write_file(Path::new("/d/x"), b"second").unwrap();
        assert_eq!(fs.read_file(Path::new("/d/x")).unwrap(), b"second");
    }

    #[test]
    fn copy_file_roundtrip() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/a")).unwrap();
        fs.create_dir_all(Path::new("/b")).unwrap();
        fs.write_file(Path::new("/a/src"), b"hello").unwrap();

        fs.copy_file(Path::new("/a/src"), Path::new("/b/dst"))
            .unwrap();
        assert_eq!(fs.read_file(Path::new("/b/dst")).unwrap(), b"hello");
        // Source survives — this is a copy, not a move.
        assert_eq!(fs.read_file(Path::new("/a/src")).unwrap(), b"hello");
    }

    #[test]
    fn copy_file_missing_source_errors() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/b")).unwrap();
        let err = fs
            .copy_file(Path::new("/nowhere"), Path::new("/b/dst"))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn copy_file_missing_destination_parent_errors() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/a")).unwrap();
        fs.write_file(Path::new("/a/src"), b"x").unwrap();
        let err = fs
            .copy_file(Path::new("/a/src"), Path::new("/missing-parent/dst"))
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn list_dir_missing_errors() {
        let fs = MemoryFs::new();
        let err = fs.list_dir(Path::new("/absent")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn list_dir_empty_dir_returns_empty() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/empty")).unwrap();
        assert!(fs.list_dir(Path::new("/empty")).unwrap().is_empty());
    }

    #[test]
    fn stat_file_returns_byte_size() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/d")).unwrap();
        fs.write_file(Path::new("/d/f"), b"abcdef").unwrap();
        let st = fs.stat(Path::new("/d/f")).unwrap();
        assert_eq!(st.file_type, FileType::File);
        assert_eq!(st.size, 6);
    }

    #[test]
    fn stat_dir_returns_directory_type() {
        let fs = MemoryFs::new();
        fs.create_dir_all(Path::new("/d/sub")).unwrap();
        let st = fs.stat(Path::new("/d/sub")).unwrap();
        assert_eq!(st.file_type, FileType::Directory);
    }

    // --- bundle helpers under edge conditions ---

    #[test]
    fn from_bundle_empty_produces_empty_fs() {
        let fs = MemoryFs::from_bundle(vec![]);
        assert!(fs.into_bundle().is_empty());
    }

    #[test]
    fn into_bundle_is_sorted_by_path() {
        let fs = MemoryFs::from_bundle(vec![
            BundleEntry {
                path: "/zzz".into(),
                bytes: b"z".to_vec(),
            },
            BundleEntry {
                path: "/aaa".into(),
                bytes: b"a".to_vec(),
            },
            BundleEntry {
                path: "/mmm".into(),
                bytes: b"m".to_vec(),
            },
        ]);
        let out = fs.into_bundle();
        assert_eq!(
            out.iter().map(|e| e.path.as_str()).collect::<Vec<_>>(),
            vec!["/aaa", "/mmm", "/zzz"]
        );
    }

    #[test]
    fn drain_subtree_with_no_matches_returns_empty() {
        let fs = MemoryFs::from_bundle(vec![BundleEntry {
            path: "/project/a.txt".into(),
            bytes: b"a".to_vec(),
        }]);
        let (files, dirs) = fs.drain_subtree(Path::new("/nothing-here"));
        assert!(files.is_empty());
        assert!(dirs.is_empty());
    }

    // --- end-to-end: drive spackle::Project::generate through MemoryFs ---
    //
    // Core generation logic (copy, template::fill, config::load) is already
    // covered by `cargo test -p spackle`. This test just confirms MemoryFs
    // satisfies the `FileSystem` contract closely enough that a real
    // end-to-end generate succeeds and produces the expected output.

    #[test]
    fn end_to_end_generate_against_memory_fs() {
        use std::collections::HashMap;

        let project_toml = br#"name = "demo"
[[slots]]
key = "name"
type = "String"
"#;
        let template = b"hello from {{ name }}\n";

        let fs = MemoryFs::from_bundle(vec![
            BundleEntry {
                path: "/project/spackle.toml".into(),
                bytes: project_toml.to_vec(),
            },
            BundleEntry {
                path: "/project/{{name}}.txt.j2".into(),
                bytes: template.to_vec(),
            },
        ]);

        let project_dir = PathBuf::from("/project");
        let out_dir = PathBuf::from("/output");

        let project = spackle::load_project(&fs, &project_dir).expect("load_project");
        project.check(&fs).expect("check passes");

        let data = HashMap::from([("name".to_string(), "world".to_string())]);
        project
            .generate(&fs, &project_dir, &out_dir, &data)
            .expect("generate succeeds");

        let (files, _dirs) = fs.drain_subtree(&out_dir);
        let rendered = files
            .iter()
            .find(|e| e.path == "world.txt")
            .expect("output file at rendered path 'world.txt'");
        assert_eq!(rendered.bytes, b"hello from world\n");
    }
}
