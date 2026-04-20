//! Pure, target-agnostic implementations of spackle's external binding
//! surface. Shared by `src/wasm.rs` (wasm-bindgen / browser path) and
//! `src/component.rs` (WASI component / server path).
//!
//! All functions take string arguments and return JSON strings. No I/O,
//! no target-specific cfgs, no `#[wasm_bindgen]` attributes.

use std::collections::HashMap;
use std::path::Path;

use crate::{config, hook, slot, template};

/// Parse spackle.toml content into structured JSON.
pub fn parse_config(toml_content: &str) -> String {
    match config::parse(toml_content) {
        Ok(cfg) => serde_json::to_string(&cfg).unwrap_or_else(|e| error_json(&e.to_string())),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Validate a spackle.toml config.
pub fn validate_config(toml_content: &str) -> String {
    let cfg = match config::parse(toml_content) {
        Ok(c) => c,
        Err(e) => return validation_json(false, &[e.to_string()]),
    };

    let mut errors = Vec::new();

    if let Err(e) = cfg.validate() {
        errors.push(e.to_string());
    }
    if let Err(e) = slot::validate(&cfg.slots) {
        errors.push(e.to_string());
    }

    if errors.is_empty() {
        validation_json(true, &[])
    } else {
        validation_json(false, &errors)
    }
}

/// Full check: config structure + slot defaults + template references.
pub fn check_project(toml_content: &str, templates_json: &str) -> String {
    let cfg = match config::parse(toml_content) {
        Ok(c) => c,
        Err(e) => return validation_json(false, &[e.to_string()]),
    };

    let mut errors = Vec::new();

    if let Err(e) = cfg.validate() {
        errors.push(e.to_string());
    }
    if let Err(e) = slot::validate(&cfg.slots) {
        errors.push(e.to_string());
    }

    let entries: Vec<TemplateEntry> = match serde_json::from_str(templates_json) {
        Ok(e) => e,
        Err(e) => {
            errors.push(format!("invalid templates_json: {}", e));
            return validation_json(false, &errors);
        }
    };
    let template_map: HashMap<String, String> = entries
        .iter()
        .map(|e| (e.path.clone(), e.content.clone()))
        .collect();
    if let Err(e) = template::validate_in_memory(&template_map, &cfg.slots) {
        errors.push(e.to_string());
    }

    if errors.is_empty() {
        validation_json(true, &[])
    } else {
        validation_json(false, &errors)
    }
}

/// Validate slot data against a parsed config given as JSON.
pub fn validate_slot_data(config_json: &str, slot_data_json: &str) -> String {
    let cfg: config::Config = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => return error_json(&format!("invalid config_json: {}", e)),
    };
    let data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return error_json(&format!("invalid slot_data_json: {}", e)),
    };

    validate_slot_data_against_config(&cfg, &data)
}

/// Validate slot data against an already-parsed config. Used by the
/// wasip2 component where the config is loaded from disk and we don't
/// need the JSON round-trip.
pub fn validate_slot_data_against_config(
    cfg: &config::Config,
    data: &HashMap<String, String>,
) -> String {
    match slot::validate_data(data, &cfg.slots) {
        Ok(()) => validation_json(true, &[]),
        Err(e) => validation_json(false, &[e.to_string()]),
    }
}

