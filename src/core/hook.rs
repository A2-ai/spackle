use std::{collections::HashMap, fmt::Display, path::Path};

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
    Skipped {
        hook: Hook,
        reason: SkipReason,
    },
    Completed {
        hook: Hook,
        stdout: String,
        stderr: String,
    },
}

#[derive(Debug, Clone)]
pub enum SkipReason {
    UserDisabled,
    FalseConditional,
}

impl Display for SkipReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SkipReason::UserDisabled => write!(f, "user disabled"),
            SkipReason::FalseConditional => write!(f, "false conditional"),
        }
    }
}

#[derive(Debug)]
pub struct Error {
    pub hook: Hook,
    pub error: ErrorKind,
}

#[derive(Debug)]
pub enum ErrorKind {
    ErrorRenderingTemplate(tera::Error),
    InvalidConditional(ConditionalError),
    ErrorExecuting(std::io::Error),
    CommandFailed(String),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.hook.key, self.error)
    }
}

impl Display for ErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorKind::ErrorRenderingTemplate(e) => {
                write!(f, "error rendering conditional\n{}", e)
            }
            ErrorKind::InvalidConditional(e) => {
                write!(f, "invalid conditional\n{}", e)
            }
            ErrorKind::ErrorExecuting(e) => write!(f, "error executing\n{}", e),
            ErrorKind::CommandFailed(e) => write!(f, "command failed\n{}", e),
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
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<String>,
) -> Result<Vec<HookResult>, Error> {
    let mut skipped_hooks = Vec::new();
    let mut queued_hooks = Vec::new();

    for hook in hooks {
        let enabled = match &hook.optional {
            Some(optional) => hook_data.get(&hook.key).unwrap_or(&optional.default),
            None => &true,
        };
        if *enabled {
            queued_hooks.push(hook.clone());
        } else {
            skipped_hooks.push((hook.clone(), SkipReason::UserDisabled));
        }
    }

    // Apply template to command
    let mut templated_hooks = Vec::new();
    for hook in queued_hooks {
        let context = Context::from_serialize(slot_data.clone()).map_err(|e| Error {
            hook: hook.clone(),
            error: ErrorKind::ErrorRenderingTemplate(e),
        })?;

        let command = hook
            .command
            .iter()
            .map(|arg| {
                Tera::one_off(arg, &context, false).map_err(|e| Error {
                    hook: hook.clone(),
                    error: ErrorKind::ErrorRenderingTemplate(e),
                })
            })
            .collect::<Result<Vec<String>, Error>>()?;

        templated_hooks.push(Hook {
            command,
            ..hook.clone()
        });
    }

    let mut ran_hooks: Vec<(&Hook, std::process::Output)> = Vec::new();
    for hook in &templated_hooks {
        // Evaluate conditional
        // also add to the context the run status of all hooks so far
        let mut cond_context = slot_data.clone();
        for hook in hooks {
            cond_context.insert(format!("hook_ran_{}", hook.key), "false".to_string());
        }
        for (hook, _) in ran_hooks.clone() {
            cond_context.insert(format!("hook_ran_{}", hook.key), "true".to_string());
        }
        let condition = evaluate_conditional(&hook, &cond_context).map_err(|e| Error {
            hook: hook.clone(),
            error: ErrorKind::InvalidConditional(e),
        })?;

        if !condition {
            skipped_hooks.push((hook.clone(), SkipReason::FalseConditional));
            continue;
        }

        let mut cmd = match run_as_user {
            Some(ref user) => match polyjuice::cmd_as_user(&hook.command[0], user.clone()) {
                Ok(cmd) => cmd,
                Err(e) => {
                    return Err(Error {
                        hook: hook.clone(),
                        error: ErrorKind::CommandFailed(e.to_string()),
                    })
                }
            },
            None => Command::new(&hook.command[0]),
        };

        let output = cmd
            .args(&hook.command[1..])
            .current_dir(dir.as_ref())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| Error {
                hook: hook.clone(),
                error: ErrorKind::ErrorExecuting(e),
            })?;

        if !output.status.success() {
            return Err(Error {
                hook: hook.clone(),
                error: ErrorKind::CommandFailed(output.status.to_string()),
            });
        }

        ran_hooks.push((hook, output));
    }

    let hook_results = ran_hooks
        .iter()
        .map(|(hook, output)| HookResult::Completed {
            hook: (*hook).clone(),
            stdout: String::from_utf8_lossy(&output.stdout).into(),
            stderr: String::from_utf8_lossy(&output.stderr).into(),
        });

    let skipped_hook_results = skipped_hooks
        .iter()
        .map(|(hook, reason)| HookResult::Skipped {
            hook: hook.clone(),
            reason: reason.clone(),
        });

    Ok(hook_results.chain(skipped_hook_results).collect())
}

#[derive(Debug)]
pub enum ConditionalError {
    InvalidContext(tera::Error),
    InvalidTemplate(tera::Error),
    NotBoolean(String),
}

impl Display for ConditionalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConditionalError::InvalidContext(e) => write!(f, "invalid context\n{}", e),
            ConditionalError::InvalidTemplate(e) => write!(f, "invalid template\n{}", e),
            ConditionalError::NotBoolean(e) => write!(f, "not a boolean\n{}", e),
        }
    }
}

fn evaluate_conditional(
    hook: &Hook,
    context: &HashMap<String, String>,
) -> Result<bool, ConditionalError> {
    let conditional = match hook.clone().r#if {
        Some(conditional) => conditional,
        None => return Ok(true),
    };

    let context =
        Context::from_serialize(context).map_err(|e| ConditionalError::InvalidContext(e))?;

    let condition_str = Tera::one_off(&conditional, &context, false)
        .map_err(|e| ConditionalError::InvalidTemplate(e))?;

    let condition = condition_str
        .trim()
        .parse::<bool>()
        .map_err(|e| ConditionalError::NotBoolean(e.to_string()))?;

    Ok(condition)
}
