use std::{collections::HashMap, fmt::Display, path::Path, str::ParseBoolError};

use futures::{stream, StreamExt};
use std::process::{Command, Stdio};
use tera::{Context, Tera};

use super::config::Hook;

#[derive(Debug)]
pub enum HookUpdate {
    HookDone(HookResult),
    AllHooksDone,
}

#[derive(Debug, Clone)]
pub enum HookResult {
    Skipped(Hook),
    Errored {
        hook: Hook,
        error: String,
    },
    Completed {
        hook: Hook,
        stdout: String,
        stderr: String,
        success: bool,
    },
}

#[derive(Debug)]
pub struct Error {
    pub hook: Hook,
    pub error: ErrorKind,
}

#[derive(Debug)]
pub enum ErrorKind {
    ErrorRenderingConditional(tera::Error),
    ErrorParsingConditional(ParseBoolError),
    ErrorSpawning(Box<dyn std::error::Error>),
    ErrorExecuting(String),
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::ErrorRenderingConditional(e) => {
                write!(f, "Error rendering conditional\n{}", e)
            }
            ErrorKind::ErrorParsingConditional(e) => {
                write!(f, "Error parsing conditional\n{}", e)
            }
            ErrorKind::ErrorSpawning(e) => write!(f, "Error spawning\n{}", e),
            ErrorKind::ErrorExecuting(e) => write!(f, "Error executing\n{}", e),
        }
    }
}

/// Run a set of hooks, returning their execution results.
///
/// The `dir` argument is the directory to run the hooks in.
///
/// The `data` argument is a map of key-value pairs to be used in the hook's conditional logic (if).
pub fn run_hooks(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
) -> Result<Vec<HookResult>, Error> {
    let mut skipped_hooks = Vec::new();

    // Filter out hooks that the user has disabled
    let mut user_valid_hooks = Vec::new();
    for hook in hooks {
        // If the hooks is optional and the user has not enabled it (if they haven't provided configuration refer to the hook default), skip it.
        if let Some(optional) = &hook.optional {
            let enabled = hook_data.get(&hook.key).unwrap_or(&optional.default);
            if !*enabled {
                skipped_hooks.push(hook);
            } else {
                user_valid_hooks.push(hook);
            }
        } else {
            user_valid_hooks.push(hook);
        }
    }

    // Filter out hooks that have an r#if condition that evaluates to false
    let mut valid_hooks: Vec<&Hook> = Vec::new();
    for hook in user_valid_hooks {
        if let Some(r#if) = hook.clone().r#if {
            let context = Context::from_serialize(slot_data.clone()).map_err(|e| Error {
                hook: hook.clone(),
                error: ErrorKind::ErrorRenderingConditional(e),
            })?;

            let condition = Tera::one_off(&r#if, &context, false).map_err(|e| Error {
                hook: hook.clone(),
                error: ErrorKind::ErrorRenderingConditional(e),
            })?;

            if condition.trim().parse::<bool>().map_err(|e| Error {
                hook: hook.clone(),
                error: ErrorKind::ErrorParsingConditional(e),
            })? {
                valid_hooks.push(hook);
            } else {
                skipped_hooks.push(hook);
            }
        } else {
            valid_hooks.push(hook);
        }
    }

    let mut children: Vec<(Hook, std::process::Child)> = Vec::new();

    let outputs = valid_hooks
        .iter()
        .map(|hook| {
            let output = Command::new(&hook.command[0])
                .args(&hook.command[1..])
                .current_dir(dir.as_ref())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output();

            (**hook, output)
        })
        .collect::<Vec<_>>();

    match children.next() {
        Some((hook, child)) => {
            let output = match child.output().await {
                Ok(output) => output,
                Err(e) => {
                    return Some((
                        HookUpdate::HookDone(HookResult::Errored {
                            hook: hook.clone(),
                            error: e.to_string(),
                        }),
                        children,
                    ));
                }
            };

            Some((
                HookUpdate::HookDone(HookResult::Completed {
                    hook: hook.clone(),
                    stdout: String::from_utf8_lossy(&output.stdout).into(),
                    stderr: String::from_utf8_lossy(&output.stderr).into(),
                    success: output.status.success(),
                }),
                children,
            ))
        }
        None => None,
    }

    let results = outputs.iter().map(|(hook, output)| match output {
        Ok(output) => HookResult::Errored {
            hook: hook.clone(),
            error: e.to_string(),
        },
        Err(e) => HookResult::Errored {
            hook: hook.clone(),
            error: e.to_string(),
        },
    });

    Ok(results)
}
