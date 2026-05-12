//! wasm-bindgen exports for spackle, bundle-in / bundle-out.
//!
//! Each export takes a project bundle — a JS `Array<{path, bytes: Uint8Array}>`
//! — hydrates it into a [`MemoryFs`], runs the requested operation, and
//! returns a serialized result. Rust never touches the host's filesystem;
//! the TS host does all disk I/O and subprocess execution.
//!
//! Per-export shapes and contracts live on the function `///` docs below.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use spackle::NameOverrides;
use wasm_bindgen::prelude::*;

pub mod memory_fs;

use memory_fs::{BundleEntry, MemoryFs};

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

// --- response shapes ---

/// Structured response from `check`. Always returns this shape — no
/// `valid: true/false` discriminant. Empty `diagnostics` ⇒ clean check.
/// `config` is `None` when the TOML couldn't be parsed (the diagnostics
/// list will contain a `config`-source entry explaining why).
#[derive(Serialize)]
struct CheckResponse<'a> {
    config: Option<&'a spackle::config::Config>,
    diagnostics: &'a Vec<spackle::Diagnostic>,
}

/// Structured response from `render`. Always returns this shape. Partial
/// preview semantics: `files` contains every template that rendered
/// successfully; `diagnostics` enumerates every failure across all stages
/// (config / slot / hook / copy / render). `hook_plan` is `null` only
/// when the config failed to load.
#[derive(Serialize)]
struct RenderResponse<'a> {
    files: Vec<BundleEntry>,
    dirs: Vec<String>,
    diagnostics: &'a Vec<spackle::Diagnostic>,
    #[serde(rename = "hookPlan")]
    hook_plan: Option<&'a Vec<spackle::hook::HookPlanEntry>>,
}

/// Legacy `validate_slot_data` response. Kept for granular use.
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

#[derive(Serialize)]
struct PlanHooksOk<'a> {
    ok: bool,
    plan: &'a Vec<spackle::hook::HookPlanEntry>,
}

#[derive(Serialize)]
struct PlanHooksErr {
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

/// Fixed virtual-fs anchors. Pinned in the crate (not exposed to the
/// host) because their values are pure implementation detail of the
/// in-memory pipeline: they're the keys files live under inside the
/// MemoryFs, the prefix `drain_subtree` extracts from, and the base for
/// the relative paths in the returned output bundle. The host's TS
/// wrapper is responsible for producing bundles rooted here (see
/// `DiskFs.readProject`) and for prepending its own real disk path
/// onto the relative output entries on the way back out.
pub(crate) const PROJECT_DIR: &str = "/project";
pub(crate) const OUT_DIR: &str = "/output";

/// Verify every entry's `path` lives under [`PROJECT_DIR`]. Catches
/// hosts that hand in a malformed bundle (paths rooted at the wrong
/// prefix); historically that surfaced deep in the renderer as a
/// confusing "no such file" error. Allows the entry path to equal
/// the constant (single-file project) or to be a descendant.
fn check_bundle_root(entries: &[BundleEntry]) -> Result<(), String> {
    if entries.is_empty() {
        return Ok(());
    }
    let prefix_slash = format!("{}/", PROJECT_DIR);
    for entry in entries {
        let path = entry.path.as_str();
        if path == PROJECT_DIR || path.starts_with(&prefix_slash) {
            continue;
        }
        return Err(format!(
            "bundle entry '{}' is not under '{}'; every bundle path must \
             equal '{}' or be a descendant of it (DiskFs.readProject and the \
             memory-fs helpers root entries here automatically)",
            entry.path, PROJECT_DIR, PROJECT_DIR
        ));
    }
    Ok(())
}

fn name_overrides<'a>(
    project_name: Option<&'a str>,
    output_name: Option<&'a str>,
) -> NameOverrides<'a> {
    NameOverrides {
        project_name,
        output_name,
    }
}

// --- exports ---

