use std::collections::HashMap;

use spackle::core::{
    config::{Slot, SlotType},
    validate,
};

#[test]
fn empty() {
    let slots = vec![];

    let data = HashMap::new();

    assert!(validate::validate(&data, slots).is_ok());
}

#[test]
fn valid() {
    let slots = vec![
        Slot {
            key: "key".to_string(),
            r#type: SlotType::String,
            name: "name".to_string(),
            description: "description".to_string(),
        },
        Slot {
            key: "key2".to_string(),
            r#type: SlotType::String,
            name: "name2".to_string(),
            description: "description2".to_string(),
        },
    ];

    let data = HashMap::from([("key", "value"), ("key2", "value2")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate::validate(&data, slots).is_ok());
}

#[test]
fn missing_data() {
    let slots = vec![
        Slot {
            key: "key".to_string(),
            r#type: SlotType::String,
            name: "name".to_string(),
            description: "description".to_string(),
        },
        Slot {
            key: "key2".to_string(),
            r#type: SlotType::String,
            name: "name2".to_string(),
            description: "description2".to_string(),
        },
    ];

    let data = HashMap::from([("key", "value")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate::validate(&data, slots).is_err());
}

#[test]
fn extra_data() {
    let slots = vec![Slot {
        key: "key".to_string(),
        r#type: SlotType::String,
        name: "name".to_string(),
        description: "description".to_string(),
    }];

    let data = HashMap::from([("key", "value"), ("key2", "value2")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate::validate(&data, slots).is_err());
}

#[test]
fn non_string_type() {
    let slots = vec![
        Slot {
            key: "key".to_string(),
            r#type: SlotType::Number,
            name: "name".to_string(),
            description: "description".to_string(),
        },
        Slot {
            key: "key2".to_string(),
            r#type: SlotType::Boolean,
            name: "name".to_string(),
            description: "description".to_string(),
        },
    ];

    let data = HashMap::from([("key", "3.14"), ("key2", "true")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate::validate(&data, slots).is_ok());
}

#[test]
fn wrong_type() {
    let slots = vec![Slot {
        key: "key".to_string(),
        r#type: SlotType::Number,
        name: "name".to_string(),
        description: "description".to_string(),
    }];

    let data = HashMap::from([("key", "value")])
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect::<HashMap<String, String>>();

    assert!(validate::validate(&data, slots).is_err());
}
