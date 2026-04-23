//! wasm-bindgen exports for spackle, bundle-in / bundle-out.
//!
//! Four exports plus `init`. Each takes a project bundle — a JS
//! `Array<{path, bytes: Uint8Array}>` — deserializes it into an in-process
//! [`MemoryFs`], runs the requested operation against that, and returns
//! a serialized result. Rust never touches the host's filesystem; the
//! host side (TS) does all disk I/O and subprocess execution before and
//! after the call.
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
//!   generate(project_bundle, project_dir, out_dir, slot_data_json) -> JsValue:
//!     success: `{ ok: true, files: Array<{path, bytes: Uint8Array}>, dirs: string[] }`
//!       where `path` is relative to `out_dir`
//!     failure: `{ ok: false, error: "..." }`
//!
//!   plan_hooks(project_bundle, project_dir, out_dir, data_json, hook_ran_json?) -> String (JSON):
//!     `Vec<HookPlanEntry>` — templated commands + should_run + skip_reason + template_errors.
//!     Hook execution is host-side; this is the planning half of the native
//!     CLI's two-step (generate then run_hooks_stream). See `ts/src/host/hooks.ts`
//!     for the TS-side runner.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

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
/// Hooks are a separate step — mirror the native CLI's two-call shape
/// (`project.generate(...)` then `project.run_hooks_stream(...)`). Call
/// `plan_hooks` after `generate` to get the resolved hook plan, then
/// execute host-side. See `ts/src/host/hooks.ts` for the reference runner.
#[wasm_bindgen]
pub fn generate(
    project_bundle: JsValue,
    project_dir: &str,
    out_dir: &str,
    slot_data_json: &str,
) -> JsValue {
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
///   open_session(bundle, project_dir) -> SessionId
///   plan_hooks_session(session_id, data, hook_ran) -> HookPlan
///   close_session(session_id)
/// That amortizes parse across the plan-execute loop. Deferred.
#[wasm_bindgen]
pub fn plan_hooks(
    project_bundle: JsValue,
    project_dir: &str,
    out_dir: &str,
    data_json: &str,
    hook_ran_json: Option<String>,
) -> String {
    let entries = match decode_bundle(project_bundle) {
        Ok(e) => e,
        Err(msg) => return plan_hooks_err(msg),
    };
    plan_hooks_from_entries(entries, project_dir, out_dir, data_json, hook_ran_json.as_deref())
}

/// Pure-Rust implementation of `plan_hooks`. Split out so native tests
/// can exercise the logic without going through `JsValue`.
fn plan_hooks_from_entries(
    entries: Vec<BundleEntry>,
    project_dir: &str,
    out_dir: &str,
    data_json: &str,
    hook_ran_json: Option<&str>,
) -> String {
    let fs = MemoryFs::from_bundle(entries);
    let project_path = PathBuf::from(project_dir);
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
    // templated with `{{ _output_name }}` render correctly.
    data.insert("_project_name".to_string(), project.get_name());
    data.insert(
        "_output_name".to_string(),
        spackle::get_output_name(Path::new(out_dir)),
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
    let mut executed_keys: std::collections::HashSet<String> =
        std::collections::HashSet::new();
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

    json_or_panic(&PlanHooksOk { ok: true, plan: &plan })
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

    const FIXTURE_TOML: &[u8] = include_bytes!(
        "../../../tests/fixtures/hooks_fixture/spackle.toml"
    );

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
        let raw = plan_hooks_from_entries(
            fixture_bundle(),
            "/project",
            "/tmp/my_output",
            data_json,
            hook_ran,
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
        let raw = plan_hooks_from_entries(
            fixture_bundle(),
            "/project",
            "/tmp/o",
            "not json",
            None,
        );
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
        let raw = plan_hooks_from_entries(entries, "/project", "/tmp/o", data_json, hook_ran);
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
        let plan = call_with_bundle(
            needs_fixture_bundle(),
            "{}",
            Some(r#"{"hook_a": true}"#),
        );
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
            plan.as_array().unwrap().iter().all(|e| e["key"] != "hook_a"),
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
}