/// Run every static project check against `project_bundle`. Always
/// returns a structured `CheckResponse` (never throws / never returns
/// `valid: false`). Diagnostics are accumulated across all stages —
/// callers see every problem in one call.
#[wasm_bindgen]
pub fn check(project_bundle: JsValue) -> String {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => {
            let diags = vec![spackle::Diagnostic::new(
                spackle::Severity::Error,
                spackle::DiagnosticSource::Config,
                msg,
            )];
            return json_or_panic(&CheckResponse {
                config: None,
                diagnostics: &diags,
            });
        }
    };
    if let Err(msg) = check_bundle_root(&entries) {
        let diags = vec![spackle::Diagnostic::new(
            spackle::Severity::Error,
            spackle::DiagnosticSource::Config,
            msg,
        )];
        return json_or_panic(&CheckResponse {
            config: None,
            diagnostics: &diags,
        });
    }
    let fs = MemoryFs::from_bundle(entries);
    let path = PathBuf::from(PROJECT_DIR);
    let report = spackle::check_project(&fs, &path);
    json_or_panic(&CheckResponse {
        config: report.config.as_ref(),
        diagnostics: &report.diagnostics,
    })
}

/// Validate slot data against the config loaded from the project bundle.
#[wasm_bindgen]
pub fn validate_slot_data(project_bundle: JsValue, slot_data_json: &str) -> String {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return invalid(vec![msg]),
    };
    if let Err(msg) = check_bundle_root(&entries) {
        return invalid(vec![msg]);
    }
    let fs = MemoryFs::from_bundle(entries);
    let path = PathBuf::from(PROJECT_DIR);
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
/// as a flat bundle with paths relative to the fixed virtual out dir.
///
/// `project_name` / `output_name` set the `_project_name` /
/// `_output_name` Tera vars. Pass `None` to fall back to the default
/// (`config.name` → basename of the fixed virtual project dir for the
/// former; basename of the fixed virtual out dir for the latter); the
/// TS wrapper forwards `basename(realOutDir)` for disk-backed callers so
/// they see the same defaults they always did.
///
/// Hooks are a separate step — mirror the native CLI's two-call shape
/// (`project.generate(...)` then `project.run_hooks_stream(...)`). Call
/// `plan_hooks` after `generate` to get the resolved hook plan, then
/// execute host-side. See `ts/src/host/hooks.ts` for the reference runner.
#[wasm_bindgen]
pub fn generate(
    project_bundle: JsValue,
    slot_data_json: &str,
    project_name: Option<String>,
    output_name: Option<String>,
) -> JsValue {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return generate_err_value(msg),
    };
    if let Err(msg) = check_bundle_root(&entries) {
        return generate_err_value(msg);
    }
    let fs = MemoryFs::from_bundle(entries);
    let project_path = PathBuf::from(PROJECT_DIR);
    let out_path = PathBuf::from(OUT_DIR);

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

    let names = name_overrides(project_name.as_deref(), output_name.as_deref());
    if let Err(e) = project.generate(&fs, &project_path, &out_path, &slot_data, names) {
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

/// Diagnostics-first render: never throws, never returns `ok: false`.
/// Empty `diagnostics` ⇒ clean render. Use this for live UI feedback;
/// `generate` keeps fail-fast semantics for write-to-disk workflows.
///
/// `project_name` / `output_name` work the same way as in [`generate`].
#[wasm_bindgen]
pub fn render(
    project_bundle: JsValue,
    slot_data_json: &str,
    project_name: Option<String>,
    output_name: Option<String>,
) -> JsValue {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => {
            let diags = vec![spackle::Diagnostic::new(
                spackle::Severity::Error,
                spackle::DiagnosticSource::Config,
                msg,
            )];
            return RenderResponse {
                files: Vec::new(),
                dirs: Vec::new(),
                diagnostics: &diags,
                hook_plan: None,
            }
            .serialize(&serializer())
            .unwrap_or(JsValue::NULL);
        }
    };
    if let Err(msg) = check_bundle_root(&entries) {
        let diags = vec![spackle::Diagnostic::new(
            spackle::Severity::Error,
            spackle::DiagnosticSource::Config,
            msg,
        )];
        return RenderResponse {
            files: Vec::new(),
            dirs: Vec::new(),
            diagnostics: &diags,
            hook_plan: None,
        }
        .serialize(&serializer())
        .unwrap_or(JsValue::NULL);
    }
    let fs = MemoryFs::from_bundle(entries);
    let project_path = PathBuf::from(PROJECT_DIR);
    let out_path = PathBuf::from(OUT_DIR);

    let slot_data: HashMap<String, String> = match serde_json::from_str(slot_data_json) {
        Ok(d) => d,
        Err(e) => {
            let diags = vec![spackle::Diagnostic::new(
                spackle::Severity::Error,
                spackle::DiagnosticSource::SlotData,
                format!("invalid slot_data_json: {}", e),
            )];
            return RenderResponse {
                files: Vec::new(),
                dirs: Vec::new(),
                diagnostics: &diags,
                hook_plan: None,
            }
            .serialize(&serializer())
            .unwrap_or(JsValue::NULL);
        }
    };

    let names = name_overrides(project_name.as_deref(), output_name.as_deref());
    let report = spackle::render(&fs, &project_path, &out_path, &slot_data, names);

    // Harvest the rendered output subtree from the MemoryFs (paths
    // relative to out_dir). Mirrors what `generate` does.
    let (files, dirs) = fs.drain_subtree(&out_path);

    RenderResponse {
        files,
        dirs,
        diagnostics: &report.diagnostics,
        hook_plan: report.hook_plan.as_ref(),
    }
    .serialize(&serializer())
    .unwrap_or(JsValue::NULL)
}

