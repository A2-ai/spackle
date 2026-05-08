//! `CallbackFs` — wasm-only `FileSystem` impl that emits each output
//! entry through a host-supplied callback as it's produced, instead of
//! buffering output in an in-memory map.
//!
//! Used by the streaming `generate` export to remove the peak ≈ 1×
//! output memory hump that the eager `MemoryFs::drain_subtree` path
//! incurs. The host hands us a JS function and a project bundle; we run
//! `Project::generate` against this fs; every `write_file` /
//! `create_dir_all` under `out_root` becomes an event delivered to the
//! callback synchronously, with the bytes dropped immediately.
//!
//! Source bundle reads still go through an inner `MemoryFs` (input-side
//! eager read is the documented remaining ceiling — out of scope here).
//!
//! Internal split:
//!   - source paths (anywhere outside `out_root`): delegated to the
//!     inner `MemoryFs` (`read_file`, `list_dir`, `stat`, `exists`).
//!   - output paths (under `out_root`): `write_file` → file event,
//!     `create_dir_all` → dir event(s) (root-to-leaf, deduped). Reads
//!     of output paths return `NotFound` — output is write-only.
//!
//! `exists(out_root)` returns `false` so `Project::generate`'s
//! `AlreadyExists` guard at `src/lib.rs:160` lets generation proceed —
//! the host has already cleared/created the real output dir before
//! calling us.
//!
//! Errors from the JS callback are latched in `callback_error`; once
//! latched, subsequent writes short-circuit with `io::Error` so the
//! template phase (which collects per-file errors at
//! `src/template.rs:235-241` rather than aborting) can surface them
//! without re-entering JS. The wasm export checks the latch after
//! `Project::generate` returns and prefers the latched JS error over
//! the synthesized `GenerateError`.
//!
//! The fs is parameterized over an `EntrySink` trait so cargo tests can
//! drive the streaming logic with a `Vec`-backed sink (no wasm runtime).

use std::cell::RefCell;
use std::collections::HashSet;
use std::io;
use std::path::{Path, PathBuf};

use serde::Serialize;
use spackle::fs::{FileEntry, FileStat, FileSystem};
use wasm_bindgen::{JsCast, JsValue};

use crate::memory_fs::MemoryFs;

/// One streamed entry passed to the sink. Borrowed — the sink is
/// expected to serialize/forward and not retain.
pub enum StreamEntry<'a> {
    File { path: &'a str, bytes: &'a [u8] },
    Dir { path: &'a str },
}

/// Sink for streamed entries. Production wasm impl wraps a
/// `js_sys::Function`; tests substitute a `Vec`-backed sink.
pub trait EntrySink {
    /// Called once per output entry. On error, return `Err(message)` —
    /// `CallbackFs` latches the message and aborts further writes.
    fn emit(&self, entry: StreamEntry<'_>) -> Result<(), String>;
}

pub struct CallbackFs<S: EntrySink> {
    source: MemoryFs,
    out_root: PathBuf,
    sink: S,
    emitted_dirs: RefCell<HashSet<PathBuf>>,
    callback_error: RefCell<Option<String>>,
}

impl<S: EntrySink> CallbackFs<S> {
    pub fn new(source: MemoryFs, out_root: PathBuf, sink: S) -> Self {
        Self {
            source,
            out_root,
            sink,
            emitted_dirs: RefCell::new(HashSet::new()),
            callback_error: RefCell::new(None),
        }
    }

    /// If the JS callback threw at any point, this returns the latched
    /// message. The wasm export consults this after `Project::generate`
    /// returns and surfaces it as the response error.
    pub fn take_callback_error(&self) -> Option<String> {
        self.callback_error.borrow_mut().take()
    }

    fn is_output(&self, path: &Path) -> bool {
        path == self.out_root || path.starts_with(&self.out_root)
    }

    fn relative_to_out(&self, path: &Path) -> String {
        let stripped = path.strip_prefix(&self.out_root).unwrap_or(path);
        normalize_to_forward(stripped)
    }

