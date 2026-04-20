//! WASI Preview 2 component entry point.
//!
//! Implements the `a2ai:spackle/api` interface exported from
//! `wit/spackle.wit`. Filesystem access uses `std::fs` (available in
//! the WASI sandbox); subprocess execution is delegated to the host
//! via the imported `a2ai:spackle/host::run-command` function.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::bindings::a2ai::spackle::host as wasi_host;
use crate::bindings::exports::a2ai::spackle::api::Guest;
use crate::hook_wasip2::{self, CommandResult, WasipHookResult};
use crate::template::RenderedFile;

struct Component;

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
    hook_results: Vec<WasipHookResult>,
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

impl From<&RenderedFile> for RenderedSummary {
    fn from(f: &RenderedFile) -> Self {
        Self {
            original_path: f.original_path.to_string_lossy().to_string(),
            rendered_path: f.path.to_string_lossy().to_string(),
        }
    }
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

impl Guest for Component {
    fn check(project_dir: String) -> String {
        let path = PathBuf::from(&project_dir);
        let project = match crate::load_project(&path) {
            Ok(p) => p,
            Err(e) => return invalid(vec![e.to_string()]),
        };
        match project.check() {
            Ok(()) => json_or_panic(&CheckOk {
                valid: true,
                config: &project.config,
                errors: vec![],
            }),
            Err(e) => invalid(vec![e.to_string()]),
        }
    }

    fn validate_slot_data(project_dir: String, slot_data_json: String) -> String {
        let path = PathBuf::from(&project_dir);
        let project = match crate::load_project(&path) {
            Ok(p) => p,
            Err(e) => return invalid(vec![e.to_string()]),
        };
        let data: HashMap<String, String> = match serde_json::from_str(&slot_data_json) {
            Ok(d) => d,
            Err(e) => return invalid(vec![format!("invalid slot_data_json: {}", e)]),
        };
        crate::api::validate_slot_data_against_config(&project.config, &data)
    }

    fn generate(
        project_dir: String,
        out_dir: String,
        slot_data_json: String,
        run_hooks: bool,
    ) -> String {
        let project_path = PathBuf::from(&project_dir);
        let out_path = PathBuf::from(&out_dir);

        let project = match crate::load_project(&project_path) {
            Ok(p) => p,
            Err(e) => return generate_err(e.to_string()),
        };

        let slot_data: HashMap<String, String> = match serde_json::from_str(&slot_data_json) {
            Ok(d) => d,
            Err(e) => return generate_err(format!("invalid slot_data_json: {}", e)),
        };

        // Pre-validate: matches wasm-pack PoC behavior (fail fast before
        // touching disk if slot data is wrong).
        if let Err(e) = crate::slot::validate_data(&slot_data, &project.config.slots) {
            return generate_err(format!("slot data invalid: {}", e));
        }

        let rendered = match project.generate(&project_path, &out_path, &slot_data) {
            Ok(files) => files,
            Err(e) => return generate_err(e.to_string()),
        };

        let hook_results = if run_hooks {
            // Build the effective context: user slot data + the special
            // _project_name / _output_name vars matching what
            // Project::generate injected. These feed into hook command
            // templating and conditionals.
            let mut hook_data = slot_data.clone();
            hook_data
                .entry("_project_name".to_string())
                .or_insert_with(|| project.get_name());
            hook_data
                .entry("_output_name".to_string())
                .or_insert_with(|| crate::get_output_name(&out_path));

            hook_wasip2::run_hooks_sync(
                &out_path,
                &project.config.hooks,
                &project.config.slots,
                &hook_data,
                |cmd, args, cwd, env| {
                    let args_owned: Vec<String> = args.to_vec();
                    let env_owned: Vec<(String, String)> = env.to_vec();
                    match wasi_host::run_command(cmd, &args_owned, cwd, &env_owned) {
                        Ok(r) => Ok(CommandResult {
                            stdout: r.stdout,
                            stderr: r.stderr,
                            exit_code: r.exit_code,
                        }),
                        Err(e) => Err(e),
                    }
                },
            )
        } else {
            vec![]
        };

        let rendered_summary: Vec<RenderedSummary> = rendered.iter().map(Into::into).collect();

        json_or_panic(&GenerateOk {
            ok: true,
            rendered: rendered_summary,
            hook_results,
        })
    }
}

crate::bindings::export!(Component with_types_in crate::bindings);