/// Evaluate a hook plan for the project. Pure — no subprocess spawning,
/// no fs writes. Returns the ordered list of hooks with their templated
/// commands, should-run flag, and skip/template-error diagnostics.
///
/// Mirrors `Project::run_hooks_stream` at `src/lib.rs:246` in input
/// shape: `data` is the full data map (slot values + hook toggles keyed
/// by the hook's own `key`, e.g. `data["my_hook"] = "false"` disables
/// it). `_project_name` and `_output_name` are injected here to match
/// native — the caller does NOT pre-inject them.
///
/// `project_name` / `output_name` mirror the equivalents on
/// [`generate`] / [`render`]: if set, they win over the basenames of
/// the fixed virtual dirs. Hosts running hooks during a
/// preview-to-temp flow should pass the same values they used for
/// `generate`/`render` so templated hook commands see the same
/// `_output_name` the rendered files did. Disk-backed callers will
/// usually pass `basename(realOutDir)` for `output_name` via the TS
/// wrapper so the templated `{{ _output_name }}` matches the real
/// write target.
///
/// `hook_ran_json` (optional): JSON of `HashMap<String, bool>` where
/// keys are hook keys and values are actual execution results. The host
/// passes this back on re-plan after a hook succeeds/fails so chained
/// conditionals (`if = "{{ hook_ran_X }}"`) evaluate against reality
/// instead of the best-case default. Without it, `evaluate_hook_plan`
/// pre-populates `hook_ran_*` with "false" (via `or_insert_with`) and
/// flips entries to "true" as it walks the plan under the best-case
/// assumption that prior hooks succeed.
///
/// Each call re-parses the bundle and rebuilds MemoryFs. Fine at current
/// scale — parse is sub-millisecond, dwarfed by subprocess spawn time.
/// If profiles ever show per-call parse dominating, or if interactive
/// multi-generation hosts land, pivot to a stateful `Session` handle:
///   open_session(bundle) -> SessionId
///   plan_hooks_session(session_id, data, hook_ran) -> HookPlan
///   close_session(session_id)
/// That amortizes parse across the plan-execute loop. Deferred.
#[wasm_bindgen]
pub fn plan_hooks(
    project_bundle: JsValue,
    data_json: &str,
    hook_ran_json: Option<String>,
    project_name: Option<String>,
    output_name: Option<String>,
) -> String {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return plan_hooks_err(msg),
    };
    if let Err(msg) = check_bundle_root(&entries) {
        return plan_hooks_err(msg);
    }
    plan_hooks_from_entries(
        entries,
        data_json,
        hook_ran_json.as_deref(),
        project_name.as_deref(),
        output_name.as_deref(),
    )
}

