//! wasm-bindgen exports for spackle. Per-file compute primitives the
//! TS host calls as it walks a project on disk:
//!
//!   - [`check`] / [`validate_slot_data`] — config-level validation
//!     against a small bundle (typically `spackle.toml` plus the
//!     project's `.j2` / `.tera` templates).
//!   - [`render_file`] — render one template body. `Tera::one_off`,
//!     no shared registry, so `{% include %}` / `{% import %}` /
//!     `{% extends %}` won't resolve. `check` rejects templates that
//!     use those tags. See `docs/design/wasm.md` for the follow-up.
//!   - [`render_path`] — render one path template.
//!   - [`plan_hooks`] — resolve the hook plan; host executes.
//!
//! Per-export shapes and contracts live on the function `///` docs.

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

/// Structured response from `check`. Always returns this shape — no
/// `valid: true/false` discriminant. Empty `diagnostics` ⇒ clean check.
/// `config` is `None` when the TOML couldn't be parsed (the diagnostics
/// list will contain a `config`-source entry explaining why).
#[derive(Serialize)]
struct CheckResponse<'a> {
    config: Option<&'a spackle::config::Config>,
    diagnostics: &'a Vec<spackle::Diagnostic>,
}

/// Response from [`render_file`]. `bytes` is the rendered output (UTF-8
/// of Tera's rendered string); `diagnostics` carries per-template
/// errors (parse / undefined var / render). Empty `diagnostics` ⇒ clean
/// render. On error `bytes` is empty — callers should branch on
/// diagnostics, not on byte count.
#[derive(Serialize)]
struct RenderFileResponse {
    #[serde(with = "serde_bytes")]
    bytes: Vec<u8>,
    diagnostics: Vec<spackle::Diagnostic>,
}

/// Response from [`render_path`]. `path` is the rendered path on
/// success; on error it falls back to the original input so callers
/// can surface the offending path in their UI. Branch on `diagnostics`.
#[derive(Serialize)]
struct RenderPathResponse {
    path: String,
    diagnostics: Vec<spackle::Diagnostic>,
}

/// Legacy `validate_slot_data` response. Kept for granular use.
#[derive(Serialize)]
struct ValidationErr {
    valid: bool,
    errors: Vec<String>,
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

fn decode_bundle(project_bundle: JsValue) -> Result<Vec<BundleEntry>, String> {
    serde_wasm_bindgen::from_value(project_bundle).map_err(|e| format!("invalid bundle: {}", e))
}

fn parse_slot_data(slot_data_json: &str) -> Result<HashMap<String, String>, String> {
    serde_json::from_str(slot_data_json).map_err(|e| format!("invalid slot_data_json: {}", e))
}

// --- exports ---

/// Run every static project check against `project_bundle`. Always
/// returns a structured `CheckResponse` (never throws). Diagnostics
/// accumulate across all stages so callers see every problem at once.
///
/// The bundle should contain `spackle.toml` plus any `.j2` / `.tera`
/// templates the host wants validated. Non-template files can be
/// passed with empty `bytes` so the path-template check covers them
/// without inhaling their contents.
///
/// On top of the core pipeline this export also rejects
/// `{% include %}` / `{% import %}` / `{% extends %}` in template
/// bodies: the wasm render path uses `Tera::one_off`, no shared
/// registry, so those tags can't resolve. The rejection is wasm-only
/// — native handles them fine via its own registry.
#[wasm_bindgen]
pub fn check(project_bundle: JsValue, project_dir: &str) -> String {
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
    let mut wasm_diags = scan_bundle_for_unsupported_tags(&entries);

    let fs = MemoryFs::from_bundle(entries);
    let path = PathBuf::from(project_dir);
    let report = spackle::check_project(&fs, &path);

    let mut all = report.diagnostics.clone();
    all.append(&mut wasm_diags);
    json_or_panic(&CheckResponse {
        config: report.config.as_ref(),
        diagnostics: &all,
    })
}

/// Scan every `.j2` / `.tera` template body in the bundle for cross-
/// template tags the wasm render path can't resolve.
fn scan_bundle_for_unsupported_tags(entries: &[BundleEntry]) -> Vec<spackle::Diagnostic> {
    let mut out = Vec::new();
    for entry in entries {
        if !has_template_ext(&entry.path) {
            continue;
        }
        let body = match std::str::from_utf8(&entry.bytes) {
            Ok(s) => s,
            Err(_) => continue,
        };
        for tag in scan_cross_template_tags(body) {
            out.push(
                spackle::Diagnostic::new(
                    spackle::Severity::Error,
                    spackle::DiagnosticSource::RenderBody,
                    format!(
                        "`{{% {tag} %}}` is not supported — the wasm render path has no template registry. Tracked follow-up: multi-template Tera semantics."
                    ),
                )
                .with_path(entry.path.clone()),
            );
        }
    }
    out
}

fn has_template_ext(name: &str) -> bool {
    name.ends_with(".j2") || name.ends_with(".tera")
}

/// Scan a template body for `{% include %}` / `{% import %}` /
/// `{% extends %}` tag *openings* that aren't inside a string literal,
/// comment, or `{% raw %}` block.
///
/// The scanner walks bytes and treats Tera block delimiters as
/// structure: `{{ … }}` expressions and `{% … %}` statements are
/// skipped wholesale once entered, with quote-aware close detection
/// so a tag-like sequence inside `"…"` / `'…'` doesn't fool us
/// (e.g. `{{ "{% include %}" }}` is *not* a real include).
fn scan_cross_template_tags(body: &str) -> Vec<&'static str> {
    let mut found: Vec<&'static str> = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    let mut in_raw = false;
    while i < bytes.len() {
        // Comment: opaque to the rest of the scanner.
        if !in_raw && i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'#' {
            i = skip_past_close(bytes, i + 2, b'#', b'}', false);
            continue;
        }
        // Expression: contents skipped with string-literal awareness.
        if !in_raw && i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            i = skip_past_close(bytes, i + 2, b'}', b'}', true);
            continue;
        }
        // Statement: read tag name, then skip to `%}` with string-
        // literal awareness so a `%}` inside a string doesn't close
        // the block early.
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'%' {
            let mut j = i + 2;
            if j < bytes.len() && bytes[j] == b'-' {
                j += 1;
            }
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            let name_start = j;
            while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
                j += 1;
            }
            let tag_name = &body[name_start..j];

