use spackle::core::{
    config::Hook,
    hook::{self, HookResult},
};
use std::collections::HashMap;

#[test]
fn basic() {
    let hooks = vec![Hook {
        name: "hello world".to_string(),
        command: vec!["echo".to_string(), "hello world".to_string()],
        r#if: None,
    }];

    assert!(hook::run_hooks(hooks, ".", HashMap::new()).is_ok());
}

#[test]
fn exec_error() {
    let hooks = vec![
        Hook {
            name: "hello world".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
        },
        Hook {
            name: "fail".to_string(),
            command: vec!["false".to_string()],
            r#if: None,
        },
    ];

    let result = hook::run_hooks(hooks, ".", HashMap::new()).unwrap_err();
    assert_eq!(result.hook.name, "fail".to_string());
}

#[test]
fn conditional() {
    let hooks = vec![
        Hook {
            name: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("true".to_string()),
        },
        Hook {
            name: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("false".to_string()),
        },
        Hook {
            name: "3".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
        },
    ];

    let results = hook::run_hooks(hooks, ".", HashMap::new()).unwrap();

    let skipped_hooks: Vec<_> = results
        .iter()
        .filter(|r| matches!(r, HookResult::Skipped(_)))
        .collect();
    assert_eq!(skipped_hooks.len(), 1);
}

#[test]
fn bad_conditional_template() {
    let hooks = vec![
        Hook {
            name: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("{{ good_var }}".to_string()),
        },
        Hook {
            name: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("{{ bad_var }}".to_string()),
        },
    ];

    assert!(hook::run_hooks(
        hooks,
        ".",
        HashMap::from([("good_var".to_string(), "true".to_string())])
    )
    .is_err());
}

#[test]
fn bad_conditional_value() {
    let hooks = vec![Hook {
        name: "1".to_string(),
        command: vec!["echo".to_string(), "hello world".to_string()],
        r#if: Some("lorem ipsum".to_string()),
    }];

    assert!(hook::run_hooks(
        hooks,
        ".",
        HashMap::from([("".to_string(), "".to_string())])
    )
    .is_err());
}