/// Pure-Rust implementation of `plan_hooks`. Split out so native tests
/// can exercise the logic without going through `JsValue`.
fn plan_hooks_from_entries(
    entries: Vec<BundleEntry>,
    data_json: &str,
    hook_ran_json: Option<&str>,
    project_name: Option<&str>,
    output_name: Option<&str>,
) -> String {
    let fs = MemoryFs::from_bundle(entries);
    let project_path = PathBuf::from(PROJECT_DIR);
    let project = match spackle::load_project(&fs, &project_path) {
        Ok(p) => p,
        Err(e) => return plan_hooks_err(e.to_string()),
    };

    let mut data: HashMap<String, String> = match serde_json::from_str(data_json) {
        Ok(d) => d,
        Err(e) => return plan_hooks_err(format!("invalid data_json: {}", e)),
    };

    // Parity with Project::run_hooks_stream at src/lib.rs:253-254:
    // inject the resolved `_project_name` + `_output_name` so hooks
    // templated with `{{ _output_name }}` render correctly. Honor the
    // override knobs the caller passed in; otherwise fall back to the
    // basename of the fixed virtual out dir constant.
    data.insert(
        "_project_name".to_string(),
        project.resolved_project_name(project_name),
    );
    data.insert(
        "_output_name".to_string(),
        output_name
            .map(str::to_owned)
            .unwrap_or_else(|| spackle::get_output_name(Path::new(OUT_DIR))),
    );

    // Merge caller-supplied hook_ran_* overrides. Keys in hook_ran_json
    // are hook keys with boolean values indicating actual execution
    // outcome. We pre-seed `data["hook_ran_<key>"] = "true"/"false"`
    // per the actual result so subsequent hooks' `if = "{{ hook_ran_X }}"`
    // conditionals evaluate against reality.
    //
    // The executed hooks themselves are marked `excluded` below, which
    // skips them from the planner's iteration while keeping them in
    // the items set (so dependent hooks' `needs` resolution still
    // finds them). Skipping iteration prevents the planner from
    // overwriting our hook_ran_<key> seed on its success branch.
    let mut executed_keys: std::collections::HashSet<String> = std::collections::HashSet::new();
    if let Some(raw) = hook_ran_json {
        let hook_ran: HashMap<String, bool> = match serde_json::from_str(raw) {
            Ok(d) => d,
            Err(e) => return plan_hooks_err(format!("invalid hook_ran_json: {}", e)),
        };
        for (key, ran) in hook_ran {
            data.insert(
                format!("hook_ran_{}", key),
                if ran { "true" } else { "false" }.to_string(),
            );
            executed_keys.insert(key);
        }
    }

    let full_plan = plan_hooks_native_parity(
        &project.config.hooks,
        &project.config.slots,
        &data,
        &executed_keys,
    );

    // Strip executed-hook entries from the returned plan — host has
    // their results already.
    let plan: Vec<spackle::hook::HookPlanEntry> = full_plan
        .into_iter()
        .filter(|e| !executed_keys.contains(&e.key))
        .collect();

    json_or_panic(&PlanHooksOk {
        ok: true,
        plan: &plan,
    })
}