    fn emit(&self, entry: StreamEntry<'_>) -> io::Result<()> {
        if self.callback_error.borrow().is_some() {
            return Err(io::Error::new(io::ErrorKind::Other, "callback aborted"));
        }
        match self.sink.emit(entry) {
            Ok(()) => Ok(()),
            Err(msg) => {
                *self.callback_error.borrow_mut() = Some(msg.clone());
                Err(io::Error::new(io::ErrorKind::Other, msg))
            }
        }
    }
}

/// Normalize a path's components to a forward-slash-joined string. We
/// never touch a real OS fs in wasm, so paths are usually already `/`-
/// separated, but templated destination paths (`copy::copy:104`) could
/// in theory contain backslashes — guard against that leaking into the
/// emitted event.
fn normalize_to_forward(path: &Path) -> String {
    let mut out = String::new();
    let mut first = true;
    for c in path.components() {
        let part = c.as_os_str().to_string_lossy();
        if part == "/" || part == "\\" {
            continue;
        }
        if !first {
            out.push('/');
        }
        out.push_str(&part);
        first = false;
    }
    out
}

impl<S: EntrySink> FileSystem for CallbackFs<S> {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        if self.is_output(path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("output is write-only: {}", path.display()),
            ));
        }
        self.source.read_file(path)
    }

    fn write_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
        if !self.is_output(path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("write outside out_root: {}", path.display()),
            ));
        }
        let rel = self.relative_to_out(path);
        self.emit(StreamEntry::File {
            path: &rel,
            bytes: content,
        })
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        if !self.is_output(path) {
            return Ok(());
        }
        if self.callback_error.borrow().is_some() {
            return Err(io::Error::new(io::ErrorKind::Other, "callback aborted"));
        }

        // Walk the ancestor chain leaf-to-root, then emit reverse for
        // root-first parent-before-child ordering. `out_root` itself is
        // skipped — host creates the real outDir up front.
        let mut chain: Vec<PathBuf> = Vec::new();
        let mut current = path;
        loop {
            if current == self.out_root.as_path() {
                break;
            }
            if !self.is_output(current) {
                break;
            }
            chain.push(current.to_path_buf());
            match current.parent() {
                Some(parent) if parent != current => current = parent,
                _ => break,
            }
        }

        for ancestor in chain.iter().rev() {
            if self.emitted_dirs.borrow().contains(ancestor) {
                continue;
            }
            let rel = self.relative_to_out(ancestor);
            if rel.is_empty() {
                continue;
            }
            self.emit(StreamEntry::Dir { path: &rel })?;
            self.emitted_dirs.borrow_mut().insert(ancestor.clone());
        }
        Ok(())
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<FileEntry>> {
        if self.is_output(path) {
            return Ok(Vec::new());
        }
        self.source.list_dir(path)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        let bytes = self.source.read_file(src)?;
        self.write_file(dst, &bytes)
    }

    fn exists(&self, path: &Path) -> bool {
        if self.is_output(path) {
            if path == self.out_root.as_path() {
                // Required: Project::generate checks
                // `fs.exists(out_dir)` at `src/lib.rs:160` and aborts
                // with AlreadyExists if true. Our streaming model
                // expects the host to have created the real outDir
                // separately, so the in-fs view of out_root is "not
                // there yet" until something has been written under it.
                return false;
            }
            return self.emitted_dirs.borrow().contains(path);
        }
        self.source.exists(path)
    }

    fn stat(&self, path: &Path) -> io::Result<FileStat> {
        if self.is_output(path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("output is write-only: {}", path.display()),
            ));
        }
        self.source.stat(path)
    }
}

// --- JS-backed sink for the wasm export ---

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum EmittedEntry<'a> {
    File {
        path: &'a str,
        #[serde(with = "serde_bytes")]
        bytes: &'a [u8],
    },
    Dir {
        path: &'a str,
    },
}

/// `EntrySink` impl that serializes the entry and dispatches to a
/// host-provided `js_sys::Function`. Errors thrown by the callback
/// (anything that makes `call1` return `Err`) are stringified into the
/// returned message and surfaced through `CallbackFs::take_callback_error`.
pub struct JsCallbackSink {
    function: js_sys::Function,
}

impl JsCallbackSink {
    pub fn new(function: js_sys::Function) -> Self {
        Self { function }
    }
}

