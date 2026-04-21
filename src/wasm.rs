//! wasm-bindgen exports driving spackle via a host-provided `SpackleFs`.
//!
//! Three exports plus `init`. Each takes a JS `SpackleFs` object and
//! routes all filesystem access through it. Rust owns generation — it
//! reads config, walks templates, renders, and writes via the adapter.
//! The host never pre-reads or post-writes; it only provides the fs
//! backend.
//!
//! Response shapes (JSON strings):
//!
//!   check_with_fs:
//!     success: `{ "valid": true, "config": { name, ignore, slots, hooks }, "errors": [] }`
//!     failure: `{ "valid": false, "errors": ["..."] }`
//!
//!   validate_slot_data_with_fs:
//!     success: `{ "valid": true }`
//!     failure: `{ "valid": false, "errors": ["..."] }`
//!
//!   generate_with_fs:
//!     success: `{ "ok": true, "rendered": [{ original_path, rendered_path }, ...] }`
//!     failure: `{ "ok": false, "error": "..." }`
//!     run_hooks=true: always failure with "hooks are unsupported in this milestone".

use std::collections::HashMap;
use std::path::PathBuf;

use js_sys::Object;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::wasm_fs::{js_fs_from_object, JsFs};

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

// --- response shapes ---

#[derive(Serialize)]
struct CheckOk<'a> {
    valid: bool,
    config: &'a crate::config::Config,
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
    rendered: Vec<RenderedSummary>,
}

#[derive(Serialize)]
struct RenderedSummary {
    original_path: String,
    rendered_path: String,
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

fn generate_err(error: String) -> String {
    json_or_panic(&GenerateErr { ok: false, error })
}

// --- exports ---

/// Validate a project directory: loads spackle.toml, checks slot config,
/// validates template references against slot keys.
#[wasm_bindgen]
pub fn check_with_fs(project_dir: &str, fs_obj: Object) -> String {
    let fs = js_fs_from_object(fs_obj);
    check_impl(&fs, project_dir)
}

fn check_impl(fs: &JsFs, project_dir: &str) -> String {
    let path = PathBuf::from(project_dir);
    let project = match crate::load_project(fs, &path) {
        Ok(p) => p,
        Err(e) => return invalid(vec![e.to_string()]),
    };
    match project.check(fs) {
        Ok(()) => json_or_panic(&CheckOk {
            valid: true,
            config: &project.config,
            errors: vec![],
        }),
        Err(e) => invalid(vec![e.to_string()]),
    }
}

/// Validate slot data against the config loaded from `project_dir`.
#[wasm_bindgen]
pub fn validate_slot_data_with_fs(
    project_dir: &str,
    slot_data_json: &str,
    fs_obj: Object,
) -> String {
    let fs = js_fs_from_object(fs_obj);
    let path = PathBuf::from(project_dir);
    let project = match crate::load_project(&fs, &path) {
        Ok(p) => p,
        Err(e) => return invalid(vec![e.to_string()]),
    };
    let data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return invalid(vec![format!("invalid slot_data_json: {}", e)]),
    };
    match crate::slot::validate_data(&data, &project.config.slots) {
        Ok(()) => r#"{"valid":true}"#.to_string(),
        Err(e) => invalid(vec![e.to_string()]),
    }
}

/// Generate a filled project into `out_dir`.
///
/// `run_hooks = true` is not supported in this milestone — hook
/// execution is deferred until the JsHooks bridge lands. The caller
/// gets an explicit error so the "unsupported" behavior is observable.
#[wasm_bindgen]
pub fn generate_with_fs(
    project_dir: &str,
    out_dir: &str,
    slot_data_json: &str,
    run_hooks: bool,
    fs_obj: Object,
) -> String {
    if run_hooks {
        return generate_err(
            "hooks are unsupported in this milestone; call with run_hooks=false".to_string(),
        );
    }

    let fs = js_fs_from_object(fs_obj);
    let project_path = PathBuf::from(project_dir);
    let out_path = PathBuf::from(out_dir);

    let project = match crate::load_project(&fs, &project_path) {
        Ok(p) => p,
        Err(e) => return generate_err(e.to_string()),
    };

    let slot_data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return generate_err(format!("invalid slot_data_json: {}", e)),
    };

    if let Err(e) = crate::slot::validate_data(&slot_data, &project.config.slots) {
        return generate_err(format!("slot data invalid: {}", e));
    }

    let rendered = match project.generate(&fs, &project_path, &out_path, &slot_data) {
        Ok(files) => files,
        Err(e) => return generate_err(e.to_string()),
    };

    let summary: Vec<RenderedSummary> = rendered
        .iter()
        .map(|f| RenderedSummary {
            original_path: f.original_path.to_string_lossy().to_string(),
            rendered_path: f.path.to_string_lossy().to_string(),
        })
        .collect();

    json_or_panic(&GenerateOk {
        ok: true,
        rendered: summary,
    })
}
