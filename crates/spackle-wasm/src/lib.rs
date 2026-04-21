//! wasm-bindgen exports for spackle, bundle-in / bundle-out.
//!
//! Three exports plus `init`. Each takes a project bundle — a JS
//! `Array<{path, bytes: Uint8Array}>` — deserializes it into an in-process
//! [`MemoryFs`], runs the requested operation against that, and returns
//! a serialized result. Rust never touches the host's filesystem; the
//! host side (TS) does all disk I/O before and after the call.
//!
//! Shapes:
//!
//!   check(project_bundle, project_dir) -> String (JSON):
//!     success: `{ "valid": true, "config": {...}, "errors": [] }`
//!     failure: `{ "valid": false, "errors": ["..."] }`
//!
//!   validate_slot_data(project_bundle, project_dir, slot_data_json) -> String (JSON):
//!     success: `{ "valid": true }`
//!     failure: `{ "valid": false, "errors": ["..."] }`
//!
//!   generate(project_bundle, project_dir, out_dir, slot_data_json, run_hooks) -> JsValue:
//!     success: `{ ok: true, files: Array<{path, bytes: Uint8Array}> }`
//!       where `path` is relative to `out_dir`
//!     failure: `{ ok: false, error: "..." }`
//!     run_hooks=true: always failure with "hooks are unsupported in this milestone".

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;
use wasm_bindgen::prelude::*;

pub mod memory_fs;

use memory_fs::{BundleEntry, MemoryFs};

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

// --- response shapes ---

#[derive(Serialize)]
struct CheckOk<'a> {
    valid: bool,
    config: &'a spackle::config::Config,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct ValidationErr {
    valid: bool,
    errors: Vec<String>,
}

#[derive(Serialize)]
struct GenerateOk {
    ok: bool,
    files: Vec<BundleEntry>,
    /// Directories under `out_dir` relative to it. Included separately
    /// from `files` so empty dirs (created by the copy pass for
    /// Directory entries that had no files pass the ignore filter)
    /// survive the bundle round-trip — host must `mkdir -p` each.
    dirs: Vec<String>,
}

#[derive(Serialize)]
struct GenerateErr {
    ok: bool,
    error: String,
}

fn json_or_panic<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).unwrap_or_else(|e| format!(r#"{{"error":"{}"}}"#, e))
}

fn invalid(errors: Vec<String>) -> String {
    json_or_panic(&ValidationErr {
        valid: false,
        errors,
    })
}

fn serializer() -> serde_wasm_bindgen::Serializer {
    // Default `Serializer::new()` already emits `Uint8Array` for byte slices
    // (and `serde_bytes`-tagged `Vec<u8>`) — `serialize_bytes_as_arrays` is
    // `false` by default. Inbound deserialization also accepts `Uint8Array`.
    serde_wasm_bindgen::Serializer::new()
}

fn generate_err_value(error: String) -> JsValue {
    GenerateErr { ok: false, error }
        .serialize(&serializer())
        .unwrap_or(JsValue::NULL)
}

fn decode_bundle(project_bundle: JsValue) -> Result<Vec<BundleEntry>, String> {
    serde_wasm_bindgen::from_value(project_bundle).map_err(|e| format!("invalid bundle: {}", e))
}

// --- exports ---

/// Validate a project bundle: loads spackle.toml, checks slot config,
/// validates template references against slot keys.
#[wasm_bindgen]
pub fn check(project_bundle: JsValue, project_dir: &str) -> String {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return invalid(vec![msg]),
    };
    let fs = MemoryFs::from_bundle(entries);
    let path = PathBuf::from(project_dir);
    let project = match spackle::load_project(&fs, &path) {
        Ok(p) => p,
        Err(e) => return invalid(vec![e.to_string()]),
    };
    match project.check(&fs) {
        Ok(()) => json_or_panic(&CheckOk {
            valid: true,
            config: &project.config,
            errors: vec![],
        }),
        Err(e) => invalid(vec![e.to_string()]),
    }
}

/// Validate slot data against the config loaded from the project bundle.
#[wasm_bindgen]
pub fn validate_slot_data(
    project_bundle: JsValue,
    project_dir: &str,
    slot_data_json: &str,
) -> String {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return invalid(vec![msg]),
    };
    let fs = MemoryFs::from_bundle(entries);
    let path = PathBuf::from(project_dir);
    let project = match spackle::load_project(&fs, &path) {
        Ok(p) => p,
        Err(e) => return invalid(vec![e.to_string()]),
    };
    let data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return invalid(vec![format!("invalid slot_data_json: {}", e)]),
    };
    match spackle::slot::validate_data(&data, &project.config.slots) {
        Ok(()) => r#"{"valid":true}"#.to_string(),
        Err(e) => invalid(vec![e.to_string()]),
    }
}

/// Generate a filled project. Runs the full generate pipeline (copy +
/// template fill) against an in-memory fs, returns the rendered subtree
/// as a flat bundle with paths relative to `out_dir`.
///
/// `run_hooks = true` is unsupported in this milestone; returns an
/// explicit error so the behavior is observable.
#[wasm_bindgen]
pub fn generate(
    project_bundle: JsValue,
    project_dir: &str,
    out_dir: &str,
    slot_data_json: &str,
    run_hooks: bool,
) -> JsValue {
    if run_hooks {
        return generate_err_value(
            "hooks are unsupported in this milestone; call with run_hooks=false".to_string(),
        );
    }

    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return generate_err_value(msg),
    };
    let fs = MemoryFs::from_bundle(entries);
    let project_path = PathBuf::from(project_dir);
    let out_path = PathBuf::from(out_dir);

    let project = match spackle::load_project(&fs, &project_path) {
        Ok(p) => p,
        Err(e) => return generate_err_value(e.to_string()),
    };

    let slot_data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return generate_err_value(format!("invalid slot_data_json: {}", e)),
    };

    if let Err(e) = spackle::slot::validate_data(&slot_data, &project.config.slots) {
        return generate_err_value(format!("slot data invalid: {}", e));
    }

    if let Err(e) = project.generate(&fs, &project_path, &out_path, &slot_data) {
        return generate_err_value(e.to_string());
    }

    let (files, dirs) = fs.drain_subtree(&out_path);
    GenerateOk {
        ok: true,
        files,
        dirs,
    }
    .serialize(&serializer())
    .unwrap_or(JsValue::NULL)
}
