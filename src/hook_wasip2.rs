//! Synchronous hook executor for the wasip2 component build.
//!
//! WASI cannot spawn processes itself, so the component imports a
//! `run-command` capability from the host and calls it per hook. This
//! mirrors the semantics of `hook::run_hooks_stream` (ordering, hook_ran
//! state propagation, skip reasons, template-error fail-fast) but:
//!
//!   - runs synchronously (no tokio, no async-process),
//!   - delegates subprocess execution to the host via a generic closure,
//!   - returns its own `WasipHookResult` DTO (the native `HookResult`
//!     references `HookError` which wraps `std::io::Error` from
//!     `async_process` — won't compile for wasm32).

use serde::Serialize;
use std::{collections::HashMap, path::Path};
use tera::{Context, Tera};

use crate::hook::Hook;
use crate::needs::Needy;
use crate::slot::Slot;

/// Subprocess result as surfaced by the host-imported `run-command`.
/// Matches the shape of the WIT `command-result` record but stays a
/// plain Rust type so this module has no wit-bindgen dependency.
#[derive(Debug, Clone)]
pub struct CommandResult {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

#[derive(Serialize, Debug)]
pub struct WasipHookResult {
    pub hook_key: String,
    #[serde(flatten)]
    pub kind: WasipHookResultKind,
}

#[derive(Serialize, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WasipHookResultKind {
    /// The planner decided not to run this hook. `reason` is one of
    /// `user_disabled`, `unsatisfied_needs`, `false_conditional`,
    /// `conditional_error`, `template_error`.
    Skipped {
        reason: String,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        template_errors: Vec<String>,
    },
    /// The host ran the command and it exited with status 0.
    Completed {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
        exit_code: i32,
    },
    /// Either the host failed to launch the command, or the command
    /// exited non-zero. `error` is a human-readable description;
    /// `stdout`/`stderr` are populated when the command ran but failed.
    Failed {
        error: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stdout: Vec<u8>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        stderr: Vec<u8>,
        exit_code: i32,
    },
}

/// Plan and execute hooks synchronously using the host-provided
/// `run_command` closure. Mirrors the semantics of `evaluate_hook_plan`
/// plus actual execution, all in one pass so hook_ran_<key> state is
/// propagated across iterations.
pub fn run_hooks_sync<F>(
    dir: &Path,
    hooks: &[Hook],
    slots: &[Slot],
    data: &HashMap<String, String>,
    run_command: F,
) -> Vec<WasipHookResult>
where
    F: Fn(&str, &[String], &str, &[(String, String)]) -> Result<CommandResult, String>,
{
    let items: Vec<&dyn Needy> = {
        let mut items = slots
            .iter()
            .map(|s| s as &dyn Needy)
            .collect::<Vec<&dyn Needy>>();
        items.extend(hooks.iter().map(|h| h as &dyn Needy));
        items
    };

    // Running context that accumulates hook_ran_* state as we execute.
    // Prepopulate hook_ran_* = "false" so conditionals that reference
    // them never hit "undefined variable" errors.
    let mut running = data.clone();
    for hook in hooks {
        running
            .entry(format!("hook_ran_{}", hook.key))
            .or_insert_with(|| "false".to_string());
    }

    let mut results = Vec::with_capacity(hooks.len());

    for hook in hooks {
        if !hook.is_enabled(&running) {
            results.push(WasipHookResult {
                hook_key: hook.key.clone(),
                kind: WasipHookResultKind::Skipped {
                    reason: "user_disabled".to_string(),
                    template_errors: vec![],
                },
            });
            continue;
        }

        if !hook.is_satisfied(&items, &running) {
            results.push(WasipHookResult {
                hook_key: hook.key.clone(),
                kind: WasipHookResultKind::Skipped {
                    reason: "unsatisfied_needs".to_string(),
                    template_errors: vec![],
                },
            });
            continue;
        }

        match hook.evaluate_conditional(&running) {
            Ok(false) => {
                results.push(WasipHookResult {
                    hook_key: hook.key.clone(),
                    kind: WasipHookResultKind::Skipped {
                        reason: "false_conditional".to_string(),
                        template_errors: vec![],
                    },
                });
                continue;
            }
            Err(e) => {
                results.push(WasipHookResult {
                    hook_key: hook.key.clone(),
                    kind: WasipHookResultKind::Skipped {
                        reason: format!("conditional_error: {}", e),
                        template_errors: vec![],
                    },
                });
                continue;
            }
            Ok(true) => {}
        }

        // Template the command args. Fail-fast: any templating error →
        // skip the hook (matches native run_hooks_stream's semantics —
        // template errors abort before execution), and DO NOT flip
        // hook_ran_<key>.
        let context = match Context::from_serialize(&running) {
            Ok(c) => c,
            Err(e) => {
                results.push(WasipHookResult {
                    hook_key: hook.key.clone(),
                    kind: WasipHookResultKind::Skipped {
                        reason: "template_error".to_string(),
                        template_errors: vec![format!("context error: {}", e)],
                    },
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
            results.push(WasipHookResult {
                hook_key: hook.key.clone(),
                kind: WasipHookResultKind::Skipped {
                    reason: "template_error".to_string(),
                    template_errors,
                },
            });
            continue;
        }

        if templated_command.is_empty() {
            results.push(WasipHookResult {
                hook_key: hook.key.clone(),
                kind: WasipHookResultKind::Failed {
                    error: "empty command".to_string(),
                    stdout: vec![],
                    stderr: vec![],
                    exit_code: -1,
                },
            });
            continue;
        }

        let cmd = &templated_command[0];
        let args: Vec<String> = templated_command[1..].to_vec();
        let cwd = dir.to_string_lossy().to_string();

        match run_command(cmd, &args, &cwd, &[]) {
            Ok(res) if res.exit_code == 0 => {
                results.push(WasipHookResult {
                    hook_key: hook.key.clone(),
                    kind: WasipHookResultKind::Completed {
                        stdout: res.stdout,
                        stderr: res.stderr,
                        exit_code: res.exit_code,
                    },
                });
                running.insert(format!("hook_ran_{}", hook.key), "true".to_string());
            }
            Ok(res) => {
                results.push(WasipHookResult {
                    hook_key: hook.key.clone(),
                    kind: WasipHookResultKind::Failed {
                        error: format!("command exited with code {}", res.exit_code),
                        stdout: res.stdout,
                        stderr: res.stderr,
                        exit_code: res.exit_code,
                    },
                });
                // DO NOT flip hook_ran — native run_hooks_stream would
                // have surfaced this as HookError::CommandExited and
                // not added to ran_hooks.
            }
            Err(e) => {
                results.push(WasipHookResult {
                    hook_key: hook.key.clone(),
                    kind: WasipHookResultKind::Failed {
                        error: format!("command launch failed: {}", e),
                        stdout: vec![],
                        stderr: vec![],
                        exit_code: -1,
                    },
                });
            }
        }
    }

    results
}
