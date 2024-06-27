use spackle::core::{config::Hook, hook};
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

    let err = hook::run_hooks(hooks, ".", HashMap::new()).unwrap_err();

    let hook_err = match err {
        hook::Error::ErrorExecuting(hook, _) => hook,
        _ => panic!("expected ErrorExecuting, got {:?}", err),
    };

    assert_eq!(hook_err.name, "fail".to_string());
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

    let result = hook::run_hooks(hooks, ".", HashMap::new()).unwrap();

    assert_eq!(result.len(), 1);
    assert_eq!(result[0].name, "2".to_string());
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