/// Planner with native `run_hooks_stream` ordering: is_enabled →
/// is_satisfied → **template first** → evaluate conditional. This
/// differs from `spackle::hook::evaluate_hook_plan` (which evaluates
/// the conditional before templating) so that template errors in
/// hooks with false-conditional `if`s still surface — matching native
/// `Error::ErrorRenderingTemplate` at `src/hook.rs:415-425` which
/// runs BEFORE conditional evaluation inside the stream.
///
/// Also tracks `hook_ran_<key>` forward-propagation on the
/// success-path only, identical to
/// `spackle::hook::evaluate_hook_plan` for that dimension.
///
/// `excluded` — hooks whose key is in this set are skipped from
/// iteration (not planned, not pushed to results) but REMAIN in the
/// `items` set used for needs resolution. Used by `plan_hooks` to
/// honor `hook_ran_json` overrides: the caller has already executed
/// these hooks, so we shouldn't re-plan them, but dependent hooks
/// still need to find them during `is_satisfied` lookups. Caller
/// pre-seeds `hook_ran_<key>` in `data` with the actual outcome.
fn plan_hooks_native_parity(
    hooks: &[spackle::hook::Hook],
    slots: &[spackle::slot::Slot],
    data: &HashMap<String, String>,
    excluded: &std::collections::HashSet<String>,
) -> Vec<spackle::hook::HookPlanEntry> {
    use spackle::hook::HookPlanEntry;
    use spackle::needs::Needy;
    use tera::{Context, Tera};

    let items: Vec<&dyn Needy> = {
        let mut v: Vec<&dyn Needy> = slots.iter().map(|s| s as &dyn Needy).collect();
        v.extend(hooks.iter().map(|h| h as &dyn Needy));
        v
    };

    let mut running = data.clone();
    for h in hooks {
        running
            .entry(format!("hook_ran_{}", h.key))
            .or_insert_with(|| "false".to_string());
    }

    let mut results = Vec::with_capacity(hooks.len());

    for hook in hooks {
        if excluded.contains(&hook.key) {
            // Already executed; host has the outcome, caller pre-seeded
            // hook_ran_<key>. Skip without flipping anything.
            continue;
        }
        if !hook.is_enabled(&running) {
            results.push(HookPlanEntry {
                key: hook.key.clone(),
                command: hook.command.clone(),
                should_run: false,
                skip_reason: Some("user_disabled".to_string()),
                template_errors: vec![],
            });
            continue;
        }

        if !hook.is_satisfied(&items, &running) {
            results.push(HookPlanEntry {
                key: hook.key.clone(),
                command: hook.command.clone(),
                should_run: false,
                skip_reason: Some("unsatisfied_needs".to_string()),
                template_errors: vec![],
            });
            continue;
        }

        let context = match Context::from_serialize(&running) {
            Ok(c) => c,
            Err(e) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: hook.command.clone(),
                    should_run: false,
                    skip_reason: Some("template_error".to_string()),
                    template_errors: vec![format!("context error: {}", e)],
                });
                continue;
            }
        };

        let mut template_errors = Vec::new();
        let templated_command: Vec<String> = hook
            .command
            .iter()
            .map(|arg| match Tera::one_off(arg, &context, false) {
                Ok(rendered) => rendered,
                Err(e) => {
                    template_errors.push(format!("arg {:?}: {}", arg, e));
                    arg.clone()
                }
            })
            .collect();

        if !template_errors.is_empty() {
            results.push(HookPlanEntry {
                key: hook.key.clone(),
                command: templated_command,
                should_run: false,
                skip_reason: Some("template_error".to_string()),
                template_errors,
            });
            continue;
        }

        // Conditional evaluation runs AFTER templating — native parity.
        // A hook whose command templates cleanly but whose `if` is false
        // is a legitimate skip. A hook whose command template breaks is
        // a hard error regardless of conditional.
        match hook.evaluate_conditional(&running) {
            Ok(false) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: templated_command,
                    should_run: false,
                    skip_reason: Some("false_conditional".to_string()),
                    template_errors: vec![],
                });
                continue;
            }
            Err(e) => {
                results.push(HookPlanEntry {
                    key: hook.key.clone(),
                    command: templated_command,
                    should_run: false,
                    skip_reason: Some(format!("conditional_error: {}", e)),
                    template_errors: vec![],
                });
                continue;
            }
            Ok(true) => {}
        }

        results.push(HookPlanEntry {
            key: hook.key.clone(),
            command: templated_command,
            should_run: true,
            skip_reason: None,
            template_errors: vec![],
        });

        running.insert(format!("hook_ran_{}", hook.key), "true".to_string());
    }

    results
}