/// Render .j2 templates in memory.
pub fn render_templates(
    templates_json: &str,
    slot_data_json: &str,
    config_json: &str,
) -> String {
    let entries: Vec<TemplateEntry> = match serde_json::from_str(templates_json) {
        Ok(e) => e,
        Err(e) => return error_json(&format!("invalid templates_json: {}", e)),
    };
    let mut data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return error_json(&format!("invalid slot_data_json: {}", e)),
    };
    let cfg: config::Config = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => return error_json(&format!("invalid config_json: {}", e)),
    };

    data.entry("_project_name".to_string())
        .or_insert_with(|| cfg.name.clone().unwrap_or_default());
    data.entry("_output_name".to_string())
        .or_insert_with(|| "output".to_string());

    let template_map: HashMap<String, String> = entries
        .iter()
        .map(|e| (e.path.clone(), e.content.clone()))
        .collect();

    match template::render_in_memory(&template_map, &data) {
        Ok(results) => {
            let rendered: Vec<RenderedEntry> = results
                .into_iter()
                .map(|r| match r {
                    Ok(file) => RenderedEntry {
                        original_path: file.original_path.to_string_lossy().to_string(),
                        rendered_path: file.path.to_string_lossy().to_string(),
                        content: file.contents,
                        error: None,
                    },
                    Err(e) => RenderedEntry {
                        original_path: e.file.clone(),
                        rendered_path: e.file.clone(),
                        content: String::new(),
                        error: Some(e.to_string()),
                    },
                })
                .collect();
            serde_json::to_string(&rendered).unwrap_or_else(|e| error_json(&e.to_string()))
        }
        Err(e) => error_json(&e.to_string()),
    }
}

/// Plan hook execution (pure — no subprocess spawn).
pub fn evaluate_hooks(config_json: &str, slot_data_json: &str) -> String {
    let cfg: config::Config = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => return error_json(&format!("invalid config_json: {}", e)),
    };
    let data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return error_json(&format!("invalid slot_data_json: {}", e)),
    };

    let plan = hook::evaluate_hook_plan(&cfg.hooks, &cfg.slots, &data);
    serde_json::to_string(&plan).unwrap_or_else(|e| error_json(&e.to_string()))
}

/// Output name for a given output directory (mirrors `crate::get_output_name`).
pub fn get_output_name(out_dir: &str) -> String {
    // crate::get_output_name tries canonicalize(); that's fine on both
    // native and WASI and falls back to the provided path. On
    // wasm32-unknown-unknown it panics — but the wasm-pack wrapper uses
    // this via the Path wrapper below and will catch the panic via the
    // panic hook. No behavior change.
    let name = crate::get_output_name(Path::new(out_dir));
    serde_json::to_string(&name).unwrap_or_else(|e| error_json(&e.to_string()))
}

/// Project name for a given config + project directory.
pub fn get_project_name(config_json: &str, project_dir: &str) -> String {
    let cfg: config::Config = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => return error_json(&format!("invalid config_json: {}", e)),
    };
    let name = project_name_from_config(&cfg, project_dir);
    serde_json::to_string(&name).unwrap_or_else(|e| error_json(&e.to_string()))
}

/// Render a single string through tera (used for templated filenames
/// on non-`.j2` files).
pub fn render_string(template: &str, data_json: &str) -> String {
    let data: HashMap<String, String> = match serde_json::from_str(data_json) {
        Ok(d) => d,
        Err(e) => return error_json(&format!("invalid data_json: {}", e)),
    };
    let context = match tera::Context::from_serialize(&data) {
        Ok(c) => c,
        Err(e) => return error_json(&e.to_string()),
    };
    match tera::Tera::one_off(template, &context, false) {
        Ok(s) => serde_json::to_string(&s).unwrap_or_else(|e| error_json(&e.to_string())),
        Err(e) => error_json(&e.to_string()),
    }
}

// --- internal helpers ---

pub(crate) fn project_name_from_config(cfg: &config::Config, project_dir: &str) -> String {
    if let Some(name) = &cfg.name {
        return name.clone();
    }
    Path::new(project_dir)
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

#[derive(serde::Deserialize)]
pub(crate) struct TemplateEntry {
    pub path: String,
    pub content: String,
}

#[derive(serde::Serialize)]
struct RenderedEntry {
    original_path: String,
    rendered_path: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

pub(crate) fn error_json(message: &str) -> String {
    let m = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_string());
    format!(r#"{{"error":{}}}"#, m)
}

pub(crate) fn validation_json(_valid: bool, errors: &[String]) -> String {
    if errors.is_empty() {
        r#"{"valid":true}"#.to_string()
    } else {
        let errs = serde_json::to_string(errors).unwrap_or_else(|_| "[]".to_string());
        format!(r#"{{"valid":false,"errors":{}}}"#, errs)
    }
}
