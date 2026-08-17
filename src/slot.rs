use colored::Colorize;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, fmt::Display};
use tera::{Context, Value};

use crate::needs::{is_satisfied, Needy};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Slot {
    pub key: String,
    #[serde(default)]
    pub r#type: SlotType,
    #[serde(default)]
    pub needs: Vec<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub default: Option<String>,
}

#[derive(Serialize, Deserialize, Debug, strum_macros::Display, Default, Clone)]
pub enum SlotType {
    Number,
    #[default]
    String,
    Boolean,
}

impl Default for Slot {
    fn default() -> Self {
        Self {
            key: "".to_string(),
            r#type: SlotType::String,
            needs: vec![],
            name: None,
            description: None,
            default: None,
        }
    }
}

impl Display for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {}{}",
            self.key.bold(),
            ("[".to_owned() + &self.r#type.to_string() + "]")
                .to_string()
                .to_lowercase()
                .truecolor(128, 128, 128),
            self.description
                .clone()
                .map(|s| format!("\n{}", s))
                .unwrap_or_default()
                .truecolor(180, 180, 180),
        )
    }
}

impl Needy for Slot {
    fn key(&self) -> String {
        self.key.clone()
    }

    fn is_enabled(&self, data: &HashMap<String, String>) -> bool {
        let binding = String::new();
        let value = data.get(&self.key).unwrap_or(&binding);

        !value.is_empty() && value != "0" && value.to_lowercase() != "false"
    }

    fn is_satisfied(&self, items: &Vec<&dyn Needy>, data: &HashMap<String, String>) -> bool {
        is_satisfied(&self.needs, items, data)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unknown slot: {0}")]
    UnknownSlot(String),
    #[error("type mismatch for key {0}: expected a {1}")]
    TypeMismatch(String, String),
    #[error("slot was not defined: {0}")]
    UndefinedSlot(String),
}

impl Slot {
    pub fn get_name(&self) -> String {
        self.name.clone().unwrap_or(self.key.clone())
    }
}

/// One slot value, converted to the type its slot declares.
///
/// Slot data is carried as strings everywhere, because it comes from CLI
/// flags, TOML defaults and JSON payloads. Tera has real types, so a value
/// left as text behaves wrongly: a non-empty string is truthy, which makes
/// `{% if flag %}` true even when `flag` is `false`, and a string cannot be
/// compared with a number, so `{% if n > 2 %}` is a type error.
///
/// A value that doesn't parse as its declared type is left as text rather
/// than rejected here. [`validate_data`] reports that case with a clearer
/// message, and a render that skipped validation should still produce output.
pub fn typed_value(raw: &str, slot_type: &SlotType) -> Value {
    match slot_type {
        SlotType::String => Value::from(raw),
        SlotType::Boolean => raw
            .parse::<bool>()
            .map(Value::from)
            .unwrap_or_else(|_| Value::from(raw)),
        // Integers first: `5` as an f64 renders as `5.0`, which would change
        // what an existing template prints for a whole number.
        SlotType::Number => raw
            .parse::<i64>()
            .map(Value::from)
            .or_else(|_| raw.parse::<f64>().map(Value::from))
            .unwrap_or_else(|_| Value::from(raw)),
    }
}

/// Build the Tera context for a render, giving every value the type its slot
/// declares. See [`typed_value`] for why.
///
/// Keys with no matching slot stay strings: the `_project_name` /
/// `_output_name` specials, and any extra key the caller passed.
pub fn context_from_data(data: &HashMap<String, String>, slots: &[Slot]) -> Context {
    let types: HashMap<&str, &SlotType> = slots
        .iter()
        .map(|slot| (slot.key.as_str(), &slot.r#type))
        .collect();

    let mut context = Context::new();
    for (key, raw) in data {
        let value = match types.get(key.as_str()) {
            Some(slot_type) => typed_value(raw, slot_type),
            None => Value::from(raw.as_str()),
        };
        context.insert_value(key.clone(), value);
    }
    context
}

/// Stand-in context for the static checks, which run before any value is
/// supplied.
///
/// Each slot gets its declared type's zero value rather than an empty string.
/// Against `""`, a check of `{% if count > 2 %}` reports a comparison error
/// for a template that renders fine once a value exists.
pub fn placeholder_context(slots: &[Slot]) -> Context {
    let mut context = Context::new();
    for slot in slots {
        let value = match slot.r#type {
            SlotType::String => Value::from(""),
            SlotType::Boolean => Value::from(false),
            SlotType::Number => Value::from(0_i64),
        };
        context.insert_value(slot.key.clone(), value);
    }
    context
}

pub fn validate(slots: &Vec<Slot>) -> Result<(), Error> {
    for slot in slots {
        if let Some(default_value) = &slot.default {
            match slot.r#type {
                SlotType::String => {
                    // String always valid, no need to check
                }
                SlotType::Number => {
                    if default_value.parse::<f64>().is_err() {
                        return Err(Error::TypeMismatch(slot.key.clone(), "number".to_string()));
                    }
                }
                SlotType::Boolean => {
                    if default_value.parse::<bool>().is_err() {
                        return Err(Error::TypeMismatch(slot.key.clone(), "boolean".to_string()));
                    }
                }
            }
        }
    }

    Ok(())
}

pub fn validate_data(data: &HashMap<String, String>, slots: &Vec<Slot>) -> Result<(), Error> {
    for entry in data.iter() {
        // Check if the data is assigned to a slot
        let slot = match slots.iter().find(|slot| slot.key == *entry.0) {
            Some(slot) => slot,
            None => {
                return Err(Error::UnknownSlot(entry.0.clone()));
            }
        };

        // Verify the data type by trying to parse it as the slot type
        if !match slot.r#type {
            SlotType::String => entry.1.parse::<String>().is_ok(),
            SlotType::Number => entry.1.parse::<f64>().is_ok(),
            SlotType::Boolean => entry.1.parse::<bool>().is_ok(),
        } {
            return Err(Error::TypeMismatch(
                entry.0.clone(),
                slot.r#type.to_string(),
            ));
        }
    }

