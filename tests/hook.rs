use spackle::core::{
    config::{Hook, HookConfigOptional},
    hook::{self, HookResult},
};
use std::collections::HashMap;

#[test]
fn basic() {
    let hooks = vec![Hook {
        key: "hello world".to_string(),
        command: vec!["echo".to_string(), "hello world".to_string()],
        r#if: None,
        optional: None,
    }];

    assert!(hook::run_hooks(&hooks, ".", HashMap::new(), &HashMap::new()).is_ok());
}

#[test]
fn exec_error() {
    let hooks = vec![
        Hook {
            key: "hello world".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
        },
        Hook {
            key: "fail".to_string(),
            command: vec!["false".to_string()],
            r#if: None,
            optional: None,
        },
    ];

    let result = hook::run_hooks(&hooks, ".", HashMap::new(), &HashMap::new()).unwrap_err();
    assert_eq!(result.hook.key, "fail".to_string());
}

#[test]
fn conditional() {
    let hooks = vec![
        Hook {
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("true".to_string()),
            optional: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("false".to_string()),
            optional: None,
        },
        Hook {
            key: "3".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
        },
    ];

    let results = hook::run_hooks(&hooks, ".", HashMap::new(), &HashMap::new()).unwrap();

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
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("{{ good_var }}".to_string()),
            optional: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("{{ bad_var }}".to_string()),
            optional: None,
        },
    ];

    assert!(hook::run_hooks(
        &hooks,
        ".",
        HashMap::from([("good_var".to_string(), "true".to_string())]),
        &HashMap::new()
    )
    .is_err());
}

#[test]
fn bad_conditional_value() {
    let hooks = vec![Hook {
        key: "1".to_string(),
        command: vec!["echo".to_string(), "hello world".to_string()],
        r#if: Some("lorem ipsum".to_string()),
        optional: None,
    }];

    assert!(hook::run_hooks(
        &hooks,
        ".",
        HashMap::from([("".to_string(), "".to_string())]),
        &HashMap::new()
    )
    .is_err());
}

#[test]
fn optional() {
    let hooks = vec![
        Hook {
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: Some(HookConfigOptional { default: false }),
        },
    ];

    let results = hook::run_hooks(&hooks, ".", HashMap::new(), &HashMap::new()).unwrap();

    assert!(
        results
            .iter()
            .filter(|r| {
                match r {
                    HookResult::Skipped(hook) => hook.key == "2".to_string(),
                    _ => false,
                }
            })
            .count()
            == 1
    );

    assert!(
        results
            .iter()
            .filter(|r| {
                match r {
                    HookResult::Completed { hook, .. } => hook.key == "1".to_string(),
                    _ => false,
                }
            })
            .count()
            == 1
    );
}
