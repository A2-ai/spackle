use spackle::core::{
    config::{Hook, HookConfigOptional},
    hook::{self, Error, HookResult},
};
use std::collections::HashMap;

#[test]
fn basic() {
    let hooks = vec![Hook {
        key: "hello world".to_string(),
        command: vec!["echo".to_string(), "hello world".to_string()],
        r#if: None,
        optional: None,
        name: None,
        description: None,
    }];

    assert!(hook::run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None).is_ok());
}

#[test]
fn command_fail() {
    let hooks = vec![
        Hook {
            key: "hello world".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "error".to_string(),
            command: vec!["false".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
    ];

    let result = hook::run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None)
        .expect("run_hooks failed, should have succeeded");

    assert_eq!(result.len(), 2, "Expected 2 results, got {:?}", result);

    assert!(matches!(result[0], HookResult::Completed { .. }));
}

#[test]
fn error_executing() {
    let hooks = vec![
        Hook {
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["invalid_cmd".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
    ];

    let results = hook::run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None)
        .expect("run_hooks failed, should have succeeded");

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "1")));

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Failed { hook, .. } if hook.key == "2")));
}

#[test]
fn conditional() {
    let hooks = vec![
        Hook {
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("true".to_string()),
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("false".to_string()),
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "3".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "4".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("{{ hook_ran_1 }}".to_string()),
            optional: None,
            name: None,
            description: None,
        },
    ];

    let results = hook::run_hooks(&hooks, ".", &HashMap::new(), &HashMap::new(), None)
        .expect("run_hooks failed, should have succeeded");

    let skipped_hooks: Vec<_> = results
        .iter()
        .filter(|r| matches!(r, HookResult::Skipped { .. }))
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
            name: None,
            description: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: Some("{{ bad_var }}".to_string()),
            optional: None,
            name: None,
            description: None,
        },
    ];

    let results = hook::run_hooks(
        &hooks,
        ".",
        &HashMap::from([("good_var".to_string(), "true".to_string())]),
        &HashMap::new(),
        None,
    )
    .expect("run_hooks failed, should have succeeded");

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "1")));

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Failed { hook, .. } if hook.key == "2")));
}

#[test]
fn bad_conditional_value() {
    let hooks = vec![Hook {
        key: "1".to_string(),
        command: vec!["echo".to_string(), "hello world".to_string()],
        r#if: Some("lorem ipsum".to_string()),
        optional: None,
        name: None,
        description: None,
    }];

    let results = hook::run_hooks(
        &hooks,
        ".",
        &HashMap::from([("".to_string(), "".to_string())]),
        &HashMap::new(),
        None,
    )
    .expect("run_hooks failed, should have succeeded");

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Failed { hook, .. } if hook.key == "1")));
}

#[test]
fn optional() {
    let hooks = vec![
        Hook {
            key: "1".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: Some(HookConfigOptional { default: false }),
            name: None,
            description: None,
        },
        Hook {
            key: "3".to_string(),
            command: vec!["echo".to_string(), "hello world".to_string()],
            r#if: None,
            optional: Some(HookConfigOptional { default: false }),
            name: None,
            description: None,
        },
    ];

    let results = hook::run_hooks(
        &hooks,
        ".",
        &HashMap::new(),
        &HashMap::from([("3".to_string(), true)]),
        None,
    )
    .expect("run_hooks failed, should have succeeded");

    assert_eq!(
        results.len(),
        3,
        "Expected 3 results, got {:?}",
        results.len()
    );

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "1")));

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Skipped { hook, .. } if hook.key == "2")));

    assert!(results
        .iter()
        .any(|x| matches!(x, HookResult::Completed { hook, .. } if hook.key == "3")));
}

#[test]
fn templated_cmd() {
    let hooks = vec![
        Hook {
            key: "1".to_string(),
            command: vec!["{{ field_1 }}".to_string(), "{{ field_2 }}".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
        Hook {
            key: "2".to_string(),
            command: vec!["echo".to_string(), "out2".to_string()],
            r#if: None,
            optional: None,
            name: None,
            description: None,
        },
    ];

    let results = hook::run_hooks(
        &hooks,
        ".",
        &HashMap::from([
            ("field_1".to_string(), "echo".to_string()),
            ("field_2".to_string(), "out1".to_string()),
        ]),
        &HashMap::new(),
        None,
    )
    .expect("run_hooks failed, should have succeeded");

    assert_eq!(
        if let HookResult::Completed { stdout, .. } = &results[0] {
            stdout
        } else {
            panic!("Expected HookResult::Completed, got {:?}", results[0]);
        },
        "out1\n"
    );
}

#[test]
fn invalid_templated_cmd() {
    let hooks = vec![Hook {
        key: "1".to_string(),
        command: vec!["{{ field_1 }}".to_string(), "{{ field_2 }}".to_string()],
        r#if: None,
        optional: None,
        name: None,
        description: None,
    }];

    let results = hook::run_hooks(
        &hooks,
        ".",
        &HashMap::from([("field_1".to_string(), "echo".to_string())]),
        &HashMap::new(),
        None,
    )
    .expect_err("run_hooks succeeded, should have failed");

    match results {
        Error::ErrorRenderingTemplate(_, _) => {}
        _ => panic!("Expected Error::ErrorRenderingTemplate, got {:?}", results),
    }
}
