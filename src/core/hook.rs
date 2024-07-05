use std::{collections::HashMap, fmt::Display, path::Path, str::ParseBoolError};

use async_process::{Command, Stdio};
use futures::{executor, pin_mut, stream, Stream, StreamExt};
use tera::{Context, Tera};

use crate::core::hook;

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

/// Run a set of hooks asynchronously and returns a stream of their execution results.
///
/// The `dir` argument is the directory to run the hooks in.
///
/// The `data` argument is a map of key-value pairs to be used in the hook's conditional logic (if).
pub fn run_hooks_async(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
) -> Result<impl Stream<Item = HookUpdate>, Error> {
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

    let mut children = Vec::new();

    for hook in valid_hooks {
        let child = Command::new(&hook.command[0])
            .args(&hook.command[1..])
            .current_dir(dir.as_ref())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        match child {
            Ok(child) => children.push((hook.clone(), child)),
            Err(e) => {
                return Err(Error {
                    hook: hook.clone(),
                    error: ErrorKind::ErrorSpawning(Box::new(e)),
                });
            }
        }
    }

    let skipped_stream = stream::iter(
        skipped_hooks
            .into_iter()
            .map(|hook| hook.clone())
            .map(|hook| HookUpdate::HookDone(HookResult::Skipped(hook)))
            .collect::<Vec<HookUpdate>>(),
    );

    let children_stream = stream::unfold(children.into_iter(), |mut children| async move {
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
    });

    Ok(skipped_stream.chain(children_stream))
}

/// Run a set of hooks, returning an error if any of the hooks fail before executing.
///
/// The `dir` argument is the directory to run the hooks in.
pub fn run_hooks(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
) -> Result<Vec<HookResult>, Error> {
    let stream = run_hooks_async(hooks, dir, slot_data, hook_data)?;
    pin_mut!(stream);

    let mut results = Vec::new();

    while let Some(status) = executor::block_on(stream.next()) {
        match status {
            hook::HookUpdate::HookDone(r) => match r.clone() {
                HookResult::Errored { hook, error } => {
                    return Err(Error {
                        hook,
                        error: ErrorKind::ErrorExecuting(error),
                    });
                }
                HookResult::Completed {
                    hook,
                    stderr,
                    success,
                    ..
                } if !success => {
                    return Err(Error {
                        hook: hook.clone(),
                        error: ErrorKind::ErrorExecuting(stderr.to_string()),
                    });
                }
                _ => results.push(r),
            },
            hook::HookUpdate::AllHooksDone => break,
        }
    }

    Ok(results)
}
