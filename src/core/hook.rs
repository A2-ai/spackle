use std::{collections::HashMap, fmt::Display, path::Path};
use std::{io, process};

use async_process::Stdio;
use async_stream::stream;
use tera::{Context, Tera};
use tokio::pin;
use tokio_stream::{Stream, StreamExt};

use super::config::Hook;
use users::User;

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
    Failed {
        hook: Hook,
        output: String,
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
pub enum Error {
    ErrorInitializingRuntime(io::Error),
    ErrorRenderingTemplate(Hook, tera::Error),
    InvalidConditional(Hook, ConditionalError),
    SetupFailed(Hook, io::Error),
}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::ErrorInitializingRuntime(e) => {
                write!(f, "error initializing runtime: {}", e)
            }
            Error::ErrorRenderingTemplate(hook, e) => {
                write!(f, "error rendering template for hook {}: {}", hook.key, e)
            }
            Error::InvalidConditional(hook, e) => {
                write!(f, "invalid conditional for hook {}: {}", hook.key, e)
            }
            Error::SetupFailed(hook, e) => {
                write!(f, "setup failed for hook {}: {}", hook.key, e)
            }
        }
    }
}

pub enum HookStreamResult {
    HookStarted(Hook),
    HookDone(HookResult),
}

pub fn run_hooks_stream(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<impl Stream<Item = HookStreamResult>, Error> {
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
        let context = Context::from_serialize(slot_data.clone())
            .map_err(|e| Error::ErrorRenderingTemplate(hook.clone(), e))?;

        let command = hook
            .command
            .iter()
            .map(|arg| {
                Tera::one_off(arg, &context, false)
                    .map_err(|e| Error::ErrorRenderingTemplate(hook.clone(), e))
            })
            .collect::<Result<Vec<String>, Error>>()?;

        templated_hooks.push(Hook {
            command,
            ..hook.clone()
        });
    }

    let mut commands = Vec::new();
    for hook in templated_hooks {
        let cmd = match run_as_user {
            Some(ref user) => match polyjuice::cmd_as_user(&hook.command[0], user.clone()) {
                Ok(cmd) => cmd,
                Err(e) => {
                    return Err(Error::SetupFailed(
                        hook.clone(),
                        io::Error::new(
                            std::io::ErrorKind::Other,
                            format!("Failed to run command as user: {}", e),
                        ),
                    )); //TODO we probably want a different error type here
                }
            },
            None => process::Command::new(&hook.command[0]),
        };

        commands.push((hook, async_process::Command::from(cmd)));
    }

    let slot_data_owned = slot_data.clone();
    let hook_keys = hooks.iter().map(|h| h.key.clone()).collect::<Vec<String>>();

    Ok(stream! {
        for hook in skipped_hooks {
            yield HookStreamResult::HookDone(HookResult::Skipped {
                hook: hook.0,
                reason: hook.1,
            });
        }

        let mut ran_hooks = Vec::new();
        for (hook, mut cmd) in commands {
            // Evaluate conditional
            // also add to the context the run status of all hooks so far
            // TODO this can be evaluated outside of stream once "needs" is implemented
            let mut cond_context = slot_data_owned.clone();
            for hook in &hook_keys {
                cond_context.insert(format!("hook_ran_{}", hook), "false".to_string());
            }
            for hook in ran_hooks.clone() {
                cond_context.insert(format!("hook_ran_{}", hook), "true".to_string());
            }

            let condition = match evaluate_conditional(&hook, &slot_data_owned) {
                Ok(condition) => condition,
                Err(e) => {
                    yield HookStreamResult::HookDone(HookResult::Failed {
                        hook: hook.clone(),
                        output: e.to_string(),
                    });
                    continue;
                }
            };

            if !condition {
                yield HookStreamResult::HookDone(HookResult::Skipped {
                    hook: hook.clone(),
                    reason: SkipReason::FalseConditional,
                });
                continue;
            }

            let cmd_result = cmd.args(&hook.command[1..])
                .current_dir(dir.as_ref())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .output().await;

            let output = match cmd_result {
                Ok(output) => output,
                Err(e) => {
                    yield HookStreamResult::HookDone(HookResult::Failed {
                        hook: hook.clone(),
                        output: e.to_string(),
                    });
                    continue;
                }
            };

            if !output.status.success() {
                yield HookStreamResult::HookDone(HookResult::Failed {
                    hook: hook.clone(),
                    output: String::from_utf8_lossy(&output.stderr).to_string(),
                });
                continue;
            }

            ran_hooks.push(hook.key.clone());

            yield HookStreamResult::HookDone(HookResult::Completed {
                hook: hook.clone(),
                stdout: String::from_utf8_lossy(&output.stdout).into(),
                stderr: String::from_utf8_lossy(&output.stderr).into(),
            });
        }
    })
}

pub fn run_hooks(
    hooks: &Vec<Hook>,
    dir: impl AsRef<Path>,
    slot_data: &HashMap<String, String>,
    hook_data: &HashMap<String, bool>,
    run_as_user: Option<User>,
) -> Result<Vec<HookResult>, Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::ErrorInitializingRuntime(e))?;

    let results = runtime.block_on(async {
        let stream = run_hooks_stream(hooks, dir, slot_data, hook_data, run_as_user)?;
        pin!(stream);

        let mut hook_results = Vec::new();

        while let Some(result) = stream.next().await {
            match result {
                HookStreamResult::HookStarted(_) => {}
                HookStreamResult::HookDone(hook_result) => {
                    hook_results.push(hook_result);
                }
            }
        }

        Ok(hook_results)
    })?;

    Ok(results)
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
