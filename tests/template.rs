use std::{collections::HashMap, env::temp_dir, path::PathBuf};

use spackle::core::template;

#[test]
fn test_fill_proj1() {
    let dir = temp_dir();

    let result = template::fill(
        &PathBuf::from("tests/data/proj1"),
        HashMap::from([
            ("person_name".to_string(), "Joe Bloggs".to_string()),
            ("person_age".to_string(), "42".to_string()),
            ("file_name".to_string(), "main".to_string()),
        ]),
        &dir,
    );

    println!("{:?}", result);

    assert!(result.is_ok());
}