            if in_raw {
                if tag_name == "endraw" {
                    in_raw = false;
                }
            } else if tag_name == "raw" {
                in_raw = true;
            } else {
                let matched: Option<&'static str> = match tag_name {
                    "include" => Some("include"),
                    "import" => Some("import"),
                    "extends" => Some("extends"),
                    _ => None,
                };
                if let Some(name) = matched {
                    if !found.contains(&name) {
                        found.push(name);
                    }
                }
            }
            // Inside raw, Tera doesn't parse string literals — skip
            // with quote-awareness only when we're outside raw.
            i = skip_past_close(bytes, j, b'%', b'}', !in_raw);
            continue;
        }
        i += 1;
    }
    found
}

/// Advance past the next `(close1, close2)` byte pair starting at
/// `from`. When `respect_quotes` is true, single- and double-quoted
/// string literals (with backslash escapes) are treated as opaque so
/// a quoted `close1 close2` doesn't terminate the block. Returns
/// `bytes.len()` if the close is never found.
fn skip_past_close(
    bytes: &[u8],
    from: usize,
    close1: u8,
    close2: u8,
    respect_quotes: bool,
) -> usize {
    let mut i = from;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == b'\\' && i + 1 < bytes.len() {
                i += 2;
                continue;
            }
            if b == q {
                quote = None;
            }
            i += 1;
            continue;
        }
        if respect_quotes && (b == b'\'' || b == b'"') {
            quote = Some(b);
            i += 1;
            continue;
        }
        if b == close1 && i + 1 < bytes.len() && bytes[i + 1] == close2 {
            return i + 2;
        }
        i += 1;
    }
    bytes.len()
}

/// Validate slot data against the config loaded from the project bundle.
/// The bundle only needs to contain `spackle.toml` — slot validation
/// doesn't need template bodies.
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
    let data: HashMap<String, String> = match parse_slot_data(slot_data_json) {
        Ok(d) => d,
        Err(e) => return invalid(vec![e]),
    };
    match spackle::slot::validate_data(&data, &project.config.slots) {
        Ok(()) => r#"{"valid":true}"#.to_string(),
        Err(e) => invalid(vec![e.to_string()]),
    }
}

