//! WASM bindings for spackle.
//!
//! Exposes spackle's core computation over wasm-bindgen so a TypeScript
//! server (or browser) can call parse_config, validate, render, and
//! evaluate hooks without needing a Rust runtime or filesystem access.
//!
//! All functions take string arguments and return JSON strings —
//! same pattern as nmparser.

use std::collections::HashMap;
use wasm_bindgen::prelude::*;

use crate::{config, hook, slot, template};

/// Initialize the WASM module. Sets up the panic hook so panics surface
/// in the host console with useful messages.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

/// Parse spackle.toml content into structured JSON.
///
/// Input: raw TOML string (the content of spackle.toml)
/// Output: JSON `{ "name": ..., "ignore": [...], "slots": [...], "hooks": [...] }`
///   or on error: `{ "error": "..." }`
#[wasm_bindgen]
pub fn parse_config(toml_content: &str) -> String {
    match config::parse(toml_content) {
        Ok(cfg) => serde_json::to_string(&cfg).unwrap_or_else(|e| error_json(&e.to_string())),
        Err(e) => error_json(&e.to_string()),
    }
}

/// Validate a spackle.toml config: check for duplicate keys, slot type
/// mismatches on defaults, etc.
///
/// Input: raw TOML string
/// Output: JSON `{ "valid": true }` or `{ "valid": false, "errors": ["..."] }`
#[wasm_bindgen]
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

/// Full check: validate config structure + slot defaults + template
/// references against known slot keys.
///
/// Input:
///   - toml_content: raw spackle.toml
///   - templates_json: JSON array `[{ "path": "...", "content": "..." }, ...]`
/// Output: JSON `{ "valid": true }` or `{ "valid": false, "errors": ["..."] }`
#[wasm_bindgen]
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

    // Template validation (matches CLI `spackle check` behavior)
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

/// Validate slot data against a parsed config.
///
/// Input:
///   - config_json: JSON of the parsed config (output of parse_config)
///   - slot_data_json: JSON object `{ "slot_key": "value", ... }`
/// Output: JSON `{ "valid": true }` or `{ "valid": false, "errors": ["..."] }`
#[wasm_bindgen]
pub fn validate_slot_data(config_json: &str, slot_data_json: &str) -> String {
    let cfg: config::Config = match serde_json::from_str(config_json) {
        Ok(c) => c,
        Err(e) => return error_json(&format!("invalid config_json: {}", e)),
    };
    let data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => return error_json(&format!("invalid slot_data_json: {}", e)),
    };

    match slot::validate_data(&data, &cfg.slots) {
        Ok(()) => validation_json(true, &[]),
        Err(e) => validation_json(false, &[e.to_string()]),
    }
}