    // Ensure all slots are assigned data
    for slot in slots.iter() {
        if !data.iter().any(|data| *data.0 == slot.key) {
            return Err(Error::UndefinedSlot(slot.key.clone()));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

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
                ..Default::default()
            },
            Slot {
                key: "key2".to_string(),
                ..Default::default()
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
                ..Default::default()
            },
            Slot {
                key: "key2".to_string(),
                ..Default::default()
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
            ..Default::default()
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
                ..Default::default()
            },
            Slot {
                key: "key2".to_string(),
                r#type: SlotType::Boolean,
                ..Default::default()
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
            ..Default::default()
        }];

        let data = HashMap::from([("key", "value")])
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<String, String>>();

        assert!(validate_data(&data, &slots).is_err());
    }

    fn slot_of(key: &str, slot_type: SlotType) -> Slot {
        Slot {
            key: key.to_string(),
            r#type: slot_type,
            ..Default::default()
        }
    }

    fn data_of(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    /// Render `body` the way a real render does, so these assert against
    /// Tera's own semantics.
    fn render_with(body: &str, data: &[(&str, &str)], slots: &[Slot]) -> String {
        let context = context_from_data(&data_of(data), slots);
        tera::Tera::one_off(body, &context, false).expect("render should succeed")
    }

    #[test]
    fn typed_value_table() {
        let cases: Vec<(&str, SlotType, &str)> = vec![
            ("false", SlotType::Boolean, "false"),
            ("true", SlotType::Boolean, "true"),
            ("5", SlotType::Number, "5"),
            ("-3", SlotType::Number, "-3"),
            ("3.14", SlotType::Number, "3.14"),
            ("hello", SlotType::String, "hello"),
            // Values that don't parse stay text; `validate_data` reports
            // them, with a message that names the slot.
            ("yes", SlotType::Boolean, "yes"),
            ("many", SlotType::Number, "many"),
        ];
        for (raw, slot_type, expected) in cases {
            let value = typed_value(raw, &slot_type);
            assert_eq!(
                value.to_string(),
                expected,
                "typed_value({:?}, {:?})",
                raw,
                slot_type
            );
        }
    }

    #[test]
    fn typed_value_keeps_whole_numbers_whole() {
        // An f64 round-trip would print `5.0` and change existing output.
        assert_eq!(typed_value("5", &SlotType::Number).to_string(), "5");
        assert_eq!(typed_value("5.5", &SlotType::Number).to_string(), "5.5");
    }

    #[test]
    fn boolean_conditionals_follow_the_value() {
        let slots = vec![slot_of("flag", SlotType::Boolean)];
        let body = "{% if flag %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("flag", "true")], &slots), "YES");
        // As text, "false" is non-empty and therefore truthy, so this
        // used to render YES.
        assert_eq!(render_with(body, &[("flag", "false")], &slots), "NO");
    }

    #[test]
    fn boolean_compares_against_a_bare_literal() {
        let slots = vec![slot_of("flag", SlotType::Boolean)];
        let body = "{% if flag == true %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("flag", "true")], &slots), "YES");
        assert_eq!(render_with(body, &[("flag", "false")], &slots), "NO");
    }

    #[test]
    fn boolean_negation_follows_the_value() {
        let slots = vec![slot_of("flag", SlotType::Boolean)];
        let body = "{% if not flag %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("flag", "false")], &slots), "YES");
        assert_eq!(render_with(body, &[("flag", "true")], &slots), "NO");
    }

    #[test]
    fn number_comparisons_work() {
        let slots = vec![slot_of("count", SlotType::Number)];
        let body = "{% if count > 2 %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("count", "5")], &slots), "YES");
        assert_eq!(render_with(body, &[("count", "1")], &slots), "NO");
    }

    #[test]
    fn zero_is_falsy_like_a_number_rather_than_truthy_like_text() {
        let slots = vec![slot_of("count", SlotType::Number)];
        let body = "{% if count %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("count", "0")], &slots), "NO");
        assert_eq!(render_with(body, &[("count", "1")], &slots), "YES");
    }

    #[test]
    fn interpolation_is_unchanged() {
        let slots = vec![
            slot_of("flag", SlotType::Boolean),
            slot_of("count", SlotType::Number),
            slot_of("name", SlotType::String),
        ];
        let out = render_with(
            "{{ flag }}|{{ count }}|{{ name }}",
            &[("flag", "false"), ("count", "5"), ("name", "spackle")],
            &slots,
        );
        assert_eq!(out, "false|5|spackle");
    }

    #[test]
    fn string_slots_are_left_alone() {
        // A String slot holding "false" is text, and non-empty text is
        // truthy. Conversion must not change that.
        let slots = vec![slot_of("label", SlotType::String)];
        let body = "{% if label %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("label", "false")], &slots), "YES");
        assert_eq!(render_with(body, &[("label", "")], &slots), "NO");
    }

    #[test]
    fn undeclared_keys_stay_text() {
        // `_project_name` / `_output_name` and any extra key a caller
        // passes have no declared type to restore.
        let out = render_with("{{ _output_name }}", &[("_output_name", "0")], &[]);
        assert_eq!(out, "0");
    }

    #[test]
    fn the_text_comparison_workaround_stops_matching() {
        // Templates written before this change compared against the text.
        // A `Boolean` slot is no longer text, so use `{% if flag %}`.
        let slots = vec![slot_of("flag", SlotType::Boolean)];
        let body = r#"{% if flag == "true" %}YES{% else %}NO{% endif %}"#;

        assert_eq!(render_with(body, &[("flag", "true")], &slots), "NO");
    }

    #[test]
    fn the_int_filter_workaround_still_works() {
        // The other workaround still works: `int` on a number is a no-op.
        let slots = vec![slot_of("count", SlotType::Number)];
        let body = "{% if count | int > 2 %}YES{% else %}NO{% endif %}";

        assert_eq!(render_with(body, &[("count", "5")], &slots), "YES");
        assert_eq!(render_with(body, &[("count", "1")], &slots), "NO");
    }

    #[test]
    fn a_value_that_does_not_parse_still_renders() {
        let slots = vec![slot_of("flag", SlotType::Boolean)];
        assert_eq!(render_with("{{ flag }}", &[("flag", "yes")], &slots), "yes");
    }
}
