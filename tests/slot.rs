use std::collections::HashMap;

use spackle::core::slot::{validate_data, Slot, SlotType};

#[test]
fn empty() {
    let slots = vec![];

    let data = HashMap::new();

    assert!(validate_data(&data, &slots).is_ok());
}

#[test]
fn valid() {
    let slots = vec![
        Slot {
            key: "key".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
        },
        Slot {
            key: "key2".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
        },
    ];

    let data = HashMap::from([("key", "value"), ("key2", "value2")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate_data(&data, &slots).is_ok());
}

#[test]
fn missing_data() {
    let slots = vec![
        Slot {
            key: "key".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
        },
        Slot {
            key: "key2".to_string(),
            r#type: SlotType::String,
            name: None,
            description: None,
        },
    ];

    let data = HashMap::from([("key", "value")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate_data(&data, &slots).is_err());
}

#[test]
fn extra_data() {
    let slots = vec![Slot {
        key: "key".to_string(),
        r#type: SlotType::String,
        name: None,
        description: None,
    }];

    let data = HashMap::from([("key", "value"), ("key2", "value2")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate_data(&data, &slots).is_err());
}

#[test]
fn non_string_type() {
    let slots = vec![
        Slot {
            key: "key".to_string(),
            r#type: SlotType::Number,
            name: None,
            description: None,
        },
        Slot {
            key: "key2".to_string(),
            r#type: SlotType::Boolean,
            name: None,
            description: None,
        },
    ];

    let data = HashMap::from([("key", "3.14"), ("key2", "true")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate_data(&data, &slots).is_ok());
}

#[test]
fn wrong_type() {
    let slots = vec![Slot {
        key: "key".to_string(),
        r#type: SlotType::Number,
        name: None,
        description: None,
    }];

    let data = HashMap::from([("key", "value")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate_data(&data, &slots).is_err());
}
