//! `FileSystem` adapter that delegates to a JS-provided fs object via
//! wasm-bindgen. Active only in wasm32 + `wasm` feature builds.
//!
//! The JS object must implement the `SpackleFs` shape (see WASM.md):
//!
//! ```ts
//! interface SpackleFs {
//!   readFile(path: string): Uint8Array;
//!   writeFile(path: string, content: Uint8Array): void;
//!   createDirAll(path: string): void;
//!   listDir(path: string): Array<{ name: string; type: SpackleFileType }>;
//!   copyFile(src: string, dst: string): void;
//!   exists(path: string): boolean;
//!   stat(path: string): { type: SpackleFileType; size: number | bigint };
//! }
//! ```
//!
//! All methods are synchronous and throw `{ kind, message }` on error.
//! Returning a Promise or throwing a non-typed Error is a contract
//! violation — `JsFs` surfaces it as `io::ErrorKind::Other`.

use std::io;
use std::path::Path;

use js_sys::{Array, Function, Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue};

use crate::fs::{FileEntry, FileStat, FileSystem, FileType};

/// Adapter wrapping a JS object that implements the `SpackleFs` shape.
/// Cheap to construct (just holds the `JsValue` reference).
pub struct JsFs {
    obj: JsValue,
}

impl JsFs {
    pub fn new(obj: JsValue) -> Self {
        JsFs { obj }
    }

    /// Look up a named method on the underlying JS object.
    fn method(&self, name: &str) -> io::Result<Function> {
        let val = Reflect::get(&self.obj, &JsValue::from_str(name)).map_err(|e| {
            io::Error::other(format!(
                "SpackleFs.{}: property lookup failed: {}",
                name,
                js_value_debug(&e)
            ))
        })?;
        val.dyn_into::<Function>().map_err(|v| {
            io::Error::other(format!(
                "SpackleFs.{} is not a function (got {})",
                name,
                js_value_debug(&v)
            ))
        })
    }

    fn call1(&self, name: &str, a: &JsValue) -> io::Result<JsValue> {
        let f = self.method(name)?;
        f.call1(&self.obj, a).map_err(|e| decode_error(name, e))
    }

    fn call2(&self, name: &str, a: &JsValue, b: &JsValue) -> io::Result<JsValue> {
        let f = self.method(name)?;
        f.call2(&self.obj, a, b).map_err(|e| decode_error(name, e))
    }
}

fn js_value_debug(v: &JsValue) -> String {
    v.as_string()
        .unwrap_or_else(|| format!("{:?}", v))
}

/// Decode a thrown JsValue into an `io::Error`. Looks for `{ kind, message }`
/// shape first, falls back to `Other` with the stringified value.
fn decode_error(method: &str, err: JsValue) -> io::Error {
    let kind_js = Reflect::get(&err, &JsValue::from_str("kind")).ok();
    let kind_str = kind_js.as_ref().and_then(|v| v.as_string());
    let msg = Reflect::get(&err, &JsValue::from_str("message"))
        .ok()
        .and_then(|v| v.as_string())
        .unwrap_or_else(|| js_value_debug(&err));

    match kind_str.as_deref() {
        Some(k) => io::Error::new(map_kind(k), format!("SpackleFs.{}: {}", method, msg)),
        None => io::Error::other(format!("SpackleFs.{}: {}", method, msg)),
    }
}

fn map_kind(kind: &str) -> io::ErrorKind {
    crate::wasm_fs_kind::map_spackle_fs_kind(kind)
}

fn path_arg(path: &Path) -> JsValue {
    JsValue::from_str(path.to_str().unwrap_or(""))
}

fn file_type_from_str(s: &str) -> FileType {
    match s {
        "file" => FileType::File,
        "directory" => FileType::Directory,
        "symlink" => FileType::Symlink,
        _ => FileType::Other,
    }
}

