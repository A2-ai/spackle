//! wasm-bindgen wrappers for spackle's external binding surface.
//!
//! All logic lives in `crate::api` (target-agnostic). This module
//! adds the `#[wasm_bindgen]` attributes and panic-hook setup so the
//! same functions are callable from a JavaScript/TypeScript host.

use wasm_bindgen::prelude::*;

use crate::api;


/// Initialize the WASM module. Sets up the panic hook so panics surface
/// in the host console with useful messages.
#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

#[wasm_bindgen]
pub fn parse_config(toml_content: &str) -> String {
    api::parse_config(toml_content)
}

#[wasm_bindgen]
pub fn validate_config(toml_content: &str) -> String {
    api::validate_config(toml_content)
}

#[wasm_bindgen]
pub fn check_project(toml_content: &str, templates_json: &str) -> String {
    api::check_project(toml_content, templates_json)
}

#[wasm_bindgen]
pub fn validate_slot_data(config_json: &str, slot_data_json: &str) -> String {
    api::validate_slot_data(config_json, slot_data_json)
}

#[wasm_bindgen]
pub fn render_templates(templates_json: &str, slot_data_json: &str, config_json: &str) -> String {
    api::render_templates(templates_json, slot_data_json, config_json)
}

#[wasm_bindgen]
pub fn evaluate_hooks(config_json: &str, slot_data_json: &str) -> String {
    api::evaluate_hooks(config_json, slot_data_json)
}

#[wasm_bindgen]
pub fn get_output_name(out_dir: &str) -> String {
    api::get_output_name(out_dir)
}

#[wasm_bindgen]
pub fn get_project_name(config_json: &str, project_dir: &str) -> String {
    api::get_project_name(config_json, project_dir)
}

#[wasm_bindgen]
pub fn render_string(template: &str, data_json: &str) -> String {
    api::render_string(template, data_json)
}

// Tests exercise the same JSON contracts as before, but call into
// `bindings` directly (no wasm_bindgen round-trip in native tests).
#[cfg(test)]
mod tests {
    use crate::api::{
        check_project, evaluate_hooks, get_output_name, get_project_name, render_string,
    };

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

    #[test]
    fn get_output_name_contract() {
        struct Case {
            input: &'static str,
            expected: &'static str,
        }
        let cases = vec![
            Case { input: "/tmp/my-output", expected: "my-output" },
            Case { input: "my-output", expected: "my-output" },
            Case { input: "a/b/c.name", expected: "c.name" },
            Case { input: "/", expected: "project" },
        ];
        for c in cases {
            let result = get_output_name(c.input);
            let parsed: String = serde_json::from_str(&result)
                .unwrap_or_else(|e| panic!("input {:?}: not a JSON string: {} raw={}", c.input, e, result));
            assert_eq!(parsed, c.expected, "input {:?}", c.input);
        }
    }

    #[test]
    fn get_project_name_contract() {
        let cfg_named = r#"{"name":"from-config","ignore":[],"slots":[],"hooks":[]}"#;
        let result = get_project_name(cfg_named, "/ignored/project-dir");
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "from-config");

        let cfg_nameless = r#"{"name":null,"ignore":[],"slots":[],"hooks":[]}"#;
        let result = get_project_name(cfg_nameless, "/tmp/my-project");
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "my-project");

        let result = get_project_name(cfg_nameless, "/tmp/my.project.git");
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "my.project");

        let result = get_project_name("NOT JSON", "/tmp/x");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed.get("error").is_some(), "bad config should return {{error}}: {}", result);
    }

    #[test]
    fn render_string_contract() {
        let result = render_string("hello {{ name }}", r#"{"name":"world"}"#);
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "hello world");

        let result = render_string("{{ _project_name }}/file", r#"{"_project_name":"proj"}"#);
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "proj/file");

        let result = render_string("plain", "{}");
        let parsed: String = serde_json::from_str(&result).unwrap();
        assert_eq!(parsed, "plain");

        let result = render_string("{{ missing }}", "{}");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed.get("error").is_some(), "missing var should return error: {}", result);

        let result = render_string("x", "NOT JSON");
        let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(parsed.get("error").is_some(), "bad data_json should error: {}", result);
    }

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

        assert_eq!(parsed[0]["key"], "broken");
        assert_eq!(parsed[0]["should_run"], false);
        assert_eq!(parsed[0]["skip_reason"], "template_error");
        let errors = parsed[0]["template_errors"].as_array().unwrap();
        assert!(!errors.is_empty(), "broken hook should have template_errors");

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
