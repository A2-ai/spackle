//! Filesystem abstraction.
//!
//! All fs operations in spackle's core go through the `FileSystem` trait
//! so the crate isn't pinned to `std::fs`. Two in-tree implementations:
//!
//!   - [`StdFs`] — wraps `std::fs` + `walkdir::WalkDir`. Native-only
//!     (cfg-gated to `not(target_arch = "wasm32")`). Used by the
//!     CLI and by any Rust code running outside wasm.
//!   - [`MockFs`] — in-memory, for unit tests that don't want a tmpdir.
//!
//! For wasm builds the adapter is [`crate::wasm_fs::JsFs`] (cfg-gated
//! to wasm32), which calls back into a JS-provided `SpackleFs` object
//! for every fs operation. See `WASM.md` for the host-side contract.
//!
//! Path semantics: the trait is path-shaped (not fd-shaped) — each call
//! takes a `&Path`. Impls decide how paths are resolved. `StdFs` uses
//! real disk paths; `JsFs` passes them through to the JS adapter, which
//! is responsible for its own containment / workspace-rooting.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};

/// File metadata surfaced by [`FileSystem::stat`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStat {
    pub file_type: FileType,
    pub size: u64,
}

/// What kind of entry a path points at. Matches the WIT `file-type` enum
/// used by the host-fs component, so the trait shape aligns on both
/// sides of the wasm boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileType {
    File,
    Directory,
    Symlink,
    Other,
}

/// A single entry returned by [`FileSystem::list_dir`]. `name` is the
/// basename only — callers compose it with the parent path themselves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub file_type: FileType,
}

/// Per-file operations spackle's core needs from any filesystem backend.
///
/// Methods are per-file and synchronous — no streams, no batch transforms.
/// This is the guardrail against accumulating a whole project's contents
/// in memory inside the component.
pub trait FileSystem {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>>;
    fn write_file(&self, path: &Path, content: &[u8]) -> io::Result<()>;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    /// Immediate children of `path`. Caller recurses via `list_dir` +
    /// `stat` if it wants a deep walk; see [`walk`].
    fn list_dir(&self, path: &Path) -> io::Result<Vec<FileEntry>>;
    fn copy_file(&self, src: &Path, dst: &Path) -> io::Result<()>;
    fn exists(&self, path: &Path) -> bool;
    fn stat(&self, path: &Path) -> io::Result<FileStat>;
}

/// Depth-first walk over a directory tree, yielding `(relative_path, FileStat)`
/// pairs. `relative_path` is relative to `root`. Directories are yielded
/// before their contents so callers can skip subtrees.
///
/// Symlinks are yielded as-is (type = `Symlink`); this walker does not
/// follow them — the caller decides (typical spackle behavior: skip).
pub fn walk<F: FileSystem + ?Sized>(
    fs: &F,
    root: &Path,
) -> io::Result<Vec<(PathBuf, FileStat)>> {
    let mut out = Vec::new();
    walk_inner(fs, root, Path::new(""), &mut out)?;
    Ok(out)
}

fn walk_inner<F: FileSystem + ?Sized>(
    fs: &F,
    abs: &Path,
    rel: &Path,
    out: &mut Vec<(PathBuf, FileStat)>,
) -> io::Result<()> {
    for entry in fs.list_dir(abs)? {
        let child_abs = abs.join(&entry.name);
        let child_rel = rel.join(&entry.name);
        let stat = fs.stat(&child_abs)?;
        out.push((child_rel.clone(), stat));
        if stat.file_type == FileType::Directory {
            walk_inner(fs, &child_abs, &child_rel, out)?;
        }
    }
    Ok(())
}

// --- StdFs: std::fs + walkdir backend ---
//
// Native-only. wasm32 builds never include StdFs — they use `JsFs`
// (see `src/wasm_fs.rs`) which delegates fs operations to a host-
// provided JS adapter. This keeps `std::fs` from leaking into the
// wasm binary's imports at all.

#[cfg(not(target_arch = "wasm32"))]
pub use self::std_fs::StdFs;

#[cfg(not(target_arch = "wasm32"))]
mod std_fs {
    use super::*;
    use std::fs;

    /// Native filesystem backend wrapping `std::fs` + `walkdir`. Not
    /// compiled for wasm targets — wasm builds use
    /// [`crate::wasm_fs::JsFs`] which delegates fs operations to a JS
    /// adapter. That keeps `std::fs` (and any transitive fs-ish imports)
    /// out of the wasm binary entirely.
    pub struct StdFs;

    impl StdFs {
        pub fn new() -> Self {
            StdFs
        }
    }

    impl Default for StdFs {
        fn default() -> Self {
            Self::new()
        }
    }

    fn file_type_from_metadata(md: &fs::Metadata) -> FileType {
        let ft = md.file_type();
        if ft.is_dir() {
            FileType::Directory
        } else if ft.is_symlink() {
            FileType::Symlink
        } else if ft.is_file() {
            FileType::File
        } else {
            FileType::Other
        }
    }

    impl FileSystem for StdFs {
        fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
            fs::read(path)
        }

