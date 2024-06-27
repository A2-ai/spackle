use std::{collections::HashMap, path::Path, str::ParseBoolError};

use async_process::Command;
use futures::{executor, pin_mut, stream, Stream, StreamExt};
use tera::{Context, Tera};

use crate::core::hook;

use super::config::Hook;

pub enum CommandResult {
    HookCompleted(Hook),
    HookFailed { hook: Hook, stderr: String },
    Done,
}

#[derive(Debug)]
pub enum Error {
    ErrorRenderingConditional(Hook, tera::Error),
    ErrorParsingConditional(Hook, ParseBoolError),
    ErrorSpawning(Hook, Box<dyn std::error::Error>),
    ErrorExecuting(Hook, String),
}

/// Run a set of hooks asynchronously and returns a stream of their execution results.
///
/// The `dir` argument is the directory to run the hooks in.
///
/// The `data` argument is a map of key-value pairs to be used in the hooks.
pub fn run_hooks_async(
    hooks: Vec<Hook>,
    dir: impl AsRef<Path>,
    data: HashMap<String, String>,
) -> Result<(impl Stream<Item = CommandResult>, Vec<Hook>), Error> {
    let mut skipped_hooks = Vec::new();

    // Filter out hooks that have an r#if condition that evaluates to false
    let mut valid_hooks = Vec::new();
    for hook in hooks {
        if let Some(r#if) = hook.clone().r#if {
            let context = Context::from_serialize(data.clone())
                .map_err(|e| Error::ErrorRenderingConditional(hook.clone(), e))?;

            let condition = Tera::one_off(&r#if, &context, false)
                .map_err(|e| Error::ErrorRenderingConditional(hook.clone(), e))?;

            if condition
                .trim()
                .parse::<bool>()
                .map_err(|e| Error::ErrorParsingConditional(hook.clone(), e))?
            {
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
            .spawn();

        match child {
            Ok(child) => children.push((hook, child)),
            Err(e) => {
                return Err(Error::ErrorSpawning(hook, Box::new(e)));
            }
        }
    }

    let stream = stream::unfold(children.into_iter(), |mut children| async move {
        match children.next() {
            Some((hook, child)) => {
                let output = match child.output().await {
                    Ok(output) => output,
                    Err(_) => {
                        return Some((
                            CommandResult::HookFailed {
                                hook,
                                stderr: "".to_string(),
                            },
                            children,
                        ));
                    }
                };

                let result = match output.status.success() {
                    true => CommandResult::HookCompleted(hook),
                    false => CommandResult::HookFailed {
                        hook,
                        stderr: String::from_utf8_lossy(&output.stderr).into(),
                    },
                };

                Some((result, children))
            }
            None => None,
        }
    });

    Ok((stream, skipped_hooks))
}

/// Run a set of hooks, returning an error if any of the hooks fail. Returns a list of hooks that were skipped.
///
/// The `dir` argument is the directory to run the hooks in.
pub fn run_hooks(
    hooks: Vec<Hook>,
    dir: impl AsRef<Path>,
    data: HashMap<String, String>,
) -> Result<Vec<Hook>, Error> {
    let (stream, skipped_hooks) = run_hooks_async(hooks, dir, data)?;
    pin_mut!(stream);

    while let Some(status) = executor::block_on(stream.next()) {
        match status {
            hook::CommandResult::HookCompleted(hook) => {
                println!("hook completed: {:?}", hook);
            }
            hook::CommandResult::HookFailed { hook, stderr } => {
                return Err(Error::ErrorExecuting(hook, stderr));
            }
            hook::CommandResult::Done => break,
        }
    }

    Ok(skipped_hooks)
}
