//! In-memory `FileSystem` impl backing the wasm `check`,
//! `validate_slot_data`, and `plan_hooks` exports. Hydrates a small
//! bundle (typically `spackle.toml` plus template bodies) into a VFS
//! that spackle core's check / load / plan routines can walk through
//! the `FileSystem` trait.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use spackle::fs::{FileEntry, FileStat, FileSystem, FileType};

/// One file in a project input bundle. Paths are absolute from the
/// bundle's root (typically `/project`). Bytes are raw — JS side
/// supplies `Uint8Array`, Rust side receives `Vec<u8>`.
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

    fn open_read<'a>(&'a self, path: &Path) -> io::Result<Box<dyn Read + 'a>> {
        let bytes = self.read_file(path)?;
        Ok(Box::new(io::Cursor::new(bytes)))
    }

    fn open_write<'a>(&'a self, path: &Path) -> io::Result<Box<dyn Write + 'a>> {
        if let Some(parent) = path.parent() {
            if !self.dirs.borrow().contains(parent) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("parent directory does not exist: {}", parent.display()),
                ));
            }
        }
        Ok(Box::new(MemoryFsWriter {
            fs: self,
            path: path.to_path_buf(),
            buf: Vec::new(),
        }))
    }
}

/// Commit-on-drop writer. `io::copy` doesn't `flush`, so the commit
/// has to ride `Drop`.
struct MemoryFsWriter<'a> {
    fs: &'a MemoryFs,
    path: PathBuf,
    buf: Vec<u8>,
}

impl<'a> Write for MemoryFsWriter<'a> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> Drop for MemoryFsWriter<'a> {
    fn drop(&mut self) {
        let _ = self.fs.write_file(&self.path, &self.buf);
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

    #[test]
    fn from_bundle_empty_produces_empty_fs() {
        let fs = MemoryFs::from_bundle(vec![]);
        assert!(fs.list_dir(Path::new("/")).unwrap().is_empty());
    }
}