/// Render a single template body. `Tera::one_off` — no shared
/// registry, so cross-template tags don't resolve (see [`check`]).
///
/// `virtual_path` is optional and shows up in any returned diagnostic's
/// `path` field for UI attribution.
///
/// `_project_name` / `_output_name` are not auto-injected; the host
/// puts them into `slot_data` if templates reference them.
///
/// Filename templating is separate ([`render_path`]). For a `.j2`
/// file with a templated name: call `render_path` on the relative
/// path AND `render_file` on the body, then strip the trailing
/// extension host-side.
#[wasm_bindgen]
pub fn render_file(
    template_bytes: &[u8],
    slot_data_json: &str,
    virtual_path: Option<String>,
) -> JsValue {
    let body = match std::str::from_utf8(template_bytes) {
        Ok(s) => s,
        Err(e) => {
            return diag_response_file(virtual_path, format!("template is not valid UTF-8: {}", e))
        }
    };

    let slot_data = match parse_slot_data(slot_data_json) {
        Ok(d) => d,
        Err(e) => return diag_response_file(virtual_path, e),
    };

    let context = match tera::Context::from_serialize(&slot_data) {
        Ok(c) => c,
        Err(e) => return diag_response_file(virtual_path, format!("context error: {}", e)),
    };

    match tera::Tera::one_off(body, &context, false) {
        Ok(rendered) => RenderFileResponse {
            bytes: rendered.into_bytes(),
            diagnostics: Vec::new(),
        }
        .serialize(&serializer())
        .unwrap_or(JsValue::NULL),
        Err(e) => {
            let mut diag = spackle::Diagnostic::new(
                spackle::Severity::Error,
                spackle::DiagnosticSource::RenderBody,
                e.to_string(),
            );
            if let Some(p) = virtual_path {
                diag = diag.with_path(p);
            }
            if let Some(span) = spackle::diagnostic::extract_tera_span(&e) {
                diag = diag.with_span(span);
            }
            RenderFileResponse {
                bytes: Vec::new(),
                diagnostics: vec![diag],
            }
            .serialize(&serializer())
            .unwrap_or(JsValue::NULL)
        }
    }
}

fn diag_response_file(virtual_path: Option<String>, message: String) -> JsValue {
    let mut diag = spackle::Diagnostic::new(
        spackle::Severity::Error,
        spackle::DiagnosticSource::RenderBody,
        message,
    );
    if let Some(p) = virtual_path {
        diag = diag.with_path(p);
    }
    RenderFileResponse {
        bytes: Vec::new(),
        diagnostics: vec![diag],
    }
    .serialize(&serializer())
    .unwrap_or(JsValue::NULL)
}

/// Render a single path template with `slot_data`. Used for filename /
/// directory-segment templating (e.g. `src/{{ project }}.txt`). On
/// error, `path` falls back to the input so the host can attribute the
/// diagnostic to a specific path in its UI; callers branch on
/// `diagnostics`, not on `path` content.
#[wasm_bindgen]
pub fn render_path(path_template: &str, slot_data_json: &str) -> JsValue {
    let slot_data = match parse_slot_data(slot_data_json) {
        Ok(d) => d,
        Err(e) => return diag_response_path(path_template, e),
    };

    let context = match tera::Context::from_serialize(&slot_data) {
        Ok(c) => c,
        Err(e) => return diag_response_path(path_template, format!("context error: {}", e)),
    };

    match tera::Tera::one_off(path_template, &context, false) {
        Ok(rendered) => RenderPathResponse {
            path: rendered,
            diagnostics: Vec::new(),
        }
        .serialize(&serializer())
        .unwrap_or(JsValue::NULL),
        Err(e) => {
            let mut diag = spackle::Diagnostic::new(
                spackle::Severity::Error,
                spackle::DiagnosticSource::RenderName,
                e.to_string(),
            )
            .with_path(path_template.to_string());
            if let Some(span) = spackle::diagnostic::extract_tera_span(&e) {
                diag = diag.with_span(span);
            }
            RenderPathResponse {
                path: path_template.to_string(),
                diagnostics: vec![diag],
            }
            .serialize(&serializer())
            .unwrap_or(JsValue::NULL)
        }
    }
}