        fn write_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
            fs::write(path, content)
        }

        fn create_dir_all(&self, path: &Path) -> io::Result<()> {
            fs::create_dir_all(path)
        }

        fn list_dir(&self, path: &Path) -> io::Result<Vec<FileEntry>> {
            let mut out = Vec::new();
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let file_type = file_type_from_metadata(&entry.metadata()?);
                out.push(FileEntry { name, file_type });
            }
            Ok(out)
        }

        fn copy_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
            fs::copy(src, dst).map(|_| ())
        }

        fn exists(&self, path: &Path) -> bool {
            path.exists()
        }

        fn stat(&self, path: &Path) -> io::Result<FileStat> {
            // `symlink_metadata` does NOT follow symlinks — matches the
            // `read_dir` + Dirent surface that `list_dir` returns, so a
            // walker relying on `stat` won't unexpectedly recurse into
            // symlink targets (or loop on cyclic symlinks).
            let md = fs::symlink_metadata(path)?;
            Ok(FileStat {
                file_type: file_type_from_metadata(&md),
                size: md.len(),
            })
        }
    }
}

// --- MockFs: in-memory backend for unit tests ---

/// In-memory filesystem for tests. Stores bytes keyed by canonical path.
/// Not thread-safe (uses `RefCell`) — tests don't need it to be.
pub struct MockFs {
    files: std::cell::RefCell<HashMap<PathBuf, Vec<u8>>>,
    dirs: std::cell::RefCell<std::collections::HashSet<PathBuf>>,
}

impl MockFs {
    pub fn new() -> Self {
        let mut dirs = std::collections::HashSet::new();
        // Implicit root dir.
        dirs.insert(PathBuf::from("/"));
        Self {
            files: std::cell::RefCell::new(HashMap::new()),
            dirs: std::cell::RefCell::new(dirs),
        }
    }

    /// Convenience: pre-populate a file.
    pub fn insert_file(&self, path: impl Into<PathBuf>, content: impl Into<Vec<u8>>) {
        let path = path.into();
        if let Some(parent) = path.parent() {
            self.insert_dir_recursive(parent.to_path_buf());
        }
        self.files.borrow_mut().insert(path, content.into());
    }

    /// Convenience: pre-populate a directory (and its ancestors).
    pub fn insert_dir(&self, path: impl Into<PathBuf>) {
        self.insert_dir_recursive(path.into());
    }

    fn insert_dir_recursive(&self, path: PathBuf) {
        let mut current = path.as_path();
        let mut to_add = Vec::new();
        while let Some(p) = Some(current) {
            to_add.push(p.to_path_buf());
            match p.parent() {
                Some(parent) if parent != p => current = parent,
                _ => break,
            }
        }
        let mut dirs = self.dirs.borrow_mut();
        for p in to_add {
            dirs.insert(p);
        }
    }
}

impl Default for MockFs {
    fn default() -> Self {
        Self::new()
    }
}

impl FileSystem for MockFs {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        self.files
            .borrow()
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no such file: {}", path.display())))
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
        self.files.borrow_mut().insert(path.to_path_buf(), content.to_vec());
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
    fn mock_fs_read_write_roundtrip() {
        let fs = MockFs::new();
        fs.insert_dir("/workspace");
        fs.write_file(Path::new("/workspace/a.txt"), b"hello").unwrap();
        assert_eq!(fs.read_file(Path::new("/workspace/a.txt")).unwrap(), b"hello");
    }

    #[test]
    fn mock_fs_list_dir_surfaces_files_and_dirs() {
        let fs = MockFs::new();
        fs.insert_dir("/project");
        fs.insert_file("/project/a.txt", *b"a");
        fs.insert_file("/project/b.txt", *b"b");
        fs.insert_dir("/project/sub");

        let mut entries = fs.list_dir(Path::new("/project")).unwrap();
        entries.sort_by(|a, b| a.name.cmp(&b.name));

        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "a.txt");
        assert_eq!(entries[0].file_type, FileType::File);
        assert_eq!(entries[1].name, "b.txt");
        assert_eq!(entries[2].name, "sub");
        assert_eq!(entries[2].file_type, FileType::Directory);
    }

    #[test]
    fn mock_fs_walk_yields_all_descendants() {
        let fs = MockFs::new();
        fs.insert_dir("/p");
        fs.insert_file("/p/a", *b"a");
        fs.insert_dir("/p/sub");
        fs.insert_file("/p/sub/b", *b"b");

        let mut results = walk(&fs, Path::new("/p")).unwrap();
        results.sort_by(|a, b| a.0.cmp(&b.0));

        let paths: Vec<_> = results.iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect();
        assert!(paths.contains(&"a".to_string()));
        assert!(paths.contains(&"sub".to_string()));
        assert!(paths.contains(&"sub/b".to_string()));
    }

    #[test]
    fn mock_fs_write_to_missing_parent_errors() {
        let fs = MockFs::new();
        let err = fs.write_file(Path::new("/nope/a.txt"), b"x").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn mock_fs_read_missing_errors() {
        let fs = MockFs::new();
        let err = fs.read_file(Path::new("/absent")).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::NotFound);
    }

    /// `StdFs::stat` uses `symlink_metadata` so it surfaces a symlink
    /// as `FileType::Symlink` without following. Walkers that recurse
    /// on `FileType::Directory` must not accidentally descend into
    /// symlink targets (or cyclic symlinks cause infinite recursion).
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn std_fs_stat_does_not_follow_symlinks() {
        use tempdir::TempDir;

        let dir = TempDir::new("spackle-stat").unwrap().into_path();
        let real = dir.join("real");
        std::fs::create_dir(&real).unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();

        let fs = StdFs::new();
        let st = fs.stat(&link).unwrap();
        assert_eq!(
            st.file_type,
            FileType::Symlink,
            "stat(symlink) must NOT follow — got {:?}",
            st.file_type,
        );

        let st_real = fs.stat(&real).unwrap();
        assert_eq!(st_real.file_type, FileType::Directory);
    }
}