fn plan_hooks_err(msg: String) -> String {
    json_or_panic(&PlanHooksErr {
        ok: false,
        error: msg,
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod plan_hooks_tests {
    use super::*;
    use serde_json::Value;

    const FIXTURE_TOML: &[u8] =
        include_bytes!("../../../tests/fixtures/hooks_fixture/spackle.toml");

    fn fixture_bundle() -> Vec<BundleEntry> {
        vec![BundleEntry {
            path: "/project/spackle.toml".to_string(),
            bytes: FIXTURE_TOML.to_vec(),
        }]
    }

    /// Parse the response and assert `ok: true`, returning the `plan`
    /// array as a serde_json::Value. `HookPlanEntry` doesn't derive
    /// Deserialize (it's Serialize-only in core); parsing via Value
    /// keeps the test self-contained without touching core.
    fn call(data_json: &str, hook_ran: Option<&str>) -> Value {
        // Disk-backed CLI/TS callers would normally feed
        // basename(realOutDir) here; for these planner-level tests we
        // pass an explicit output_name so the templated _output_name
        // matches the historical "my_output" expectation.
        let raw = plan_hooks_from_entries(
            fixture_bundle(),
            data_json,
            hook_ran,
            None,
            Some("my_output"),
        );
        let v: Value = serde_json::from_str(&raw).expect("plan_hooks returned invalid JSON");
        assert_eq!(v["ok"], true, "unexpected err response: {}", raw);
        v["plan"].clone()
    }

    fn key_of(entry: &Value) -> &str {
        entry["key"].as_str().unwrap()
    }

    fn find<'a>(plan: &'a Value, key: &str) -> &'a Value {
        plan.as_array()
            .unwrap()
            .iter()
            .find(|e| key_of(e) == key)
            .unwrap_or_else(|| panic!("hook '{}' not in plan: {}", key, plan))
    }

    #[test]
    fn best_case_plan_marks_chained_hook_runnable() {
        let plan = call("{}", None);
        let arr = plan.as_array().unwrap();
        let keys: Vec<&str> = arr.iter().map(key_of).collect();
        assert_eq!(keys, vec!["hook_a", "hook_b", "hook_names"]);

        for entry in arr {
            assert_eq!(entry["should_run"], true, "entry not runnable: {}", entry);
            // template_errors is skip-if-empty → absent on a clean plan.
            assert!(entry.get("template_errors").is_none());
        }
    }

    #[test]
    fn hook_ran_override_demotes_chained_hook() {
        // Tell the planner that hook_a actually did NOT run (e.g. it
        // failed at execution time). hook_b's conditional
        // `{{ hook_ran_hook_a }}` should now evaluate false.
        let plan = call("{}", Some(r#"{"hook_a": false}"#));
        let hook_b = find(&plan, "hook_b");
        assert_eq!(hook_b["should_run"], false);
        assert_eq!(hook_b["skip_reason"], "false_conditional");
    }

    #[test]
    fn project_and_output_names_injected() {
        // hook_names: printf '%s/%s' '{{ _project_name }}' '{{ _output_name }}'.
        // _project_name comes from `name = "hooks-demo"` in spackle.toml.
        // _output_name from get_output_name("/tmp/my_output") = "my_output".
        let plan = call("{}", None);
        let names = find(&plan, "hook_names");
        let body = names["command"][2].as_str().unwrap();
        assert!(body.contains("hooks-demo"), "got: {}", body);
        assert!(body.contains("my_output"), "got: {}", body);
    }

    #[test]
    fn invalid_data_json_returns_err_shape() {
        let raw = plan_hooks_from_entries(fixture_bundle(), "not json", None, None, None);
        let v: Value = serde_json::from_str(&raw).unwrap();
        assert_eq!(v["ok"], false);
        assert!(
            v["error"].as_str().unwrap().contains("invalid data_json"),
            "got: {}",
            v["error"]
        );
    }

    #[test]
    fn hook_disabled_by_raw_key_in_data() {
        // Native Hook::is_enabled at src/hook.rs:79-85 checks
        // data[&self.key] — raw hook key, NOT "hook_<key>".
        let plan = call(r#"{"hook_a": "false"}"#, None);
        let hook_a = find(&plan, "hook_a");
        assert_eq!(hook_a["should_run"], false);
        assert_eq!(hook_a["skip_reason"], "user_disabled");
    }

    // Dedicated fixture with a `needs` chain + a `if = "false"` hook
    // whose command contains a broken template. Inlined here so this
    // test is self-contained (fixture-based ones cover the rest).
    fn needs_fixture_bundle() -> Vec<BundleEntry> {
        let toml = br#"
[[hooks]]
key = "hook_a"
command = ["true"]
default = true

[[hooks]]
key = "hook_c"
command = ["true"]
needs = ["hook_a"]
default = true
"#;
        vec![BundleEntry {
            path: "/project/spackle.toml".to_string(),
            bytes: toml.to_vec(),
        }]
    }

    fn call_with_bundle(
        entries: Vec<BundleEntry>,
        data_json: &str,
        hook_ran: Option<&str>,
    ) -> serde_json::Value {
        let raw = plan_hooks_from_entries(entries, data_json, hook_ran, None, None);
        let v: serde_json::Value = serde_json::from_str(&raw).expect("invalid JSON");
        assert_eq!(v["ok"], true, "unexpected err response: {}", raw);
        v["plan"].clone()
    }

    #[test]
    fn executed_hook_stays_in_items_for_needs_resolution() {
        // hook_c needs hook_a. hook_a has been executed (ran=true). The
        // remaining plan must still mark hook_c as satisfied — dropping
        // hook_a from the items set would wrongly demote hook_c to
        // unsatisfied_needs.
        let plan = call_with_bundle(needs_fixture_bundle(), "{}", Some(r#"{"hook_a": true}"#));
        let hook_c = plan
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == "hook_c")
            .unwrap_or_else(|| panic!("hook_c missing from plan: {}", plan));
        assert_eq!(
            hook_c["should_run"], true,
            "hook_c must remain satisfied when hook_a is already executed; got {}",
            hook_c
        );
        // hook_a was executed — should be stripped from the returned plan.
        assert!(
            plan.as_array()
                .unwrap()
                .iter()
                .all(|e| e["key"] != "hook_a"),
            "hook_a should not appear in the remaining plan: {}",
            plan
        );
    }

    #[test]
    fn tera_builtin_filters_available_to_hook_command_templating() {
        // Parity check: spackle core and spackle-wasm must resolve to
        // the same tera version/features, so native hook command
        // templating and the wasm-side planner render identically. A
        // misaligned `default-features = false` (or a feature drift) on
        // `tera` in spackle-wasm's Cargo.toml would strip builtin
        // filters and diverge from native behavior. This test forces a
        // builtin filter through the plan_hooks_native_parity pipeline
        // and asserts it rendered (no template_errors). Uses `upper`,
        // which tera 2 registers unconditionally in `Tera::default()`.
        let toml = br#"
[[hooks]]
key = "with_filter"
command = ["echo", "{{ raw | upper }}"]
default = true
"#;
        let entries = vec![BundleEntry {
            path: "/project/spackle.toml".to_string(),
            bytes: toml.to_vec(),
        }];
        let plan = call_with_bundle(entries, r#"{"raw": "hello world"}"#, None);
        let h = plan
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == "with_filter")
            .unwrap();
        // Clean plan — builtin filter rendered without a template_error.
        assert_eq!(h["should_run"], true, "got: {}", h);
        assert!(h.get("template_errors").is_none(), "got: {}", h);
        // upper("hello world") = "HELLO WORLD" — asserts the filter
        // actually ran, not just that the raw template passed through.
        assert_eq!(h["command"][1], "HELLO WORLD", "got: {}", h);
    }

    #[test]
    fn template_error_surfaces_even_when_conditional_is_false() {
        // Native run_hooks_stream templates all queued_hooks at
        // src/hook.rs:412-425 BEFORE evaluating the `if` conditional.
        // A hook whose command has an unresolved template AND whose `if`
        // evaluates false is therefore a hard error natively, not a
        // silent skip. Our native-parity planner must match.
        let toml = br#"
[[hooks]]
key = "masked"
command = ["echo", "{{ definitely_undefined }}"]
"if" = "false"
default = true
"#;
        let entries = vec![BundleEntry {
            path: "/project/spackle.toml".to_string(),
            bytes: toml.to_vec(),
        }];
        let plan = call_with_bundle(entries, "{}", None);
        let masked = plan
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == "masked")
            .unwrap();
        assert_eq!(masked["should_run"], false);
        assert_eq!(masked["skip_reason"], "template_error");
        assert!(
            masked["template_errors"].is_array(),
            "template_errors array missing: {}",
            masked
        );
    }

    #[test]
    fn name_overrides_flow_into_hook_templating() {
        // hook_names templates {{ _project_name }}/{{ _output_name }}.
        // Override both — neither the config `name` ("hooks-demo") nor
        // the fallback "output" (basename of the virtual out dir
        // constant) should show up in the rendered command.
        let raw = plan_hooks_from_entries(
            fixture_bundle(),
            "{}",
            None,
            Some("custom-proj"),
            Some("custom-out"),
        );
        let v: Value = serde_json::from_str(&raw).expect("invalid JSON");
        assert_eq!(v["ok"], true, "got: {}", raw);
        let names = v["plan"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == "hook_names")
            .unwrap();
        let body = names["command"][2].as_str().unwrap();
        assert!(body.contains("custom-proj"), "got: {}", body);
        assert!(body.contains("custom-out"), "got: {}", body);
        assert!(!body.contains("hooks-demo"), "default leaked: {}", body);
        assert!(!body.contains("output"), "default leaked: {}", body);
    }

    #[test]
    fn output_name_falls_back_to_virtual_dir_basename() {
        // No output_name override → planner uses basename(OUT_DIR)
        // which is "output". Hosts that want a meaningful name pass
        // basename(realOutDir) via the TS wrapper.
        let raw = plan_hooks_from_entries(fixture_bundle(), "{}", None, None, None);
        let v: Value = serde_json::from_str(&raw).expect("invalid JSON");
        let names = v["plan"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["key"] == "hook_names")
            .unwrap();
        let body = names["command"][2].as_str().unwrap();
        // Project name still falls back to config.name = "hooks-demo".
        assert!(body.contains("hooks-demo"), "got: {}", body);
        // Output name fell back to the constant basename.
        assert!(body.contains("output"), "got: {}", body);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod bundle_root_tests {
    use super::*;

    #[test]
    fn drift_off_project_constant_is_rejected() {
        let entries = vec![BundleEntry {
            path: "/elsewhere/spackle.toml".to_string(),
            bytes: b"".to_vec(),
        }];
        let err = check_bundle_root(&entries).unwrap_err();
        assert!(err.contains(PROJECT_DIR), "unexpected error: {}", err);
    }

    #[test]
    fn entry_equal_to_project_constant_allowed_for_single_file_project() {
        // Single-file template projects come in as one entry whose path
        // is exactly PROJECT_DIR (no extension on the virtual side —
        // the host strips that before bundling).
        let entries = vec![BundleEntry {
            path: PROJECT_DIR.to_string(),
            bytes: b"".to_vec(),
        }];
        check_bundle_root(&entries).expect("single-file path allowed");
    }

    #[test]
    fn descendant_paths_allowed() {
        let entries = vec![
            BundleEntry {
                path: format!("{}/spackle.toml", PROJECT_DIR),
                bytes: b"".to_vec(),
            },
            BundleEntry {
                path: format!("{}/src/a.txt", PROJECT_DIR),
                bytes: b"".to_vec(),
            },
        ];
        check_bundle_root(&entries).expect("descendants allowed");
    }

    #[test]
    fn empty_bundle_passes_root_check() {
        // No entries → nothing to validate.
        check_bundle_root(&[]).expect("empty bundle passes");
    }

    #[test]
    fn sibling_prefix_collision_is_rejected() {
        // `/project_other/...` should not be treated as living under
        // `/project` just because of the literal-prefix match.
        let entries = vec![BundleEntry {
            path: "/project_other/spackle.toml".to_string(),
            bytes: b"".to_vec(),
        }];
        check_bundle_root(&entries).expect_err("must reject sibling-prefix path");
    }
}