fn diag_response_path(path_template: &str, message: String) -> JsValue {
    let diag = spackle::Diagnostic::new(
        spackle::Severity::Error,
        spackle::DiagnosticSource::RenderName,
        message,
    )
    .with_path(path_template.to_string());
    RenderPathResponse {
        path: path_template.to_string(),
        diagnostics: vec![diag],
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
    plan_hooks_from_entries(
        entries,
        project_dir,
        out_dir,
        data_json,
        hook_ran_json.as_deref(),
    )
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
mod scan_tests {
    use super::scan_cross_template_tags;

    #[test]
    fn plain_include_import_extends_caught() {
        assert_eq!(scan_cross_template_tags(r#"{% include "x" %}"#), vec!["include"]);
        assert_eq!(scan_cross_template_tags(r#"{% import "m" as m %}"#), vec!["import"]);
        assert_eq!(scan_cross_template_tags(r#"{% extends "b" %}"#), vec!["extends"]);
    }

    #[test]
    fn whitespace_trim_variant_caught() {
        assert_eq!(scan_cross_template_tags(r#"{%- include "x" -%}"#), vec!["include"]);
    }

    #[test]
    fn raw_block_suppresses_detection() {
        let body = r#"{% raw %}{% include "x" %}{% endraw %}"#;
        assert!(scan_cross_template_tags(body).is_empty());
    }

    #[test]
    fn tag_outside_raw_still_caught_after_block_closes() {
        let body = r#"{% raw %}{% include "x" %}{% endraw %}{% include "y" %}"#;
        assert_eq!(scan_cross_template_tags(body), vec!["include"]);
    }

    #[test]
    fn comment_block_suppresses_detection() {
        let body = r#"{# example: {% include "x" %} #}"#;
        assert!(scan_cross_template_tags(body).is_empty());
    }

    #[test]
    fn distinct_tags_deduped_per_template() {
        let body = r#"{% include "a" %}{% include "b" %}{% extends "c" %}"#;
        assert_eq!(scan_cross_template_tags(body), vec!["include", "extends"]);
    }

    #[test]
    fn similar_identifiers_not_matched() {
        // `include_foo` shares a prefix but is a distinct identifier;
        // the scanner reads the full identifier and only matches exact
        // tag names.
        let body = r#"{% include_foo "x" %}"#;
        assert!(scan_cross_template_tags(body).is_empty());
    }

    #[test]
    fn control_flow_tags_not_flagged() {
        let body = r#"{% if x %}a{% else %}b{% endif %}{% for i in xs %}{% endfor %}"#;
        assert!(scan_cross_template_tags(body).is_empty());
    }

    #[test]
    fn empty_body_is_empty() {
        assert!(scan_cross_template_tags("").is_empty());
        assert!(scan_cross_template_tags("plain text").is_empty());
    }

    #[test]
    fn tag_inside_expression_string_literal_is_not_flagged() {
        // `{{ "..." }}` — the tag-like text is inside a string literal
        // inside an expression. Tera parses this as a `String` value,
        // not as an include directive.
        let body = r#"{{ "{% include \"x\" %}" }}"#;
        assert!(
            scan_cross_template_tags(body).is_empty(),
            "got: {:?}",
            scan_cross_template_tags(body),
        );
    }

    #[test]
    fn tag_inside_single_quoted_expression_string_is_not_flagged() {
        let body = r#"{{ '{% include %}' }}"#;
        assert!(scan_cross_template_tags(body).is_empty());
    }

    #[test]
    fn tag_inside_statement_string_literal_is_not_flagged() {
        // Outer `{% set %}` contains a string literal with tag-like
        // text. Only the outer statement is real; the inside is data.
        let body = r#"{% set x = "{% include %}" %}"#;
        assert!(
            scan_cross_template_tags(body).is_empty(),
            "got: {:?}",
            scan_cross_template_tags(body),
        );
    }

    #[test]
    fn real_tag_alongside_string_literal_still_caught() {
        // Real outer include with a string literal containing a
        // tag-like substring. Should flag exactly one include.
        let body = r#"{% include "fragment_{% include %}.j2" %}"#;
        assert_eq!(scan_cross_template_tags(body), vec!["include"]);
    }
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
        let raw = plan_hooks_from_entries(fixture_bundle(), "/project", "/tmp/o", "not json", None);
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
}