fn decode_size(v: &JsValue) -> io::Result<u64> {
    // Accept either Number (<= 2^53) or BigInt.
    if let Some(n) = v.as_f64() {
        if n < 0.0 {
            return Err(io::Error::other(format!("negative file size: {}", n)));
        }
        return Ok(n as u64);
    }
    // BigInt fallback — use JS string coercion.
    let s = v.as_string().or_else(|| {
        // If it's a BigInt, String() coercion yields decimal digits.
        js_sys::JsString::from(v.clone()).as_string()
    });
    s.ok_or_else(|| io::Error::other("file size is neither a Number nor a stringifiable BigInt"))
        .and_then(|raw| {
            raw.parse::<u64>()
                .map_err(|e| io::Error::other(format!("invalid file size {:?}: {}", raw, e)))
        })
}

impl FileSystem for JsFs {
    fn read_file(&self, path: &Path) -> io::Result<Vec<u8>> {
        let result = self.call1("readFile", &path_arg(path))?;
        let arr = Uint8Array::new(&result);
        Ok(arr.to_vec())
    }

    fn write_file(&self, path: &Path, content: &[u8]) -> io::Result<()> {
        // Allocate a Uint8Array view over Rust memory and copy into JS.
        let js_content = Uint8Array::new_with_length(content.len() as u32);
        js_content.copy_from(content);
        self.call2("writeFile", &path_arg(path), &js_content.into())?;
        Ok(())
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        self.call1("createDirAll", &path_arg(path))?;
        Ok(())
    }

    fn list_dir(&self, path: &Path) -> io::Result<Vec<FileEntry>> {
        let result = self.call1("listDir", &path_arg(path))?;
        let arr: Array = result.dyn_into().map_err(|v| {
            io::Error::other(format!(
                "SpackleFs.listDir: expected Array, got {}",
                js_value_debug(&v)
            ))
        })?;

        let mut out = Vec::with_capacity(arr.length() as usize);
        for i in 0..arr.length() {
            let entry = arr.get(i);
            let name = Reflect::get(&entry, &JsValue::from_str("name"))
                .ok()
                .and_then(|v| v.as_string())
                .ok_or_else(|| {
                    io::Error::other(format!("SpackleFs.listDir[{}]: missing `name`", i))
                })?;
            let type_str = Reflect::get(&entry, &JsValue::from_str("type"))
                .ok()
                .and_then(|v| v.as_string())
                .ok_or_else(|| {
                    io::Error::other(format!("SpackleFs.listDir[{}]: missing `type`", i))
                })?;
            out.push(FileEntry {
                name,
                file_type: file_type_from_str(&type_str),
            });
        }
        Ok(out)
    }

    fn copy_file(&self, src: &Path, dst: &Path) -> io::Result<()> {
        self.call2("copyFile", &path_arg(src), &path_arg(dst))?;
        Ok(())
    }

    fn exists(&self, path: &Path) -> bool {
        // Errors collapse to `false` — matches `Path::exists` semantics
        // in std (which also returns false for any io error).
        self.call1("exists", &path_arg(path))
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    fn stat(&self, path: &Path) -> io::Result<FileStat> {
        let result = self.call1("stat", &path_arg(path))?;
        let type_str = Reflect::get(&result, &JsValue::from_str("type"))
            .ok()
            .and_then(|v| v.as_string())
            .ok_or_else(|| io::Error::other("SpackleFs.stat: missing `type`"))?;
        let size_val = Reflect::get(&result, &JsValue::from_str("size"))
            .map_err(|_| io::Error::other("SpackleFs.stat: missing `size`"))?;
        let size = decode_size(&size_val)?;
        Ok(FileStat {
            file_type: file_type_from_str(&type_str),
            size,
        })
    }
}

/// Helper: construct a `JsFs` from a JS object passed into a wasm-bindgen
/// export. Takes an `Object` (more specific than `JsValue`) to surface
/// type errors at the wasm-bindgen boundary rather than inside methods.
pub fn js_fs_from_object(obj: Object) -> JsFs {
    JsFs::new(obj.into())
}