/// Render .j2 templates in memory.
///
/// Input:
///   - templates_json: JSON array `[{ "path": "README.md.j2", "content": "# {{ name }}" }, ...]`
///   - slot_data_json: JSON object `{ "name": "world", ... }`
///   - config_json: JSON of the parsed config (used for special vars)
/// Output: JSON array `[{ "original_path": "...", "rendered_path": "...", "content": "..." }, ...]`
///   or on error: `{ "error": "..." }`
#[wasm_bindgen]
pub fn render_templates(templates_json: &str, slot_data_json: &str, config_json: &str) -> String {
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

    // Insert special variables
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

/// Evaluate hook execution plan without running any commands.
///
/// Input:
///   - config_json: JSON of the parsed config
///   - slot_data_json: JSON object of slot values
/// Output: JSON array `[{ "key": "...", "command": [...], "should_run": true, "skip_reason": null }, ...]`
#[wasm_bindgen]
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

// --- internal helpers ---

#[derive(serde::Deserialize)]
struct TemplateEntry {
    path: String,
    content: String,
}

#[derive(serde::Serialize)]
struct RenderedEntry {
    original_path: String,
    rendered_path: String,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn error_json(message: &str) -> String {
    let m = serde_json::to_string(message).unwrap_or_else(|_| "\"\"".to_string());
    format!(r#"{{"error":{}}}"#, m)
}

fn validation_json(valid: bool, errors: &[String]) -> String {
    if errors.is_empty() {
        r#"{"valid":true}"#.to_string()
    } else {
        let errs = serde_json::to_string(errors).unwrap_or_else(|_| "[]".to_string());
        format!(r#"{{"valid":false,"errors":{}}}"#, errs)
    }
}

// The wasm module's functions use wasm_bindgen and can't be called directly
// in native tests. These tests exercise the same code paths by calling the
// underlying functions and verifying the JSON contracts.
#[cfg(test)]
mod tests {
    use super::*;

    /// check_project must ALWAYS return { valid, errors } — never
    /// { error } — regardless of which input is malformed.
    #[test]
    fn check_project_response_shape_table() {
        struct Case {
            name: &'static str,
            toml: &'static str,
            templates_json: &'static str,
            expect_valid: bool,
            errors_contain: Option<&'static str>,
        }

        let cases = vec![
            Case {
                name: "valid config + templates",
                toml: "[[slots]]\nkey = \"x\"\n",
                templates_json: r#"[{"path":"t.j2","content":"{{ x }}"}]"#,
                expect_valid: true,
                errors_contain: None,
            },
            Case {
                name: "template references undefined slot",
                toml: "[[slots]]\nkey = \"x\"\n",
                templates_json: r#"[{"path":"t.j2","content":"{{ missing }}"}]"#,
                expect_valid: false,
                errors_contain: Some("rendering"),
            },
            Case {
                name: "invalid toml",
                toml: "[[[ broken",
                templates_json: "[]",
                expect_valid: false,
                errors_contain: Some("parsing"),
            },
            Case {
                name: "invalid templates_json (bad JSON)",
                toml: "",
                templates_json: "NOT JSON",
                expect_valid: false,
                errors_contain: Some("invalid templates_json"),
            },
            Case {
                name: "duplicate keys in config",
                toml: "[[slots]]\nkey = \"x\"\n[[hooks]]\nkey = \"x\"\ncommand = [\"echo\"]\n",
                templates_json: "[]",
                expect_valid: false,
                errors_contain: Some("Duplicate"),
            },
        ];

        for c in cases {
            let result = check_project(c.toml, c.templates_json);
            let parsed: serde_json::Value = serde_json::from_str(&result).unwrap_or_else(|e| {
                panic!("case {}: result is not valid JSON: {} — raw: {}", c.name, e, result)
            });

            // Shape contract: ALWAYS { valid: bool, ... } — never { error: ... }
            assert!(
                parsed.get("valid").is_some(),
                "case {}: response must have 'valid' key, got: {}",
                c.name,
                result,
            );
            assert!(
                parsed.get("error").is_none(),
                "case {}: response must NOT have 'error' key (use 'valid'+errors), got: {}",
                c.name,
                result,
            );

            let valid = parsed["valid"].as_bool().unwrap();
            assert_eq!(valid, c.expect_valid, "case {}", c.name);

            if let Some(needle) = c.errors_contain {
                let errors = parsed["errors"]
                    .as_array()
                    .expect(&format!("case {}: errors should be an array", c.name));
                let joined = errors
                    .iter()
                    .map(|e| e.as_str().unwrap_or(""))
                    .collect::<Vec<_>>()
                    .join(" ");
                assert!(
                    joined.to_lowercase().contains(&needle.to_lowercase()),
                    "case {}: errors should contain {:?}, got: {}",
                    c.name,
                    needle,
                    joined,
                );
            }
        }
    }

    /// evaluate_hooks: verify template_errors are surfaced and block
    /// downstream hook_ran state. This covers the WASM JSON contract
    /// end-to-end (except the wasm_bindgen layer which is a pass-through).
    #[test]
    fn evaluate_hooks_template_errors_contract() {
        let config_json = r#"{
            "slots": [],
            "hooks": [
                { "key": "broken", "command": ["echo", "{{ undefined }}"], "default": true, "needs": [] },
                { "key": "after", "command": ["echo", "ok"], "if": "{{ hook_ran_broken }}", "default": true, "needs": [] }
            ],
            "ignore": []
        }"#;
        let slot_data = "{}";

        let result = evaluate_hooks(config_json, slot_data);
        let parsed: Vec<serde_json::Value> = serde_json::from_str(&result)
            .expect("evaluate_hooks should return valid JSON array");

        assert_eq!(parsed.len(), 2);

        // First hook: template error → should_run=false
        assert_eq!(parsed[0]["key"], "broken");
        assert_eq!(parsed[0]["should_run"], false);
        assert_eq!(parsed[0]["skip_reason"], "template_error");
        let errors = parsed[0]["template_errors"].as_array().unwrap();
        assert!(!errors.is_empty(), "broken hook should have template_errors");

        // Second hook: hook_ran_broken never flipped → false_conditional
        assert_eq!(parsed[1]["key"], "after");
        assert_eq!(parsed[1]["should_run"], false);
        let reason = parsed[1]["skip_reason"].as_str().unwrap();
        assert!(
            reason.contains("false_conditional"),
            "after hook should be false_conditional, got: {}",
            reason
        );
    }
}