impl EntrySink for JsCallbackSink {
    fn emit(&self, entry: StreamEntry<'_>) -> Result<(), String> {
        let event = match entry {
            StreamEntry::File { path, bytes } => EmittedEntry::File { path, bytes },
            StreamEntry::Dir { path } => EmittedEntry::Dir { path },
        };
        let value = event
            .serialize(&serde_wasm_bindgen::Serializer::new())
            .map_err(|e| format!("serialize entry: {}", e))?;
        self.function
            .call1(&JsValue::NULL, &value)
            .map(|_| ())
            .map_err(|e| stringify_jsvalue(&e))
    }
}

fn stringify_jsvalue(v: &JsValue) -> String {
    if let Some(s) = v.as_string() {
        return s;
    }
    if let Some(err) = v.dyn_ref::<js_sys::Error>() {
        return String::from(err.to_string());
    }
    format!("{:?}", v)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use crate::memory_fs::BundleEntry;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// Vec-backed sink for cargo tests — captures every emitted entry
    /// and (optionally) errors on the Nth call.
    struct VecSink {
        events: RefCell<Vec<OwnedEvent>>,
        fail_after: Option<usize>,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum OwnedEvent {
        File { path: String, bytes: Vec<u8> },
        Dir { path: String },
    }

    impl VecSink {
        fn new() -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                fail_after: None,
            }
        }

        fn failing_after(n: usize) -> Self {
            Self {
                events: RefCell::new(Vec::new()),
                fail_after: Some(n),
            }
        }

        fn into_events(self) -> Vec<OwnedEvent> {
            self.events.into_inner()
        }
    }

    impl EntrySink for VecSink {
        fn emit(&self, entry: StreamEntry<'_>) -> Result<(), String> {
            if let Some(n) = self.fail_after {
                if self.events.borrow().len() >= n {
                    return Err(format!("simulated failure after {} events", n));
                }
            }
            self.events.borrow_mut().push(match entry {
                StreamEntry::File { path, bytes } => OwnedEvent::File {
                    path: path.to_string(),
                    bytes: bytes.to_vec(),
                },
                StreamEntry::Dir { path } => OwnedEvent::Dir {
                    path: path.to_string(),
                },
            });
            Ok(())
        }
    }

    fn make_fs(bundle: Vec<BundleEntry>, sink: VecSink) -> CallbackFs<VecSink> {
        let source = MemoryFs::from_bundle(bundle);
        CallbackFs::new(source, PathBuf::from("/output"), sink)
    }

    #[test]
    fn out_root_exists_returns_false_for_alreadyexists_guard() {
        // Project::generate at src/lib.rs:160 calls fs.exists(out_dir)
        // and aborts if true. Streaming model expects host to manage
        // the real outDir; the in-fs view of out_root must report false
        // until something has been written under it.
        let fs = make_fs(vec![], VecSink::new());
        assert!(!fs.exists(Path::new("/output")));
    }

    #[test]
    fn create_dir_all_on_out_root_emits_nothing() {
        // copy::copy:51 calls fs.create_dir_all(dest) eagerly. The
        // streaming sink should NOT receive an event for out_root
        // itself — the host already created the real outDir.
        let fs = make_fs(vec![], VecSink::new());
        fs.create_dir_all(Path::new("/output")).unwrap();
        assert!(fs.emitted_dirs.borrow().is_empty());
    }

    #[test]
    fn nested_create_dir_all_emits_root_to_leaf_deduped() {
        let sink = VecSink::new();
        let source = MemoryFs::from_bundle(vec![]);
        let fs = CallbackFs::new(source, PathBuf::from("/output"), sink);

        fs.create_dir_all(Path::new("/output/a/b/c")).unwrap();
        fs.create_dir_all(Path::new("/output/a/b/d")).unwrap();

        let events = fs.sink.into_events();
        let dir_paths: Vec<&str> = events
            .iter()
            .filter_map(|e| match e {
                OwnedEvent::Dir { path } => Some(path.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(dir_paths, vec!["a", "a/b", "a/b/c", "a/b/d"]);
    }

    #[test]
    fn write_file_emits_file_event_with_relative_path() {
        let sink = VecSink::new();
        let fs = make_fs(vec![], sink);

        fs.create_dir_all(Path::new("/output/sub")).unwrap();
        fs.write_file(Path::new("/output/sub/a.txt"), b"hello")
            .unwrap();

        let events = fs.sink.into_events();
        assert_eq!(
            events,
            vec![
                OwnedEvent::Dir {
                    path: "sub".to_string(),
                },
                OwnedEvent::File {
                    path: "sub/a.txt".to_string(),
                    bytes: b"hello".to_vec(),
                },
            ]
        );
    }

    #[test]
    fn callback_error_latches_and_short_circuits_subsequent_writes() {
        // Sink throws on the 2nd emit. First call goes through, second
        // latches; subsequent write_file/create_dir_all must error
        // without re-entering the sink.
        let source = MemoryFs::from_bundle(vec![]);
        let fs = CallbackFs::new(source, PathBuf::from("/output"), VecSink::failing_after(1));

        fs.create_dir_all(Path::new("/output/a")).unwrap();
        // Second emission attempt → sink fails → latched.
        let err = fs
            .write_file(Path::new("/output/a/x.txt"), b"x")
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Other);

        // Subsequent writes short-circuit without reaching the sink.
        let err2 = fs
            .write_file(Path::new("/output/a/y.txt"), b"y")
            .unwrap_err();
        assert_eq!(err2.kind(), io::ErrorKind::Other);
        assert!(err2.to_string().contains("callback aborted"));

        // Latched message is recoverable.
        assert!(fs
            .take_callback_error()
            .unwrap()
            .contains("simulated failure"));
    }

    #[test]
    fn copy_file_reads_source_and_emits_file_event() {
        let bundle = vec![BundleEntry {
            path: "/project/a.txt".to_string(),
            bytes: b"src-bytes".to_vec(),
        }];
        let fs = make_fs(bundle, VecSink::new());

        fs.create_dir_all(Path::new("/output")).unwrap();
        fs.copy_file(Path::new("/project/a.txt"), Path::new("/output/a.txt"))
            .unwrap();

        let events = fs.sink.into_events();
        assert_eq!(
            events,
            vec![OwnedEvent::File {
                path: "a.txt".to_string(),
                bytes: b"src-bytes".to_vec(),
            }]
        );
    }

    #[test]
    fn end_to_end_streaming_generate_against_callback_fs() {
        // Exercise the full Project::generate pipeline through
        // CallbackFs. Mirrors `memory_fs::tests::end_to_end_generate_against_memory_fs`
        // but asserts the streaming-event sequence rather than the
        // drained bundle.
        let project_toml = br#"name = "demo"
[[slots]]
key = "name"
type = "String"
"#;
        let template = b"hello from {{ name }}\n";

        let bundle = vec![
            BundleEntry {
                path: "/project/spackle.toml".into(),
                bytes: project_toml.to_vec(),
            },
            BundleEntry {
                path: "/project/{{name}}.txt.j2".into(),
                bytes: template.to_vec(),
            },
        ];
        let source = MemoryFs::from_bundle(bundle);
        let fs = CallbackFs::new(source, PathBuf::from("/output"), VecSink::new());

        let project_dir = PathBuf::from("/project");
        let out_dir = PathBuf::from("/output");
        // CallbackFs delegates source-path reads to the inner MemoryFs,
        // so load_project can run through it directly.
        let project = spackle::load_project(&fs, &project_dir).expect("load_project");

        let data = HashMap::from([("name".to_string(), "world".to_string())]);
        project
            .generate(&fs, &project_dir, &out_dir, &data)
            .expect("streaming generate succeeds");

        assert!(fs.take_callback_error().is_none());
        let events = fs.sink.into_events();

        // The rendered file lands under out_root with the templated
        // name. We don't assert the exact event order here (that's
        // copy/template implementation detail) — just that the
        // expected file event is present with the rendered bytes.
        let found = events.iter().any(|e| match e {
            OwnedEvent::File { path, bytes } => {
                path == "world.txt" && bytes == b"hello from world\n"
            }
            _ => false,
        });
        assert!(found, "rendered file event missing: {:?}", events);
    }
}
